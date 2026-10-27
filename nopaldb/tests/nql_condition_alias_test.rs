// tests/nql_condition_alias_test.rs
//
// What a `where` / `having` condition can be, and what a projection alias
// means inside one (#294). Until 0.9.3, with no error:
// - `where p.active` (a bare property) let every node through in `find`,
//   and matched none in `delete` / `update`;
// - `find degree(n) as dg … where dg > 1` returned no rows, and
//   `where degree(n) > 1` with `degree(n) as dg` projected returned all;
// - `avg(r.w) as m … having m > 0.5` returned nothing, and
//   `having avg(r.w) > 0.5` asked for `r.w` in the GROUP BY.
//
// Fictional domain: gardeners and the seed packets they trade.

use std::collections::BTreeSet;

use nopaldb::types::{Edge, Node, PropertyValue};
use nopaldb::Graph;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

/// ana -> beto (w 0.9), beto -> carla (0.3), carla -> dora (0.3),
/// beto -> dora (0.3), dora -> eva (0.3). Degrees: ana 1, beto 3, carla 2,
/// dora 3, eva 1. Only beto is active; carla has no `active` property.
async fn garden() -> Graph {
    let g = Graph::in_memory().await.unwrap();
    let mut ids = Vec::new();
    for (name, active) in [("ana", Some(false)), ("beto", Some(true)), ("carla", None), ("dora", Some(false)), ("eva", Some(false))] {
        let mut node = Node::new("Gardener").with_property("name", s(name));
        if let Some(a) = active {
            node = node.with_property("active", PropertyValue::Bool(a));
        }
        ids.push(g.add_node(node).await.unwrap());
    }
    for (a, b, w) in [(0, 1, 0.9), (1, 2, 0.3), (2, 3, 0.3), (1, 3, 0.3), (3, 4, 0.3)] {
        g.add_edge(Edge::new(ids[a], ids[b], "TRADES").with_property("w", PropertyValue::Float(w)))
            .await
            .unwrap();
    }
    g
}

async fn names(g: &Graph, q: &str, col: &str) -> BTreeSet<String> {
    let result = g.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    result
        .rows()
        .iter()
        .map(|r| match r.get(col) {
            Some(PropertyValue::String(x)) => x.clone(),
            other => panic!("{q}: {col} = {other:?}"),
        })
        .collect()
}

fn set(xs: &[&str]) -> BTreeSet<String> {
    xs.iter().map(|x| x.to_string()).collect()
}

async fn error_of(g: &Graph, q: &str) -> String {
    match g.execute_nql(q).await {
        Ok(r) => panic!("{q} should fail, got {} rows", r.rows().len()),
        Err(e) => e.to_string(),
    }
}

#[tokio::test]
async fn a_bare_property_is_not_a_condition() {
    let g = garden().await;
    for q in [
        "find p.name from (p:Gardener) where p.active",
        "find p.name from (p:Gardener) where p.name = \"ana\" and p.active",
        "find a.name from (a:Gardener)-[:TRADES]->(b:Gardener) where b.active",
    ] {
        let err = error_of(&g, q).await;
        assert!(err.contains("not a comparison") && err.contains(".active = true"), "{q}\n{err}");
    }
    // The comparison is the way to write it.
    assert_eq!(names(&g, "find p.name from (p:Gardener) where p.active = true", "p.name").await, set(&["beto"]));
}

#[tokio::test]
async fn a_bare_property_in_a_write_is_an_error_and_writes_nothing() {
    let g = garden().await;
    for stmt in [
        "delete (p:Gardener) where p.active",
        "update (p:Gardener) set p.checked = true where p.active",
    ] {
        let err = match g.execute_statement(stmt).await {
            Ok(r) => panic!("{stmt} should fail, got {r:?}"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("not a comparison"), "{stmt}\n{err}");
    }
    let all = names(&g, "find p.name from (p:Gardener)", "p.name").await;
    assert_eq!(all.len(), 5, "nothing deleted");
    let checked = g.execute_nql("find p.name from (p:Gardener) where p.checked = true").await.unwrap();
    assert!(checked.rows().is_empty(), "nothing updated");
}

#[tokio::test]
async fn a_boolean_literal_is_its_value_and_other_literals_are_errors() {
    let g = garden().await;
    // Until 0.9.3 `where false` let every node through.
    assert!(names(&g, "find p.name from (p:Gardener) where false", "p.name").await.is_empty());
    assert_eq!(names(&g, "find p.name from (p:Gardener) where true", "p.name").await.len(), 5);
    let q = "find a.name from (a:Gardener)-[:TRADES]->(b:Gardener) where b.name = \"beto\" and false";
    assert!(names(&g, q, "a.name").await.is_empty());
    let err = error_of(&g, "find p.name from (p:Gardener) where 3").await;
    assert!(err.contains("literal"), "{err}");
}

#[tokio::test]
async fn an_algorithm_alias_in_where_is_the_algorithm() {
    let g = garden().await;
    let want = set(&["beto", "carla", "dora"]);
    for q in [
        "find n.name from (n:Gardener) where degree(n) > 1",
        "find n.name, degree(n) as dg from (n:Gardener) where dg > 1",
        "find n.name, degree(n) as dg from (n:Gardener) where degree(n) > 1",
    ] {
        assert_eq!(names(&g, q, "n.name").await, want, "{q}");
    }
}

#[tokio::test]
async fn a_property_alias_in_where_is_the_property() {
    let g = garden().await;
    let q = "find p.name as who from (p:Gardener) where who = \"carla\"";
    assert_eq!(names(&g, q, "who").await, set(&["carla"]));
}

#[tokio::test]
async fn having_on_an_aggregate_alias_filters_the_groups() {
    let g = garden().await;
    // ana's only trade weighs 0.9; everyone else's average is 0.3.
    for q in [
        "find a.name, avg(r.w) as m from (a:Gardener)-[r:TRADES]->(b:Gardener) group by a.name having m > 0.5",
        "find a.name, avg(r.w) as m from (a:Gardener)-[r:TRADES]->(b:Gardener) group by a.name having avg(r.w) > 0.5",
    ] {
        assert_eq!(names(&g, q, "a.name").await, set(&["ana"]), "{q}");
    }
}

#[tokio::test]
async fn an_aggregate_alias_in_where_is_still_an_aggregate_error() {
    let g = garden().await;
    let q = "find a.name, avg(r.w) as m from (a:Gardener)-[r:TRADES]->(b:Gardener) where m > 0.5 group by a.name";
    let err = error_of(&g, q).await;
    assert!(err.to_lowercase().contains("aggregat"), "{err}");
}
