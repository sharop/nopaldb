//! Communities and neighbourhoods as the universe of a search (#214 phase 2b).
//!
//! - `community_members(key)` is exactly the set the NQL pattern
//!   `(e)-[:IN_COMMUNITY]->(:Community {key})` returns;
//! - `community_members_via(key, "MENTIONS")` is exactly the set of chunks
//!   that the two-hop pattern `(c)-[:MENTIONS]->(e)-[:IN_COMMUNITY]->(k)`
//!   returns. These are equal as **sets**: the pattern gives one row per
//!   path (`docs/NQL_SEMANTICS.md` § 1);
//! - a search over those candidates returns the exact top-k among them;
//! - `ExpandOptions::within` restricts the walk, like `labels`.
//!
//! Fictional domain: plant entities in two communities, recipe chunks that
//! mention them.

use std::collections::{BTreeSet, HashMap};

use nopaldb::graph::communities::CommunityMaterializeOptions;
use nopaldb::types::{Edge, Node, NodeId, PropertyValue};
use nopaldb::{CandidateSet, ExpandOptions, Graph, HybridQuery};

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

fn vector_of(i: usize) -> Vec<f32> {
    let a = i as f32 * 0.37;
    let v = vec![a.cos(), a.sin(), (i % 5) as f32 / 10.0, 0.2];
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.into_iter().map(|x| x / n).collect()
}

struct Fx {
    graph: Graph,
    /// Community key → its entities.
    members: HashMap<String, BTreeSet<NodeId>>,
    /// Chunk → (vector, the entities it mentions).
    chunks: HashMap<NodeId, (Vec<f32>, Vec<NodeId>)>,
}

/// 12 entities in two communities (by parity), 40 chunks that mention one or
/// two entities each.
async fn fixture() -> Fx {
    let graph = Graph::in_memory().await.unwrap();
    let mut entities = Vec::new();
    for i in 0..12 {
        entities.push(graph.add_node(Node::new("Entity").with_property("name", s(&format!("planta-{i}")))).await.unwrap());
    }
    // Edges inside each community, so a neighbourhood test has something to walk.
    for i in 0..10 {
        graph.add_edge(Edge::new(entities[i], entities[i + 2], "RELATED")).await.unwrap();
    }
    let level: HashMap<NodeId, usize> = entities.iter().enumerate().map(|(i, id)| (*id, i % 2)).collect();
    graph.materialize_communities(&[level], &CommunityMaterializeOptions::default()).await.unwrap();

    let mut chunks = HashMap::new();
    for i in 0..40 {
        let id = graph.add_node(Node::new("Chunk").with_property("text", s("nopal en la ladera"))).await.unwrap();
        let v = vector_of(i);
        graph.add_node_embedding(id, v.clone(), "m").await.unwrap();
        let mentioned: Vec<NodeId> = if i % 7 == 0 {
            vec![entities[i % 12], entities[(i + 1) % 12]]
        } else {
            vec![entities[i % 12]]
        };
        for e in &mentioned {
            graph.add_edge(Edge::new(id, *e, "MENTIONS")).await.unwrap();
        }
        chunks.insert(id, (v, mentioned));
    }

    // Community keys and their members, read back through NQL.
    let mut members: HashMap<String, BTreeSet<NodeId>> = HashMap::new();
    let rows = graph
        .execute_nql("find e.id, k.key from (e:Entity)-[:IN_COMMUNITY]->(k:Community)")
        .await
        .unwrap();
    for r in rows.rows() {
        let (Some(PropertyValue::String(e)), Some(PropertyValue::String(k))) = (r.get("e.id"), r.get("k.key")) else {
            panic!("{r:?}")
        };
        members.entry(k.clone()).or_default().insert(e.parse().unwrap());
    }
    assert_eq!(members.len(), 2);
    Fx { graph, members, chunks }
}

#[tokio::test]
async fn community_members_are_the_pattern_s_set() {
    let fx = fixture().await;
    for (key, want) in &fx.members {
        let got: BTreeSet<NodeId> = fx.graph.community_members(key).await.unwrap().into_iter().collect();
        assert_eq!(&got, want, "{key}");
        assert_eq!(got.len(), 6);
    }
}

#[tokio::test]
async fn members_via_mentions_are_the_two_hop_pattern_s_set() {
    let fx = fixture().await;
    for (key, members) in &fx.members {
        let got: Vec<NodeId> = fx.graph.community_members_via(key, "MENTIONS").await.unwrap();
        let got_set: BTreeSet<NodeId> = got.iter().copied().collect();
        assert_eq!(got.len(), got_set.len(), "each chunk once");

        // The same universe, through the pattern (one row per path: compare sets).
        let q = format!(
            r#"find c.id from (c:Chunk)-[:MENTIONS]->(e:Entity)-[:IN_COMMUNITY]->(k:Community) where k.key = "{key}""#
        );
        let via_pattern: BTreeSet<NodeId> = fx
            .graph
            .execute_nql(&q)
            .await
            .unwrap()
            .rows()
            .iter()
            .map(|r| match r.get("c.id") {
                Some(PropertyValue::String(x)) => x.parse().unwrap(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(got_set, via_pattern, "{key}");

        // And by construction.
        let want: BTreeSet<NodeId> = fx
            .chunks
            .iter()
            .filter(|(_, (_, mentioned))| mentioned.iter().any(|e| members.contains(e)))
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(got_set, want, "{key}");
    }
}

#[tokio::test]
async fn a_search_inside_a_community_is_the_exact_top_k_of_its_chunks() {
    let fx = fixture().await;
    let key = fx.members.keys().next().unwrap().clone();
    let universe = fx.graph.community_members_via(&key, "MENTIONS").await.unwrap();
    let query = vector_of(3);

    let mut q = HybridQuery::new();
    q.vector = Some((query.clone(), "m".into()));
    q.k = 5;
    q.candidates = universe.iter().copied().collect::<CandidateSet>();
    let hits: Vec<NodeId> = fx.graph.search_hybrid(q).await.unwrap().into_iter().map(|h| h.node_id).collect();

    let mut brute: Vec<(NodeId, f32)> = universe
        .iter()
        .map(|id| {
            let v = &fx.chunks[id].0;
            (*id, 1.0 - v.iter().zip(&query).map(|(a, b)| a * b).sum::<f32>())
        })
        .collect();
    brute.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let want: Vec<NodeId> = brute.into_iter().take(5).map(|(id, _)| id).collect();
    assert_eq!(hits, want, "exact top-k inside the community's chunks");
}

#[tokio::test]
async fn an_unknown_community_is_an_error() {
    let fx = fixture().await;
    let err = fx.graph.community_members("leiden/L0/no-existe").await.expect_err("unknown key").to_string();
    assert!(err.contains("not found"), "{err}");
    assert!(fx.graph.community_members_via("leiden/L0/no-existe", "MENTIONS").await.is_err());
}

#[tokio::test]
async fn within_restricts_the_walk_like_labels() {
    let g = Graph::in_memory().await.unwrap();
    let a = g.add_node(Node::new("P")).await.unwrap();
    let b = g.add_node(Node::new("P")).await.unwrap();
    let c = g.add_node(Node::new("P")).await.unwrap();
    let d = g.add_node(Node::new("P")).await.unwrap();
    for (x, y) in [(a, b), (b, c), (a, d)] {
        g.add_edge(Edge::new(x, y, "T")).await.unwrap();
    }
    let ids = |nb: &nopaldb::Neighborhood| nb.nodes.iter().map(|n| n.id).collect::<BTreeSet<_>>();

    // Unrestricted: the whole chain.
    let all = g.neighborhood(&[a], 2, &ExpandOptions::default()).await.unwrap();
    assert_eq!(ids(&all), [a, b, c, d].into_iter().collect());

    // `within = {b, c}`: d is outside; the seed a is never filtered.
    let opts = ExpandOptions { within: Some([b, c].into_iter().collect()), ..Default::default() };
    let nb = g.neighborhood(&[a], 2, &opts).await.unwrap();
    assert_eq!(ids(&nb), [a, b, c].into_iter().collect());

    // `within = {c}`: b is outside, so c is unreachable — the walk is restricted, not just the result.
    let opts = ExpandOptions { within: Some([c].into_iter().collect()), ..Default::default() };
    let nb = g.neighborhood(&[a], 2, &opts).await.unwrap();
    assert_eq!(ids(&nb), [a].into_iter().collect());

    // A neighbourhood is a universe.
    let set = CandidateSet::from(&all);
    assert!(set.contains(&c) && set.contains(&a));
}
