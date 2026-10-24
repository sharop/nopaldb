//! NQL `in_community(v, key[, via = "T"])` (#214 phase 2b, PR 3).
//!
//! A seed constraint (`docs/NQL_SEMANTICS.md` § 2). The search on `v` ranks
//! inside the community, or inside the nodes that point at its members
//! through `via`. It is sugar for the universe of `Graph::community_members`
//! / `community_members_via`, and equals, as a set, what the explicit pattern
//! `(c)-[:MENTIONS]->(e)-[:IN_COMMUNITY]->(k:Community {key})` returns.
//!
//! Fictional domain: plant entities in two communities, recipe chunks that
//! mention them.
#![cfg(feature = "hybrid")]

use std::collections::{BTreeSet, HashMap};

use nopaldb::graph::communities::CommunityMaterializeOptions;
use nopaldb::index::IndexType;
use nopaldb::types::{Edge, Node, NodeId, PropertyValue};
use nopaldb::{CandidateSet, Graph, HybridQuery};

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

fn vector_of(i: usize) -> Vec<f32> {
    let a = i as f32 * 0.37;
    let v = vec![a.cos(), a.sin(), (i % 5) as f32 / 10.0, 0.2];
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.into_iter().map(|x| x / n).collect()
}

fn literal(v: &[f32]) -> String {
    format!("[{}]", v.iter().map(|x| format!("{x:.6}")).collect::<Vec<_>>().join(", "))
}

struct Fx {
    graph: Graph,
    key: String,
    /// chunk → vector, for the brute-force check.
    vectors: HashMap<NodeId, Vec<f32>>,
}

async fn fixture() -> Fx {
    let graph = Graph::in_memory().await.unwrap();
    let mut entities = Vec::new();
    for i in 0..12 {
        let id = graph.add_node(Node::new("Entity").with_property("name", s(&format!("planta-{i}")))).await.unwrap();
        graph.add_node_embedding(id, vector_of(100 + i), "m").await.unwrap();
        entities.push(id);
    }
    let level: HashMap<NodeId, usize> = entities.iter().enumerate().map(|(i, id)| (*id, i % 2)).collect();
    graph.materialize_communities(&[level], &CommunityMaterializeOptions::default()).await.unwrap();
    let mut vectors = HashMap::new();
    for i in 0..40 {
        let id = graph.add_node(Node::new("Chunk").with_property("text", s("nopal en la ladera"))).await.unwrap();
        let v = vector_of(i);
        graph.add_node_embedding(id, v.clone(), "m").await.unwrap();
        graph.add_edge(Edge::new(id, entities[i % 12], "MENTIONS")).await.unwrap();
        if i % 7 == 0 {
            graph.add_edge(Edge::new(id, entities[(i + 1) % 12], "MENTIONS")).await.unwrap();
        }
        vectors.insert(id, v);
    }
    graph.create_index("Chunk", "text", IndexType::FullText).await.unwrap();
    let rows = graph
        .execute_nql(r#"find k.key from (e:Entity)-[:IN_COMMUNITY]->(k:Community) where e.name = "planta-0""#)
        .await
        .unwrap();
    let key = match rows.rows()[0].get("k.key") {
        Some(PropertyValue::String(k)) => k.clone(),
        other => panic!("{other:?}"),
    };
    Fx { graph, key, vectors }
}

fn ids(result: &nopaldb::query::nql::executor::result::QueryResult, col: &str) -> Vec<NodeId> {
    result
        .rows()
        .iter()
        .map(|r| match r.get(col) {
            Some(PropertyValue::String(x)) => x.parse().unwrap(),
            other => panic!("{col}: {other:?}"),
        })
        .collect()
}

#[tokio::test]
async fn hybrid_inside_a_community_equals_search_hybrid_with_its_candidates() {
    let fx = fixture().await;
    let q = vector_of(3);
    let nql = format!(
        r#"find c.id, score(c) from (c:Chunk) where hybrid(c, text = "nopal", vector = {}, model = "m", k = 5) and in_community(c, "{}", via = "MENTIONS")"#,
        literal(&q),
        fx.key
    );
    let rows = fx.graph.execute_nql(&nql).await.unwrap();
    let got = ids(&rows, "c.id");

    let universe = fx.graph.community_members_via(&fx.key, "MENTIONS").await.unwrap();
    let mut hq = HybridQuery::new();
    hq.text = Some("nopal".into());
    hq.vector = Some((rows_vector(&q), "m".into()));
    hq.k = 5;
    hq.filter = Some(nopaldb::HybridFilter { label: Some("Chunk".into()), props: Vec::new() });
    hq.candidates = universe.iter().copied().collect::<CandidateSet>();
    let want: Vec<NodeId> = fx.graph.search_hybrid(hq).await.unwrap().into_iter().map(|h| h.node_id).collect();
    assert_eq!(got, want, "same top-k, same order");
    assert_eq!(got.len(), 5);
    assert!(got.iter().all(|id| universe.contains(id)));
}

/// The NQL literal goes through text with 6 decimals: compare with the same vector.
fn rows_vector(v: &[f32]) -> Vec<f32> {
    v.iter().map(|x| format!("{x:.6}").parse().unwrap()).collect()
}

#[tokio::test]
async fn similar_to_inside_a_community_is_the_exact_top_k() {
    let fx = fixture().await;
    let q = rows_vector(&vector_of(11));
    let nql = format!(
        r#"find c.id from (c:Chunk) where similar_to(c, vector = {}, model = "m", k = 4) and in_community(c, "{}", via = "MENTIONS")"#,
        literal(&q),
        fx.key
    );
    let got = ids(&fx.graph.execute_nql(&nql).await.unwrap(), "c.id");
    let universe = fx.graph.community_members_via(&fx.key, "MENTIONS").await.unwrap();
    let mut brute: Vec<(NodeId, f32)> = universe
        .iter()
        .map(|id| (*id, 1.0 - fx.vectors[id].iter().zip(&q).map(|(a, b)| a * b).sum::<f32>()))
        .collect();
    brute.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let want: Vec<NodeId> = brute.into_iter().take(4).map(|(id, _)| id).collect();
    assert_eq!(got, want);
}

#[tokio::test]
async fn the_universe_equals_the_explicit_pattern_as_a_set() {
    let fx = fixture().await;
    let nql = format!(
        r#"find c.id from (c:Chunk) where similar_to(c, vector = {}, model = "m", k = 100) and in_community(c, "{}", via = "MENTIONS")"#,
        literal(&vector_of(0)),
        fx.key
    );
    let via_function: BTreeSet<NodeId> = ids(&fx.graph.execute_nql(&nql).await.unwrap(), "c.id").into_iter().collect();
    let pattern = format!(
        r#"find c.id from (c:Chunk)-[:MENTIONS]->(e:Entity)-[:IN_COMMUNITY]->(k:Community) where k.key = "{}""#,
        fx.key
    );
    let via_pattern: BTreeSet<NodeId> = ids(&fx.graph.execute_nql(&pattern).await.unwrap(), "c.id").into_iter().collect();
    assert!(!via_pattern.is_empty());
    assert_eq!(via_function, via_pattern, "same universe (sets: the pattern gives one row per path)");

    // Without `via`: the members themselves, for a search over entities.
    let nql = format!(
        r#"find e.id from (e:Entity) where similar_to(e, vector = {}, model = "m", k = 100) and in_community(e, "{}")"#,
        literal(&vector_of(100)),
        fx.key
    );
    let members: BTreeSet<NodeId> = ids(&fx.graph.execute_nql(&nql).await.unwrap(), "e.id").into_iter().collect();
    let want: BTreeSet<NodeId> = fx.graph.community_members(&fx.key).await.unwrap().into_iter().collect();
    assert_eq!(members, want);
}

#[tokio::test]
async fn in_community_intersects_with_id_lists() {
    let fx = fixture().await;
    let universe = fx.graph.community_members_via(&fx.key, "MENTIONS").await.unwrap();
    let outside: NodeId = *fx.vectors.keys().find(|id| !universe.contains(id)).unwrap();
    let chosen = [universe[0], universe[1], outside];
    let list = chosen.iter().map(|id| format!("\"{id}\"")).collect::<Vec<_>>().join(", ");
    let nql = format!(
        r#"find c.id from (c:Chunk) where similar_to(c, vector = {}, model = "m", k = 10) and in_community(c, "{}", via = "MENTIONS") and c.id in [{list}]"#,
        literal(&vector_of(0)),
        fx.key
    );
    let got: BTreeSet<NodeId> = ids(&fx.graph.execute_nql(&nql).await.unwrap(), "c.id").into_iter().collect();
    assert_eq!(got, [universe[0], universe[1]].into_iter().collect(), "the intersection, never wider");
}

#[tokio::test]
async fn explain_names_the_community() {
    let fx = fixture().await;
    let nql = format!(
        r#"explain find c.id from (c:Chunk) where similar_to(c, vector = {}, model = "m", k = 3) and in_community(c, "{}", via = "MENTIONS")"#,
        literal(&vector_of(0)),
        fx.key
    );
    let plan = format!("{:?}", fx.graph.execute_nql(&nql).await.unwrap().rows());
    assert!(plan.contains("Search universe") && plan.contains("in_community(c,"), "{plan}");
}

#[tokio::test]
async fn misplaced_or_malformed_in_community_is_a_named_error() {
    let fx = fixture().await;
    let v = literal(&vector_of(0));
    let key = &fx.key;
    let search = format!(r#"similar_to(c, vector = {v}, model = "m", k = 3)"#);
    for (q, needle) in [
        (format!(r#"find c.id from (c:Chunk) where in_community(c, "{key}")"#), "top-level AND"),
        (format!(r#"find c.id from (c:Chunk) where {search} and (c.text = "x" or in_community(c, "{key}"))"#), "top-level AND"),
        (format!(r#"find c.id from (c:Chunk)-[:MENTIONS]->(e:Entity) where {search} and in_community(e, "{key}")"#), "same variable"),
        (format!(r#"find c.id from (c:Chunk) where {search} and in_community(c, "{key}", hops = 2)"#), "unknown option"),
        (format!(r#"find c.id from (c:Chunk) where {search} and in_community(c)"#), "expected"),
        (format!(r#"find c.id, in_community(c, "{key}") from (c:Chunk) where {search}"#), "top-level AND"),
    ] {
        let err = match fx.graph.execute_nql(&q).await {
            Ok(_) => panic!("{q} should fail"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("in_community") && err.contains(needle), "{q}\n{err}");
    }
    let unknown = format!(r#"find c.id from (c:Chunk) where {search} and in_community(c, "leiden/L0/no-existe")"#);
    let err = fx.graph.execute_nql(&unknown).await.expect_err("unknown key").to_string();
    assert!(err.contains("not found"), "{err}");
}
