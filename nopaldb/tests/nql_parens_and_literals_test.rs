// tests/nql_parens_and_literals_test.rs
//
// #255: parentheses in NQL expressions, and the operators the AST builder
// used to drop without a word.
//
// Until 0.8.2 `where (a or b)` failed with "Invalid primary expression", and
// worse: the builder kept the first operand of `+ - * / %` and dropped the
// operator, and the `-` of a negative number lost its sign. So
// `where m.temp = -5` matched `temp = 5` and `set m.temp = -7` wrote 7.
// Now parentheses group and negative numbers are literals. Arithmetic, `!`
// and `not` were a parse error from 0.8.3 to 0.10.4 and are evaluated since
// #269 (see also nql_arithmetic_test).
//
// Fictional domain: recipes and temperature readings.

use std::collections::BTreeSet;

use nopaldb::types::{Node, PropertyValue};
use nopaldb::Graph;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

async fn recipes() -> Graph {
    let graph = Graph::in_memory().await.unwrap();
    for (key, portions, region) in [("a", 3, "norte"), ("b", 1, "sur"), ("c", 5, "centro"), ("d", 4, "sur")] {
        graph
            .add_node(
                Node::new("Receta")
                    .with_property("key", s(key))
                    .with_property("porciones", PropertyValue::Int(portions))
                    .with_property("region", s(region)),
            )
            .await
            .unwrap();
    }
    graph
}

async fn readings() -> Graph {
    let graph = Graph::in_memory().await.unwrap();
    for (key, temp) in [("a", PropertyValue::Int(-5)), ("b", PropertyValue::Int(5)), ("c", PropertyValue::Int(0)), ("f", PropertyValue::Float(-2.5))] {
        graph
            .add_node(Node::new("Medida").with_property("key", s(key)).with_property("temp", temp))
            .await
            .unwrap();
    }
    graph
}

async fn keys(graph: &Graph, var: &str, q: &str) -> BTreeSet<String> {
    let result = graph.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    result
        .rows()
        .iter()
        .map(|r| match r.get(&format!("{var}.key")) {
            Some(PropertyValue::String(k)) => k.clone(),
            other => panic!("{q}: key {other:?}"),
        })
        .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|x| x.to_string()).collect()
}

async fn parse_error(graph: &Graph, q: &str) -> String {
    match graph.execute_nql(q).await {
        Ok(_) => panic!("{q} debía fallar"),
        Err(e) => e.to_string(),
    }
}

#[tokio::test]
async fn the_issue_examples_return_rows() {
    let g = recipes().await;
    assert_eq!(
        keys(&g, "r", r#"find r.key from (r:Receta) where r.porciones > 1 and (r.region = "norte")"#).await,
        set(&["a"])
    );
    assert_eq!(
        keys(&g, "r", r#"find r.key from (r:Receta) where (r.region = "norte" or r.region = "sur")"#).await,
        set(&["a", "b", "d"])
    );
}

#[tokio::test]
async fn parentheses_change_the_precedence() {
    let g = recipes().await;
    // `and` binds tighter than `or`: a and b or c = (a and b) or c.
    let flat = r#"find r.key from (r:Receta) where r.porciones > 2 and r.region = "norte" or r.region = "sur""#;
    let grouped = r#"find r.key from (r:Receta) where r.porciones > 2 and (r.region = "norte" or r.region = "sur")"#;
    assert_eq!(keys(&g, "r", flat).await, set(&["a", "b", "d"]));
    assert_eq!(keys(&g, "r", grouped).await, set(&["a", "d"]));
    // Nested and redundant parentheses.
    assert_eq!(
        keys(&g, "r", r#"find r.key from (r:Receta) where ((r.region = "sur") and (r.porciones > 2 or (r.porciones = 1)))"#).await,
        set(&["b", "d"])
    );
    assert_eq!(keys(&g, "r", r#"find r.key from (r:Receta) where (r.key = "c")"#).await, set(&["c"]));
}

#[tokio::test]
async fn negative_numbers_keep_their_sign() {
    let g = readings().await;
    assert_eq!(keys(&g, "m", "find m.key from (m:Medida) where m.temp = -5").await, set(&["a"]));
    assert_eq!(keys(&g, "m", "find m.key from (m:Medida) where m.temp < -1").await, set(&["a", "f"]));
    assert_eq!(keys(&g, "m", "find m.key from (m:Medida) where m.temp = -2.5").await, set(&["f"]));
    assert_eq!(keys(&g, "m", "find m.key from (m:Medida) where m.temp in [-5, 5]").await, set(&["a", "b"]));
    assert_eq!(keys(&g, "m", "find m.key from (m:Medida) where (m.temp > -6 and m.temp < 0)").await, set(&["a", "f"]));
}

#[tokio::test]
async fn negative_numbers_in_writes() {
    let g = readings().await;
    g.execute_statement(r#"update (m:Medida) set m.temp = -7 where m.key = "c""#).await.unwrap();
    assert_eq!(keys(&g, "m", "find m.key from (m:Medida) where m.temp = -7").await, set(&["c"]));
    assert!(keys(&g, "m", "find m.key from (m:Medida) where m.temp = 7").await.is_empty(), "no 7: el signo se guarda");

    // `delete … where x = -5` borra el -5, no el 5.
    g.execute_statement("delete (m:Medida) where m.temp = -5").await.unwrap();
    assert_eq!(keys(&g, "m", "find m.key from (m:Medida)").await, set(&["b", "c", "f"]));
}

/// From 0.8.3 to 0.10.4 these were a parse error that named them; since
/// #269 they are evaluated, and the answer is checked here by hand.
#[tokio::test]
async fn arithmetic_and_negation_are_evaluated() {
    let g = recipes().await;
    for (q, expected) in [
        ("find r.key from (r:Receta) where r.porciones * 2 = 6", &["a"][..]),
        ("find r.key from (r:Receta) where r.porciones + 10 > 4", &["a", "b", "c", "d"][..]),
        ("find r.key from (r:Receta) where r.porciones - 1 = 2", &["a"][..]),
        ("find r.key from (r:Receta) where r.porciones % 2 = 1", &["a", "b", "c"][..]),
        ("find r.key from (r:Receta) where -r.porciones < -2", &["a", "c", "d"][..]),
        ("find r.key from (r:Receta) where !(r.porciones = 3)", &["b", "c", "d"][..]),
        ("find r.key from (r:Receta) where not (r.porciones = 3)", &["b", "c", "d"][..]),
    ] {
        assert_eq!(keys(&g, "r", q).await, set(expected), "{q}");
    }
    let r = g.execute_nql("find r.key, r.porciones * 2 from (r:Receta)").await.unwrap();
    assert_eq!(r.columns, ["r.key", "r.porciones * 2"]);
    g.execute_nql(r#"update (r:Receta) set r.porciones = r.porciones + 1 where r.key = "a""#).await.unwrap();
    assert_eq!(keys(&g, "r", "find r.key from (r:Receta) where r.porciones = 4").await, set(&["a", "d"]));
}

/// `hybrid(c, …) and (c.p = 1 or c.p = 2)`: the search ranks inside the
/// label (an OR on the searched variable is not pushed into the universe) and
/// the OR filters the rows. A search under the parenthesised OR, or under
/// `!`, is rejected.
#[cfg(feature = "hybrid")]
#[tokio::test]
async fn hybrid_and_a_parenthesised_or() {
    use nopaldb::index::IndexType;
    let graph = Graph::in_memory().await.unwrap();
    let n = 30usize;
    for i in 0..n {
        let id = graph
            .add_node(
                Node::new("Chunk")
                    .with_property("key", s(&format!("c{i}")))
                    .with_property("p", PropertyValue::Int((i % 3) as i64))
                    .with_property("text", s("nopal en la ladera")),
            )
            .await
            .unwrap();
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        graph.add_node_embedding(id, vec![a.cos(), a.sin(), 0.1, 0.1], "m").await.unwrap();
    }
    graph.create_index("Chunk", "text", IndexType::FullText).await.unwrap();

    let search = r#"hybrid(c, text = "nopal", vector = [1.0, 0.0, 0.1, 0.1], model = "m", k = 9)"#;
    let unfiltered = graph
        .execute_nql(&format!("find c.key, c.p from (c:Chunk) where {search}"))
        .await
        .unwrap();
    let top: Vec<(String, i64)> = unfiltered
        .rows()
        .iter()
        .map(|r| match (r.get("c.key"), r.get("c.p")) {
            (Some(PropertyValue::String(k)), Some(PropertyValue::Int(p))) => (k.clone(), *p),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(top.len(), 9);
    let expected: BTreeSet<String> = top.iter().filter(|(_, p)| *p == 1 || *p == 2).map(|(k, _)| k.clone()).collect();
    assert!(!expected.is_empty() && expected.len() < 9);

    let got = keys(&graph, "c", &format!("find c.key from (c:Chunk) where {search} and (c.p = 1 or c.p = 2)")).await;
    assert_eq!(got, expected, "top-k de la etiqueta, y el OR sobre esas filas");

    for q in [
        format!("find c.key from (c:Chunk) where ({search} or c.p = 1)"),
        format!("find c.key from (c:Chunk) where !({search})"),
    ] {
        let err = parse_error(&graph, &q).await;
        assert!(!err.is_empty(), "{q}");
    }
}
