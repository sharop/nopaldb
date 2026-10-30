// tests/nql_update_set_values_test.rs
//
// `update … set var.prop = <value>`: until 0.10.2 any value that was not a
// literal (`n.nombre`, a function) was dropped and the UPDATE answered OK
// with nothing updated (#330). Since then what SET cannot evaluate is a
// named error, and literals of every kind write; since #269 properties of
// the same variable and arithmetic write too.
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

/// What SET cannot evaluate is an error and writes nothing. Since #269 it
/// also takes properties of its own variable and arithmetic (see
/// nql_arithmetic_test); another variable of the pattern or a function is
/// still an error.
#[tokio::test]
async fn a_value_it_cannot_evaluate_is_an_error_and_writes_nothing() {
    let g = orchard().await;
    for q in [
        "update (a:Arbol)-[r:JUNTO_A]->(b:Arbol) set b.riego = a.fruta",
        "update (n:Arbol) set n.riego = degree(n)",
        "update (n:Arbol) set n.riego = 1, n.copia = degree(n)",
    ] {
        let err = match g.execute_nql(q).await {
            Ok(r) => panic!("{q}: should fail, got {:?}", r.rows().first().map(|r| &r.values)),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("only accepts literals, properties of"), "{q}: {err}");
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

