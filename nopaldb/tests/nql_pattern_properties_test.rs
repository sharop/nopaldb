// tests/nql_pattern_properties_test.rs
//
// Properties that every way of running an NQL pattern must keep (contract 5
// of docs/NQL_SEMANTICS.md), checked on random small graphs:
//
// - direction symmetry: `(a)-[:T]->(b)` and `(b)<-[:T]-(a)` give the same
//   pairs, with the same multiplicity;
// - two hops with predicates on the expanded variables give exactly the
//   paths a brute-force enumeration of the edges gives (so pruning during
//   the expansion never changes the rows);
// - partitioning the WHERE (the TLP oracle of SQLancer/GDBMeter): the rows
//   without a filter are the rows of `P` plus the rows of its complement;
// - a DELETE with a pattern deletes exactly the edges the equivalent FIND
//   returns.
//
// Each graph: up to 12 nodes labelled A or B with an integer `x` in 0..4,
// up to 30 edges of type T or U with an integer `w` in 1..=2.

use std::collections::BTreeMap;

use nopaldb::types::{Edge, Node, NodeId, PropertyValue};
use nopaldb::Graph;
use proptest::prelude::*;

#[derive(Debug, Clone)]
struct Spec {
    nodes: Vec<(bool, i64)>,               // (is A, x)
    edges: Vec<(usize, usize, bool, i64)>, // (from, to, is T, w)
}

fn spec() -> impl Strategy<Value = Spec> {
    proptest::collection::vec((any::<bool>(), 0i64..4), 2..=12).prop_flat_map(|nodes| {
        let n = nodes.len();
        let edges = proptest::collection::vec((0..n, 0..n, any::<bool>(), 1i64..=2), 0..=30);
        (Just(nodes), edges).prop_map(|(nodes, edges)| Spec { nodes, edges })
    })
}

struct Built {
    graph: Graph,
    ids: Vec<NodeId>,
}

async fn build(spec: &Spec) -> Built {
    let graph = Graph::in_memory().await.unwrap();
    let mut ids = Vec::new();
    for (i, (is_a, x)) in spec.nodes.iter().enumerate() {
        let node = Node::new(if *is_a { "A" } else { "B" })
            .with_property("x", PropertyValue::Int(*x))
            .with_property("name", PropertyValue::String(format!("n{i}")));
        ids.push(graph.add_node(node).await.unwrap());
    }
    for (from, to, is_t, w) in &spec.edges {
        let e = Edge::new(ids[*from], ids[*to], if *is_t { "T" } else { "U" }).with_property("w", PropertyValue::Int(*w));
        graph.add_edge(e).await.unwrap();
    }
    Built { graph, ids }
}

/// Rows as a multiset of the given columns.
async fn bag(graph: &Graph, q: &str, cols: &[&str]) -> BTreeMap<Vec<String>, usize> {
    let result = graph.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    let mut out = BTreeMap::new();
    for r in result.rows() {
        let key: Vec<String> = cols.iter().map(|c| format!("{:?}", r.get(c))).collect();
        *out.entry(key).or_insert(0) += 1;
    }
    out
}

fn name(i: usize) -> String {
    format!("{:?}", Some(&PropertyValue::String(format!("n{i}"))))
}

fn sum(a: &BTreeMap<Vec<String>, usize>, b: &BTreeMap<Vec<String>, usize>) -> BTreeMap<Vec<String>, usize> {
    let mut out = a.clone();
    for (k, v) in b {
        *out.entry(k.clone()).or_insert(0) += v;
    }
    out
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    #[test]
    fn direction_is_symmetric(spec in spec()) {
        rt().block_on(async {
            let b = build(&spec).await;
            let out = bag(&b.graph, "find a.name, z.name from (a:A)-[:T]->(z:B)", &["a.name", "z.name"]).await;
            let inc = bag(&b.graph, "find a.name, z.name from (z:B)<-[:T]-(a:A)", &["a.name", "z.name"]).await;
            assert_eq!(out, inc, "{spec:?}");
            // And both match the edges.
            let mut want = BTreeMap::new();
            for (f, t, is_t, _) in &spec.edges {
                if *is_t && spec.nodes[*f].0 && !spec.nodes[*t].0 {
                    *want.entry(vec![name(*f), name(*t)]).or_insert(0) += 1;
                }
            }
            assert_eq!(out, want, "{spec:?}");
        });
    }

    #[test]
    fn two_hops_with_predicates_match_brute_force(spec in spec(), k in 0i64..4, w in 1i64..=2) {
        rt().block_on(async {
            let b = build(&spec).await;
            let q = format!(
                "find a.name, m.name, z.name from (a:A)-[:T {{w: {w}}}]->(m)-[:U]->(z) where m.x >= {k} and (z.x < {k} or z.x = 3)"
            );
            let got = bag(&b.graph, &q, &["a.name", "m.name", "z.name"]).await;
            let mut want = BTreeMap::new();
            for (f1, t1, is_t1, w1) in &spec.edges {
                if !(*is_t1 && *w1 == w && spec.nodes[*f1].0 && spec.nodes[*t1].1 >= k) {
                    continue;
                }
                for (f2, t2, is_t2, _) in &spec.edges {
                    let zx = spec.nodes[*t2].1;
                    if f2 == t1 && !*is_t2 && (zx < k || zx == 3) {
                        *want.entry(vec![name(*f1), name(*t1), name(*t2)]).or_insert(0) += 1;
                    }
                }
            }
            assert_eq!(got, want, "{q}\n{spec:?}");
        });
    }

    #[test]
    fn partitioning_the_where_adds_up(spec in spec(), k in 0i64..4) {
        rt().block_on(async {
            let b = build(&spec).await;
            let cols = ["a.name", "m.name", "z.name"];
            let base = "find a.name, m.name, z.name from (a)-[:T]->(m)-[:U]->(z)";
            let all = bag(&b.graph, base, &cols).await;
            let p = bag(&b.graph, &format!("{base} where m.x < {k}"), &cols).await;
            let not_p = bag(&b.graph, &format!("{base} where m.x >= {k}"), &cols).await;
            assert_eq!(all, sum(&p, &not_p), "{spec:?}");
            // One hop too.
            let base1 = "find a.name, m.name from (a)-[:T]->(m)";
            let all1 = bag(&b.graph, base1, &cols[..2]).await;
            let p1 = bag(&b.graph, &format!("{base1} where m.x < {k}"), &cols[..2]).await;
            let not_p1 = bag(&b.graph, &format!("{base1} where m.x >= {k}"), &cols[..2]).await;
            assert_eq!(all1, sum(&p1, &not_p1), "{spec:?}");
        });
    }

    #[test]
    fn delete_with_a_pattern_deletes_what_find_returns(spec in spec(), w in 1i64..=2) {
        rt().block_on(async {
            let b = build(&spec).await;
            let pattern = format!("(z:B)<-[r:T {{w: {w}}}]-(a:A)");
            let found = b.graph.execute_nql(&format!("find a.name from {pattern}")).await.unwrap().rows().len();
            let before = b.graph.get_all_edges().await.unwrap().len();
            b.graph.execute_statement(&format!("delete {pattern}")).await.unwrap();
            let after = b.graph.get_all_edges().await.unwrap();
            assert_eq!(before - after.len(), found, "{spec:?}");
            // What remains has no edge that the pattern describes.
            for e in &after {
                let a = b.ids.iter().position(|id| *id == e.source).unwrap();
                let z = b.ids.iter().position(|id| *id == e.target).unwrap();
                let matches = e.edge_type == "T"
                    && e.properties.get("w") == Some(&PropertyValue::Int(w))
                    && spec.nodes[a].0
                    && !spec.nodes[z].0;
                assert!(!matches, "{spec:?}");
            }
        });
    }
}
