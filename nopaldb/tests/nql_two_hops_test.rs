// tests/nql_two_hops_test.rs
//
// #231: `hybrid` / `similar_to` seed a linear two-hop chain
// `(a)-[:T1]->(b)-[:T2]->(c)` with `a` searched. The multi-hop of GraphRAG
// ("chunk → entity → related entity") used to need Python (`search_hybrid` +
// `neighborhood`) and lost the query's `score`, `limit` and EXPLAIN.
//
// Fictional domain: Chunk -[:MENTIONS]-> Entity -[:RELATED]-> Entity. The
// edges are directed, so the only route to depth 2 from a chunk is
// MENTIONS then RELATED.
//
// Run with: cargo test --features core --test nql_two_hops_test

use std::collections::{BTreeSet, HashMap};

use nopaldb::index::IndexType;
use nopaldb::types::{Edge, Node, NodeId, PropertyValue};
use nopaldb::{Direction, ExpandOptions, Graph, HybridFilter, HybridQuery};

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

const CHUNKS: [(&str, &str, [f32; 3]); 4] = [
    ("c0", "riego por goteo", [1.0, 0.0, 0.0]),
    ("c1", "riego y drenaje", [0.95, 0.31, 0.0]),
    ("c2", "plagas del nopal", [0.80, 0.60, 0.0]),
    ("c3", "cosecha de tuna", [0.0, 1.0, 0.0]),
];
const ENTITIES: [&str; 5] = ["riego", "agua", "nopal", "plaga", "tuna"];
/// chunk → entities it mentions (indexes into ENTITIES).
const MENTIONS: [&[usize]; 4] = [&[0], &[0, 1], &[2, 3], &[4]];
/// entity → related entities.
const RELATED: [&[usize]; 5] = [&[1, 2], &[2], &[3, 4], &[], &[2]];

struct Fx {
    graph: Graph,
    chunks: Vec<NodeId>,
    entities: Vec<NodeId>,
}

async fn fixture() -> Fx {
    let graph = Graph::in_memory().await.unwrap();
    let mut entities = vec![];
    for name in ENTITIES {
        entities.push(graph.add_node(Node::new("Entity").with_property("name", s(name))).await.unwrap());
    }
    for (e, rel) in RELATED.iter().enumerate() {
        for &r in rel.iter() {
            graph.add_edge(Edge::new(entities[e], entities[r], "RELATED")).await.unwrap();
        }
    }
    let mut chunks = vec![];
    for (i, (name, text, v)) in CHUNKS.iter().enumerate() {
        let id = graph.add_node(Node::new("Chunk").with_property("name", s(name)).with_property("text", s(text))).await.unwrap();
        graph.add_node_embedding(id, v.to_vec(), "m").await.unwrap();
        for &e in MENTIONS[i] {
            graph.add_edge(Edge::new(id, entities[e], "MENTIONS")).await.unwrap();
        }
        chunks.push(id);
    }
    graph.create_index("Chunk", "text", IndexType::FullText).await.unwrap();
    Fx { graph, chunks, entities }
}

/// `(c, f)` pairs the chain yields for a seed, computed by hand from the fixture.
fn expected_pairs(fx: &Fx, seed: NodeId) -> BTreeSet<(NodeId, NodeId)> {
    let i = fx.chunks.iter().position(|c| *c == seed).unwrap();
    MENTIONS[i]
        .iter()
        .flat_map(|&e| RELATED[e].iter().copied())
        .map(|f| (seed, fx.entities[f]))
        .collect()
}

fn id_of(v: &PropertyValue) -> NodeId {
    match v {
        PropertyValue::String(x) => x.parse().unwrap(),
        other => panic!("not an id: {other:?}"),
    }
}

/// What the chain's rows say: the `(c, f)` pairs, `score(c)` per seed, and
/// the seeds in the order they appear.
struct ChainRows {
    pairs: BTreeSet<(NodeId, NodeId)>,
    scores: HashMap<NodeId, f64>,
    seed_order: Vec<NodeId>,
}

fn chain_rows(result: &nopaldb::query::nql::QueryResult) -> ChainRows {
    let mut pairs = BTreeSet::new();
    let mut scores = HashMap::new();
    let mut seed_order = Vec::new();
    for r in result.rows() {
        let c = id_of(r.get("c.id").unwrap());
        let f = id_of(r.get("f.id").unwrap());
        pairs.insert((c, f));
        if let Some(PropertyValue::Float(sc)) = r.get("score(c)") {
            scores.insert(c, *sc);
        }
        if seed_order.last() != Some(&c) {
            seed_order.push(c);
        }
    }
    ChainRows { pairs, scores, seed_order }
}

const K: usize = 2;

fn hybrid_query() -> HybridQuery {
    let mut hq = HybridQuery::new();
    hq.text = Some("riego".into());
    hq.vector = Some((vec![1.0, 0.0, 0.0], "m".into()));
    hq.k = K;
    hq.filter = Some(HybridFilter { label: Some("Chunk".into()), props: vec![] });
    hq
}

/// The two-hop rows are exactly `search_hybrid` chained with the expansion
/// of each seed, as sets of `(seed, f)`; `score(c)` is the seed's score.
#[tokio::test]
async fn hybrid_two_hops_equals_search_hybrid_plus_expansion() {
    let fx = fixture().await;
    let q = format!(
        r#"find c.id, f.id, score(c) from (c:Chunk)-[:MENTIONS]->(e:Entity)-[:RELATED]->(f:Entity) where hybrid(c, text = "riego", vector = [1, 0, 0], model = "m", k = {K})"#
    );
    let result = fx.graph.execute_nql(&q).await.unwrap();
    let ChainRows { pairs, scores, seed_order } = chain_rows(&result);

    let hits = fx.graph.search_hybrid(hybrid_query()).await.unwrap();
    assert_eq!(hits.len(), K);
    let mut expected = BTreeSet::new();
    for hit in &hits {
        expected.extend(expected_pairs(&fx, hit.node_id));
        // neighborhood(depth = 2) from the seed over MENTIONS/RELATED: every
        // node it reaches at depth 2 is a row of the chain.
        let opts = ExpandOptions {
            direction: Direction::Outgoing,
            edge_types: Some(vec!["MENTIONS".into(), "RELATED".into()]),
            labels: Some(vec!["Entity".into()]),
            ..Default::default()
        };
        let nb = fx.graph.neighborhood(&[hit.node_id], 2, &opts).await.unwrap();
        for (node, depth) in &nb.depth_of {
            if *depth == 2 {
                assert!(pairs.contains(&(hit.node_id, *node)), "depth-2 node missing from the chain");
            }
        }
        let got = scores.get(&hit.node_id).copied().unwrap_or(f64::NAN);
        assert!((got - hit.score as f64).abs() < 1e-6, "score(c) of seed = search_hybrid score: {got} vs {}", hit.score);
    }
    assert_eq!(pairs, expected);
    assert!(!pairs.is_empty());
    // Seeds come out best first, as search_hybrid orders them.
    let expected_order: Vec<NodeId> = hits.iter().map(|h| h.node_id).filter(|h| seed_order.contains(h)).collect();
    assert_eq!(seed_order, expected_order);
    // The auxiliary id column is the one the query asked for: no extra columns.
    assert_eq!(result.columns, ["c.id", "f.id", "score(c)"]);
}

#[tokio::test]
async fn similar_to_two_hops_and_raw_scores() {
    let fx = fixture().await;
    let q = format!(
        r#"find c.name, f.name, score(c), rank(c), vector_score(c), text_score(c) from (c:Chunk)-[:MENTIONS]->(e:Entity)-[:RELATED]->(f:Entity) where similar_to(c, vector = [1, 0, 0], model = "m", k = {K})"#
    );
    let result = fx.graph.execute_nql(&q).await.unwrap();
    assert_eq!(result.columns, ["c.name", "f.name", "score(c)", "rank(c)", "vector_score(c)", "text_score(c)"]);
    assert!(!result.rows().is_empty());
    // c0 and c1 are the two nearest; c0 mentions riego → {agua, nopal};
    // c1 mentions riego, agua → {agua, nopal} ∪ {nopal}.
    let mut names: Vec<(String, String)> = result
        .rows()
        .iter()
        .map(|r| (r.get("c.name").unwrap().as_str().unwrap().to_string(), r.get("f.name").unwrap().as_str().unwrap().to_string()))
        .collect();
    names.sort();
    names.dedup();
    assert_eq!(
        names,
        vec![("c0".into(), "agua".into()), ("c0".into(), "nopal".into()), ("c1".into(), "agua".into()), ("c1".into(), "nopal".into())]
    );
    for r in result.rows() {
        let PropertyValue::Float(score) = r.get("score(c)").unwrap() else { panic!("{r:?}") };
        let PropertyValue::Float(vs) = r.get("vector_score(c)").unwrap() else { panic!("{r:?}") };
        assert!((score - vs).abs() < 1e-9, "similar_to: vector_score = score");
        assert_eq!(r.get("text_score(c)"), Some(&PropertyValue::Null));
        let PropertyValue::Int(rank) = r.get("rank(c)").unwrap() else { panic!("{r:?}") };
        let name = r.get("c.name").unwrap().as_str().unwrap();
        assert_eq!(*rank, if name == "c0" { 0 } else { 1 });
    }
    // No auxiliary id column leaks.
    assert!(result.rows().iter().all(|r| r.get("c.id").is_none()));
}

#[tokio::test]
async fn k_bounds_the_seeds_and_limit_the_expanded_rows() {
    let fx = fixture().await;
    // k = 3 seeds, LIMIT 2 rows: fewer rows than seeds expanded, and the
    // rows kept are the best seed's.
    let q = r#"find c.name, f.name from (c:Chunk)-[:MENTIONS]->(e:Entity)-[:RELATED]->(f:Entity) where similar_to(c, vector = [1, 0, 0], model = "m", k = 3) limit 2"#;
    let result = fx.graph.execute_nql(q).await.unwrap();
    assert_eq!(result.rows().len(), 2);
    assert!(result.rows().iter().all(|r| r.get("c.name") == Some(&s("c0"))), "{:?}", result.rows());

    // Without LIMIT, k = 1 expands only the best seed.
    let q = r#"find c.name, f.name from (c:Chunk)-[:MENTIONS]->(e:Entity)-[:RELATED]->(f:Entity) where similar_to(c, vector = [1, 0, 0], model = "m", k = 1)"#;
    let result = fx.graph.execute_nql(q).await.unwrap();
    assert_eq!(result.rows().len(), 2, "{:?}", result.rows());
    assert!(result.rows().iter().all(|r| r.get("c.name") == Some(&s("c0"))));

    // The WHERE predicates on b / c are expansion constraints: they prune
    // rows, and never change which seeds the search picks (k = seeds).
    // One row per path, as in any pattern query: c1 reaches nopal through
    // riego and through agua, so it has two rows; `distinct` collapses them.
    let q = r#"find c.name, f.name from (c:Chunk)-[:MENTIONS]->(e:Entity)-[:RELATED]->(f:Entity) where similar_to(c, vector = [1, 0, 0], model = "m", k = 2) and f.name = "nopal""#;
    let result = fx.graph.execute_nql(q).await.unwrap();
    assert!(result.rows().iter().all(|r| r.get("f.name") == Some(&s("nopal"))), "{:?}", result.rows());
    assert_eq!(result.rows().len(), 3, "one row per path: {:?}", result.rows());
    let result = fx.graph.execute_nql(&q.replace("find c.name", "find distinct c.name")).await.unwrap();
    assert_eq!(result.rows().len(), 2, "one per seed with distinct: {:?}", result.rows());
}

#[tokio::test]
async fn explain_says_seeds_hops_and_limit() {
    let fx = fixture().await;
    let q = r#"find c.name, f.name from (c:Chunk)-[:MENTIONS]->(e:Entity)-[:RELATED]->(f:Entity) where hybrid(c, text = "riego", vector = [1, 0, 0], model = "m", k = 2) limit 5"#;
    let plan = format!("{:?}", fx.graph.execute_nql(&format!("explain {q}")).await.unwrap().rows());
    assert!(plan.contains("PATTERN PIPELINE (seed: HYBRID, hops: 2)"), "{plan}");
    assert!(plan.contains("Seeds: k = 2 candidates of c (hybrid); hops: 2; LIMIT (5) applies to the expanded rows"), "{plan}");

    let q1 = r#"find c.name, e.name from (c:Chunk)-[:MENTIONS]->(e:Entity) where similar_to(c, vector = [1, 0, 0], model = "m")"#;
    let plan = format!("{:?}", fx.graph.execute_nql(&format!("explain {q1}")).await.unwrap().rows());
    assert!(plan.contains("PATTERN PIPELINE (seed: SIMILAR_TO)") && !plan.contains("hops: 1)"), "one hop keeps the old wording: {plan}");
    assert!(plan.contains("Seeds: k = 10 candidates of c (similar_to); hops: 1; LIMIT (none)"), "{plan}");
}

#[tokio::test]
async fn id_seed_also_expands_two_hops() {
    // Side effect of seeding the linear executor: `where c.id = …` seeds a
    // two-hop chain too, instead of scanning the label.
    let fx = fixture().await;
    let q = format!(
        r#"find f.name from (c:Chunk)-[:MENTIONS]->(e:Entity)-[:RELATED]->(f:Entity) where c.id = "{}""#,
        fx.chunks[2]
    );
    let result = fx.graph.execute_nql(&q).await.unwrap();
    let fs: BTreeSet<String> = result.rows().iter().map(|r| r.get("f.name").unwrap().as_str().unwrap().to_string()).collect();
    // c2 mentions nopal {plaga, tuna} and plaga {} → {plaga, tuna}.
    assert_eq!(fs, BTreeSet::from(["plaga".to_string(), "tuna".to_string()]));
    let plan = format!("{:?}", fx.graph.execute_nql(&format!("explain {q}")).await.unwrap().rows());
    assert!(plan.contains("PATTERN PIPELINE (seed: ID LOOKUP, hops: 2)"), "{plan}");
}
