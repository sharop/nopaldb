// tests/nql_projection_columns_test.rs
//
// A row has exactly the projected columns (#331). Until 0.10.2:
// - an aliased property kept its original key too (`find n.name as j` gave
//   `n.name` and `j` in the row);
// - `group by` and per-node algorithms added `n.id` to the columns, so
//   Python and `export` showed a column nobody asked for;
// - `find all(n)` on a single-node query gave empty rows.
//
// Fictional domain: an orchard and its trees.

use std::collections::BTreeSet;

use nopaldb::types::{Edge, Node, PropertyValue};
use nopaldb::Graph;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

async fn orchard() -> Graph {
    let g = Graph::in_memory().await.unwrap();
    let mango = g
        .add_node(Node::new("Arbol").with_property("fruta", s("mango")).with_property("edad", PropertyValue::Int(12)))
        .await
        .unwrap();
    let lima = g
        .add_node(Node::new("Arbol").with_property("fruta", s("lima")).with_property("edad", PropertyValue::Int(4)))
        .await
        .unwrap();
    g.add_edge(Edge::new(mango, lima, "JUNTO_A")).await.unwrap();
    g
}

/// The columns of the result and the keys of every row, which must match.
async fn shape(g: &Graph, q: &str) -> Vec<String> {
    let r = g.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    let columns: BTreeSet<&String> = r.columns.iter().collect();
    assert!(!r.rows().is_empty(), "{q}: no rows");
    for row in r.rows() {
        let keys: BTreeSet<&String> = row.values.keys().collect();
        assert_eq!(keys, columns, "{q}: row keys differ from the columns");
    }
    r.columns.clone()
}

fn cols(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|x| x.to_string()).collect()
}

#[tokio::test]
async fn an_alias_replaces_the_original_column() {
    let g = orchard().await;
    assert_eq!(shape(&g, "find n.fruta as f from (n:Arbol)").await, cols(&["f"]));
    assert_eq!(shape(&g, "find n.id as i from (n:Arbol)").await, cols(&["i"]));
    assert_eq!(shape(&g, "find n.fruta as f from (n:Arbol) where n.fruta = \"mango\"").await, cols(&["f"]));
    // Asked for both ways, both stay.
    assert_eq!(shape(&g, "find n.fruta as f, n.fruta from (n:Arbol)").await, cols(&["f", "n.fruta"]));
    assert_eq!(
        shape(&g, "find a.fruta as desde, b.fruta from (a:Arbol)-[r:JUNTO_A]->(b:Arbol)").await,
        cols(&["desde", "b.fruta"])
    );
}

#[tokio::test]
async fn group_by_and_algorithms_do_not_add_the_id() {
    let g = orchard().await;
    let mut c = shape(&g, "find n.fruta, count(*) as c from (n:Arbol) group by n.fruta").await;
    c.sort();
    assert_eq!(c, cols(&["c", "n.fruta"]));
    let mut c = shape(&g, "find degree(n) as d, n.fruta from (n:Arbol)").await;
    c.sort();
    assert_eq!(c, cols(&["d", "n.fruta"]));
    // Asked for, it stays.
    let mut c = shape(&g, "find n.id, degree(n) as d from (n:Arbol)").await;
    c.sort();
    assert_eq!(c, cols(&["d", "n.id"]));
}

#[tokio::test]
async fn all_of_a_variable_gives_its_properties() {
    let g = orchard().await;
    assert_eq!(
        shape(&g, "find all(n) from (n:Arbol) where n.fruta = \"mango\"").await,
        cols(&["n.id", "n.label", "n.edad", "n.fruta"])
    );
    let r = g.execute_nql("find all(n) from (n:Arbol) where n.fruta = \"mango\"").await.unwrap();
    assert_eq!(r.rows()[0].get("n.edad"), Some(&PropertyValue::Int(12)));
    assert_eq!(
        shape(&g, "find all(a), b.fruta from (a:Arbol)-[r:JUNTO_A]->(b:Arbol)").await,
        cols(&["a.id", "a.label", "a.edad", "a.fruta", "b.fruta"])
    );
}

#[tokio::test]
async fn the_wildcard_keeps_every_key() {
    let g = orchard().await;
    let r = g.execute_nql("find * from (n:Arbol) where n.fruta = \"mango\"").await.unwrap();
    assert_eq!(r.columns, cols(&["*"]));
    let keys: BTreeSet<&str> = r.rows()[0].values.keys().map(String::as_str).collect();
    assert_eq!(keys, BTreeSet::from(["n.edad", "n.fruta", "n.id", "n.label"]));
}

#[tokio::test]
async fn csv_export_has_only_the_projected_columns() {
    let g = orchard().await;
    let r = g
        .execute_nql("find n.fruta, count(*) as c from (n:Arbol) group by n.fruta export csv")
        .await
        .unwrap();
    let text = format!("{r:?}");
    assert!(!text.contains("n.id"), "{text}");
}
