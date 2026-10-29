// tests/nql_result_schema_test.rs
//
// The schema of a result is a contract (#335): one column per projection,
// in the order of the projection, named by its alias or by the text of its
// expression; `*` and `all(var)` expanded; the same whatever path runs the
// query. Until 0.10.3 each execution path named and ordered the columns its
// own way: `count(*)` was `count`, `group by` put the group keys first, a
// repeated alias or a projection outside `group by` lost a column without
// an error, `find *` declared a symbolic `*`, `order by count(*)` did
// nothing and `find 1` gave empty rows.
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
    let mango = g
        .add_node(Node::new("Arbol").with_property("fruta", s("mango")).with_property("edad", PropertyValue::Int(12)))
        .await
        .unwrap();
    g.add_node(Node::new("Arbol").with_property("fruta", s("mango")).with_property("edad", PropertyValue::Int(3)))
        .await
        .unwrap();
    let lima = g
        .add_node(Node::new("Arbol").with_property("fruta", s("lima")).with_property("edad", PropertyValue::Int(4)))
        .await
        .unwrap();
    g.add_edge(Edge::new(mango, lima, "JUNTO_A").with_property("metros", PropertyValue::Int(3)))
        .await
        .unwrap();
    g
}

async fn columns(g: &Graph, q: &str) -> Vec<String> {
    let r = g.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    for row in r.rows() {
        for k in row.values.keys() {
            assert!(r.columns.contains(k), "{q}: key {k} is not a column of {:?}", r.columns);
        }
    }
    r.columns.clone()
}

async fn error_of(g: &Graph, q: &str) -> String {
    match g.execute_nql(q).await {
        Ok(r) => panic!("{q}: should fail, got columns {:?}", r.columns),
        Err(e) => e.to_string(),
    }
}

fn cols(xs: &[&str]) -> Vec<String> {
    xs.iter().map(|x| x.to_string()).collect()
}

/// `distinct` compares the columns of `find`, not the keys a path adds for
/// its own use (`n.id`), which the schema drops at the end.
#[tokio::test]
async fn distinct_compares_the_columns_of_find() {
    let g = orchard().await;
    for q in [
        "find distinct n.fruta from (n:Arbol)",
        "find distinct n.fruta as f from (n:Arbol)",
        "find distinct n.fruta from (n:Arbol) where n.edad > 0",
    ] {
        let r = g.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
        assert_eq!(r.rows().len(), 2, "{q}");
    }
}

/// Rule 3: without an alias the name is the text of the expression.
#[tokio::test]
async fn the_default_name_is_the_text_of_the_expression() {
    let g = orchard().await;
    for (q, expected) in [
        ("find count(*) from (n:Arbol)", vec!["count(*)"]),
        ("find sum(n.edad), max(n.edad) from (n:Arbol)", vec!["sum(n.edad)", "max(n.edad)"]),
        ("find COUNT(*) from (n:Arbol)", vec!["count(*)"]),
        ("find count(*) from (a:Arbol)-[r:JUNTO_A]->(b:Arbol)", vec!["count(*)"]),
        ("find n.fruta, degree(n) from (n:Arbol)", vec!["n.fruta", "degree(n)"]),
        ("find avg(degree(n)) from (n:Arbol)", vec!["avg(degree(n))"]),
        ("find n.fruta, 1 from (n:Arbol)", vec!["n.fruta", "1"]),
    ] {
        assert_eq!(columns(&g, q).await, cols(&expected), "{q}");
    }
    let r = g.execute_nql("find count(*) from (n:Arbol)").await.unwrap();
    assert_eq!(r.rows()[0].get("count(*)"), Some(&PropertyValue::Int(3)));
}

/// A literal is a constant column; until 0.10.3 it was dropped.
#[tokio::test]
async fn a_literal_is_a_constant_column() {
    let g = orchard().await;
    let r = g.execute_nql("find n.fruta, 1 as uno from (n:Arbol)").await.unwrap();
    assert_eq!(r.rows().len(), 3);
    assert!(r.rows().iter().all(|row| row.get("uno") == Some(&PropertyValue::Int(1))));
}

/// Rule 4: the columns follow the projection, whatever the path.
#[tokio::test]
async fn columns_follow_the_projection() {
    let g = orchard().await;
    for (q, expected) in [
        ("find count(*) as c, n.fruta from (n:Arbol) group by n.fruta", vec!["c", "n.fruta"]),
        ("find n.fruta, count(*) as c from (n:Arbol) group by n.fruta", vec!["n.fruta", "c"]),
        ("find degree(n) as d, n.fruta from (n:Arbol)", vec!["d", "n.fruta"]),
        ("find n.edad, n.fruta from (n:Arbol)", vec!["n.edad", "n.fruta"]),
        ("find b.fruta, r.metros, a.fruta from (a:Arbol)-[r:JUNTO_A]->(b:Arbol)", vec!["b.fruta", "r.metros", "a.fruta"]),
        ("find count(*) as c, a.fruta from (a:Arbol)-[r:JUNTO_A]->(b:Arbol) group by a.fruta", vec!["c", "a.fruta"]),
    ] {
        assert_eq!(columns(&g, q).await, cols(&expected), "{q}");
    }
}

/// Rule 5: two columns with one name are an error, not a lost column.
#[tokio::test]
async fn a_repeated_name_is_an_error() {
    let g = orchard().await;
    for q in [
        "find n.fruta as x, n.edad as x from (n:Arbol)",
        "find n.fruta, n.fruta from (n:Arbol)",
        "find count(*), count(*) from (n:Arbol)",
    ] {
        let err = error_of(&g, q).await;
        assert!(err.contains("two columns of find are named"), "{q}: {err}");
    }
}

/// Rule 6: with `group by`, a projection is a group key or an aggregate.
#[tokio::test]
async fn with_group_by_every_column_is_a_key_or_an_aggregate() {
    let g = orchard().await;
    let err = error_of(&g, "find n.edad, count(*) as c from (n:Arbol) group by n.fruta").await;
    assert!(err.contains("n.edad is not in group by"), "{err}");
    let err = error_of(&g, "find n.fruta, pagerank(n) from (n:Arbol) group by n.fruta").await;
    assert!(err.contains("write avg(pagerank(n))"), "{err}");
    let err = error_of(&g, "find * from (n:Arbol) group by n.fruta").await;
    assert!(err.contains("cannot be used with group by"), "{err}");

    // A group key that is not projected does not appear.
    assert_eq!(columns(&g, "find count(*) as c from (n:Arbol) group by n.fruta").await, cols(&["c"]));
    let r = g.execute_nql("find count(*) as c from (n:Arbol) group by n.fruta").await.unwrap();
    assert_eq!(r.rows().len(), 2);
    // The aggregate of an algorithm is fine.
    assert_eq!(
        columns(&g, "find n.fruta, avg(degree(n)) as d from (n:Arbol) group by n.fruta").await,
        cols(&["n.fruta", "d"])
    );
}

/// Rule 7: `*` and `all(var)` are expanded to concrete columns.
#[tokio::test]
async fn the_wildcard_is_expanded() {
    let g = orchard().await;
    assert_eq!(
        columns(&g, "find * from (n:Arbol)").await,
        cols(&["n.id", "n.label", "n.edad", "n.fruta"])
    );
    assert_eq!(
        columns(&g, "find * from (a:Arbol)-[r:JUNTO_A]->(b:Arbol)").await,
        cols(&[
            "a.id", "a.label", "a.edad", "a.fruta", "r.id", "r.type", "r.metros", "b.id", "b.label", "b.edad", "b.fruta",
        ])
    );
    assert_eq!(
        columns(&g, "find all(r), a.fruta from (a:Arbol)-[r:JUNTO_A]->(b:Arbol)").await,
        cols(&["r.id", "r.type", "r.metros", "a.fruta"])
    );
    // With no rows the schema still has the id and the label.
    assert_eq!(
        columns(&g, "find * from (n:Arbol) where n.fruta = \"pera\"").await,
        cols(&["n.id", "n.label"])
    );
}

/// `order by` on an expression of `find` sorts by its column; until 0.10.3
/// it did nothing.
#[tokio::test]
async fn order_by_an_expression_sorts_by_its_column() {
    let g = orchard().await;
    let r = g
        .execute_nql("find n.fruta, count(*) from (n:Arbol) group by n.fruta order by count(*) desc")
        .await
        .unwrap();
    let order: Vec<_> = r.rows().iter().map(|row| row.get("n.fruta").cloned()).collect();
    assert_eq!(order, vec![Some(s("mango")), Some(s("lima"))]);
    let r = g
        .execute_nql("find n.fruta, count(*) from (n:Arbol) group by n.fruta order by count(*)")
        .await
        .unwrap();
    let order: Vec<_> = r.rows().iter().map(|row| row.get("n.fruta").cloned()).collect();
    assert_eq!(order, vec![Some(s("lima")), Some(s("mango"))]);

    let err = error_of(&g, "find n.fruta from (n:Arbol) order by degree(n) desc").await;
    assert!(err.contains("not a column of find"), "{err}");
}

/// Rule 8: the schema does not depend on the path: with or without an
/// index, and read now or `at timestamp`.
#[tokio::test]
async fn the_schema_does_not_depend_on_the_path() {
    let queries = [
        "find n.edad, n.fruta as f from (n:Arbol) where n.fruta = \"mango\"",
        "find * from (n:Arbol) where n.fruta = \"mango\"",
        "find all(n) from (n:Arbol) where n.fruta = \"mango\"",
        "find count(*), n.fruta from (n:Arbol) where n.fruta = \"mango\" group by n.fruta",
        "find degree(n), n.fruta from (n:Arbol) where n.fruta = \"mango\"",
        "find avg(n.edad) from (n:Arbol) where n.fruta = \"mango\"",
        "find b.fruta, a.edad from (a:Arbol)-[r:JUNTO_A]->(b:Arbol) where a.fruta = \"mango\"",
    ];
    let plain = orchard().await;
    let indexed = orchard().await;
    indexed.execute_nql("create index on Arbol(fruta)").await.unwrap();
    let ts = plain.last_commit_timestamp();
    for q in queries {
        let base = columns(&plain, q).await;
        assert_eq!(columns(&indexed, q).await, base, "{q}: index");
        let past = format!("{q} at timestamp {ts}");
        assert_eq!(columns(&plain, &past).await, base, "{past}");
    }
}
