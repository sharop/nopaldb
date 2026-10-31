// tests/nql_join_limits_test.rs
//
// The join of several patterns in `from` applies the `where` while it
// combines them, stops at `limit` when it can, and has a row limit with a
// named error (#349). Until 0.10.5 it kept the whole product in memory and
// filtered at the end: `from (a:X), (b:X) where … limit 10` over 2000 nodes
// held 4 million rows (10 GB) to answer 10.
//
// Fictional domain: an orchard. Trees with a row number and a yield; some
// trees have no `poda` (pruning year).

use std::collections::BTreeSet;

use nopaldb::storage::StorageProfile;
use nopaldb::types::{Edge, Node, PropertyValue};
use nopaldb::Graph;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

fn int(i: i64) -> PropertyValue {
    PropertyValue::Int(i)
}

/// Twelve trees `t0`..`t11`: `fila = i % 4`, `cosecha = i`; even trees have
/// `poda = 2020 + i`. Each tree `ti` with i < 11 is NEXT_TO `t(i+1)`.
async fn orchard() -> Graph {
    let g = Graph::in_memory().await.unwrap();
    let mut ids = Vec::new();
    for i in 0..12i64 {
        let mut n = Node::new("Arbol")
            .with_property("nombre", s(&format!("t{i}")))
            .with_property("fila", int(i % 4))
            .with_property("cosecha", int(i));
        if i % 2 == 0 {
            n = n.with_property("poda", int(2020 + i));
        }
        ids.push(g.add_node(n).await.unwrap());
    }
    for w in ids.windows(2) {
        g.add_edge(Edge::new(w[0], w[1], "JUNTO_A")).await.unwrap();
    }
    g
}

fn tree(i: i64) -> (i64, i64, Option<i64>) {
    (i % 4, i, (i % 2 == 0).then_some(2020 + i))
}

/// The rows as a set of tuples of the given columns.
async fn rows(g: &Graph, q: &str, keys: &[&str]) -> BTreeSet<Vec<String>> {
    let r = g.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    r.rows()
        .iter()
        .map(|row| {
            keys.iter()
                .map(|k| match row.get(k) {
                    Some(PropertyValue::String(v)) => v.clone(),
                    other => panic!("{q}: {k} = {other:?}"),
                })
                .collect()
        })
        .collect()
}

fn names(list: impl IntoIterator<Item = Vec<i64>>) -> BTreeSet<Vec<String>> {
    list.into_iter().map(|t| t.iter().map(|i| format!("t{i}")).collect()).collect()
}

#[tokio::test]
async fn conditions_across_patterns_give_the_rows_they_gave_before() {
    let g = orchard().await;
    let all = 0..12i64;
    // A condition on each pattern and one that relates them.
    let q = "find a.nombre, b.nombre from (a:Arbol), (b:Arbol) \
             where a.fila = 1 and b.fila = 2 and a.cosecha < b.cosecha";
    let expected = names(all.clone().flat_map(|a| all.clone().map(move |b| vec![a, b])).filter(|t| {
        let (fa, ca, _) = tree(t[0]);
        let (fb, cb, _) = tree(t[1]);
        fa == 1 && fb == 2 && ca < cb
    }));
    assert_eq!(rows(&g, q, &["a.nombre", "b.nombre"]).await, expected);

    // An `or` that crosses the patterns is one part: it waits for both.
    let q = "find a.nombre, b.nombre from (a:Arbol), (b:Arbol) where a.fila = 0 or b.cosecha = 11";
    let expected = names(
        all.clone().flat_map(|a| all.clone().map(move |b| vec![a, b])).filter(|t| tree(t[0]).0 == 0 || t[1] == 11),
    );
    assert_eq!(rows(&g, q, &["a.nombre", "b.nombre"]).await, expected);

    // Arithmetic across the patterns.
    let q = "find a.nombre, b.nombre from (a:Arbol), (b:Arbol) where a.cosecha + b.cosecha = 5";
    let expected = names(all.clone().flat_map(|a| all.clone().map(move |b| vec![a, b])).filter(|t| t[0] + t[1] == 5));
    assert_eq!(rows(&g, q, &["a.nombre", "b.nombre"]).await, expected);
}

#[tokio::test]
async fn a_condition_waits_until_its_variables_are_bound() {
    // `a.cosecha < c.cosecha` reads `c`, which the third pattern binds. If
    // it ran on the join of `a` and `b` it would read `c` as missing and
    // drop every row.
    let g = orchard().await;
    let q = "find a.nombre, b.nombre, c.nombre from (a:Arbol), (b:Arbol), (c:Arbol) \
             where a.fila = 3 and b.cosecha = 0 and a.cosecha < c.cosecha";
    let expected = names(
        (0..12i64)
            .filter(|a| tree(*a).0 == 3)
            .flat_map(|a| (0..12i64).filter(move |c| a < *c).map(move |c| vec![a, 0, c])),
    );
    assert!(!expected.is_empty());
    assert_eq!(rows(&g, q, &["a.nombre", "b.nombre", "c.nombre"]).await, expected);

    // A missing property is false only where it is read: odd trees have no
    // `poda`, so `b.poda > 2025` keeps even trees after 2025.
    let q = "find a.nombre, b.nombre from (a:Arbol), (b:Arbol) where a.cosecha = 1 and b.poda > 2025";
    let expected = names((0..12i64).filter(|b| tree(*b).2.is_some_and(|p| p > 2025)).map(|b| vec![1, b]));
    assert_eq!(rows(&g, q, &["a.nombre", "b.nombre"]).await, expected);
}

#[tokio::test]
async fn connected_patterns_keep_their_rows() {
    let g = orchard().await;
    // Trees next to the same tree, with conditions on both ends.
    let q = "find a.nombre, b.nombre from (a:Arbol)-[:JUNTO_A]->(m:Arbol), (b:Arbol)-[:JUNTO_A]->(m) \
             where a.cosecha < 6 and b.fila >= 0";
    // Each tree has one tree before it, so `a` and `b` are the same tree.
    let expected = names((0..6i64).map(|a| vec![a, a]));
    assert_eq!(rows(&g, q, &["a.nombre", "b.nombre"]).await, expected);
}

#[tokio::test]
async fn limit_without_order_by_stops_the_join() {
    let g = orchard().await;
    let base = "find a.nombre, b.nombre from (a:Arbol), (b:Arbol) where a.fila = b.fila";
    let full = rows(&g, base, &["a.nombre", "b.nombre"]).await;
    assert_eq!(full.len(), 36);
    let some = rows(&g, &format!("{base} limit 4 offset 2"), &["a.nombre", "b.nombre"]).await;
    assert_eq!(some.len(), 4);
    assert!(some.is_subset(&full));
    // With `order by` the join cannot stop early: the first rows are the
    // first in order.
    let r = g
        .execute_nql(&format!("{base} order by a.nombre, b.nombre limit 2"))
        .await
        .unwrap();
    let first: Vec<_> = r.rows().iter().map(|row| row.get("b.nombre").cloned()).collect();
    assert_eq!(first, [Some(s("t0")), Some(s("t4"))]);
}

#[tokio::test]
async fn the_row_limit_is_a_named_error() {
    let g = orchard().await;
    assert_eq!(g.max_join_rows(), 400_000);
    g.set_max_join_rows(20);
    assert_eq!(g.max_join_rows(), 20);

    let err = g.execute_nql("find a.nombre from (a:Arbol), (b:Arbol)").await.unwrap_err();
    let text = err.to_string();
    assert!(text.contains("more than 20 rows") && text.contains("max_join_rows"), "{text}");

    // The rows that pass the `where` are what count: 12 × 12 = 144 pairs,
    // 3 kept.
    let ok = rows(&g, "find a.nombre, b.nombre from (a:Arbol), (b:Arbol) where a.cosecha = 0 and b.fila = 1", &["a.nombre", "b.nombre"]).await;
    assert_eq!(ok.len(), 3);

    // Every join counts, in the order of FROM. `b` is related to `a` only
    // through `c`: with `b` second, the join of `a` (1 tree) and `b` keeps
    // all 12 pairs, more than 10; with `c` second it keeps 3, and the
    // answer is the same 9 rows.
    g.set_max_join_rows(10);
    let q = |order: &str| {
        format!("find a.nombre, b.nombre from {order} where a.cosecha = 0 and a.fila = c.fila and b.fila = c.fila")
    };
    let err = g.execute_nql(&q("(a:Arbol), (b:Arbol), (c:Arbol)")).await.unwrap_err();
    assert!(err.to_string().contains("every join counts"), "{err}");
    assert_eq!(g.execute_nql(&q("(a:Arbol), (c:Arbol), (b:Arbol)")).await.unwrap().rows().len(), 9);
    g.set_max_join_rows(20);

    // `order by` needs every row: the join cannot stop at `limit`.
    let err = g
        .execute_nql("find a.nombre from (a:Arbol), (b:Arbol) order by a.nombre limit 1")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("more than 20 rows"), "{err}");
    assert_eq!(
        g.execute_nql("find a.nombre from (a:Arbol), (b:Arbol) limit 1").await.unwrap().rows().len(),
        1
    );

    // The past graph keeps the limit of the graph that asks.
    let ts = g.last_commit_timestamp();
    let err = g
        .execute_nql(&format!("find a.nombre from (a:Arbol), (b:Arbol) at timestamp {ts}"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("more than 20 rows"), "{err}");

    g.set_max_join_rows(0);
    assert_eq!(g.max_join_rows(), 400_000);
    assert_eq!(g.execute_nql("find a.nombre from (a:Arbol), (b:Arbol)").await.unwrap().rows().len(), 144);
}

#[tokio::test]
async fn the_mobile_profile_has_a_lower_limit() {
    let g = Graph::in_memory_with_profile(StorageProfile::Mobile).await.unwrap();
    assert_eq!(g.max_join_rows(), 100_000);
}

#[tokio::test]
async fn explain_describes_the_join() {
    let g = orchard().await;
    let r = g
        .execute_nql("explain find a.nombre from (a:Arbol), (b:Arbol) where a.fila = 1 and a.cosecha < b.cosecha limit 5")
        .await
        .unwrap();
    let plan = format!("{:?}", r.rows());
    assert!(plan.contains("PATTERN PIPELINE"), "{plan}");
    assert!(plan.contains("Join: pattern 1 reads with a.fila = 1"), "{plan}");
    assert!(plan.contains("the join keeps rows with a.cosecha < b.cosecha"), "{plan}");
    assert!(plan.contains("stops at 5 rows"), "{plan}");
    assert!(plan.contains("at most 400000 rows"), "{plan}");
}
