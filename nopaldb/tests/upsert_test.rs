// Tests for the idempotent upsert primitive (issue M1-4).

use std::collections::HashMap;
use std::sync::Arc;

use nopaldb::graph::{LinkSpec, UpsertOutcome, UpsertRequest};
use nopaldb::types::{Node, PropertyValue};
use nopaldb::Graph;

fn props(pairs: &[(&str, &str)]) -> HashMap<String, PropertyValue> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), PropertyValue::String(v.to_string())))
        .collect()
}

fn req(label: &str, key: &str, pairs: &[(&str, &str)]) -> UpsertRequest {
    UpsertRequest {
        label: label.to_string(),
        key: key.to_string(),
        props: props(pairs),
        embedding: None,
        links: Vec::new(),
    }
}

#[tokio::test]
async fn create_then_unchanged_is_a_noop() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await?;

    let (o1, id1) = graph
        .upsert_node(req("Chunk", "key", &[("key", "note:a"), ("path", "a.md")]))
        .await?;
    assert_eq!(o1, UpsertOutcome::Created);
    let count_after_create = graph.get_all_nodes().await?.len();

    // Identical re-run → Unchanged, same id, no new node.
    let (o2, id2) = graph
        .upsert_node(req("Chunk", "key", &[("key", "note:a"), ("path", "a.md")]))
        .await?;
    assert_eq!(o2, UpsertOutcome::Unchanged);
    assert_eq!(id1, id2);
    assert_eq!(graph.get_all_nodes().await?.len(), count_after_create);
    Ok(())
}

#[tokio::test]
async fn update_reconciles_property_index() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await?;

    let (_, id) = graph
        .upsert_node(req("Chunk", "key", &[("key", "note:a"), ("path", "old.md")]))
        .await?;

    // Change a non-key property.
    let (o, id2) = graph
        .upsert_node(req("Chunk", "key", &[("key", "note:a"), ("path", "new.md")]))
        .await?;
    assert_eq!(o, UpsertOutcome::Updated);
    assert_eq!(id, id2);

    // The property index must reflect the new value and not the old one.
    let by_new = graph
        .get_all_nodes_by_property("path", &PropertyValue::String("new.md".into()))
        .await?;
    let by_old = graph
        .get_all_nodes_by_property("path", &PropertyValue::String("old.md".into()))
        .await?;
    assert_eq!(by_new, vec![id], "new value must be indexed");
    assert!(by_old.is_empty(), "stale old-value index entry must be gone");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_upserts_of_same_key_create_one_node() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let graph = Arc::new(Graph::open(dir.path()).await?);

    let mut handles = Vec::new();
    for i in 0..8 {
        let g = graph.clone();
        handles.push(tokio::spawn(async move {
            g.upsert_node(req(
                "Chunk",
                "key",
                &[("key", "note:same"), ("worker", &i.to_string())],
            ))
            .await
        }));
    }
    let mut ids = std::collections::HashSet::new();
    for h in handles {
        let (_, id) = h.await.unwrap()?;
        ids.insert(id);
    }
    // Exactly one node exists for the shared key.
    assert_eq!(ids.len(), 1, "concurrent upserts must converge to one node");
    let matches = graph
        .get_all_nodes_by_property("key", &PropertyValue::String("note:same".into()))
        .await?;
    assert_eq!(matches.len(), 1);
    Ok(())
}

#[tokio::test]
async fn ambiguous_key_is_an_error() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await?;

    // Seed two nodes sharing the same key value, bypassing upsert.
    for _ in 0..2 {
        let node = Node::new("Chunk").with_property("key", PropertyValue::String("dup".into()));
        graph.add_node(node).await?;
    }

    let err = graph
        .upsert_node(req("Chunk", "key", &[("key", "dup"), ("path", "x.md")]))
        .await;
    assert!(
        matches!(err, Err(nopaldb::error::NopalError::AmbiguousUpsertKey(_))),
        "expected AmbiguousUpsertKey, got {err:?}"
    );
    Ok(())
}

#[tokio::test]
async fn links_create_stub_and_are_idempotent() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await?;

    let link = || LinkSpec {
        edge_type: "MENTIONS".into(),
        target_label: "Note".into(),
        target_key: "key".into(),
        target_key_value: PropertyValue::String("note:b".into()),
        props: HashMap::new(),
        create_target_stub: true,
    };
    let mut r = req("Note", "key", &[("key", "note:a")]);
    r.links = vec![link()];

    // First upsert: creates source + a stub for note:b + one edge.
    let (_, src) = graph.upsert_node(r.clone()).await?;
    let stub = graph
        .get_all_nodes_by_property("key", &PropertyValue::String("note:b".into()))
        .await?;
    assert_eq!(stub.len(), 1, "stub target created");
    let stub_id = stub[0];
    assert_eq!(graph.get_outgoing_edges(src).await?.len(), 1);

    // Re-run identical: no duplicate edge.
    graph.upsert_node(r.clone()).await?;
    assert_eq!(
        graph.get_outgoing_edges(src).await?.len(),
        1,
        "re-upsert must not duplicate the edge"
    );

    // Upsert the target itself: fills the stub (same id, not a new node).
    let (o, filled) = graph
        .upsert_node(req("Note", "key", &[("key", "note:b"), ("title", "B")]))
        .await?;
    assert_eq!(filled, stub_id, "target upsert reuses the stub node");
    assert_eq!(o, UpsertOutcome::Updated);
    Ok(())
}

#[cfg(feature = "embeddings-index")]
#[tokio::test]
async fn embedding_update_is_reflected_in_knn() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await?;

    let mut r = req("Chunk", "key", &[("key", "note:a")]);
    r.embedding = Some((vec![1.0, 0.0, 0.0], "m".into()));
    let (_, id) = graph.upsert_node(r).await?;

    // Change only the vector.
    let mut r2 = req("Chunk", "key", &[("key", "note:a")]);
    r2.embedding = Some((vec![0.0, 1.0, 0.0], "m".into()));
    let (o, id2) = graph.upsert_node(r2).await?;
    assert_eq!(o, UpsertOutcome::Updated);
    assert_eq!(id, id2);

    // A query near the NEW vector should return the node as the closest hit.
    let idx = graph.build_embedding_index("m").await?;
    let hits = idx.search_knn(&[0.0, 1.0, 0.0], 1)?;
    assert_eq!(hits[0].0, id, "knn must reflect the updated vector");
    Ok(())
}
