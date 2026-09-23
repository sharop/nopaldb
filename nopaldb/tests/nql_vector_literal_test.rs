// tests/nql_vector_literal_test.rs
//
// R7 (0.6.9): el vector de la pregunta escrito dentro de la consulta —
// `similar_to(c, vector = [...], model = "…")` y
// `hybrid(c, text = "…", vector = [...], model = "…")` — y "buscar +
// expandir en una sola consulta": la búsqueda siembra un patrón de un salto.
//
// Dominio ficticio: Chunk -[:MENTIONS]-> Entity.

use nopaldb::index::IndexType;
use nopaldb::types::{Edge, Node, PropertyValue};
use nopaldb::Graph;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

/// 6 Chunk con vectores unitarios en R³ (ranking claro por coseno respecto a
/// [1,0,0]: c0, c1, c2, c3, c4/c5), 3 Entity, cada chunk menciona una entidad,
/// un nodo de OTRA etiqueta con el mejor vector posible, un nodo de
/// referencia "q" con [1,0,0] para las formas posicionales, y un índice
/// full-text sobre Chunk.text para `hybrid`.
async fn fixture() -> Graph {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await.unwrap();
    let chunks: Vec<(&str, &str, [f32; 3])> = vec![
        ("c0", "riego por goteo en nopal", [1.0, 0.0, 0.0]),
        ("c1", "riego y drenaje", [0.95, 0.31, 0.0]),
        ("c2", "plagas del nopal", [0.80, 0.60, 0.0]),
        ("c3", "cosecha de tuna", [0.50, 0.87, 0.0]),
        ("c4", "suelo arenoso", [0.0, 1.0, 0.0]),
        ("c5", "temporada de lluvias", [0.0, 0.0, 1.0]),
    ];
    let mut entity_ids = vec![];
    for name in ["planta", "riego", "suelo"] {
        let e = Node::new("Entity").with_property("name", s(name));
        entity_ids.push(graph.add_node(e).await.unwrap());
    }
    for (i, (name, text, vec)) in chunks.into_iter().enumerate() {
        let c = Node::new("Chunk").with_property("name", s(name)).with_property("text", s(text));
        let id = graph.add_node(c).await.unwrap();
        graph.add_node_embedding(id, vec.to_vec(), "minilm").await.unwrap();
        graph.add_edge(Edge::new(id, entity_ids[i % 3], "MENTIONS")).await.unwrap();
    }
    let other = Node::new("Other").with_property("name", s("x0")).with_property("text", s("riego riego"));
    let ox = graph.add_node(other).await.unwrap();
    graph.add_node_embedding(ox, vec![1.0, 0.0, 0.0], "minilm").await.unwrap();
    let refn = Node::new("Ref").with_property("name", s("q"));
    let rid = graph.add_node(refn).await.unwrap();
    graph.add_node_embedding(rid, vec![1.0, 0.0, 0.0], "minilm").await.unwrap();
    graph.create_index("Chunk", "text", IndexType::FullText).await.unwrap();
    graph
}

async fn names(g: &Graph, q: &str, col: &str) -> Vec<String> {
    g.execute_nql(q)
        .await
        .unwrap_or_else(|e| panic!("{q}\n  {e}"))
        .rows()
        .iter()
        .filter_map(|r| r.get(col).and_then(|v| v.as_str().map(String::from)))
        .collect()
}

async fn explain(g: &Graph, q: &str) -> String {
    match g.execute_statement(&format!("explain {q}")).await.unwrap() {
        nopaldb::NqlResult::Explain(plan) => plan,
        other => panic!("expected an explain result, got {other:?}"),
    }
}

#[tokio::test]
async fn vector_literal_equals_the_reference_node_form_and_rows_come_best_first() {
    let g = fixture().await;
    let by_ref = names(&g, r#"find c.name from (c:Chunk) where similar_to(c, "q", "minilm") limit 3"#, "c.name").await;
    let by_vec = names(
        &g,
        r#"find c.name from (c:Chunk) where similar_to(c, vector = [1.0, 0.0, 0.0], model = "minilm") limit 3"#,
        "c.name",
    )
    .await;
    assert_eq!(by_vec, vec!["c0", "c1", "c2"], "closest first, no ORDER BY needed");
    assert_eq!(by_ref, by_vec, "the literal and the reference node give the same answer");
    // ORDER BY sigue mandando cuando se pide.
    let ordered = names(
        &g,
        r#"find c.name from (c:Chunk) where similar_to(c, vector = [1, 0, 0], model = "minilm") order by c.name desc limit 3"#,
        "c.name",
    )
    .await;
    assert_eq!(ordered, vec!["c2", "c1", "c0"]);
}

#[tokio::test]
async fn integers_and_exponents_are_valid_vector_components() {
    let g = fixture().await;
    // `[1, 0, 0]` llega como Int y se coerciona; `1e-05` es como Python
    // serializa los componentes chicos de un embedding.
    let rows = names(
        &g,
        r#"find c.name from (c:Chunk) where similar_to(c, vector = [1, 1e-05, -2.5E-3], model = "minilm") limit 1"#,
        "c.name",
    )
    .await;
    assert_eq!(rows, vec!["c0"]);
    // El exponente también vale fuera de un vector.
    let r = g.execute_nql("find c.name from (c:Chunk) where c.name = \"c0\" and 1e2 = 100.0").await.unwrap();
    assert_eq!(r.rows().len(), 1);
}

#[tokio::test]
async fn wrong_dimension_is_a_named_error() {
    let g = fixture().await;
    let err = g
        .execute_nql(r#"find c.name from (c:Chunk) where similar_to(c, vector = [1.0, 0.0], model = "minilm")"#)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("dimension"), "{err}");
}

#[tokio::test]
async fn malformed_calls_are_errors_not_every_row() {
    let g = fixture().await;
    let cases = [
        (r#"find c.name from (c:Chunk) where similar_to(c, vector = [], model = "minilm")"#, "option `vector`"),
        (r#"find c.name from (c:Chunk) where similar_to(c, vector = [1, "x", 0], model = "minilm")"#, "option `vector`"),
        (r#"find c.name from (c:Chunk) where similar_to(c, vector = [1, 0, 0])"#, "needs `model"),
        (r#"find c.name from (c:Chunk) where similar_to(c, model = "minilm")"#, "needs `vector"),
        (r#"find c.name from (c:Chunk) where similar_to(c)"#, "give a reference node name or `vector"),
        (r#"find c.name from (c:Chunk) where similar_to(c, "q", "minilm", vector = [1, 0, 0])"#, "not both"),
        (r#"find c.name from (c:Chunk) where similar_to(c, vector = [1, 0, 0], model = "minilm", k = 0)"#, "option `k`"),
        (r#"find c.name from (c:Chunk) where similar_to(c, vector = [1, 0, 0], model = "minilm", rrf_k = 3)"#, "unknown option `rrf_k`"),
        (r#"find c.name from (c:Chunk) where similar_to(c, "q", "minilm", "extra")"#, "takes 2 or 3 positional"),
        (r#"find c.name from (c:Chunk) where similar_to("q", "minilm")"#, "first argument must be the pattern variable"),
        (r#"find c.name from (c:Chunk) where hybrid(c, vector = [1, 0, 0], model = "minilm", k = 2, rrf = 1)"#, "unknown option `rrf`"),
        (r#"find c.name from (c:Chunk) where hybrid(c, k = 2)"#, "needs `text"),
        (r#"find c.name from (c:Chunk) where hybrid(c, "riego", "q")"#, "exactly 4 positional"),
        (r#"find count(x = 1) from (c:Chunk)"#, "only similar_to(...) and hybrid(...) take named options"),
        // Formas de consulta que el executor no sabe sembrar: error, no todo el grafo.
        (r#"find e.name from (c:Chunk)-[:MENTIONS]->(e:Entity) where similar_to(e, vector = [1, 0, 0], model = "minilm")"#, "must be the first node"),
        (r#"find e.name from (c:Chunk)-[:MENTIONS]->(e:Entity)-[:MENTIONS]->(f:Entity) where similar_to(c, vector = [1, 0, 0], model = "minilm")"#, "single hop"),
        (r#"find e.name from (c:Chunk)-[:MENTIONS]->{1,2}(e:Entity) where similar_to(c, vector = [1, 0, 0], model = "minilm")"#, "quantifiers"),
        (r#"find e.name from (c:Chunk), (e:Entity) where similar_to(c, vector = [1, 0, 0], model = "minilm")"#, "several patterns"),
    ];
    for (nql, needle) in cases {
        let err = g.execute_nql(nql).await.unwrap_err().to_string();
        assert!(err.contains(needle), "{nql}\n  got: {err}\n  want: {needle}");
    }
}

#[tokio::test]
async fn explicit_k_wins_over_limit_and_limit_still_caps_rows() {
    let g = fixture().await;
    let q = |tail: &str| {
        format!(r#"find c.name from (c:Chunk) where similar_to(c, vector = [1, 0, 0], model = "minilm"{tail}"#)
    };
    assert_eq!(names(&g, &q(", k = 2) limit 5"), "c.name").await, vec!["c0", "c1"]);
    assert_eq!(names(&g, &q(") limit 2"), "c.name").await, vec!["c0", "c1"]);
    assert_eq!(names(&g, &q(", k = 3) limit 1"), "c.name").await, vec!["c0"]);
    assert_eq!(names(&g, &q(")"), "c.name").await.len(), 6, "default k = 10 covers the 6 chunks");
    // `k` también vale en la forma clásica.
    assert_eq!(names(&g, r#"find c.name from (c:Chunk) where similar_to(c, "q", "minilm", k = 1)"#, "c.name").await, vec!["c0"]);
}

#[tokio::test]
async fn the_label_is_honoured_without_losing_rows() {
    let g = fixture().await;
    // x0 (Other) tiene el mejor vector posible; antes el k-NN lo traía, el
    // stream lo tiraba y `limit 1` devolvía 0 filas.
    let rows = names(&g, r#"find c.name from (c:Chunk) where similar_to(c, vector = [1, 0, 0], model = "minilm") limit 1"#, "c.name").await;
    assert_eq!(rows, vec!["c0"]);
    // El resto del WHERE se sigue aplicando sobre los candidatos.
    let rows = names(
        &g,
        r#"find c.name from (c:Chunk) where similar_to(c, vector = [1, 0, 0], model = "minilm", k = 3) and c.name != "c1""#,
        "c.name",
    )
    .await;
    assert_eq!(rows, vec!["c0", "c2"]);
    // El property map inline del patrón también filtra los candidatos.
    let rows = names(
        &g,
        r#"find c.name from (c:Chunk {name: "c1"}) where similar_to(c, vector = [1, 0, 0], model = "minilm", k = 3)"#,
        "c.name",
    )
    .await;
    assert_eq!(rows, vec!["c1"]);
}

#[tokio::test]
async fn search_and_expand_in_one_query_touches_only_the_top_k() {
    let g = fixture().await;
    // Antes de 0.6.9 esta consulta devolvía TODAS las aristas: el patrón
    // nunca precomputaba `similar_to` y el predicado pasaba como `true`.
    let q = r#"find c.name, e.name from (c:Chunk)-[:MENTIONS]->(e:Entity) where similar_to(c, vector = [1, 0, 0], model = "minilm", k = 2)"#;
    let r = g.execute_nql(q).await.unwrap();
    let pairs: Vec<(String, String)> = r
        .rows()
        .iter()
        .map(|row| {
            (
                row.get("c.name").and_then(|v| v.as_str()).unwrap().to_string(),
                row.get("e.name").and_then(|v| v.as_str()).unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(pairs, vec![("c0".into(), "planta".into()), ("c1".into(), "riego".into())], "top-2 chunks, in rank order");
    let plan = explain(&g, q).await;
    assert!(plan.contains("PATTERN PIPELINE (seed: SIMILAR_TO)"), "{plan}");
    assert!(plan.contains("SimilarTo: similar_to(c): vector=<literal dim=3> model=\"minilm\" k=2 filter.label=Chunk"), "{plan}");

    // LIMIT acota filas expandidas; k sigue siendo el de la llamada.
    let q = r#"find e.name from (c:Chunk)-[:MENTIONS]->(e:Entity) where similar_to(c, vector = [1, 0, 0], model = "minilm", k = 3) limit 2"#;
    assert_eq!(names(&g, q, "e.name").await, vec!["planta", "riego"]);

    // La forma clásica también siembra el patrón, y el WHERE restante se aplica.
    let q = r#"find e.name from (c:Chunk)-[:MENTIONS]->(e:Entity) where similar_to(c, "q", "minilm", k = 3) and e.name != "riego""#;
    assert_eq!(names(&g, q, "e.name").await, vec!["planta", "suelo"]);

    // Y `hybrid` igual.
    let q = r#"find e.name from (c:Chunk)-[:MENTIONS]->(e:Entity) where hybrid(c, text = "riego", vector = [1, 0, 0], model = "minilm", k = 1)"#;
    assert_eq!(names(&g, q, "e.name").await, vec!["planta"]);
    assert!(explain(&g, q).await.contains("PATTERN PIPELINE (seed: HYBRID)"));
}

#[tokio::test]
async fn hybrid_named_form_equals_positional_and_text_only_works() {
    let g = fixture().await;
    let positional = names(&g, r#"find c.name from (c:Chunk) where hybrid(c, "riego", "q", "minilm") limit 3"#, "c.name").await;
    let named = names(
        &g,
        r#"find c.name from (c:Chunk) where hybrid(c, text = "riego", vector = [1, 0, 0], model = "minilm", k = 3)"#,
        "c.name",
    )
    .await;
    assert_eq!(positional, named);
    assert_eq!(named[0], "c0", "in both branches: {named:?}");
    // Solo texto: sin vector ni modelo.
    let text_only = names(&g, r#"find c.name from (c:Chunk) where hybrid(c, text = "riego", k = 5)"#, "c.name").await;
    assert_eq!(text_only.len(), 2, "c0 y c1 mencionan riego: {text_only:?}");
    // Solo vector: la forma nombrada sin `text`.
    let vec_only = names(&g, r#"find c.name from (c:Chunk) where hybrid(c, vector = [0, 0, 1], model = "minilm", k = 1)"#, "c.name").await;
    assert_eq!(vec_only, vec!["c5"]);
    let plan = explain(&g, r#"find c.name from (c:Chunk) where hybrid(c, text = "riego", vector = [1, 0, 0], model = "minilm", rrf_k = 30) limit 2"#).await;
    assert!(plan.contains("VECTOR SEARCH (hybrid)"), "{plan}");
    assert!(plan.contains("Hybrid: hybrid(c): text=\"riego\" vector=<literal dim=3> model=\"minilm\" k=2 rrf_k=30"), "{plan}");
}

#[tokio::test]
async fn explain_names_the_vector_search_on_single_node_queries() {
    let g = fixture().await;
    let plan = explain(&g, r#"find c.name from (c:Chunk) where similar_to(c, "q", "minilm") limit 3"#).await;
    assert!(plan.contains("VECTOR SEARCH (similar_to)"), "{plan}");
    assert!(plan.contains("SimilarTo: similar_to(c): ref=\"q\" model=\"minilm\" k=3 filter.label=Chunk"), "{plan}");
}
