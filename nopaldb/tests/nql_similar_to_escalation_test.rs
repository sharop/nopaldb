// #174b: `similar_to` con etiqueta en el patrón devuelve K filas cuando la
// etiqueta tiene K nodos con embedding, aunque otras etiquetas acaparen los
// vecinos más cercanos.
//
// Hasta 0.6.9 pedía un `4·k` fijo al índice y se quedaba con los de la
// etiqueta: con embeddings repartidos en varias etiquetas devolvía menos de K
// filas sin avisar. Ahora escala ×4 y, si la etiqueta es demasiado escasa
// entre los vecinos, busca entre los nodos de la propia etiqueta.
//
// Dominio ficticio: Chunk, Entity y Report comparten el modelo de embeddings.

use nopaldb::types::{Node, PropertyValue};
use nopaldb::Graph;

const DIM: usize = 8;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

fn query_vec() -> Vec<f32> {
    (0..DIM).map(|d| if d == 0 { 1.0 } else { 0.0 }).collect()
}

/// Vector a ángulo creciente de la consulta: `t = 0` es la consulta misma,
/// `t` mayor queda más lejos. Determinista, sin RNG.
fn at_angle(t: f32, axis: usize) -> Vec<f32> {
    (0..DIM)
        .map(|d| match d {
            0 => t.cos(),
            d if d == axis => t.sin(),
            _ => 0.0,
        })
        .collect()
}

async fn add(graph: &Graph, label: &str, name: &str, vector: Vec<f32>) {
    let id = graph.add_node(Node::new(label).with_property("name", s(name))).await.unwrap();
    graph.add_node_embedding(id, vector, "m").await.unwrap();
}

fn literal(v: &[f32]) -> String {
    v.iter().map(|x| format!("{x:?}")).collect::<Vec<_>>().join(", ")
}

async fn names(graph: &Graph, nql: &str) -> Vec<String> {
    graph
        .execute_nql(nql)
        .await
        .unwrap_or_else(|e| panic!("{nql}\n  {e}"))
        .rows()
        .iter()
        .filter_map(|r| r.get("n.name").and_then(|v| v.as_str().map(String::from)))
        .collect()
}

/// Tres etiquetas con embeddings. Los 200 nodos de Entity y Report están más
/// cerca de la consulta que cualquier Chunk: con el `4·k` fijo, `similar_to`
/// sobre Chunk devolvía 0 filas. Ahora devuelve exactamente K, y son los K
/// Chunk más cercanos (el orden de la etiqueta se conoce por construcción).
#[tokio::test]
async fn returns_k_rows_when_other_labels_crowd_the_neighbours() {
    let graph = Graph::in_memory().await.unwrap();
    for i in 0..100 {
        add(&graph, "Entity", &format!("e{i}"), at_angle(0.001 * i as f32, 1)).await;
        add(&graph, "Report", &format!("r{i}"), at_angle(0.001 * i as f32, 2)).await;
    }
    for i in 0..20 {
        // Chunk: lejos (ángulo ≥ 1 rad) y cada vez más lejos.
        add(&graph, "Chunk", &format!("c{i:02}"), at_angle(1.0 + 0.05 * i as f32, 3)).await;
    }
    let q = literal(&query_vec());
    let got = names(&graph, &format!(r#"find n.name from (n:Chunk) where similar_to(n, vector = [{q}], model = "m", k = 5)"#)).await;
    assert_eq!(got, ["c00", "c01", "c02", "c03", "c04"], "K filas, las K más cercanas de la etiqueta, en orden");

    // La etiqueta solo tiene 20: pedir 30 devuelve las 20, no menos.
    let all = names(&graph, &format!(r#"find n.name from (n:Chunk) where similar_to(n, vector = [{q}], model = "m", k = 30)"#)).await;
    assert_eq!(all.len(), 20);
}

/// Por encima del umbral exacto (1024 puntos) el índice es HNSW y la
/// escalada tiene tope (4096): con más vecinos ajenos que eso, la búsqueda
/// pasa al conjunto de la etiqueta y sigue devolviendo K.
#[tokio::test]
async fn falls_back_to_the_label_when_it_is_too_rare_among_the_neighbours() {
    let graph = Graph::in_memory().await.unwrap();
    for i in 0..4200 {
        add(&graph, "Entity", &format!("e{i}"), at_angle(0.0001 * (i % 97) as f32, 1 + i % 3)).await;
    }
    for i in 0..3 {
        add(&graph, "Chunk", &format!("c{i}"), at_angle(2.0 + 0.1 * i as f32, 5)).await;
    }
    let q = literal(&query_vec());
    let got = names(&graph, &format!(r#"find n.name from (n:Chunk) where similar_to(n, vector = [{q}], model = "m", k = 3)"#)).await;
    assert_eq!(got, ["c0", "c1", "c2"]);
}

/// `overfetch` y `ef_search` son opciones válidas de `similar_to`, se validan
/// y EXPLAIN muestra la estrategia efectiva.
#[tokio::test]
async fn overfetch_and_ef_search_options_are_validated_and_explained() {
    let graph = Graph::in_memory().await.unwrap();
    add(&graph, "Chunk", "c0", query_vec()).await;
    let q = literal(&query_vec());

    let ok = names(&graph, &format!(r#"find n.name from (n:Chunk) where similar_to(n, vector = [{q}], model = "m", k = 1, overfetch = 8, ef_search = 64)"#)).await;
    assert_eq!(ok, ["c0"]);

    let explain = graph
        .execute_nql(&format!(r#"explain find n.name from (n:Chunk) where similar_to(n, vector = [{q}], model = "m", k = 1, overfetch = 8)"#))
        .await
        .unwrap();
    let text = format!("{:?}", explain.rows());
    assert!(text.contains("fetch 8·k"), "{text}");
    assert!(text.contains("then the label's own nodes"), "{text}");
    assert!(text.contains("ef_search=default"), "{text}");

    for (bad, needle) in [
        ("overfetch = 0", "option `overfetch`"),
        ("ef_search = \"x\"", "option `ef_search`"),
        ("rrf_k = 30", "unknown option `rrf_k`"),
    ] {
        let err = graph
            .execute_nql(&format!(r#"find n.name from (n:Chunk) where similar_to(n, vector = [{q}], model = "m", {bad})"#))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains(needle), "{bad}: {err}");
    }
}
