// Explicación del retrieval híbrido: que los números que reporta sean los
// que el motor realmente usó, y que pedirlos no cambie el resultado.
//
// El score RRF ordena pero no explica: es una suma de recíprocos de RANGOS,
// así que dos documentos con el mismo score pueden haber llegado por caminos
// distintos. Lo que se verifica aquí es que la traza no sea decorativa —
// que sus rangos coincidan con consultar cada rama por separado, y que sus
// scores crudos sean los de tantivy y no una reconstrucción.

use nopaldb::index::IndexType;
use nopaldb::types::{Node, NodeId, PropertyValue};
use nopaldb::{Graph, HybridFilter, HybridQuery, VectorPath};

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

/// Corpus de biblioteca: textos que se solapan a propósito para que las dos
/// ramas discrepen y la fusión tenga algo que fusionar.
async fn fixture() -> (Graph, tempfile::TempDir, Vec<(String, NodeId)>) {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await.unwrap();

    let docs = [
        ("nopal", "cactus nopal tunas", [1.0f32, 0.0, 0.0]),
        ("agave", "agave maguey cactus", [0.9, 0.1, 0.0]),
        ("pino", "pino conifera bosque", [0.0, 1.0, 0.0]),
        ("helecho", "helecho sombra humedad", [0.0, 0.0, 1.0]),
    ];
    let mut ids = Vec::new();
    for (name, body, vec) in docs {
        let node = Node::new("Doc")
            .with_property("name", s(name))
            .with_property("body", s(body));
        let id = graph.add_node(node).await.unwrap();
        graph.add_node_embedding(id, vec.to_vec(), "m").await.unwrap();
        ids.push((name.to_string(), id));
    }
    graph.create_index("Doc", "body", IndexType::FullText).await.unwrap();
    (graph, dir, ids)
}

fn query_ambas() -> HybridQuery {
    let mut q = HybridQuery::new();
    q.text = Some("cactus".into());
    q.vector = Some((vec![1.0, 0.0, 0.0], "m".into()));
    q.k = 4;
    q
}

/// Pedir la explicación no puede cambiar lo que se devuelve.
#[tokio::test]
async fn explain_does_not_alter_the_result() {
    let (graph, _dir, _ids) = fixture().await;

    let plain = graph.search_hybrid(query_ambas()).await.unwrap();
    let explained = graph.search_hybrid_explain(query_ambas()).await.unwrap();

    assert_eq!(plain.len(), explained.hits.len());
    for (p, e) in plain.iter().zip(&explained.hits) {
        assert_eq!(p.node_id, e.node_id, "mismo orden");
        assert_eq!(p.score, e.score, "mismo score RRF");
        assert_eq!(p.text_rank, e.text_rank);
        assert_eq!(p.vector_rank, e.vector_rank);
    }
}

/// Los rangos de la traza son los de cada rama consultada por separado.
///
/// Es la prueba de que la explicación describe el cómputo real y no una
/// reconstrucción plausible: se corre cada rama SOLA y se comparan posiciones.
#[tokio::test]
async fn ranks_match_each_branch_queried_alone() {
    let (graph, _dir, _ids) = fixture().await;

    // Rama de texto sola.
    let mut solo_texto = HybridQuery::new();
    solo_texto.text = Some("cactus".into());
    solo_texto.k = 10;
    let texto: Vec<NodeId> = graph
        .search_hybrid(solo_texto)
        .await
        .unwrap()
        .into_iter()
        .map(|h| h.node_id)
        .collect();

    // Rama vectorial sola.
    let mut solo_vector = HybridQuery::new();
    solo_vector.vector = Some((vec![1.0, 0.0, 0.0], "m".into()));
    solo_vector.k = 10;
    let vector: Vec<NodeId> = graph
        .search_hybrid(solo_vector)
        .await
        .unwrap()
        .into_iter()
        .map(|h| h.node_id)
        .collect();

    let explain = graph.search_hybrid_explain(query_ambas()).await.unwrap();

    for hit in &explain.hits {
        if let Some(rank) = hit.text_rank {
            assert_eq!(
                texto.get(rank),
                Some(&hit.node_id),
                "text_rank={rank} no coincide con la rama de texto sola"
            );
        }
        if let Some(rank) = hit.vector_rank {
            assert_eq!(
                vector.get(rank),
                Some(&hit.node_id),
                "vector_rank={rank} no coincide con la rama vectorial sola"
            );
        }
    }
}

/// Los números crudos existen y son coherentes: BM25 solo donde hubo texto,
/// distancia solo donde hubo vector, y la distancia ordena como el rango.
#[tokio::test]
async fn raw_scores_accompany_the_branch_that_produced_them() {
    let (graph, _dir, ids) = fixture().await;
    let explain = graph.search_hybrid_explain(query_ambas()).await.unwrap();

    let mut vistos_texto = 0;
    let mut por_vector: Vec<(usize, f32)> = Vec::new();
    for hit in &explain.hits {
        assert_eq!(
            hit.text_score.is_some(),
            hit.text_rank.is_some(),
            "el score BM25 debe acompañar al rango de texto, y solo a él"
        );
        assert_eq!(
            hit.vector_distance.is_some(),
            hit.vector_rank.is_some(),
            "la distancia debe acompañar al rango vectorial, y solo a él"
        );
        if let Some(sc) = hit.text_score {
            assert!(sc > 0.0, "BM25 de una coincidencia real es > 0");
            vistos_texto += 1;
        }
        if let (Some(r), Some(d)) = (hit.vector_rank, hit.vector_distance) {
            por_vector.push((r, d));
        }
    }
    assert!(vistos_texto >= 2, "«cactus» aparece en dos documentos del corpus");

    // El nodo cuyo vector ES la query debe estar a distancia ~0.
    let nopal = ids.iter().find(|(n, _)| n == "nopal").unwrap().1;
    let hit = explain.hits.iter().find(|h| h.node_id == nopal).unwrap();
    assert!(
        hit.vector_distance.unwrap() < 1e-4,
        "el vector idéntico a la query debe dar distancia ~0, dio {:?}",
        hit.vector_distance
    );

    // Rango vectorial y distancia deben contar la misma historia.
    por_vector.sort_by_key(|(r, _)| *r);
    assert!(
        por_vector.windows(2).all(|w| w[0].1 <= w[1].1),
        "un rango vectorial mejor debe tener distancia menor o igual: {por_vector:?}"
    );
}

/// La configuración efectiva reporta lo que se USÓ, no lo que se pidió: el
/// `ef_search` por defecto y el índice full-text resuelto no los eligió el
/// llamador, y sin esto no hay forma de saber cuáles fueron.
#[tokio::test]
async fn effective_configuration_is_reported() {
    let (graph, _dir, _ids) = fixture().await;

    let mut q = query_ambas();
    q.k = 3;
    q.rrf_k = 42.0;
    q.overfetch = 5;
    let explain = graph.search_hybrid_explain(q).await.unwrap();

    assert_eq!(explain.k, 3);
    assert_eq!(explain.rrf_k, 42.0);
    assert_eq!(explain.overfetch, 5);
    assert_eq!(explain.candidates, 15, "k × overfetch");
    assert_eq!(
        explain.text_index.as_deref(),
        Some("Doc_body"),
        "el índice resuelto se nombra aunque el llamador no lo eligiera"
    );
    assert!(explain.ef_search.is_some(), "el ef_search por defecto se reporta");
    assert_eq!(explain.allowed_set_size, None, "sin filtro no hay conjunto permitido");
}

/// Con filtro se reporta su cardinalidad, y el camino vectorial dice si el
/// resultado fue exacto o aproximado — que es lo que decide cómo leer un
/// resultado corto.
#[tokio::test]
async fn filter_and_vector_path_are_visible() {
    let (graph, _dir, _ids) = fixture().await;

    let mut q = query_ambas();
    q.filter = Some(HybridFilter {
        label: Some("Doc".into()),
        props: vec![],
    });
    let explain = graph.search_hybrid_explain(q).await.unwrap();

    assert_eq!(explain.allowed_set_size, Some(4), "los 4 nodos Doc");
    // Índice chico ⇒ el camino filtrado del índice es exacto de por sí.
    assert_eq!(explain.vector_path, Some(VectorPath::HnswFiltered));

    let sin_filtro = graph.search_hybrid_explain(query_ambas()).await.unwrap();
    assert_eq!(sin_filtro.vector_path, Some(VectorPath::Unfiltered));
}

/// Underfill: pedir más de lo que existe se reporta como tal en cada rama,
/// en vez de devolver una lista corta sin explicación.
#[tokio::test]
async fn underfill_is_reported_per_branch() {
    let (graph, _dir, _ids) = fixture().await;

    let mut q = HybridQuery::new();
    q.text = Some("cactus".into()); // solo 2 documentos lo tienen
    q.k = 10;
    q.overfetch = 4; // pide 40 candidatos
    let explain = graph.search_hybrid_explain(q).await.unwrap();

    let texto = explain.text.expect("hubo rama de texto");
    assert_eq!(texto.requested, 40);
    assert_eq!(texto.returned, 2, "solo dos documentos dicen «cactus»");
    assert!(texto.underfilled(), "menos de lo pedido debe marcarse");
    assert!(explain.vector.is_none(), "no hubo rama vectorial");
    assert!(explain.vector_path.is_none());
}

/// Una rama que entrega todo lo pedido no se marca como underfill.
#[tokio::test]
async fn a_full_branch_is_not_flagged() {
    let (graph, _dir, _ids) = fixture().await;

    let mut q = HybridQuery::new();
    q.vector = Some((vec![1.0, 0.0, 0.0], "m".into()));
    q.k = 2;
    q.overfetch = 1; // pide 2, y hay 4 nodos con embedding
    let explain = graph.search_hybrid_explain(q).await.unwrap();

    let vector = explain.vector.expect("hubo rama vectorial");
    assert_eq!(vector.requested, 2);
    assert_eq!(vector.returned, 2);
    assert!(!vector.underfilled());
}
