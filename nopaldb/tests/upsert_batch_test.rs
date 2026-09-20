//! `upsert_batch` after #151: identity through the property index (cost per
//! row independent of the graph size), one transaction per chunk, and the
//! in-batch rules (repeated keys, links between rows, atomic chunks).

use std::collections::HashMap;
use std::time::Instant;

use nopaldb::graph::upsert::{LinkSpec, UpsertOutcome, UpsertRequest};
use nopaldb::types::{Node, PropertyValue};
use nopaldb::{Graph, StorageOptions};

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

fn req(label: &str, key: &str, extra: Option<(&str, &str)>) -> UpsertRequest {
    let mut props = HashMap::new();
    props.insert("key".to_string(), s(key));
    if let Some((k, v)) = extra {
        props.insert(k.to_string(), s(v));
    }
    UpsertRequest {
        label: label.to_string(),
        key: "key".to_string(),
        props,
        embedding: None,
        links: vec![],
    }
}

fn link(target_key: &str, stub: bool) -> LinkSpec {
    LinkSpec {
        edge_type: "REFERS_TO".into(),
        target_label: "Nota".into(),
        target_key: "key".into(),
        target_key_value: s(target_key),
        props: HashMap::new(),
        create_target_stub: stub,
    }
}

/// Default engine of the build (redb; sled in the sled-only CI step).
async fn open(dir: &std::path::Path) -> Graph {
    Graph::open_with_options(dir.join("db"), StorageOptions::default()).await.unwrap()
}

/// The regression the issue is about: a batch of 300 rows must cost about the
/// same per row whether the graph holds 300 or 4 800 nodes. Before #151 the
/// lookup deserialized every node, so the second batch was ~16× slower per
/// row; a 3× bound leaves room for CI noise and catches a return to the scan.
#[tokio::test]
async fn upsert_cost_per_row_does_not_grow_with_the_graph() {
    let dir = tempfile::tempdir().unwrap();
    let g = open(dir.path()).await;
    let batch = |from: usize, n: usize| -> Vec<UpsertRequest> {
        (from..from + n).map(|i| req("Nota", &format!("n{i}"), None)).collect()
    };

    let t = Instant::now();
    g.upsert_batch(batch(0, 300)).await.unwrap();
    let small = t.elapsed().as_secs_f64();

    g.upsert_batch(batch(300, 4_500)).await.unwrap();
    assert_eq!(g.get_all_nodes().await.unwrap().len(), 4_800);

    let t = Instant::now();
    g.upsert_batch(batch(10_000, 300)).await.unwrap();
    let large = t.elapsed().as_secs_f64();

    eprintln!("300 rows @300 nodes: {small:.3}s; @4800 nodes: {large:.3}s");
    assert!(
        large < small * 3.0 + 0.05,
        "per-row cost grew with the graph: {small:.3}s → {large:.3}s"
    );
}

#[tokio::test]
async fn a_batch_creates_updates_and_leaves_unchanged_in_input_order() {
    let dir = tempfile::tempdir().unwrap();
    let g = open(dir.path()).await;
    let first = g
        .upsert_batch(vec![req("Nota", "a", None), req("Nota", "b", None)])
        .await
        .unwrap();
    assert_eq!(first[0].0, UpsertOutcome::Created);
    assert_eq!(first[1].0, UpsertOutcome::Created);

    let second = g
        .upsert_batch(vec![
            req("Nota", "a", None),
            req("Nota", "b", Some(("title", "B"))),
            req("Nota", "c", None),
        ])
        .await
        .unwrap();
    assert_eq!(second[0], (UpsertOutcome::Unchanged, first[0].1));
    assert_eq!(second[1], (UpsertOutcome::Updated, first[1].1));
    assert_eq!(second[2].0, UpsertOutcome::Created);
    assert_eq!(g.get_all_nodes().await.unwrap().len(), 3);
}

/// Two rows with the same key in one batch converge on one node: the second
/// row updates the node the first one created, inside the same transaction.
#[tokio::test]
async fn a_repeated_key_inside_a_batch_yields_one_node() {
    let dir = tempfile::tempdir().unwrap();
    let g = open(dir.path()).await;
    let out = g
        .upsert_batch(vec![
            req("Nota", "dup", Some(("title", "v1"))),
            req("Nota", "dup", Some(("title", "v2"))),
            req("Nota", "dup", Some(("title", "v2"))),
        ])
        .await
        .unwrap();
    assert_eq!(out[0].0, UpsertOutcome::Created);
    assert_eq!(out[1], (UpsertOutcome::Updated, out[0].1));
    assert_eq!(out[2], (UpsertOutcome::Unchanged, out[0].1));
    assert_eq!(g.get_all_nodes().await.unwrap().len(), 1);
    let node = g.get_node(out[0].1).await.unwrap();
    assert_eq!(node.properties.get("title"), Some(&s("v2")));
}

/// A link to a row that appears later in the batch creates a stub; when the
/// later row arrives it fills the stub (same NodeId) instead of creating a
/// second node. A link to an earlier row resolves without a stub.
#[tokio::test]
async fn links_between_rows_of_one_batch_resolve_to_the_same_nodes() {
    let dir = tempfile::tempdir().unwrap();
    let g = open(dir.path()).await;
    let mut a = req("Nota", "a", None);
    a.links.push(link("b", true));
    let b = req("Nota", "b", Some(("title", "B")));
    let mut c = req("Nota", "c", None);
    c.links.push(link("a", false));
    let out = g.upsert_batch(vec![a, b, c]).await.unwrap();

    assert_eq!(g.get_all_nodes().await.unwrap().len(), 3);
    assert_eq!(out[1].0, UpsertOutcome::Updated, "b fills the stub a linked to");
    let b_node = g.get_node(out[1].1).await.unwrap();
    assert_eq!(b_node.properties.get("title"), Some(&s("B")));
    let a_out = g.get_outgoing_edges(out[0].1).await.unwrap();
    assert_eq!(a_out.len(), 1);
    assert_eq!(a_out[0].target, out[1].1);
    let c_out = g.get_outgoing_edges(out[2].1).await.unwrap();
    assert_eq!(c_out.len(), 1);
    assert_eq!(c_out[0].target, out[0].1);

    // Re-running the batch adds no edge and writes nothing.
    let mut a = req("Nota", "a", None);
    a.links.push(link("b", true));
    let mut c = req("Nota", "c", None);
    c.links.push(link("a", false));
    let again = g
        .upsert_batch(vec![a, req("Nota", "b", Some(("title", "B"))), c])
        .await
        .unwrap();
    assert!(again.iter().all(|(o, _)| *o == UpsertOutcome::Unchanged), "{again:?}");
    assert_eq!(g.get_outgoing_edges(out[0].1).await.unwrap().len(), 1);
}

/// A row that fails takes its whole chunk down: nothing of the batch is
/// written, the earlier rows included.
#[tokio::test]
async fn a_failing_row_rolls_back_its_chunk() {
    let dir = tempfile::tempdir().unwrap();
    let g = open(dir.path()).await;
    let mut bad = req("Nota", "bad", None);
    bad.links.push(link("missing", false));
    let err = g
        .upsert_batch(vec![req("Nota", "a", None), req("Nota", "b", None), bad])
        .await
        .unwrap_err();
    assert!(matches!(err, nopaldb::NopalError::NodeNotFound(_)), "{err:?}");
    assert_eq!(g.get_all_nodes().await.unwrap().len(), 0);
}

/// Nodes written by the bulk loader must be found by a later upsert: the
/// lookup now goes through the property index, so the bulk path has to feed
/// it (it did not before #151, which would have made this create a duplicate).
#[tokio::test]
async fn upsert_finds_nodes_written_by_the_bulk_loader() {
    let dir = tempfile::tempdir().unwrap();
    let g = open(dir.path()).await;
    let mut loader = g.bulk_loader(256);
    for i in 0..600 {
        let node = Node::new("Nota").with_property("key", s(&format!("bulk{i}")));
        loader.add_node(node).await.unwrap();
    }
    loader.finish().await.unwrap();

    let out = g
        .upsert_batch(vec![
            req("Nota", "bulk0", None),
            req("Nota", "bulk599", Some(("title", "T"))),
            req("Nota", "new", None),
        ])
        .await
        .unwrap();
    assert_eq!(out[0].0, UpsertOutcome::Unchanged);
    assert_eq!(out[1].0, UpsertOutcome::Updated);
    assert_eq!(out[2].0, UpsertOutcome::Created);
    assert_eq!(g.get_all_nodes().await.unwrap().len(), 601);

    // Same after a reopen (the index entries are on disk, not just in RAM).
    g.close().await.unwrap();
    drop(g);
    let g = open(dir.path()).await;
    let (outcome, _) = g.upsert_node(req("Nota", "bulk300", None)).await.unwrap();
    assert_eq!(outcome, UpsertOutcome::Unchanged);
    assert_eq!(g.get_all_nodes().await.unwrap().len(), 601);
}

/// Business keys the property index does not encode (bytes, lists, objects)
/// keep working through the scan fallback.
#[tokio::test]
async fn a_non_indexable_key_still_upserts_correctly() {
    let dir = tempfile::tempdir().unwrap();
    let g = open(dir.path()).await;
    let mk = |title: &str| {
        let mut props = HashMap::new();
        props.insert("key".to_string(), PropertyValue::Bytes(vec![1, 2, 3]));
        props.insert("title".to_string(), s(title));
        UpsertRequest { label: "Blob".into(), key: "key".into(), props, embedding: None, links: vec![] }
    };
    let (o1, id) = g.upsert_node(mk("a")).await.unwrap();
    let (o2, id2) = g.upsert_node(mk("a")).await.unwrap();
    let (o3, id3) = g.upsert_node(mk("b")).await.unwrap();
    assert_eq!((o1, o2, o3), (UpsertOutcome::Created, UpsertOutcome::Unchanged, UpsertOutcome::Updated));
    assert_eq!(id, id2);
    assert_eq!(id, id3);
    assert_eq!(g.get_all_nodes().await.unwrap().len(), 1);
}
