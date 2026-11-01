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

    // Every join counts. `b` is related to `a` only through `c`: the join
    // goes `a`, `c`, `b` (#349), so the join of `a` (1 tree) and `c` keeps
    // 3 rows instead of the 12 of `a` and `b`, under a limit of 10.
    g.set_max_join_rows(10);
    let q = |order: &str| {
        format!("find a.nombre, b.nombre from {order} where a.cosecha = 0 and a.fila = c.fila and b.fila = c.fila")
    };
    assert_eq!(g.execute_nql(&q("(a:Arbol), (b:Arbol), (c:Arbol)")).await.unwrap().rows().len(), 9);
    assert_eq!(g.execute_nql(&q("(a:Arbol), (c:Arbol), (b:Arbol)")).await.unwrap().rows().len(), 9);
    // A part that reads all three relates none of them two by two: the
    // join follows FROM, and `a` with `b` keeps 12 rows.
    let q = |order: &str| {
        format!("find a.nombre, b.nombre from {order} where a.cosecha = 0 and c.cosecha = 0 and a.fila + b.fila = c.fila")
    };
    let err = g.execute_nql(&q("(a:Arbol), (b:Arbol), (c:Arbol)")).await.unwrap_err();
    assert!(err.to_string().contains("every join counts"), "{err}");
    assert_eq!(g.execute_nql(&q("(a:Arbol), (c:Arbol), (b:Arbol)")).await.unwrap().rows().len(), 3);
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

/// The rows in the order they come, as `Debug` text of the given columns.
async fn ordered(g: &Graph, q: &str, keys: &[&str]) -> Vec<Vec<String>> {
    let r = g.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    r.rows()
        .iter()
        .map(|row| keys.iter().map(|k| format!("{:?}", row.get(k))).collect())
        .collect()
}

#[tokio::test]
async fn an_equality_across_patterns_joins_by_hash_with_the_same_rows_in_the_same_order() {
    // `a.fila = b.fila` groups the join; `a.fila - b.fila = 0` reads both
    // sides in one expression, so it cannot, and takes the nested loop. Both
    // give the same rows in the same order.
    let g = orchard().await;
    let keys = ["a.nombre", "b.nombre"];
    let hashed = ordered(&g, "find a.nombre, b.nombre from (a:Arbol), (b:Arbol) where a.fila = b.fila", &keys).await;
    let nested = ordered(&g, "find a.nombre, b.nombre from (a:Arbol), (b:Arbol) where a.fila - b.fila = 0", &keys).await;
    assert_eq!(hashed.len(), 36);
    assert_eq!(hashed, nested);

    // An expression on each side, with `limit`: the same first rows.
    let hashed = ordered(
        &g,
        "find a.nombre, b.nombre from (a:Arbol), (b:Arbol) where a.cosecha + 1 = b.cosecha limit 5",
        &keys,
    )
    .await;
    let nested = ordered(
        &g,
        "find a.nombre, b.nombre from (a:Arbol), (b:Arbol) where a.cosecha + 1 - b.cosecha = 0 limit 5",
        &keys,
    )
    .await;
    assert_eq!(hashed.len(), 5);
    assert_eq!(hashed, nested);

    // A property that is missing never matches: odd trees have no `poda`.
    let rows = ordered(&g, "find a.nombre, b.nombre from (a:Arbol), (b:Arbol) where a.poda = b.poda", &keys).await;
    assert_eq!(rows.len(), 6);
}

#[tokio::test]
async fn the_hash_join_keeps_the_equality_of_numbers() {
    // `=` compares an integer with a decimal by value (`3 = 3.0`), `0 =
    // -0.0`, and lists element by element; text is not a number. The join
    // groups by a key that keeps all of it.
    let g = Graph::in_memory().await.unwrap();
    let values = [
        ("i3", int(3)),
        ("f3", PropertyValue::Float(3.0)),
        ("f3.5", PropertyValue::Float(3.5)),
        ("i0", int(0)),
        ("neg0", PropertyValue::Float(-0.0)),
        ("l12", PropertyValue::List(vec![int(1), int(2)])),
        ("lf12", PropertyValue::List(vec![PropertyValue::Float(1.0), PropertyValue::Float(2.0)])),
        ("txt", s("3")),
    ];
    for (name, v) in values {
        g.add_node(Node::new("Caja").with_property("nombre", s(name)).with_property("peso", v)).await.unwrap();
    }
    g.add_node(Node::new("Caja").with_property("nombre", s("sin"))).await.unwrap();
    let keys = ["a.nombre", "b.nombre"];
    let q = "find a.nombre, b.nombre from (a:Caja), (b:Caja) where a.peso = b.peso";
    let groups: [&[&str]; 5] = [&["i3", "f3"], &["f3.5"], &["i0", "neg0"], &["l12", "lf12"], &["txt"]];
    let expected: BTreeSet<Vec<String>> = groups
        .iter()
        .flat_map(|g| g.iter().flat_map(move |a| g.iter().map(move |b| vec![a.to_string(), b.to_string()])))
        .collect();
    assert_eq!(rows(&g, q, &keys).await, expected);

    // A NaN cannot be grouped (`=` gives it equal to every integer, as the
    // order of `PropertyValue` does): the join falls back to the nested loop
    // and gives what `=` gives.
    g.add_node(Node::new("Caja").with_property("nombre", s("nan")).with_property("peso", PropertyValue::Float(f64::NAN)))
        .await
        .unwrap();
    let q = "find a.nombre, b.nombre from (a:Caja), (b:Caja) where a.nombre = 'nan' and a.peso = b.peso";
    let expected: BTreeSet<Vec<String>> =
        ["i3", "i0", "nan"].iter().map(|b| vec!["nan".to_string(), b.to_string()]).collect();
    assert_eq!(rows(&g, q, &keys).await, expected);
}

#[tokio::test]
async fn a_shared_variable_joins_by_hash_with_the_same_rows_in_the_same_order() {
    // Trees tied to the same post. `m` is shared: the join groups by it.
    // With a second variable `n` and `m.nombre <= n.nombre and m.nombre >=
    // n.nombre` (no equality) the join is the nested loop; the rows and
    // their order are the same.
    let g = Graph::in_memory().await.unwrap();
    let posts: Vec<_> = label_ids(&g, "Poste", 3).await;
    let trees: Vec<_> = label_ids(&g, "Arbol", 9).await;
    for (i, t) in trees.iter().enumerate() {
        g.add_edge(Edge::new(*t, posts[i % 3], "ATADO_A")).await.unwrap();
    }
    let keys = ["a.nombre", "b.nombre"];
    let hashed = ordered(
        &g,
        "find a.nombre, b.nombre from (a:Arbol)-[:ATADO_A]->(m:Poste), (b:Arbol)-[:ATADO_A]->(m)",
        &keys,
    )
    .await;
    let nested = ordered(
        &g,
        "find a.nombre, b.nombre from (a:Arbol)-[:ATADO_A]->(m:Poste), (b:Arbol)-[:ATADO_A]->(n:Poste) \
         where m.nombre <= n.nombre and m.nombre >= n.nombre",
        &keys,
    )
    .await;
    assert_eq!(hashed.len(), 27);
    assert_eq!(hashed, nested);
}

async fn label_ids(g: &Graph, label: &str, n: usize) -> Vec<nopaldb::types::NodeId> {
    let mut ids = Vec::new();
    for i in 0..n {
        ids.push(g.add_node(Node::new(label).with_property("nombre", s(&format!("{label}{i}")))).await.unwrap());
    }
    ids
}

#[tokio::test]
async fn explain_says_what_the_join_groups_by() {
    let g = orchard().await;
    let plan = |q: &str| {
        let g = &g;
        let q = q.to_string();
        async move { format!("{:?}", g.execute_nql(&format!("explain {q}")).await.unwrap().rows()) }
    };
    let p = plan("find a.nombre from (a:Arbol), (b:Arbol) where a.fila = b.fila").await;
    assert!(p.contains("joins by hash on a.fila = b.fila"), "{p}");
    let p = plan("find a.nombre from (a:Arbol)-[:JUNTO_A]->(m:Arbol), (b:Arbol)-[:JUNTO_A]->(m)").await;
    assert!(p.contains("joins by hash on m"), "{p}");
    let p = plan("find a.nombre from (a:Arbol), (b:Arbol) where a.fila < b.fila").await;
    assert!(!p.contains("joins by hash"), "{p}");
}

/// The nodes of a label in the order a single pattern reads them.
async fn scan(g: &Graph, label: &str) -> Vec<String> {
    let r = g.execute_nql(&format!("find x.nombre from (x:{label})")).await.unwrap();
    r.rows()
        .iter()
        .map(|row| match row.get("x.nombre") {
            Some(PropertyValue::String(v)) => v.clone(),
            other => panic!("{other:?}"),
        })
        .collect()
}

#[tokio::test]
async fn a_reordered_join_gives_the_rows_in_the_order_of_from() {
    // `b` relates to `a` only through `c`: the join goes `a`, `c`, `b`, and
    // the rows come back in the order the nested loop over `a`, `b`, `c`
    // gives them, also with `limit`.
    let g = orchard().await;
    let trees = scan(&g, "Arbol").await;
    let fila = |name: &str| name[1..].parse::<i64>().unwrap() % 4;
    let mut expected = Vec::new();
    for a in &trees {
        for b in &trees {
            for c in &trees {
                if fila(a) == 1 && fila(a) == fila(c) && fila(b) == fila(c) {
                    expected.push(vec![format!("{:?}", Some(s(a))), format!("{:?}", Some(s(b))), format!("{:?}", Some(s(c)))]);
                }
            }
        }
    }
    let keys = ["a.nombre", "b.nombre", "c.nombre"];
    let q = "find a.nombre, b.nombre, c.nombre from (a:Arbol), (b:Arbol), (c:Arbol) \
             where a.fila = 1 and a.fila = c.fila and b.fila = c.fila";
    assert_eq!(expected.len(), 27);
    assert_eq!(ordered(&g, q, &keys).await, expected);
    assert_eq!(ordered(&g, &format!("{q} limit 4 offset 3"), &keys).await, expected[3..7].to_vec());
}

#[tokio::test]
async fn a_reordered_join_keeps_the_paths_of_each_pattern() {
    // Patterns with edges: `(a)->(b)` and `(c)->(d)` relate only through
    // `(b)->(c)`, written last, so the join goes 1, 3, 2. The rows come in
    // the sequence of the nested loop over the FROM order.
    let g = orchard().await;
    let edges = ordered(&g, "find x.nombre, y.nombre from (x:Arbol)-[:JUNTO_A]->(y:Arbol)", &["x.nombre", "y.nombre"]).await;
    // `cosecha` is the number in the name.
    let cosecha = |cell: &str| cell.chars().filter(char::is_ascii_digit).collect::<String>().parse::<i64>().unwrap();
    let mut expected = Vec::new();
    for ab in &edges {
        for cd in &edges {
            for bc in &edges {
                if bc[0] == ab[1] && bc[1] == cd[0] && cosecha(&ab[0]) < 3 {
                    expected.push(vec![ab[0].clone(), cd[1].clone()]);
                }
            }
        }
    }
    let q = "find a.nombre, d.nombre from (a:Arbol)-[:JUNTO_A]->(b:Arbol), (c:Arbol)-[:JUNTO_A]->(d:Arbol), \
             (b)-[:JUNTO_A]->(c) where a.cosecha < 3";
    let rows = ordered(&g, q, &["a.nombre", "d.nombre"]).await;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows, expected);
}

#[tokio::test]
async fn explain_says_the_order_of_the_join() {
    let g = orchard().await;
    let r = g
        .execute_nql(
            "explain find a.nombre from (a:Arbol), (b:Arbol), (c:Arbol) where a.fila = c.fila and b.fila = c.fila limit 5",
        )
        .await
        .unwrap();
    let plan = format!("{:?}", r.rows());
    assert!(plan.contains("Join: order 1, 3, 2"), "{plan}");
    assert!(plan.contains("pattern 3; joins by hash on a.fila = c.fila"), "{plan}");
    assert!(!plan.contains("stops at"), "{plan}");
    // In the order of FROM, no order line.
    let r = g
        .execute_nql("explain find a.nombre from (a:Arbol), (b:Arbol), (c:Arbol) where a.fila = b.fila and b.fila = c.fila")
        .await
        .unwrap();
    let plan = format!("{:?}", r.rows());
    assert!(!plan.contains("order 1"), "{plan}");
}
