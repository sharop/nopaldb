//! NQL search with several patterns in `from` (#384, #214 phase 2c).
//!
//! The first pattern starts at the searched variable and expands from the
//! seeds. Every other pattern that reaches the variable, directly or through
//! variables shared with other extra patterns, is a graph predicate on it:
//! with the parts of the `where` that only read its variables, it defines
//! the universe where the search computes its top-k
//! (`docs/NQL_SEMANTICS.md` § 2). Patterns that join the first one through
//! another variable are expansion constraints and never change the seeds.
//!
//! Fictional domain: recipes that use ingredients (some of them winter
//! ingredients) and come from regions; the ingredients form two communities.
//!
//! Run with: cargo test --features core --test nql_search_several_patterns_test

use std::collections::{BTreeSet, HashMap, HashSet};

use nopaldb::graph::communities::CommunityMaterializeOptions;
use nopaldb::index::IndexType;
use nopaldb::types::{Edge, Node, NodeId, PropertyValue};
use nopaldb::{CandidateSet, Graph, HybridFilter, HybridQuery};

const RECIPES: usize = 120;
const INGREDIENTS: usize = 10;
const DIM: usize = 8;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

fn unit(v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.into_iter().map(|x| x / n).collect()
}

/// Recipe `i` sits on a circle, so the nearest ones to the query are the
/// lowest `i` and the highest.
fn vector_of(i: usize) -> Vec<f32> {
    let a = (i as f32) * std::f32::consts::TAU / (RECIPES as f32);
    let mut v = vec![a.cos(), a.sin()];
    for d in 2..DIM {
        v.push((((i * 31 + d * 17) % 97) as f32) / 970.0);
    }
    unit(v)
}

/// The query vector, rounded as the NQL literal rounds it.
fn query_vector() -> Vec<f32> {
    vector_of(0).iter().map(|x| format!("{x:.6}").parse().unwrap()).collect()
}

fn literal(v: &[f32]) -> String {
    format!("[{}]", v.iter().map(|x| format!("{x:.6}")).collect::<Vec<_>>().join(", "))
}

/// Ingredients of recipe `i`: `i % 10`, and `(i + 5) % 10` too when `i % 4 == 0`.
fn ingredients_of(i: usize) -> Vec<usize> {
    let mut v = vec![i % INGREDIENTS];
    if i % 4 == 0 {
        v.push((i + 5) % INGREDIENTS);
    }
    v
}

/// Ingredients 0, 1 and 2 are winter ingredients.
fn winter(j: usize) -> bool {
    j < 3
}

struct Fx {
    graph: Graph,
    recipe: HashMap<NodeId, usize>,
    /// Community key of the ingredients with an even index.
    key: String,
}

async fn fixture() -> Fx {
    let graph = Graph::in_memory().await.unwrap();
    let mut ingredients = Vec::new();
    for j in 0..INGREDIENTS {
        let node = Node::new("Ingrediente")
            .with_property("nombre", s(&format!("ing{j}")))
            .with_property("temporada", s(if winter(j) { "invierno" } else { "verano" }));
        ingredients.push(graph.add_node(node).await.unwrap());
    }
    let mut regions = Vec::new();
    for g in 0..3 {
        regions.push(graph.add_node(Node::new("Region").with_property("nombre", s(&format!("region{g}")))).await.unwrap());
    }
    let level: HashMap<NodeId, usize> = ingredients.iter().enumerate().map(|(j, id)| (*id, j % 2)).collect();
    graph.materialize_communities(&[level], &CommunityMaterializeOptions::default()).await.unwrap();

    let mut recipe = HashMap::new();
    for i in 0..RECIPES {
        // More "sopa" for lower i: the text branch has a clear order.
        let reps = 1 + (RECIPES - i) * 5 / RECIPES;
        let node = Node::new("Receta")
            .with_property("nombre", s(&format!("receta{i}")))
            .with_property("n", PropertyValue::Int(i as i64))
            .with_property("texto", s(&format!("{} caliente {i}", vec!["sopa"; reps].join(" "))));
        let id = graph.add_node(node).await.unwrap();
        graph.add_node_embedding(id, vector_of(i), "m").await.unwrap();
        for j in ingredients_of(i) {
            graph.add_edge(Edge::new(id, ingredients[j], "USA")).await.unwrap();
        }
        graph.add_edge(Edge::new(id, regions[i % 3], "DE")).await.unwrap();
        recipe.insert(id, i);
    }
    graph.create_index("Receta", "texto", IndexType::FullText).await.unwrap();

    let rows = graph
        .execute_nql(r#"find k.key from (i:Ingrediente)-[:IN_COMMUNITY]->(k:Community) where i.nombre = "ing0""#)
        .await
        .unwrap();
    let key = match rows.rows()[0].get("k.key") {
        Some(PropertyValue::String(k)) => k.clone(),
        other => panic!("{other:?}"),
    };
    Fx { graph, recipe, key }
}

fn id_of(v: Option<&PropertyValue>) -> NodeId {
    match v {
        Some(PropertyValue::String(x)) => x.parse().unwrap(),
        other => panic!("not an id: {other:?}"),
    }
}

async fn run(fx: &Fx, q: &str) -> Vec<HashMap<String, PropertyValue>> {
    let result = fx.graph.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}"));
    result.rows().iter().map(|r| r.keys().into_iter().map(|k| (k.clone(), r.get(k).unwrap().clone())).collect()).collect()
}

/// The text of an `explain`.
async fn explain(fx: &Fx, q: &str) -> String {
    format!("{:?}", fx.graph.execute_nql(q).await.unwrap_or_else(|e| panic!("{q}\n{e}")).rows())
}

/// The values of `r.id` in row order, each once: the seeds in their order.
fn seeds(rows: &[HashMap<String, PropertyValue>]) -> Vec<NodeId> {
    let mut seen = HashSet::new();
    rows.iter().map(|r| id_of(r.get("r.id"))).filter(|id| seen.insert(*id)).collect()
}

/// Recipes that use a winter ingredient.
fn winter_universe(fx: &Fx) -> HashSet<NodeId> {
    fx.recipe.iter().filter(|(_, i)| ingredients_of(**i).into_iter().any(winter)).map(|(id, _)| *id).collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Exact top-`k` recipes by cosine inside `universe`.
fn exact_top(fx: &Fx, universe: &HashSet<NodeId>, k: usize) -> Vec<NodeId> {
    let q = query_vector();
    let mut all: Vec<(NodeId, f32)> =
        universe.iter().map(|id| (*id, cosine(&q, &vector_of(fx.recipe[id])))).collect();
    all.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    all.into_iter().take(k).map(|(id, _)| id).collect()
}

fn similar(k: usize) -> String {
    format!(r#"similar_to(r, vector = {}, model = "m", k = {k})"#, literal(&query_vector()))
}

fn hybrid(k: usize) -> String {
    format!(r#"hybrid(r, text = "sopa", vector = {}, model = "m", k = {k})"#, literal(&query_vector()))
}

#[tokio::test]
async fn hybrid_ranks_inside_the_universe_of_the_other_patterns() {
    let fx = fixture().await;
    let q = format!(
        r#"find r.id, i.nombre, i.temporada, score(r) from (r:Receta), (r)-[:USA]->(i:Ingrediente) where {} and i.temporada = "invierno""#,
        hybrid(5)
    );
    let rows = run(&fx, &q).await;
    let universe = winter_universe(&fx);

    let mut hq = HybridQuery::new();
    hq.text = Some("sopa".into());
    hq.vector = Some((query_vector(), "m".into()));
    hq.k = 5;
    hq.filter = Some(HybridFilter { label: Some("Receta".into()), props: Vec::new() });
    hq.candidates = universe.iter().copied().collect::<CandidateSet>();
    let hits = fx.graph.search_hybrid(hq).await.unwrap();
    let want: Vec<NodeId> = hits.iter().map(|h| h.node_id).collect();
    assert_eq!(want.len(), 5);
    assert_eq!(seeds(&rows), want, "top-k inside the universe, in search order");

    // One row per winter ingredient of each seed, every row carries the
    // seed's score.
    let score: HashMap<NodeId, f32> = hits.iter().map(|h| (h.node_id, h.score)).collect();
    let expected_rows: usize =
        want.iter().map(|id| ingredients_of(fx.recipe[id]).into_iter().filter(|j| winter(*j)).count()).sum();
    assert_eq!(rows.len(), expected_rows);
    for row in &rows {
        assert_eq!(row.get("i.temporada"), Some(&s("invierno")));
        let id = id_of(row.get("r.id"));
        match row.get("score(r)") {
            Some(PropertyValue::Float(x)) => assert!((*x - score[&id] as f64).abs() < 1e-6),
            other => panic!("score: {other:?}"),
        }
    }
}

#[tokio::test]
async fn similar_to_is_the_exact_top_k_inside_the_universe() {
    let fx = fixture().await;
    let universe = winter_universe(&fx);
    for k in [1, 5, 12, universe.len(), universe.len() + 10] {
        let q = format!(
            r#"find r.id from (r:Receta), (r)-[:USA]->(i:Ingrediente) where {} and i.temporada = "invierno""#,
            similar(k)
        );
        let got = seeds(&run(&fx, &q).await);
        assert_eq!(got, exact_top(&fx, &universe, k), "k = {k}");
        assert_eq!(got.len(), k.min(universe.len()), "|seeds| = min(k, |universe|)");
    }
}

#[tokio::test]
async fn one_pattern_expands_and_several_restrict() {
    // The documented consequence: the same graph shape written as one
    // pattern is an expansion constraint, as two patterns a universe.
    let fx = fixture().await;
    let one = format!(
        r#"find r.id from (r:Receta)-[:USA]->(i:Ingrediente) where {} and i.temporada = "invierno""#,
        similar(10)
    );
    let two = format!(
        r#"find r.id from (r:Receta), (r)-[:USA]->(i:Ingrediente) where {} and i.temporada = "invierno""#,
        similar(10)
    );
    let universe = winter_universe(&fx);
    let all: HashSet<NodeId> = fx.recipe.keys().copied().collect();
    let global: Vec<NodeId> = exact_top(&fx, &all, 10);
    let expanded = seeds(&run(&fx, &one).await);
    assert_eq!(expanded, global.iter().copied().filter(|id| universe.contains(id)).collect::<Vec<_>>());
    assert!(expanded.len() < 10);
    assert_eq!(seeds(&run(&fx, &two).await), exact_top(&fx, &universe, 10));
}

#[tokio::test]
async fn predicates_on_the_searched_variable_keep_their_rules() {
    // A range on `r` filters the rows of the search (contract 2), also when
    // another pattern mentions `r`: every recipe has a region, so the extra
    // pattern's universe is every recipe and the seeds must not change.
    let fx = fixture().await;
    let alone = format!(r#"find r.id from (r:Receta) where {} and r.n > 100"#, similar(5));
    let with_pattern =
        format!(r#"find r.id from (r:Receta), (r)-[:DE]->(g:Region) where {} and r.n > 100"#, similar(5));
    let a = seeds(&run(&fx, &alone).await);
    assert!(a.len() < 5, "the range is applied to the top-5, not inside it: {a:?}");
    assert_eq!(seeds(&run(&fx, &with_pattern).await), a);
}

#[tokio::test]
async fn a_pattern_that_does_not_reach_the_searched_variable_only_filters_rows() {
    let fx = fixture().await;
    // `(i)-[:IN_COMMUNITY]->(k)` joins through `i`, a variable of the first
    // pattern: an expansion constraint, the seeds are the global top-k.
    let q = format!(
        r#"find r.id from (r:Receta)-[:USA]->(i:Ingrediente), (i)-[:IN_COMMUNITY]->(k:Community) where {} and k.key = "{}""#,
        similar(10),
        fx.key
    );
    let all: HashSet<NodeId> = fx.recipe.keys().copied().collect();
    let global = exact_top(&fx, &all, 10);
    let got = seeds(&run(&fx, &q).await);
    assert!(!got.is_empty());
    assert!(got.len() < 10, "some seeds have no ingredient in the community: {got:?}");
    let in_order: Vec<NodeId> = global.iter().copied().filter(|id| got.contains(id)).collect();
    assert_eq!(got, in_order, "a subsequence of the global top-k, in order");
}

#[tokio::test]
async fn the_pattern_universe_equals_in_community_as_a_set() {
    let fx = fixture().await;
    let explicit = format!(
        r#"find r.id from (r:Receta), (r)-[:USA]->(i:Ingrediente)-[:IN_COMMUNITY]->(k:Community) where {} and k.key = "{}""#,
        similar(1000),
        fx.key
    );
    let function = format!(
        r#"find r.id from (r:Receta) where {} and in_community(r, "{}", via = "USA")"#,
        similar(1000),
        fx.key
    );
    let a: BTreeSet<NodeId> = seeds(&run(&fx, &explicit).await).into_iter().collect();
    let b: BTreeSet<NodeId> = seeds(&run(&fx, &function).await).into_iter().collect();
    assert!(!a.is_empty());
    assert_eq!(a, b);
    assert!(a.len() < fx.recipe.len());
}

#[tokio::test]
async fn seed_order_survives_a_reordered_join() {
    let fx = fixture().await;
    // `(g:Region)` shares no variable with the first pattern, so the join
    // goes 1, 3, 2; the rows still come in seed order.
    let q = format!(
        r#"find r.id, g.nombre, score(r) from (r:Receta), (g:Region), (r)-[:DE]->(h:Region) where {} and g.nombre = h.nombre"#,
        hybrid(6)
    );
    let rows = run(&fx, &q).await;
    let all: HashSet<NodeId> = fx.recipe.keys().copied().collect();
    let mut hq = HybridQuery::new();
    hq.text = Some("sopa".into());
    hq.vector = Some((query_vector(), "m".into()));
    hq.k = 6;
    hq.filter = Some(HybridFilter { label: Some("Receta".into()), props: Vec::new() });
    hq.candidates = all.iter().copied().collect::<CandidateSet>();
    let want: Vec<NodeId> = fx.graph.search_hybrid(hq).await.unwrap().into_iter().map(|h| h.node_id).collect();
    assert_eq!(rows.len(), 6, "one region per recipe");
    let ids: Vec<NodeId> = rows.iter().map(|r| id_of(r.get("r.id"))).collect();
    assert_eq!(ids, want);
    let plan = explain(&fx, &format!("explain {q}")).await;
    assert!(plan.contains("order 1, 3, 2"), "{plan}");
}

#[tokio::test]
async fn aggregations_count_the_rows_of_the_seeds() {
    let fx = fixture().await;
    let rows_q = format!(
        r#"find r.id, i.nombre from (r:Receta), (r)-[:USA]->(i:Ingrediente) where {} and i.temporada = "invierno""#,
        similar(8)
    );
    let rows = run(&fx, &rows_q).await;
    let count_q = format!(
        r#"find count(*) as n from (r:Receta), (r)-[:USA]->(i:Ingrediente) where {} and i.temporada = "invierno""#,
        similar(8)
    );
    let count = run(&fx, &count_q).await;
    assert_eq!(count[0].get("n"), Some(&PropertyValue::Int(rows.len() as i64)));

    let grouped_q = format!(
        r#"find i.nombre, count(*) as n from (r:Receta), (r)-[:USA]->(i:Ingrediente) where {} and i.temporada = "invierno" group by i.nombre"#,
        similar(8)
    );
    let mut want: HashMap<String, i64> = HashMap::new();
    for r in &rows {
        if let Some(PropertyValue::String(n)) = r.get("i.nombre") {
            *want.entry(n.clone()).or_default() += 1;
        }
    }
    let got: HashMap<String, i64> = run(&fx, &grouped_q)
        .await
        .into_iter()
        .map(|r| match (r.get("i.nombre"), r.get("n")) {
            (Some(PropertyValue::String(n)), Some(PropertyValue::Int(c))) => (n.clone(), *c),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(got, want);
}

#[tokio::test]
async fn explain_names_the_universe_and_its_size() {
    let fx = fixture().await;
    let q = format!(
        r#"explain find r.id from (r:Receta), (r)-[:USA]->(i:Ingrediente) where {} and i.temporada = "invierno""#,
        similar(5)
    );
    let plan = explain(&fx, &q).await;
    let n = winter_universe(&fx).len();
    assert!(plan.contains("Search universe from patterns"), "{plan}");
    assert!(plan.contains("pattern 2"), "{plan}");
    assert!(plan.contains(&format!("({n} nodes)")), "{plan}");
}

#[tokio::test]
async fn misplaced_searches_stay_named_errors() {
    let fx = fixture().await;
    let v = literal(&query_vector());
    let first = "must be the first node of the first pattern";
    let errors = [
        // The searched variable does not open the first pattern.
        (format!(r#"find r.id from (i:Ingrediente), (r:Receta)-[:USA]->(i) where {}"#, similar(5)), first),
        (
            format!(r#"find r.id from (i:Ingrediente)<-[:USA]-(r:Receta), (r)-[:DE]->(g:Region) where {}"#, similar(5)),
            first,
        ),
        // A second search, on another variable.
        (
            format!(
                r#"find r.id from (r:Receta), (r)-[:USA]->(i:Ingrediente) where {} and similar_to(i, vector = {v}, model = "m", k = 3)"#,
                similar(5)
            ),
            first,
        ),
        // A search has no history.
        (
            format!(r#"find r.id from (r:Receta), (r)-[:USA]->(i:Ingrediente) where {} at timestamp 1"#, similar(5)),
            "AT TIMESTAMP",
        ),
        // Under `or`.
        (
            format!(
                r#"find r.id from (r:Receta), (r)-[:USA]->(i:Ingrediente) where {} or i.temporada = "invierno""#,
                similar(5)
            ),
            "top-level AND",
        ),
    ];
    for (q, why) in errors {
        let err = fx.graph.execute_nql(&q).await.expect_err(&q).to_string();
        assert!(err.contains(why), "{q}\n{err}");
    }
}

