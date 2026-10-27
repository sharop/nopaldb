// tests/nql_pattern_binding_test.rs
//
// Which node each pattern variable names (#293). Two bugs that returned
// wrong rows without an error until 0.9.3:
// - an anonymous endpoint took the name the pattern gives to the other
//   endpoint: in `()-[:F]->(u)`, `find u.name` returned the sources;
// - a repeated variable was re-bound instead of constraining the row:
//   `(a)-[:F]->(b)-[:F]->(a)` returned paths that do not close.
//
// Fictional domain: seed packets that are traded between gardeners.

use std::collections::BTreeSet;

use nopaldb::types::{Edge, Node, PropertyValue};
use nopaldb::Graph;

async fn names(graph: &Graph, q: &str, cols: &[&str]) -> BTreeSet<Vec<String>> {
    let result = graph.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    result
        .rows()
        .iter()
        .map(|r| {
            cols.iter()
                .map(|c| match r.get(c) {
                    Some(PropertyValue::String(x)) => x.clone(),
                    other => format!("{other:?}"),
                })
                .collect()
        })
        .collect()
}

fn set(rows: &[&[&str]]) -> BTreeSet<Vec<String>> {
    rows.iter().map(|r| r.iter().map(|x| x.to_string()).collect()).collect()
}

/// ana -> beto, carla -> dora, dora -> ana; eva -> eva (a self-loop);
/// ana -> fede -> ana (the only 2-cycle).
async fn garden() -> Graph {
    let g = Graph::in_memory().await.unwrap();
    let mut ids = std::collections::HashMap::new();
    for name in ["ana", "beto", "carla", "dora", "eva", "fede"] {
        let id = g
            .add_node(Node::new("Gardener").with_property("name", PropertyValue::String(name.into())))
            .await
            .unwrap();
        ids.insert(name, id);
    }
    for (a, b) in [("ana", "beto"), ("carla", "dora"), ("dora", "ana"), ("eva", "eva"), ("ana", "fede"), ("fede", "ana")] {
        g.add_edge(Edge::new(ids[a], ids[b], "TRADES")).await.unwrap();
    }
    g
}

#[tokio::test]
async fn an_anonymous_source_never_takes_the_target_name() {
    let g = garden().await;
    let targets = set(&[&["beto"], &["dora"], &["ana"], &["eva"], &["fede"]]);
    for q in [
        "find u.name from ()-[:TRADES]->(u:Gardener)",
        "find u.name from (:Gardener)-[:TRADES]->(u:Gardener)",
        "find u.name from (x)-[:TRADES]->(u:Gardener)",
    ] {
        assert_eq!(names(&g, q, &["u.name"]).await, targets, "{q}");
    }
}

#[tokio::test]
async fn an_anonymous_target_never_takes_the_source_name() {
    let g = garden().await;
    let sources = set(&[&["ana"], &["carla"], &["dora"], &["eva"], &["fede"]]);
    for q in [
        "find a.name from (a:Gardener)-[:TRADES]->()",
        "find a.name from (a:Gardener)-[:TRADES]->(:Gardener)",
    ] {
        assert_eq!(names(&g, q, &["a.name"]).await, sources, "{q}");
    }
}

#[tokio::test]
async fn aggregating_over_an_anonymous_source_counts_the_targets() {
    let g = garden().await;
    // The "most followed" query of the Python quickstart: ana receives two
    // trades (from dora and fede), everyone else one.
    let q = "find u.name, count(*) as n from ()-[:TRADES]->(u:Gardener) group by u.name";
    let result = g.execute_nql(q).await.unwrap();
    let mut counts: Vec<(String, i64)> = result
        .rows()
        .iter()
        .map(|r| {
            let name = match r.get("u.name") {
                Some(PropertyValue::String(x)) => x.clone(),
                other => panic!("{other:?}"),
            };
            let n = match r.get("n") {
                Some(PropertyValue::Int(n)) => *n,
                Some(PropertyValue::Float(f)) => *f as i64,
                other => panic!("{other:?}"),
            };
            (name, n)
        })
        .collect();
    counts.sort();
    assert_eq!(
        counts,
        vec![("ana".into(), 2), ("beto".into(), 1), ("dora".into(), 1), ("eva".into(), 1), ("fede".into(), 1)]
    );
}

#[tokio::test]
async fn a_repeated_variable_closes_the_cycle() {
    let g = garden().await;
    // Only ana <-> fede is a 2-cycle (eva's self-loop also closes: eva, eva).
    let q = "find a.name, b.name from (a:Gardener)-[:TRADES]->(b:Gardener)-[:TRADES]->(a:Gardener)";
    assert_eq!(
        names(&g, q, &["a.name", "b.name"]).await,
        set(&[&["ana", "fede"], &["fede", "ana"], &["eva", "eva"]])
    );
}

#[tokio::test]
async fn the_same_variable_on_both_ends_of_one_hop_is_a_self_loop() {
    let g = garden().await;
    let q = "find a.name from (a:Gardener)-[:TRADES]->(a:Gardener)";
    assert_eq!(names(&g, q, &["a.name"]).await, set(&[&["eva"]]));
}
