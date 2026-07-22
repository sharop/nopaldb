// Tests for delete-by-business-key (M2-8): the delete counterpart of upsert.

use std::collections::HashMap;

use nopaldb::types::{Node, PropertyValue};
use nopaldb::{Graph, LinkSpec, UpsertRequest};

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

fn req(label: &str, key: &str, pairs: &[(&str, &str)]) -> UpsertRequest {
    UpsertRequest {
        label: label.to_string(),
        key: key.to_string(),
        props: pairs
            .iter()
            .map(|(k, v)| (k.to_string(), s(v)))
            .collect::<HashMap<_, _>>(),
        embedding: None,
        links: Vec::new(),
    }
}

#[tokio::test]
async fn delete_by_key_removes_node_edges_and_index() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await?;

    // A note that MENTIONS a target (creates a stub), so the target has an
    // incoming edge we can check gets cleaned up.
    let mut r = req("Note", "key", &[("key", "a"), ("body", "hello")]);
    r.links = vec![LinkSpec {
        edge_type: "MENTIONS".into(),
        target_label: "Note".into(),
        target_key: "key".into(),
        target_key_value: s("b"),
        props: HashMap::new(),
        create_target_stub: true,
    }];
    let (_, src) = graph.upsert_node(r).await?;
    let target = graph
        .get_all_nodes_by_property("key", &s("b"))
        .await?[0];
    assert_eq!(graph.get_incoming_edges(target).await?.len(), 1);

    // Delete the source by key.
    let deleted = graph.delete_node_by_key("Note", "key", &s("a")).await?;
    assert_eq!(deleted, Some(src));

    // Node gone from the property index; the target's incoming edge is cleaned.
    assert!(graph.get_all_nodes_by_property("key", &s("a")).await?.is_empty());
    assert_eq!(graph.get_incoming_edges(target).await?.len(), 0);
    Ok(())
}

#[tokio::test]
async fn delete_missing_key_is_none_and_idempotent() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await?;

    graph.upsert_node(req("Note", "key", &[("key", "a")])).await?;
    assert_eq!(graph.delete_node_by_key("Note", "key", &s("a")).await?.is_some(), true);
    // Second delete of the same key is a no-op.
    assert_eq!(graph.delete_node_by_key("Note", "key", &s("a")).await?, None);
    // A never-existed key is None too.
    assert_eq!(graph.delete_node_by_key("Note", "key", &s("zzz")).await?, None);
    Ok(())
}

#[tokio::test]
async fn delete_ambiguous_key_is_an_error() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await?;

    for _ in 0..2 {
        let node = Node::new("Note").with_property("key", s("dup"));
        graph.add_node(node).await?;
    }
    let err = graph.delete_node_by_key("Note", "key", &s("dup")).await;
    assert!(
        matches!(err, Err(nopaldb::error::NopalError::AmbiguousUpsertKey(_))),
        "expected AmbiguousUpsertKey, got {err:?}"
    );
    Ok(())
}

#[tokio::test]
async fn reconcile_deletes_absent_keys() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await?;

    // Ingest three notes.
    for k in ["a", "b", "c"] {
        graph.upsert_node(req("Note", "key", &[("key", k)])).await?;
    }

    // Desired set no longer contains "c" → reconcile deletes it.
    let desired = ["a", "b"];
    let existing: Vec<String> = graph
        .get_nodes_by_label("Note")
        .await?
        .into_iter()
        .filter_map(|n| n.properties.get("key").and_then(|v| v.as_str().map(String::from)))
        .collect();
    for key in existing.iter().filter(|k| !desired.contains(&k.as_str())) {
        graph.delete_node_by_key("Note", "key", &s(key)).await?;
    }

    let remaining: Vec<String> = graph
        .get_nodes_by_label("Note")
        .await?
        .into_iter()
        .filter_map(|n| n.properties.get("key").and_then(|v| v.as_str().map(String::from)))
        .collect();
    let mut sorted = remaining.clone();
    sorted.sort();
    assert_eq!(sorted, vec!["a".to_string(), "b".to_string()]);
    Ok(())
}
