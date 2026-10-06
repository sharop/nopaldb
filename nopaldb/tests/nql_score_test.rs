// #174c: `score(var)` en el FIND devuelve el score de la búsqueda vectorial
// del WHERE para el nodo de cada fila: similitud coseno (1 - distancia) con
// `similar_to`, score RRF con `hybrid`. Un cliente lo necesita para recortar
// el contexto por umbral o repartir un presupuesto de tokens; hasta 0.6.9 el
// orden de las filas era el ranking, pero el número se tiraba.
//
// Dominio ficticio: Chunk -[:MENTIONS]-> Entity.

use nopaldb::index::IndexType;
use nopaldb::types::{Edge, Node, NodeId, PropertyValue};
use nopaldb::{Graph, HybridFilter, HybridQuery};

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

const CHUNKS: [(&str, &str, [f32; 3]); 4] = [
    ("c0", "riego por goteo", [1.0, 0.0, 0.0]),
    ("c1", "riego y drenaje", [0.95, 0.31, 0.0]),
    ("c2", "plagas del nopal", [0.80, 0.60, 0.0]),
    ("c3", "cosecha de tuna", [0.0, 1.0, 0.0]),
];

async fn fixture() -> (Graph, Vec<NodeId>) {
    let graph = Graph::in_memory().await.unwrap();
    let ent = graph.add_node(Node::new("Entity").with_property("name", s("riego"))).await.unwrap();
    let mut ids = vec![];
    for (name, text, v) in CHUNKS {
        let id = graph.add_node(Node::new("Chunk").with_property("name", s(name)).with_property("text", s(text))).await.unwrap();
        graph.add_node_embedding(id, v.to_vec(), "m").await.unwrap();
        graph.add_edge(Edge::new(id, ent, "MENTIONS")).await.unwrap();
        ids.push(id);
    }
    graph.create_index("Chunk", "text", IndexType::FullText).await.unwrap();
    (graph, ids)
}

/// Similitud coseno calculada aparte del motor.
fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| (*x as f64) * (*y as f64)).sum();
    let na: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    dot / (na * nb)
}

fn rows(result: &nopaldb::query::nql::QueryResult, cols: &[&str]) -> Vec<Vec<PropertyValue>> {
    result
        .rows()
        .iter()
        .map(|r| cols.iter().map(|c| r.get(c).cloned().unwrap_or(PropertyValue::Null)).collect())
        .collect()
}

#[tokio::test]
async fn similar_to_score_is_the_cosine_similarity_best_first() {
    let (graph, _) = fixture().await;
    let q = r#"find c.name, score(c) from (c:Chunk) where similar_to(c, vector = [1, 0, 0], model = "m", k = 3)"#;
    let result = graph.execute_nql(q).await.unwrap();
    assert_eq!(result.columns, ["c.name", "score(c)"]);
    let got = rows(&result, &["c.name", "score(c)"]);
    assert_eq!(got.len(), 3);
    let mut last = f64::INFINITY;
    for (row, (name, _, v)) in got.iter().zip(CHUNKS) {
        assert_eq!(row[0], s(name), "mejor primero");
        let PropertyValue::Float(score) = row[1] else { panic!("score no es número: {:?}", row[1]) };
        assert!((score - cosine(&[1.0, 0.0, 0.0], &v)).abs() < 1e-4, "{name}: {score}");
        assert!(score <= last, "descendente");
        last = score;
    }

    let plan = format!("{:?}", graph.execute_nql(&format!("explain {q}")).await.unwrap().rows());
    assert!(plan.contains("Score: score(c) = cosine similarity of similar_to(c)"), "{plan}");

    // Con alias, y antes de otra columna.
    let q = r#"find score(c) as s, c.name from (c:Chunk) where similar_to(c, vector = [1, 0, 0], model = "m", k = 1)"#;
    let result = graph.execute_nql(q).await.unwrap();
    assert_eq!(result.columns, ["s", "c.name"]);
    assert_eq!(rows(&result, &["c.name"]), vec![vec![s("c0")]]);
    // `c.id` se añade solo para resolver el score: no puede quedar en la fila.
    assert!(result.rows().iter().all(|r| r.get("c.id").is_none()), "{:?}", result.rows());
}

/// En un patrón de un salto cada fila expandida lleva el score de SU chunk.
#[tokio::test]
async fn score_follows_the_searched_node_in_a_one_hop_pattern() {
    let (graph, _) = fixture().await;
    let q = r#"find c.name, e.name, score(c) from (c:Chunk)-[:MENTIONS]->(e:Entity) where similar_to(c, vector = [1, 0, 0], model = "m", k = 2)"#;
    let result = graph.execute_nql(q).await.unwrap();
    let got = rows(&result, &["c.name", "e.name", "score(c)"]);
    assert_eq!(got.len(), 2);
    for row in &got {
        let name = match &row[0] { PropertyValue::String(n) => n.clone(), _ => unreachable!() };
        let v = CHUNKS.iter().find(|(n, _, _)| *n == name).unwrap().2;
        let PropertyValue::Float(score) = row[2] else { panic!("{row:?}") };
        assert!((score - cosine(&[1.0, 0.0, 0.0], &v)).abs() < 1e-4);
        assert_eq!(row[1], s("riego"));
    }
    assert!(!result.columns.iter().any(|c| c == "c.id"), "la columna auxiliar no se filtra: {:?}", result.columns);
    assert!(result.rows().iter().all(|r| r.get("c.id").is_none()), "{:?}", result.rows());
}

/// Con `hybrid`, el score es el RRF de `search_hybrid` para el mismo nodo.
#[tokio::test]
async fn hybrid_score_is_the_rrf_score_of_search_hybrid() {
    let (graph, ids) = fixture().await;
    let q = r#"find c.name, score(c) from (c:Chunk) where hybrid(c, text = "riego", vector = [1, 0, 0], model = "m", k = 3)"#;
    let result = graph.execute_nql(q).await.unwrap();

    let mut hq = HybridQuery::new();
    hq.text = Some("riego".into());
    hq.vector = Some((vec![1.0, 0.0, 0.0], "m".into()));
    hq.k = 3;
    hq.filter = Some(HybridFilter { label: Some("Chunk".into()), props: vec![] });
    let expected = graph.search_hybrid(hq).await.unwrap();

    let plan = format!("{:?}", graph.execute_nql(&format!("explain {q}")).await.unwrap().rows());
    assert!(plan.contains("Score: score(c) = RRF score of hybrid(c)"), "{plan}");

    let got = rows(&result, &["c.name", "score(c)"]);
    assert_eq!(got.len(), expected.len());
    for (row, hit) in got.iter().zip(&expected) {
        let i = ids.iter().position(|id| *id == hit.node_id).unwrap();
        assert_eq!(row[0], s(CHUNKS[i].0), "mismo orden que search_hybrid");
        let PropertyValue::Float(score) = row[1] else { panic!("{row:?}") };
        assert!((score - hit.score as f64).abs() < 1e-6, "{score} vs {}", hit.score);
    }
}

#[tokio::test]
async fn invalid_score_usages_are_named_errors() {
    let (graph, _) = fixture().await;
    let search = r#"similar_to(c, vector = [1, 0, 0], model = "m", k = 2)"#;
    let cases = [
        (r#"find c.name, score(c) from (c:Chunk)"#.to_string(), "needs similar_to(c"),
        (format!(r#"find e.name, score(e) from (c:Chunk)-[:MENTIONS]->(e:Entity) where {search}"#), "needs similar_to(e"),
        (format!(r#"find c.name from (c:Chunk) where {search} order by score(c) desc"#), "only supported as a FIND column"),
        (format!(r#"find c.name from (c:Chunk) where {search} and score(c) > 0.5"#), "only supported as a FIND column"),
        (format!(r#"find count(*), score(c) from (c:Chunk) where {search}"#), "cannot be combined with aggregations"),
        (format!(r#"find score(c.name) from (c:Chunk) where {search}"#), "exactly one argument"),
    ];
    for (nql, needle) in cases {
        let err = graph.execute_nql(&nql).await.unwrap_err().to_string();
        assert!(err.contains(needle), "{nql}\n  got: {err}\n  want: {needle}");
    }
}
