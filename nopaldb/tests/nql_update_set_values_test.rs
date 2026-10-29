// tests/nql_update_set_values_test.rs
//
// `update … set var.prop = <value>` only writes literals. Until 0.10.2 any
// other value (`n.nombre`, a function) was dropped and the UPDATE answered
// OK with nothing updated (#330). Now it is a named error, and literals of
// every kind still write.
//
// Fictional domain: an orchard and its trees.

use nopaldb::types::{Edge, Node, PropertyValue};
use nopaldb::Graph;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

async fn orchard() -> Graph {
    let g = Graph::in_memory().await.unwrap();
    let mango = g.add_node(Node::new("Arbol").with_property("fruta", s("mango"))).await.unwrap();
    let lima = g.add_node(Node::new("Arbol").with_property("fruta", s("lima"))).await.unwrap();
    g.add_edge(Edge::new(mango, lima, "JUNTO_A")).await.unwrap();
    g
}

async fn riego(g: &Graph) -> Vec<Option<PropertyValue>> {
    let r = g.execute_nql("find n.fruta, n.riego from (n:Arbol) order by n.fruta").await.unwrap();
    r.rows().iter().map(|row| row.get("n.riego").cloned()).collect()
}

#[tokio::test]
async fn a_value_that_is_not_a_literal_is_an_error_and_writes_nothing() {
    let g = orchard().await;
    for q in [
        "update (n:Arbol) set n.riego = n.fruta",
        "update (n:Arbol) set n.riego = 1, n.copia = n.fruta",
        "update (a:Arbol)-[r:JUNTO_A]->(b:Arbol) set b.riego = a.fruta",
        "update (n:Arbol) set n.riego = degree(n)",
    ] {
        let err = match g.execute_nql(q).await {
            Ok(r) => panic!("{q}: should fail, got {:?}", r.rows().first().map(|r| &r.values)),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("only accepts a literal value"), "{q}: {err}");
    }
    assert_eq!(riego(&g).await, vec![None, None]);
}

#[tokio::test]
async fn literals_of_every_kind_are_written() {
    let g = orchard().await;
    for (literal, expected) in [
        ("3", PropertyValue::Int(3)),
        ("-5", PropertyValue::Int(-5)),
        ("2.5", PropertyValue::Float(2.5)),
        ("true", PropertyValue::Bool(true)),
        ("\"diario\"", s("diario")),
        ("[1, 2]", PropertyValue::List(vec![PropertyValue::Int(1), PropertyValue::Int(2)])),
        ("null", PropertyValue::Null),
    ] {
        let q = format!("update (n:Arbol) set n.riego = {literal} where n.fruta = \"mango\"");
        g.execute_nql(&q).await.unwrap_or_else(|e| panic!("{q}: {e}"));
        // order by n.fruta: lima, mango
        assert_eq!(riego(&g).await, vec![None, Some(expected)], "{q}");
    }
}

