// tests/nql_having_time_travel_test.rs
//
// Two NQL clauses that were accepted and then ignored until 0.9.3:
// - `group by … having …` on a single-node query returned every group
//   (#299); the pattern path applied it;
// - `at timestamp <n>` returned the current state (#298). Since 0.10.2 it
//   reads the graph as it was then (full coverage in nql_at_timestamp_test).
//
// Fictional domain: gardeners and their towns.

use std::collections::BTreeSet;

use nopaldb::types::{Node, PropertyValue};
use nopaldb::Graph;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

/// Two gardeners in "Oaxaca", one in "Puebla", one in "Tlaxcala".
async fn towns() -> Graph {
    let g = Graph::in_memory().await.unwrap();
    for (name, town, age) in [("ana", "Oaxaca", 30), ("beto", "Oaxaca", 50), ("carla", "Puebla", 40), ("dora", "Tlaxcala", 60)] {
        g.add_node(
            Node::new("Gardener")
                .with_property("name", s(name))
                .with_property("town", s(town))
                .with_property("age", PropertyValue::Int(age)),
        )
        .await
        .unwrap();
    }
    g
}

async fn town_set(g: &Graph, q: &str) -> BTreeSet<String> {
    let result = g.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    result
        .rows()
        .iter()
        .map(|r| match r.get("p.town") {
            Some(PropertyValue::String(x)) => x.clone(),
            other => panic!("{q}: {other:?}"),
        })
        .collect()
}

fn set(xs: &[&str]) -> BTreeSet<String> {
    xs.iter().map(|x| x.to_string()).collect()
}

#[tokio::test]
async fn having_filters_the_groups_of_a_single_node_query() {
    let g = towns().await;
    for q in [
        "find p.town, count(*) as n from (p:Gardener) group by p.town having count(*) > 1",
        "find p.town, count(*) as n from (p:Gardener) group by p.town having n > 1",
    ] {
        assert_eq!(town_set(&g, q).await, set(&["Oaxaca"]), "{q}");
    }
    let q = "find p.town, avg(p.age) as m from (p:Gardener) group by p.town having avg(p.age) >= 40 and count(*) = 1";
    assert_eq!(town_set(&g, q).await, set(&["Puebla", "Tlaxcala"]));
    let q = "find p.town, count(*) as n from (p:Gardener) group by p.town having p.town = \"Puebla\" or count(*) > 1";
    assert_eq!(town_set(&g, q).await, set(&["Oaxaca", "Puebla"]));
}

#[tokio::test]
async fn at_timestamp_reads_the_past_instead_of_the_current_state() {
    let g = towns().await;
    let before = g.last_commit_timestamp();
    g.add_node(Node::new("Gardener").with_property("name", s("eva")).with_property("town", s("Puebla")))
        .await
        .unwrap();
    let q = format!("find p.town, count(*) as n from (p:Gardener) group by p.town having count(*) > 1 at timestamp {before}");
    assert_eq!(town_set(&g, &q).await, set(&["Oaxaca"]), "{q}");
    let q = "find p.town, count(*) as n from (p:Gardener) group by p.town having count(*) > 1";
    assert_eq!(town_set(&g, q).await, set(&["Oaxaca", "Puebla"]), "{q}");
}
