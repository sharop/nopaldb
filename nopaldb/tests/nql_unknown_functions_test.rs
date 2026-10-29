// tests/nql_unknown_functions_test.rs
//
// A function NQL does not know was dropped without an error until 0.10.2
// (#328): the column vanished, `where foo(n)` let every row through,
// `order by foo(n)` did nothing, and `COUNT(*)` (a known name in another
// case) gave an empty row. Now an unknown name is a named error in every
// clause, and a known one is matched without case.
//
// Fictional domain: an orchard and its trees.

use nopaldb::types::{Edge, Node, PropertyValue};
use nopaldb::Graph;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

/// Three trees; the mango is next to the guava.
async fn orchard() -> Graph {
    let g = Graph::in_memory().await.unwrap();
    let mango = g.add_node(Node::new("Arbol").with_property("fruta", s("mango"))).await.unwrap();
    let guayaba = g.add_node(Node::new("Arbol").with_property("fruta", s("guayaba"))).await.unwrap();
    g.add_node(Node::new("Arbol").with_property("fruta", s("lima"))).await.unwrap();
    g.add_edge(Edge::new(mango, guayaba, "JUNTO_A")).await.unwrap();
    g
}

async fn error_of(g: &Graph, q: &str) -> String {
    match g.execute_nql(q).await {
        Ok(r) => panic!("{q}: should fail, got {} rows", r.rows().len()),
        Err(e) => e.to_string(),
    }
}

async fn trees(g: &Graph) -> usize {
    g.execute_nql("find n.fruta from (n:Arbol)").await.unwrap().rows().len()
}

#[tokio::test]
async fn an_unknown_function_is_a_named_error_in_every_clause() {
    let g = orchard().await;
    for q in [
        "find foo(n) as x from (n:Arbol)",
        "find n.fruta from (n:Arbol) where foo(n)",
        "find n.fruta from (n:Arbol) where foo(n) = 1",
        "find n.fruta from (n:Arbol) where n.fruta = \"mango\" and foo(n) = 1",
        "find n.fruta from (n:Arbol) order by foo(n)",
        "find count(foo(n)) as c from (n:Arbol)",
        "find n.fruta, count(*) as c from (n:Arbol) group by n.fruta having foo(n) > 1",
        "explain find n.fruta from (n:Arbol) where foo(n)",
    ] {
        let err = error_of(&g, q).await;
        assert!(err.contains("unknown function foo"), "{q}: {err}");
    }
}

#[tokio::test]
async fn writes_with_an_unknown_function_are_errors_and_change_nothing() {
    let g = orchard().await;
    let err = error_of(&g, "delete (n:Arbol) where foo(n)").await;
    assert!(err.contains("unknown function foo"), "{err}");
    let err = error_of(&g, "update (n:Arbol) set n.riego = 1 where foo(n)").await;
    assert!(err.contains("unknown function foo"), "{err}");
    let err = error_of(&g, "update (n:Arbol) set n.riego = now()").await;
    assert!(err.contains("unknown function now"), "{err}");
    assert_eq!(trees(&g).await, 3);
}

#[tokio::test]
async fn id_of_a_variable_points_to_the_id_property() {
    let g = orchard().await;
    for q in [
        "find id(t) as i from (t:Arbol)",
        "find t.fruta from (t:Arbol) where id(t) = \"x\"",
    ] {
        let err = error_of(&g, q).await;
        assert!(err.contains("write t.id"), "{q}: {err}");
    }
    let r = g.execute_nql("find t.id as i from (t:Arbol)").await.unwrap();
    assert_eq!(r.rows().len(), 3);
}

#[tokio::test]
async fn known_functions_are_matched_without_case() {
    let g = orchard().await;
    let r = g.execute_nql("find COUNT(*) as c from (n:Arbol)").await.unwrap();
    assert_eq!(r.rows()[0].get("c"), Some(&PropertyValue::Int(3)));

    let r = g
        .execute_nql("find n.fruta, Count(*) as c from (n:Arbol) group by n.fruta order by n.fruta")
        .await
        .unwrap();
    let counts: Vec<_> = r.rows().iter().map(|row| row.get("c").cloned()).collect();
    assert_eq!(counts, vec![Some(PropertyValue::Int(1)); 3]);

    let r = g.execute_nql("find n.fruta, DEGREE(n) as d from (n:Arbol) where n.fruta = \"mango\"").await.unwrap();
    assert!(r.rows()[0].get("d").is_some(), "{:?}", r.rows());
}
