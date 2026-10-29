// tests/nql_algorithm_rows_test.rs
//
// A per-node algorithm gives one row per node (#336). Until 0.10.2 a
// single-node query with only algorithms and properties went through the
// aggregation path: `find degree(n)` gave one row for the whole graph, and
// `find degree(n) as d, n.fruta` grouped by `n.fruta`, merging trees with
// the same fruit.
//
// Fictional domain: an orchard. Two mango trees and a lime tree; one mango
// is next to the lime.

use nopaldb::types::{Edge, Node, PropertyValue};
use nopaldb::Graph;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

async fn orchard() -> Graph {
    let g = Graph::in_memory().await.unwrap();
    let mango = g.add_node(Node::new("Arbol").with_property("fruta", s("mango"))).await.unwrap();
    g.add_node(Node::new("Arbol").with_property("fruta", s("mango"))).await.unwrap();
    let lima = g.add_node(Node::new("Arbol").with_property("fruta", s("lima"))).await.unwrap();
    g.add_edge(Edge::new(mango, lima, "JUNTO_A")).await.unwrap();
    g
}

async fn rows(g: &Graph, q: &str) -> Vec<nopaldb::query::nql::Row> {
    g.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}")).rows().to_vec()
}

fn degrees(rows: &[nopaldb::query::nql::Row], key: &str) -> Vec<f64> {
    let mut d: Vec<f64> = rows
        .iter()
        .map(|r| match r.get(key) {
            Some(PropertyValue::Float(x)) => *x,
            Some(PropertyValue::Int(x)) => *x as f64,
            other => panic!("{key}: {other:?}"),
        })
        .collect();
    d.sort_by(f64::total_cmp);
    d
}

#[tokio::test]
async fn an_algorithm_alone_gives_one_row_per_node() {
    let g = orchard().await;
    for (q, key) in [("find degree(n) from (n:Arbol)", "degree"), ("find degree(n) as d from (n:Arbol)", "d")] {
        let r = rows(&g, q).await;
        assert_eq!(r.len(), 3, "{q}");
        assert_eq!(degrees(&r, key), vec![0.0, 1.0, 1.0], "{q}");
    }
}

#[tokio::test]
async fn nodes_with_the_same_property_are_not_merged() {
    let g = orchard().await;
    let r = rows(&g, "find degree(n) as d, n.fruta from (n:Arbol)").await;
    assert_eq!(r.len(), 3);
    let mangos: Vec<_> = r.iter().filter(|row| row.get("n.fruta") == Some(&s("mango"))).collect();
    assert_eq!(mangos.len(), 2);
    let r = rows(&g, "find n.fruta, pagerank(n) as pr from (n:Arbol)").await;
    assert_eq!(r.len(), 3);
}

#[tokio::test]
async fn with_a_real_aggregation_rows_are_still_grouped() {
    let g = orchard().await;
    let r = rows(&g, "find n.fruta, count(*) as c from (n:Arbol) group by n.fruta").await;
    assert_eq!(r.len(), 2);
    let r = rows(&g, "find count(*) as c from (n:Arbol)").await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].get("c"), Some(&PropertyValue::Int(3)));
}

/// The average that `find degree(n)` gave until 0.10.2 is asked for
/// explicitly now.
#[tokio::test]
async fn the_average_is_an_explicit_aggregation() {
    let g = orchard().await;
    let r = rows(&g, "find avg(degree(n)) as m, max(degree(n)) as top, sum(degree(n)) as s from (n:Arbol)").await;
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].get("m"), Some(&PropertyValue::Float(2.0 / 3.0)));
    assert_eq!(r[0].get("top"), Some(&PropertyValue::Float(1.0)));
    assert_eq!(r[0].get("s"), Some(&PropertyValue::Float(2.0)));
    let r = rows(&g, "find n.fruta, avg(degree(n)) as m from (n:Arbol) group by n.fruta").await;
    assert_eq!(r.len(), 2);
    let err = g.execute_nql("find avg(community(n)) as m from (n:Arbol)").await.unwrap_err().to_string();
    assert!(err.contains("community id is a label"), "{err}");
}

#[tokio::test]
async fn where_on_an_algorithm_filters_per_node() {
    let g = orchard().await;
    let r = rows(&g, "find n.fruta, degree(n) as d from (n:Arbol) where degree(n) > 0").await;
    assert_eq!(r.len(), 2, "{:?}", r.iter().map(|x| &x.values).collect::<Vec<_>>());
}
