// tests/nql_pattern_expansion_test.rs
//
// How NQL patterns expand (#214 phase 2b, PR 1).
//
// Three bugs that returned wrong rows without an error, found while
// designing phase 2b:
// - a one-hop pattern ignored the direction: `(e)<-[:T]-(c)` walked the
//   outgoing edges of `e`, so it returned nothing, and `(c)<-[:T]-(e)` (no
//   such edges) returned rows;
// - a two-hop pattern ignored the inline properties of an edge:
//   `-[:T {w: 1}]->` also matched `w = 2`;
// - one hop with `distinct … limit N` could return fewer than N rows.
//
// And the pruning: predicates of the `where` on the expanded variables are
// applied while expanding. The rows must be exactly those of applying them
// afterwards.
//
// Fictional domain: recipe chunks that mention ingredients, ingredients
// related to others.

use std::collections::BTreeSet;

use nopaldb::types::{Edge, Node, NodeId, PropertyValue};
use nopaldb::Graph;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

async fn rows(graph: &Graph, q: &str, cols: &[&str]) -> Vec<Vec<String>> {
    let result = graph.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    result
        .rows()
        .iter()
        .map(|r| {
            cols.iter()
                .map(|c| match r.get(c) {
                    Some(PropertyValue::String(x)) => x.clone(),
                    Some(other) => format!("{other:?}"),
                    None => "∅".into(),
                })
                .collect()
        })
        .collect()
}

async fn row_set(graph: &Graph, q: &str, cols: &[&str]) -> BTreeSet<Vec<String>> {
    rows(graph, q, cols).await.into_iter().collect()
}

/// c1 -MENTIONS{w:1}-> nopal -RELATED-> tuna
/// c2 -MENTIONS{w:2}-> maguey -RELATED-> pulque
/// c1 -MENTIONS{w:2}-> maguey
async fn small() -> Graph {
    let g = Graph::in_memory().await.unwrap();
    let node = |label: &str, name: &str| Node::new(label).with_property("name", s(name));
    let c1 = g.add_node(node("Chunk", "c1")).await.unwrap();
    let c2 = g.add_node(node("Chunk", "c2")).await.unwrap();
    let nopal = g.add_node(node("Entity", "nopal")).await.unwrap();
    let maguey = g.add_node(node("Entity", "maguey")).await.unwrap();
    let tuna = g.add_node(node("Entity", "tuna")).await.unwrap();
    let pulque = g.add_node(node("Entity", "pulque")).await.unwrap();
    for (a, b, t, w) in [
        (c1, nopal, "MENTIONS", Some(1)),
        (c2, maguey, "MENTIONS", Some(2)),
        (c1, maguey, "MENTIONS", Some(2)),
        (nopal, tuna, "RELATED", None),
        (maguey, pulque, "RELATED", None),
    ] {
        let mut e = Edge::new(a, b, t);
        if let Some(w) = w {
            e = e.with_property("w", PropertyValue::Int(w));
        }
        g.add_edge(e).await.unwrap();
    }
    g
}

fn set(items: &[&[&str]]) -> BTreeSet<Vec<String>> {
    items.iter().map(|r| r.iter().map(|x| x.to_string()).collect()).collect()
}

#[tokio::test]
async fn one_hop_honours_the_direction() {
    let g = small().await;
    assert_eq!(
        row_set(&g, "find e.name, c.name from (e:Entity)<-[:MENTIONS]-(c:Chunk)", &["e.name", "c.name"]).await,
        set(&[&["nopal", "c1"], &["maguey", "c2"], &["maguey", "c1"]])
    );
    assert!(
        rows(&g, "find c.name from (c:Chunk)<-[:MENTIONS]-(e:Entity)", &["c.name"]).await.is_empty(),
        "no edge goes from an Entity to a Chunk"
    );
    // Both ways: each MENTIONS edge of maguey, whichever end it is on.
    assert_eq!(
        row_set(&g, r#"find x.name from (m:Entity {name: "maguey"})<->(x)"#, &["x.name"]).await,
        set(&[&["c1"], &["c2"], &["pulque"]])
    );
    // Outgoing stays as it was.
    assert_eq!(
        row_set(&g, "find c.name, e.name from (c:Chunk)-[:MENTIONS]->(e:Entity)", &["c.name", "e.name"]).await,
        set(&[&["c1", "nopal"], &["c2", "maguey"], &["c1", "maguey"]])
    );
}

#[tokio::test]
async fn two_hops_honour_inline_edge_properties() {
    let g = small().await;
    let q = "find c.name, f.name from (c:Chunk)-[:MENTIONS {w: 1}]->(e:Entity)-[:RELATED]->(f:Entity)";
    assert_eq!(row_set(&g, q, &["c.name", "f.name"]).await, set(&[&["c1", "tuna"]]));
    let q = "find c.name, f.name from (c:Chunk)-[:MENTIONS {w: 2}]->(e:Entity)-[:RELATED]->(f:Entity)";
    assert_eq!(row_set(&g, q, &["c.name", "f.name"]).await, set(&[&["c1", "pulque"], &["c2", "pulque"]]));
}

#[tokio::test]
async fn one_hop_distinct_then_limit() {
    let g = Graph::in_memory().await.unwrap();
    // Three chunks per ingredient: `distinct e.name` has 4 values, each
    // repeated three times in the expansion.
    let mut ents: Vec<NodeId> = Vec::new();
    for name in ["nopal", "maguey", "tuna", "pulque"] {
        ents.push(g.add_node(Node::new("Entity").with_property("name", s(name))).await.unwrap());
    }
    for i in 0..12 {
        let c = g.add_node(Node::new("Chunk").with_property("name", s(&format!("c{i}")))).await.unwrap();
        g.add_edge(Edge::new(c, ents[i / 3], "MENTIONS")).await.unwrap();
    }
    let got = rows(&g, "find distinct e.name from (c:Chunk)-[:MENTIONS]->(e:Entity) limit 3", &["e.name"]).await;
    assert_eq!(got.len(), 3, "distinct first, then the limit: {got:?}");
    let unique: BTreeSet<_> = got.iter().collect();
    assert_eq!(unique.len(), 3, "{got:?}");
}

#[tokio::test]
async fn single_node_distinct_then_limit() {
    let g = Graph::in_memory().await.unwrap();
    for i in 0..12 {
        g.add_node(Node::new("Receta").with_property("region", s(["norte", "sur", "centro", "costa"][i / 3])))
            .await
            .unwrap();
    }
    let got = rows(&g, "find distinct r.region from (r:Receta) limit 3", &["r.region"]).await;
    let unique: BTreeSet<_> = got.iter().collect();
    assert_eq!((got.len(), unique.len()), (3, 3), "{got:?}");
}

async fn mentions(g: &Graph) -> BTreeSet<Vec<String>> {
    row_set(g, "find c.name, e.name from (c:Chunk)-[:MENTIONS]->(e:Entity)", &["c.name", "e.name"]).await
}

/// DELETE / UPDATE with a pattern used only the labels and the type: a
/// reversed arrow or an inline property did not restrict what they wrote.
#[tokio::test]
async fn write_patterns_honour_direction_and_inline_properties() {
    let g = small().await;
    let before = mentions(&g).await;
    assert_eq!(before.len(), 3);

    // No edge goes from an Entity to a Chunk: nothing to delete.
    g.execute_statement("delete (c:Chunk)<-[:MENTIONS]-(e:Entity)").await.unwrap();
    assert_eq!(mentions(&g).await, before, "a reversed arrow deletes nothing");

    // Only the w = 1 edge.
    g.execute_statement("delete (c:Chunk)-[:MENTIONS {w: 1}]->(e:Entity)").await.unwrap();
    assert_eq!(mentions(&g).await, set(&[&["c2", "maguey"], &["c1", "maguey"]]));

    // Inline properties of the source node restrict an UPDATE too.
    g.execute_statement(r#"update (c:Chunk {name: "c2"})-[r:MENTIONS]->(e:Entity) set r.w = 9"#).await.unwrap();
    let w9 = row_set(&g, "find c.name from (c:Chunk)-[:MENTIONS {w: 9}]->(e:Entity)", &["c.name"]).await;
    assert_eq!(w9, set(&[&["c2"]]));

    // A longer pattern is an error, not its first hop.
    let err = g
        .execute_statement("delete (c:Chunk)-[:MENTIONS]->(e:Entity)-[:RELATED]->(f:Entity)")
        .await
        .expect_err("two hops")
        .to_string();
    assert!(err.contains("single hop"), "{err}");
    assert_eq!(mentions(&g).await.len(), 2, "nothing deleted");
}

/// EXPLAIN names the predicates applied while expanding, per variable.
#[tokio::test]
async fn explain_names_the_expansion_filters() {
    let g = small().await;
    let q = r#"explain find c.name, f.name from (c:Chunk)-[:MENTIONS]->(e:Entity)-[:RELATED]->(f:Entity) where e.name = "nopal" and (f.name = "tuna" or f.name = "x") and c.name != f.name"#;
    let plan = format!("{:?}", g.execute_nql(q).await.unwrap().rows());
    assert!(plan.contains("Expansion filters: "), "{plan}");
    assert!(plan.contains(r#"e: e.name = \"nopal\""#), "{plan}");
    assert!(plan.contains(r#"f: (f.name = \"tuna\" or f.name = \"x\")"#), "{plan}");
    // `c.name != f.name` crosses variables: not a filter of the expansion.
    assert!(!plan.contains("c: "), "{plan}");

    // Same rows as without EXPLAIN's knowledge: the filter only prunes.
    let rows = row_set(&g, &q["explain ".len()..], &["c.name", "f.name"]).await;
    assert_eq!(rows, set(&[&["c1", "tuna"]]));
}
