// Tests for hybrid search (issue M1-5): RRF fusion of full-text + vector.

use nopaldb::index::IndexType;
use nopaldb::types::{Node, PropertyValue};
use nopaldb::{Graph, HybridFilter, HybridQuery};

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

/// Build a graph with Doc nodes (name/body + embedding) and a fulltext index on
/// Doc(body). Returns (graph, ids-by-name).
async fn fixture() -> (Graph, std::collections::HashMap<String, nopaldb::types::NodeId>) {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await.unwrap();

    // (name, body, vector)
    let docs = [
        ("d0", "apple banana", [1.0f32, 0.0, 0.0]),
        ("d1", "banana cherry", [0.0, 1.0, 0.0]),
        ("d2", "apple cherry", [0.9, 0.1, 0.0]),
        ("d3", "durian", [0.0, 0.0, 1.0]),
    ];
    let mut ids = std::collections::HashMap::new();
    for (name, body, vec) in docs {
        let node = Node::new("Doc")
            .with_property("name", s(name))
            .with_property("body", s(body));
        let id = graph.add_node(node).await.unwrap();
        graph.add_node_embedding(id, vec.to_vec(), "m").await.unwrap();
        ids.insert(name.to_string(), id);
    }

    // A node of a different label that would rank high but must be filtered out.
    let other = Node::new("Other")
        .with_property("name", s("x0"))
        .with_property("body", s("apple apple"));
    let ox = graph.add_node(other).await.unwrap();
    graph.add_node_embedding(ox, vec![1.0, 0.0, 0.0], "m").await.unwrap();
    ids.insert("x0".to_string(), ox);

    // Full-text index over the body property (populates from existing nodes).
    graph.create_index("Doc", "body", IndexType::FullText).await.unwrap();

    (graph, ids)
}

#[tokio::test]
async fn rrf_ranks_docs_in_both_paths_first() {
    let (graph, ids) = fixture().await;

    // "apple" matches d0, d2; vector [1,0,0] is closest to d0 → d0 in both paths.
    let mut q = HybridQuery::new();
    q.text = Some("apple".into());
    q.vector = Some((vec![1.0, 0.0, 0.0], "m".into()));
    q.filter = Some(HybridFilter { label: Some("Doc".into()), props: vec![] });
    q.k = 4;

    let hits = graph.search_hybrid(q).await.unwrap();
    assert!(!hits.is_empty());
    assert_eq!(hits[0].node_id, ids["d0"], "d0 appears in both paths → rank #1");
    // d0 should have both ranks populated.
    assert!(hits[0].text_rank.is_some() && hits[0].vector_rank.is_some());
    // The Other-label node must not appear (filtered).
    assert!(hits.iter().all(|h| h.node_id != ids["x0"]));
}

#[tokio::test]
async fn filter_excludes_other_label() {
    let (graph, ids) = fixture().await;
    let mut q = HybridQuery::new();
    q.text = Some("apple".into());
    q.vector = Some((vec![1.0, 0.0, 0.0], "m".into()));
    q.filter = Some(HybridFilter { label: Some("Doc".into()), props: vec![] });

    let hits = graph.search_hybrid(q).await.unwrap();
    assert!(hits.iter().all(|h| h.node_id != ids["x0"]));

    // Without the filter, x0 (Other, "apple apple", [1,0,0]) should appear.
    let mut q2 = HybridQuery::new();
    q2.text = Some("apple".into());
    q2.vector = Some((vec![1.0, 0.0, 0.0], "m".into()));
    let hits2 = graph.search_hybrid(q2).await.unwrap();
    assert!(hits2.iter().any(|h| h.node_id == ids["x0"]));
}

#[tokio::test]
async fn single_path_degrades_cleanly() {
    let (graph, ids) = fixture().await;

    // vector-only: closest to [1,0,0] is d0.
    let mut qv = HybridQuery::new();
    qv.vector = Some((vec![1.0, 0.0, 0.0], "m".into()));
    qv.filter = Some(HybridFilter { label: Some("Doc".into()), props: vec![] });
    let hv = graph.search_hybrid(qv).await.unwrap();
    assert_eq!(hv[0].node_id, ids["d0"]);
    assert!(hv[0].text_rank.is_none() && hv[0].vector_rank.is_some());

    // text-only: "durian" matches only d3.
    let mut qt = HybridQuery::new();
    qt.text = Some("durian".into());
    qt.filter = Some(HybridFilter { label: Some("Doc".into()), props: vec![] });
    let ht = graph.search_hybrid(qt).await.unwrap();
    assert_eq!(ht.len(), 1);
    assert_eq!(ht[0].node_id, ids["d3"]);
    assert!(ht[0].vector_rank.is_none() && ht[0].text_rank.is_some());
}

#[tokio::test]
async fn invariants_no_dups_capped_sorted() {
    let (graph, _) = fixture().await;
    let mut q = HybridQuery::new();
    q.text = Some("apple banana cherry".into());
    q.vector = Some((vec![0.5, 0.5, 0.0], "m".into()));
    q.k = 3;
    let hits = graph.search_hybrid(q).await.unwrap();

    assert!(hits.len() <= 3, "capped at k");
    let mut seen = std::collections::HashSet::new();
    for h in &hits {
        assert!(seen.insert(h.node_id), "no duplicate node ids");
    }
    for w in hits.windows(2) {
        assert!(w[0].score >= w[1].score, "scores descending");
    }
}

#[tokio::test]
async fn reopen_db_with_fulltext_index() {
    // Regression: a persisted full-text index used to fail to reopen because
    // FullTextIndex::new called tantivy `create_in_dir` (errors if the index
    // already exists). Now it opens the existing index.
    let dir = tempfile::tempdir().unwrap();
    {
        let graph = Graph::open(dir.path()).await.unwrap();
        let node = Node::new("Doc")
            .with_property("name", s("a"))
            .with_property("body", s("apple"));
        graph.add_node(node).await.unwrap();
        graph.create_index("Doc", "body", IndexType::FullText).await.unwrap();
        graph.checkpoint().await.unwrap();
    }
    // Reopen must succeed and the index must still answer.
    let graph = Graph::open(dir.path()).await.unwrap();
    let mut q = HybridQuery::new();
    q.text = Some("apple".into());
    q.filter = Some(HybridFilter { label: Some("Doc".into()), props: vec![] });
    let hits = graph.search_hybrid(q).await.unwrap();
    assert_eq!(hits.len(), 1, "fulltext index survives reopen");
}

#[tokio::test]
async fn nql_hybrid_function_filters_to_topk() {
    let (graph, _) = fixture().await;
    // Reference node whose embedding is the query vector for the vector path.
    let refn = Node::new("Ref")
        .with_property("name", s("q"));
    let rid = graph.add_node(refn).await.unwrap();
    graph.add_node_embedding(rid, vec![1.0, 0.0, 0.0], "m").await.unwrap();

    let result = graph
        .execute_nql(r#"find n.name from (n:Doc) where hybrid(n, "apple", "q", "m") limit 5"#)
        .await
        .unwrap();

    let names: Vec<String> = result
        .rows()
        .iter()
        .filter_map(|r| r.get("n.name").and_then(|v| v.as_str().map(String::from)))
        .collect();
    assert!(!names.is_empty(), "hybrid NQL returns rows");
    assert!(names.contains(&"d0".to_string()), "d0 (apple + near [1,0,0]) present");
    // The Other-label node is excluded by the (n:Doc) pattern.
    assert!(!names.contains(&"x0".to_string()));
}
