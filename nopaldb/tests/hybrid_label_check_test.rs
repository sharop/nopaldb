// #174a: un filtro de solo etiqueta en la búsqueda híbrida se resuelve
// comprobando la etiqueta sobre los candidatos de cada rama, sin recorrer la
// etiqueta para armar el conjunto permitido.
//
// Lo que importa no es que sea rápido (eso lo mide el bench), sino que el
// resultado sea el MISMO que con el conjunto permitido: el top-k se sigue
// calculando dentro de la etiqueta (#115). La referencia es el camino del
// conjunto permitido en el mismo grafo, forzado con una propiedad que todos
// los nodos de la etiqueta tienen (`kind`). En el mismo grafo los empates de
// BM25 se rompen igual en las dos consultas; entre dos grafos, no.

use nopaldb::index::IndexType;
use nopaldb::types::{Node, PropertyValue};
use nopaldb::{Graph, HybridFilter, HybridQuery, VectorPath};

const DIM: usize = 8;
const WORDS: [&str; 6] = ["nopal", "maguey", "pino", "helecho", "cactus", "bosque"];

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

/// Vectores pseudoaleatorios deterministas, sin dependencia de RNG.
fn vector(seed: u64) -> Vec<f32> {
    let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (0..DIM)
        .map(|_| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((x >> 33) as f32 / (1u64 << 31) as f32) - 0.5
        })
        .collect()
}

fn body(i: u64) -> String {
    let a = WORDS[(i % 6) as usize];
    let b = WORDS[((i / 6) % 6) as usize];
    let c = WORDS[((i / 36) % 6) as usize];
    format!("{a} {b} {c} {a}")
}

async fn add(graph: &Graph, label: &str, i: u64, vec: Vec<f32>) {
    let node = Node::new(label)
        .with_property("name", s(&format!("{label}{i}")))
        .with_property("kind", s(label))
        .with_property("body", s(&body(i)));
    let id = graph.add_node(node).await.unwrap();
    graph.add_node_embedding(id, vec, "m").await.unwrap();
}

/// `(nodo, score, rango de texto, rango vectorial)` de cada hit, en orden.
async fn traza(graph: &Graph, q: HybridQuery) -> Vec<(nopaldb::types::NodeId, f32, Option<usize>, Option<usize>)> {
    let hits = graph.search_hybrid(q).await.unwrap();
    hits.iter().map(|h| (h.node_id, h.score, h.text_rank, h.vector_rank)).collect()
}

fn query(filter: Option<HybridFilter>) -> HybridQuery {
    let mut q = HybridQuery::new();
    q.text = Some("cactus nopal".into());
    q.vector = Some((vector(7), "m".into()));
    q.k = 10;
    q.filter = filter;
    q
}

fn solo_doc() -> Option<HybridFilter> {
    Some(HybridFilter {
        label: Some("Doc".into()),
        props: vec![],
    })
}

#[tokio::test]
async fn label_check_ranks_like_a_graph_with_only_that_label() {
    // En memoria: en disco cada escritura es un commit durable.
    let graph = Graph::in_memory().await.unwrap();

    // Doc y Other intercalados; los Other tienen vectores de la misma
    // distribución, así que compiten de verdad en la rama vectorial. La de
    // texto no los ve: el índice full-text es por etiqueta (Doc), y ahí el
    // filtro solo confirma.
    for i in 0..300u64 {
        add(&graph, "Doc", i, vector(i)).await;
        add(&graph, "Other", i, vector(10_000 + i)).await;
    }
    graph.create_index("Doc", "body", IndexType::FullText).await.unwrap();

    let explain = graph.search_hybrid_explain(query(solo_doc())).await.unwrap();
    assert_eq!(explain.vector_path, Some(VectorPath::LabelChecked));
    assert_eq!(explain.allowed_set_size, None, "sin conjunto permitido");
    assert_eq!(explain.hits.len(), 10);

    let con_conjunto = Some(HybridFilter {
        label: Some("Doc".into()),
        props: vec![("kind".into(), s("Doc"))],
    });
    let referencia = graph.search_hybrid_explain(query(con_conjunto.clone())).await.unwrap();
    assert_eq!(referencia.allowed_set_size, Some(300), "la referencia sí usa el conjunto");

    let filtrado = traza(&graph, query(solo_doc())).await;
    assert_eq!(filtrado, traza(&graph, query(con_conjunto)).await, "mismo top-k, mismo orden, mismos rangos");

    // Y sin filtro el top-k sí cambia: los Other compiten, así que la
    // igualdad de arriba no es trivial.
    let sin_filtro = traza(&graph, query(None)).await;
    assert_ne!(filtrado, sin_filtro);
}

/// Los vecinos más cercanos son todos de otra etiqueta: la rama vectorial
/// tiene que escalar hasta encontrar los Doc, y devolver los más cercanos de
/// ellos, no menos de los pedidos.
#[tokio::test]
async fn escalates_when_other_labels_crowd_the_nearest_neighbours() {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await.unwrap();
    let q_vec = vector(7);
    // 200 Other casi idénticos a la query; 20 Doc lejos y cada vez más lejos.
    for i in 0..200u64 {
        let v: Vec<f32> = q_vec.iter().map(|x| x + (i as f32) * 1e-4).collect();
        add(&graph, "Other", i, v).await;
    }
    for i in 0..20u64 {
        let v: Vec<f32> = q_vec.iter().enumerate().map(|(d, x)| if d == 0 { -x - (i as f32) } else { *x }).collect();
        add(&graph, "Doc", i, v).await;
    }

    let mut q = HybridQuery::new();
    q.vector = Some((q_vec, "m".into()));
    q.k = 5;
    q.overfetch = 1;
    q.filter = solo_doc();
    let explain = graph.search_hybrid_explain(q).await.unwrap();
    assert_eq!(explain.vector_path, Some(VectorPath::LabelChecked));
    let vector = explain.vector.unwrap();
    assert_eq!(vector.returned, 5, "escala en vez de quedarse corto");

    let ids: Vec<_> = explain.hits.iter().map(|h| h.node_id).collect();
    let mut names: Vec<String> = graph
        .get_nodes(&ids)
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.unwrap().properties.get("name").unwrap().as_str().unwrap().to_string())
        .collect();
    names.sort();
    assert_eq!(names, ["Doc0", "Doc1", "Doc2", "Doc3", "Doc4"], "los cinco Doc más cercanos");
}

/// Una etiqueta tan escasa que la escalada llega a su tope sin juntar los
/// candidatos: se vuelve al conjunto permitido, que es exacto, en vez de
/// devolver menos de lo que hay.
#[tokio::test]
async fn a_label_too_rare_among_the_neighbours_falls_back_to_the_allowed_set() {
    // En memoria: son 4200 embeddings cargados uno por uno.
    let graph = Graph::in_memory().await.unwrap();
    let q_vec = vector(7);
    // Más Other que el tope de la escalada (4096), todos más cerca de la
    // query que los 3 Doc.
    for i in 0..4200u64 {
        let v: Vec<f32> = q_vec.iter().map(|x| x + ((i % 97) as f32) * 1e-4).collect();
        let node = Node::new("Other").with_property("name", s(&format!("Other{i}")));
        let id = graph.add_node(node).await.unwrap();
        graph.add_node_embedding(id, v, "m").await.unwrap();
    }
    for i in 0..3u64 {
        let v: Vec<f32> = q_vec.iter().map(|x| -x - (i as f32)).collect();
        add(&graph, "Doc", i, v).await;
    }

    let mut q = HybridQuery::new();
    q.vector = Some((q_vec, "m".into()));
    q.k = 3;
    q.filter = solo_doc();
    let explain = graph.search_hybrid_explain(q).await.unwrap();
    assert_eq!(explain.vector_path, Some(VectorPath::ExactOverAllowed));
    assert_eq!(explain.allowed_set_size, Some(3));
    assert_eq!(explain.hits.len(), 3, "los tres Doc, aunque ningún vecino cercano lo sea");
}

/// Rama de texto con un filtro de propiedad cuyos nodos quedan al fondo del
/// ranking BM25 (#174): pide `k × overfetch` a tantivy y escala mientras el
/// filtro deje menos, así que devuelve los que existen aunque estén más
/// allá de los primeros candidatos.
#[tokio::test]
async fn text_branch_escalates_past_the_first_candidates_when_the_filter_is_selective() {
    let graph = Graph::in_memory().await.unwrap();
    // 400 documentos con "nopal" y textos cortos (BM25 alto); los 5 "raro"
    // tienen texto largo, así que quedan al final del ranking.
    for i in 0..400u64 {
        let node = Node::new("Doc").with_property("kind", s("comun")).with_property("body", s("nopal"));
        let _ = i;
        graph.add_node(node).await.unwrap();
    }
    let mut raros = Vec::new();
    for i in 0..5u64 {
        let body = format!("nopal {}", "relleno largo ".repeat(20 + i as usize));
        let id = graph
            .add_node(Node::new("Doc").with_property("kind", s("raro")).with_property("body", s(&body)))
            .await
            .unwrap();
        raros.push(id);
    }
    graph.create_index("Doc", "body", IndexType::FullText).await.unwrap();

    let mut q = HybridQuery::new();
    q.text = Some("nopal".into());
    q.k = 5;
    q.filter = Some(HybridFilter { label: Some("Doc".into()), props: vec![("kind".into(), s("raro"))] });
    let mut got: Vec<_> = graph.search_hybrid(q).await.unwrap().into_iter().map(|h| h.node_id).collect();
    got.sort();
    raros.sort();
    assert_eq!(got, raros, "los 5 raros, aunque estén detrás de 400 mejores");
}

