// tests/nql_aggregation_semantics_test.rs
//
// One semantics of aggregation for the three paths of NQL (#364): the nodes
// of one pattern, a one-hop pattern, and several patterns. Until 0.10.10
// the same question gave different numbers depending on the shape of
// `from`: in the node path `count(x)` counted every node, `avg(x)` divided
// by every node, `min`/`max` with no values failed, the argument could only
// be a property, and the groups came ordered by the text of their key
// (`10` before `9`), with the keys as text (`"4"` for 4).
//
// The same rows reach the three paths: every recipe goes to the one
// kitchen (`-[:EN]->(c:Cocina)` and `, (c:Cocina)` give one row per recipe).
//
// Fictional domain: a kitchen. Recipes with a region, a heat level that
// some do not have, and servings.

use nopaldb::types::{Edge, Node, PropertyValue};
use nopaldb::Graph;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

/// (name, region, heat, servings). Regions first appear as sur, norte,
/// centro, 10, 9 (the last two are numbers, to see the order of numbers).
const RECIPES: [(&str, Region, Option<i64>, i64); 7] = [
    ("mole", Region::Text("sur"), Some(3), 6),
    ("tamal", Region::Text("norte"), None, 3),
    ("pozole", Region::Text("centro"), Some(2), 4),
    ("sopa", Region::Text("sur"), None, 2),
    ("chile", Region::Text("centro"), Some(5), 1),
    ("atole", Region::Number(10), None, 2),
    ("elote", Region::Number(9), Some(1), 1),
];

#[derive(Clone, Copy)]
enum Region {
    Text(&'static str),
    Number(i64),
}

impl Region {
    fn value(self) -> PropertyValue {
        match self {
            Region::Text(t) => s(t),
            Region::Number(n) => PropertyValue::Int(n),
        }
    }
}

async fn kitchen() -> Graph {
    let g = Graph::in_memory().await.unwrap();
    let kitchen = g.add_node(Node::new("Cocina").with_property("nombre", s("casa"))).await.unwrap();
    for (name, region, heat, servings) in RECIPES {
        let mut n = Node::new("Receta")
            .with_property("nombre", s(name))
            .with_property("region", region.value())
            .with_property("porciones", PropertyValue::Int(servings));
        if let Some(h) = heat {
            n = n.with_property("picante", PropertyValue::Int(h));
        }
        let id = g.add_node(n).await.unwrap();
        g.add_edge(Edge::new(id, kitchen, "EN")).await.unwrap();
    }
    g
}

/// The `from` of each path, with the same rows.
const PATHS: [(&str, &str); 3] = [
    ("nodes", "(r:Receta)"),
    ("one hop", "(r:Receta)-[:EN]->(c:Cocina)"),
    ("several patterns", "(r:Receta), (c:Cocina)"),
];

async fn values(g: &Graph, q: &str) -> Vec<Vec<PropertyValue>> {
    let r = g.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    r.rows()
        .iter()
        .map(|row| r.columns.iter().map(|c| row.get(c).cloned().unwrap_or(PropertyValue::Null)).collect())
        .collect()
}

#[tokio::test]
async fn the_aggregates_give_the_same_by_every_path() {
    let g = kitchen().await;
    let heats: Vec<i64> = RECIPES.iter().filter_map(|r| r.2).collect();
    let expected = vec![vec![
        PropertyValue::Int(RECIPES.len() as i64),
        PropertyValue::Int(heats.len() as i64),
        PropertyValue::Float(heats.iter().sum::<i64>() as f64),
        PropertyValue::Float(heats.iter().sum::<i64>() as f64 / heats.len() as f64),
        PropertyValue::Int(*heats.iter().min().unwrap()),
        PropertyValue::Int(*heats.iter().max().unwrap()),
        PropertyValue::Float(heats.iter().map(|h| h * 2).sum::<i64>() as f64),
    ]];
    for (path, from) in PATHS {
        let q = format!(
            "find count(*), count(r.picante), sum(r.picante), avg(r.picante), min(r.picante), max(r.picante), \
             sum(r.picante * 2) from {from}"
        );
        assert_eq!(values(&g, &q).await, expected, "{path}");
    }
}

#[tokio::test]
async fn no_values_give_null_by_every_path() {
    let g = kitchen().await;
    for (path, from) in PATHS {
        // No recipe has `dulce`: present values, none.
        let q = format!("find count(r.dulce), sum(r.dulce), avg(r.dulce), min(r.dulce), max(r.dulce) from {from}");
        assert_eq!(
            values(&g, &q).await,
            [[PropertyValue::Int(0), PropertyValue::Float(0.0), PropertyValue::Null, PropertyValue::Null, PropertyValue::Null]],
            "{path}"
        );
    }
}

#[tokio::test]
async fn groups_come_in_the_order_they_first_appear_by_every_path() {
    let g = kitchen().await;
    // The order of the rows is the order in which a pattern of one node
    // reads the recipes; the groups follow it.
    let scan = values(&g, "find r.region, r.picante from (r:Receta)").await;
    let mut expected: Vec<(PropertyValue, i64, i64)> = Vec::new();
    for row in &scan {
        let present = i64::from(row[1] != PropertyValue::Null);
        match expected.iter_mut().find(|(k, _, _)| *k == row[0]) {
            Some(e) => {
                e.1 += 1;
                e.2 += present;
            }
            None => expected.push((row[0].clone(), 1, present)),
        }
    }
    let expected: Vec<Vec<PropertyValue>> = expected
        .into_iter()
        .map(|(k, n, present)| vec![k, PropertyValue::Int(n), PropertyValue::Int(present)])
        .collect();
    // The numeric keys are numbers, not text.
    assert!(expected.iter().any(|row| row[0] == PropertyValue::Int(10)));
    for (path, from) in PATHS {
        let q = format!("find r.region, count(*), count(r.picante) from {from} group by r.region");
        assert_eq!(values(&g, &q).await, expected, "{path}");
    }
}

#[tokio::test]
async fn a_missing_key_is_a_null_group_by_every_path() {
    let g = kitchen().await;
    for (path, from) in PATHS {
        let q = format!("find r.picante, count(*) from {from} group by r.picante");
        let rows = values(&g, &q).await;
        let missing = RECIPES.iter().filter(|r| r.2.is_none()).count() as i64;
        assert!(rows.contains(&vec![PropertyValue::Null, PropertyValue::Int(missing)]), "{path}: {rows:?}");
    }
}
