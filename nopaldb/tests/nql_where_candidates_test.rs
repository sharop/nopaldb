// tests/nql_where_candidates_test.rs
//
// #214 phase 2: the predicates of the NQL `where` on the searched variable
// are the universe of `hybrid` / `similar_to`, not a filter applied to the
// top-k afterwards. Until 0.6.20 `where hybrid(c, …, k = 5) and c.tenant = "t7"`
// computed the top-5 of the whole label and then dropped the foreign tenants,
// so it could return fewer than 5 rows (0 for a 1 % tenant at 100k chunks).
//
// Fictional domain: recipe chunks of 40 tenants (10 each), every chunk
// mentions "nopal" (so the text branch ranks all of them), vectors spread on
// a circle so the nearest neighbours of the query cross tenants.
//
// Run with: cargo test --features core --test nql_where_candidates_test

use std::collections::{BTreeSet, HashMap};

use nopaldb::index::IndexType;
use nopaldb::types::{Edge, Node, NodeId, PropertyValue};
use nopaldb::{Graph, HybridFilter, HybridQuery, TextPath, VectorPath};

const TENANTS: usize = 40;
const PER_TENANT: usize = 10;
const DIM: usize = 8;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

fn unit(v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.into_iter().map(|x| x / n).collect()
}

/// Deterministic vector for chunk `i`: a point on a circle in the first two
/// dimensions plus a small, index-dependent tail.
fn vector_of(i: usize, total: usize) -> Vec<f32> {
    let a = (i as f32) * std::f32::consts::TAU / (total as f32);
    let mut v = vec![a.cos(), a.sin()];
    for d in 2..DIM {
        v.push((((i * 31 + d * 17) % 97) as f32) / 970.0);
    }
    unit(v)
}

fn query_vector() -> Vec<f32> {
    vector_of(0, 1)
}

fn literal(v: &[f32]) -> String {
    format!("[{}]", v.iter().map(|x| format!("{x:.6}")).collect::<Vec<_>>().join(", "))
}

struct Fx {
    graph: Graph,
    /// chunk id → (name, tenant)
    chunks: HashMap<NodeId, (String, String)>,
    vectors: HashMap<NodeId, Vec<f32>>,
}

/// `total` chunks; chunk `i` belongs to tenant `t{i % TENANTS}`; the broad
/// property `lang` is "es" on 90 % of them and `half` on 50 %.
async fn fixture_n(total: usize) -> Fx {
    let graph = Graph::in_memory().await.unwrap();
    let mut chunks = HashMap::new();
    let mut vectors = HashMap::new();
    let mut entities = HashMap::new();
    for t in 0..TENANTS {
        let id = graph.add_node(Node::new("Entity").with_property("name", s(&format!("e{t}")))).await.unwrap();
        entities.insert(t, id);
    }
    for i in 0..total {
        let tenant = format!("t{}", i % TENANTS);
        // More "nopal" for lower i: the text branch has a clear order.
        let reps = 1 + (total - i) * 5 / total;
        let text = format!("{} receta {i}", vec!["nopal"; reps].join(" "));
        let node = Node::new("Chunk")
            .with_property("name", s(&format!("c{i}")))
            .with_property("text", s(&text))
            .with_property("tenant", s(&tenant))
            .with_property("lang", s(if i % 10 == 0 { "en" } else { "es" }))
            .with_property("half", PropertyValue::Bool(i % 2 == 0))
            .with_property("n", PropertyValue::Int(i as i64))
            .with_property("tags", PropertyValue::List(vec![s("receta"), s(&tenant)]));
        let id = graph.add_node(node).await.unwrap();
        let v = vector_of(i, total);
        graph.add_node_embedding(id, v.clone(), "m").await.unwrap();
        graph.add_edge(Edge::new(id, entities[&(i % TENANTS)], "MENTIONS")).await.unwrap();
        chunks.insert(id, (format!("c{i}"), tenant));
        vectors.insert(id, v);
    }
    graph.create_index("Chunk", "text", IndexType::FullText).await.unwrap();
    Fx { graph, chunks, vectors }
}

async fn fixture() -> Fx {
    fixture_n(TENANTS * PER_TENANT).await
}

fn id_of(v: &PropertyValue) -> NodeId {
    match v {
        PropertyValue::String(x) => x.parse().unwrap(),
        other => panic!("not an id: {other:?}"),
    }
}

fn hybrid_q(where_rest: &str, k: usize) -> String {
    format!(
        r#"find c.id, c.tenant, score(c) from (c:Chunk) where hybrid(c, text = "nopal", vector = {}, model = "m", k = {k}){where_rest}"#,
        literal(&query_vector())
    )
}

fn similar_q(where_rest: &str, k: usize) -> String {
    format!(
        r#"find c.id, c.tenant, score(c) from (c:Chunk) where similar_to(c, vector = {}, model = "m", k = {k}){where_rest}"#,
        literal(&query_vector())
    )
}

async fn rows(fx: &Fx, q: &str) -> Vec<(NodeId, String, f64)> {
    let result = fx.graph.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    result
        .rows()
        .iter()
        .map(|r| {
            let score = match r.get("score(c)") {
                Some(PropertyValue::Float(x)) => *x,
                other => panic!("score: {other:?}"),
            };
            (id_of(r.get("c.id").unwrap()), match r.get("c.tenant") {
                Some(PropertyValue::String(t)) => t.clone(),
                other => panic!("tenant: {other:?}"),
            }, score)
        })
        .collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Exact top-`k` by cosine among the chunks that pass `keep`.
fn exact_top(fx: &Fx, k: usize, keep: impl Fn(&str) -> bool) -> Vec<NodeId> {
    let q = query_vector();
    let mut all: Vec<(NodeId, f32)> = fx
        .vectors
        .iter()
        .filter(|(id, _)| keep(&fx.chunks[id].1))
        .map(|(id, v)| (*id, cosine(&q, v)))
        .collect();
    all.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    all.into_iter().take(k).map(|(id, _)| id).collect()
}

#[tokio::test]
async fn hybrid_returns_k_rows_inside_the_tenant() {
    let fx = fixture().await;
    let got = rows(&fx, &hybrid_q(r#" and c.tenant = "t7""#, 5)).await;
    assert_eq!(got.len(), 5, "top-k is computed inside the tenant: {got:?}");
    assert!(got.iter().all(|(_, t, _)| t == "t7"));

    // Same universe, same answer as the Rust/Python API with `props`.
    let mut q = HybridQuery::new();
    q.text = Some("nopal".into());
    q.vector = Some((query_vector(), "m".into()));
    q.k = 5;
    q.filter = Some(HybridFilter { label: Some("Chunk".into()), props: vec![("tenant".into(), s("t7"))] });
    let api = fx.graph.search_hybrid(q).await.unwrap();
    assert_eq!(
        got.iter().map(|(id, _, _)| *id).collect::<Vec<_>>(),
        api.iter().map(|h| h.node_id).collect::<Vec<_>>(),
        "NQL and search_hybrid(props=) rank the same universe the same way"
    );
    for ((_, _, score), hit) in got.iter().zip(&api) {
        assert!((*score - hit.score as f64).abs() < 1e-6);
    }
}

#[tokio::test]
async fn similar_to_returns_the_exact_top_k_inside_the_tenant() {
    let fx = fixture().await;
    let got = rows(&fx, &similar_q(r#" and c.tenant = "t7""#, 5)).await;
    let ids: Vec<NodeId> = got.iter().map(|(id, _, _)| *id).collect();
    assert_eq!(ids, exact_top(&fx, 5, |t| t == "t7"));
}

#[tokio::test]
async fn in_lists_and_inline_props_are_pushed_too() {
    let fx = fixture().await;
    let got = rows(&fx, &similar_q(r#" and c.tenant in ["t7", "t13"]"#, 6)).await;
    let ids: Vec<NodeId> = got.iter().map(|(id, _, _)| *id).collect();
    assert_eq!(ids, exact_top(&fx, 6, |t| t == "t7" || t == "t13"));

    let inline = format!(
        r#"find c.id, c.tenant, score(c) from (c:Chunk {{tenant: "t7"}}) where hybrid(c, text = "nopal", vector = {}, model = "m", k = 5)"#,
        literal(&query_vector())
    );
    let got = rows(&fx, &inline).await;
    assert_eq!(got.len(), 5);
    assert!(got.iter().all(|(_, t, _)| t == "t7"));
}

#[tokio::test]
async fn ids_are_the_universe_and_intersect() {
    let fx = fixture().await;
    let t7: Vec<NodeId> = fx.chunks.iter().filter(|(_, (_, t))| t == "t7").map(|(id, _)| *id).collect();
    let list = t7.iter().map(|id| format!("\"{id}\"")).collect::<Vec<_>>().join(", ");
    let got = rows(&fx, &hybrid_q(&format!(" and c.id in [{list}]"), 3)).await;
    assert_eq!(got.len(), 3);
    assert!(got.iter().all(|(id, _, _)| t7.contains(id)));

    // `c.id = a and c.id = b` is the empty set, not the union.
    let both = format!(r#" and c.id = "{}" and c.id = "{}""#, t7[0], t7[1]);
    assert!(rows(&fx, &hybrid_q(&both, 3)).await.is_empty());
}

#[tokio::test]
async fn residual_predicates_still_filter_and_are_never_widened() {
    let fx = fixture().await;
    // A range is not pushed: it applies to the top-k, as before.
    let got = rows(&fx, &similar_q(" and c.n >= 200", 5)).await;
    assert!(got.len() <= 5);
    for (id, _, _) in &got {
        let n: usize = fx.chunks[id].0[1..].parse().unwrap();
        assert!(n >= 200);
    }
    // A list-valued equality is not indexable: it is not pushed, and it
    // still filters by the evaluator's equality (no row is lost to an empty
    // index lookup).
    let tags = r#" and c.tags = ["receta", "t0"]"#;
    let got = rows(&fx, &similar_q(&format!("{tags} and c.tenant = \"t0\""), 5)).await;
    assert_eq!(got.len(), 5, "tenant pushed, tags residual: {got:?}");
    assert!(got.iter().all(|(_, t, _)| t == "t0"));
}

#[tokio::test]
async fn a_search_must_be_a_root_conjunct() {
    let fx = fixture().await;
    let lit = literal(&query_vector());
    let cases = [
        (
            format!(r#"find c.id from (c:Chunk) where hybrid(c, text = "nopal", vector = {lit}, model = "m") or c.tenant = "t7""#),
            "top-level AND",
        ),
        (
            // AND binds tighter than OR: this is `(similar_to and n > 3) or tenant`.
            format!(r#"find c.id from (c:Chunk) where similar_to(c, vector = {lit}, model = "m") and c.n > 3 or c.tenant = "t7""#),
            "top-level AND",
        ),
        (
            format!(r#"find c.id from (c:Chunk) where hybrid(c, text = "nopal", model = "m", vector = {lit}) and hybrid(c, text = "tuna", model = "m", vector = {lit})"#),
            "only one",
        ),
    ];
    for (q, needle) in cases {
        let err = fx.graph.execute_nql(&q).await.expect_err(&q).to_string();
        assert!(err.contains(needle), "{q}\n→ {err}");
    }
}

#[tokio::test]
async fn a_seeded_pattern_pushes_on_the_seed() {
    let fx = fixture().await;
    let q = format!(
        r#"find c.id, e.name from (c:Chunk)-[:MENTIONS]->(e:Entity) where hybrid(c, text = "nopal", vector = {}, model = "m", k = 4) and c.tenant = "t7""#,
        literal(&query_vector())
    );
    let result = fx.graph.execute_nql(&q).await.unwrap();
    let seeds: BTreeSet<NodeId> = result.rows().iter().map(|r| id_of(r.get("c.id").unwrap())).collect();
    assert_eq!(seeds.len(), 4, "k = 4 seeds inside the tenant");
    assert!(result.rows().iter().all(|r| r.get("e.name") == Some(&s("e7"))));
}

#[tokio::test]
async fn explain_names_the_pushed_predicates() {
    let fx = fixture().await;
    let q = hybrid_q(r#" and c.tenant = "t7" and c.n >= 3"#, 5);
    let plan = format!("{:?}", fx.graph.execute_nql(&format!("explain {q}")).await.unwrap().rows());
    assert!(plan.contains("Search universe: top-k computed inside c.tenant = "), "{plan}");
    assert!(!plan.contains("c.n"), "a range is not pushed: {plan}");
}

#[tokio::test]
async fn update_keeps_the_property_index_in_step() {
    let fx = fixture().await;
    fx.graph
        .execute_statement(r#"update (c:Chunk) set c.tenant = "t99" where c.name = "c7""#)
        .await
        .unwrap();
    let got = rows(&fx, &similar_q(r#" and c.tenant = "t99""#, 5)).await;
    assert_eq!(got.len(), 1, "the moved chunk is found under its new value: {got:?}");
    let got = rows(&fx, &similar_q(r#" and c.tenant = "t7""#, 40)).await;
    assert_eq!(got.len(), PER_TENANT - 1, "and no longer under the old one");
}

/// The planner: a broad property filter is checked on the candidates (no
/// set, like a label); a selective one builds the set (exact when small).
#[tokio::test]
async fn the_planner_checks_broad_filters_and_builds_sets_for_selective_ones() {
    let fx = fixture_n(2400).await; // the vector index is above the exact threshold
    let base = || {
        let mut q = HybridQuery::new();
        q.text = Some("nopal".into());
        q.vector = Some((query_vector(), "m".into()));
        q.k = 10;
        q
    };

    let mut broad = base();
    broad.filter = Some(HybridFilter { label: Some("Chunk".into()), props: vec![("lang".into(), s("es"))] });
    let ex = fx.graph.search_hybrid_explain(broad).await.unwrap();
    assert_eq!(ex.vector_path, Some(VectorPath::LabelChecked));
    assert_eq!(ex.text_path, Some(TextPath::GlobalLabelChecked));
    assert_eq!(ex.allowed_set_size, None, "no set built for a 90 % filter");
    assert_eq!(ex.hits.len(), 10);

    let mut selective = base();
    selective.filter = Some(HybridFilter { label: Some("Chunk".into()), props: vec![("tenant".into(), s("t7"))] });
    let ex = fx.graph.search_hybrid_explain(selective).await.unwrap();
    assert_eq!(ex.allowed_set_size, Some(2400 / TENANTS));
    assert_eq!(ex.vector_path, Some(VectorPath::ExactOverAllowed), "a 60-node set ranks exactly");
    assert_eq!(ex.hits.len(), 10);
    assert!(ex.hits.iter().all(|h| fx.chunks[&h.node_id].1 == "t7"));
}

/// A list-valued `props` filter: the property index cannot key it, so it is
/// checked on the nodes. Until 0.6.20 its empty index lookup emptied the set
/// and the search returned nothing.
#[tokio::test]
async fn a_list_valued_property_filter_is_checked_on_the_nodes() {
    let fx = fixture().await;
    let tags = PropertyValue::List(vec![s("receta"), s("t7")]);
    for label in [Some("Chunk".to_string()), None] {
        let mut q = HybridQuery::new();
        q.text = Some("nopal".into());
        q.vector = Some((query_vector(), "m".into()));
        q.k = 5;
        q.filter = Some(HybridFilter { label: label.clone(), props: vec![("tags".into(), tags.clone())] });
        let hits = fx.graph.search_hybrid(q).await.unwrap();
        assert_eq!(hits.len(), 5, "label {label:?}");
        assert!(hits.iter().all(|h| fx.chunks[&h.node_id].1 == "t7"));
    }
}
