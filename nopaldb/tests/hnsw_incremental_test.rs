// tests/hnsw_incremental_test.rs
//
// #113: a new, updated or deleted embedding changes the cached HNSW index in
// place instead of throwing it away. Every test here asserts the MECHANISM
// (same index object before and after, tombstone counters), not just the
// result, so it fails if someone reintroduces the invalidation.

use std::sync::Arc;

use nopaldb::embeddings::{REBUILD_TOMBSTONE_MIN, REBUILD_TOMBSTONE_RATIO};
use nopaldb::types::{Node, NodeId, PropertyValue};
use nopaldb::{Graph, Result};

const DIM: usize = 8;
const MODEL: &str = "m";

fn vec_for(i: usize) -> Vec<f32> {
    // Deterministic, well separated: a one-hot-ish vector with a small tail.
    let mut v = vec![0.01f32; DIM];
    v[i % DIM] = 1.0;
    v[(i / DIM) % DIM] += 0.1 * ((i % 7) as f32);
    v
}

async fn seeded(n: usize) -> Result<(Graph, Vec<NodeId>)> {
    let graph = Graph::in_memory().await?;
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        let id = graph.add_node(Node::new("Doc").with_property("i", PropertyValue::Int(i as i64))).await?;
        graph.add_node_embedding(id, vec_for(i), MODEL).await?;
        ids.push(id);
    }
    Ok((graph, ids))
}

#[tokio::test]
async fn insert_after_build_keeps_the_same_index_and_finds_the_new_vector() -> Result<()> {
    let (graph, _ids) = seeded(50).await?;
    let before = graph.get_or_build_embedding_index(MODEL).await?;
    assert_eq!(before.read().unwrap().len(), 50);

    let new_id = graph.add_node(Node::new("Doc")).await?;
    let mut q = vec![0.0f32; DIM];
    q[3] = 1.0;
    q[5] = 1.0; // a direction nobody else has
    graph.add_node_embedding(new_id, q.clone(), MODEL).await?;

    let after = graph.get_or_build_embedding_index(MODEL).await?;
    assert!(Arc::ptr_eq(&before, &after), "the cached index must be the same object: no rebuild");
    let idx = after.read().unwrap();
    assert_eq!(idx.len(), 51);
    assert_eq!(idx.tombstones(), 0);
    let top = idx.search_knn(&q, 1)?;
    assert_eq!(top[0].0, new_id, "the inserted vector is its own nearest neighbour");
    Ok(())
}

#[tokio::test]
async fn updating_an_embedding_replaces_it_with_one_tombstone() -> Result<()> {
    let (graph, ids) = seeded(50).await?;
    let index = graph.get_or_build_embedding_index(MODEL).await?;
    let target = ids[7];
    let mut moved = vec![0.0f32; DIM];
    moved[1] = 1.0;
    moved[6] = 1.0;
    graph.add_node_embedding(target, moved.clone(), MODEL).await?;

    let again = graph.get_or_build_embedding_index(MODEL).await?;
    assert!(Arc::ptr_eq(&index, &again));
    let idx = again.read().unwrap();
    assert_eq!(idx.len(), 50, "same number of live points");
    assert_eq!(idx.tombstones(), 1);
    assert_eq!(idx.search_knn(&moved, 1)?[0].0, target, "the NEW vector is what the index answers");
    let old_hits = idx.search_knn(&vec_for(7), 3)?;
    assert!(old_hits.iter().all(|(id, _)| *id != target) || old_hits[0].0 != target,
        "the old position is no longer the best match for the node: {old_hits:?}");
    let stats = graph.embedding_index_stats(MODEL).await.unwrap();
    assert_eq!((stats.size, stats.tombstones, stats.needs_rebuild), (50, 1, false));
    Ok(())
}

#[tokio::test]
async fn deleting_a_node_removes_it_from_search_without_rebuild() -> Result<()> {
    let (graph, ids) = seeded(50).await?;
    let index = graph.get_or_build_embedding_index(MODEL).await?;
    let victim = ids[10];
    let q = vec_for(10);
    assert_eq!(index.read().unwrap().search_knn(&q, 1)?[0].0, victim);

    graph.delete_node(victim).await?;
    let again = graph.get_or_build_embedding_index(MODEL).await?;
    assert!(Arc::ptr_eq(&index, &again), "delete must not drop the cache");
    let idx = again.read().unwrap();
    assert_eq!(idx.len(), 49);
    assert_eq!(idx.tombstones(), 1);
    assert!(idx.search_knn(&q, 5)?.iter().all(|(id, _)| *id != victim));
    Ok(())
}

#[tokio::test]
async fn tombstones_over_the_ratio_trigger_a_rebuild_on_next_lookup() -> Result<()> {
    // Enough deletes to cross both the absolute minimum and the ratio.
    let n = 300;
    let (graph, ids) = seeded(n).await?;
    let index = graph.get_or_build_embedding_index(MODEL).await?;
    let to_delete = REBUILD_TOMBSTONE_MIN.max(((n as f64) * REBUILD_TOMBSTONE_RATIO) as usize + 1);
    for id in ids.iter().take(to_delete - 1) {
        graph.delete_node(*id).await?;
    }
    assert!(!graph.embedding_index_stats(MODEL).await.unwrap().needs_rebuild, "one short of the threshold");
    graph.delete_node(ids[to_delete - 1]).await?;
    assert!(graph.embedding_index_stats(MODEL).await.unwrap().needs_rebuild);

    let rebuilt = graph.get_or_build_embedding_index(MODEL).await?;
    assert!(!Arc::ptr_eq(&index, &rebuilt), "a fresh index replaces the one full of tombstones");
    let idx = rebuilt.read().unwrap();
    assert_eq!(idx.len(), n - to_delete);
    assert_eq!(idx.tombstones(), 0);
    Ok(())
}

#[tokio::test]
async fn without_a_cached_index_nothing_changes_until_the_first_search() -> Result<()> {
    let (graph, _ids) = seeded(20).await?;
    assert!(graph.embedding_index_stats(MODEL).await.is_none(), "nothing built yet");
    let id = graph.add_node(Node::new("Doc")).await?;
    graph.add_node_embedding(id, vec_for(99), MODEL).await?;
    assert!(graph.embedding_index_stats(MODEL).await.is_none(), "still lazy");
    let index = graph.get_or_build_embedding_index(MODEL).await?;
    assert_eq!(index.read().unwrap().len(), 21);
    Ok(())
}

#[tokio::test]
async fn dimension_mismatch_on_insert_is_an_error_and_the_index_stays_consistent() -> Result<()> {
    let (graph, _ids) = seeded(20).await?;
    let index = graph.get_or_build_embedding_index(MODEL).await?;
    let id = graph.add_node(Node::new("Doc")).await?;
    let err = graph.add_node_embedding(id, vec![1.0; DIM + 1], MODEL).await.unwrap_err();
    assert!(err.to_string().contains("dimension"), "{err}");
    assert_eq!(index.read().unwrap().len(), 20);
    Ok(())
}

#[tokio::test]
async fn hnsw_path_above_the_exact_threshold_also_inserts_and_removes_in_place() -> Result<()> {
    use nopaldb::embeddings::EXACT_SEARCH_THRESHOLD;
    let n = EXACT_SEARCH_THRESHOLD + 200;
    let (graph, ids) = seeded(n).await?;
    let index = graph.get_or_build_embedding_index(MODEL).await?;
    assert!(index.read().unwrap().len() > EXACT_SEARCH_THRESHOLD);

    let new_id = graph.add_node(Node::new("Doc")).await?;
    let mut q = vec![0.0f32; DIM];
    q[2] = 1.0;
    q[4] = 1.0;
    q[6] = 1.0;
    graph.add_node_embedding(new_id, q.clone(), MODEL).await?;
    graph.delete_node(ids[0]).await?;

    let same = graph.get_or_build_embedding_index(MODEL).await?;
    assert!(Arc::ptr_eq(&index, &same));
    let idx = same.read().unwrap();
    assert_eq!(idx.len(), n);
    assert_eq!(idx.tombstones(), 1);
    let hits = idx.search_knn(&q, 5)?;
    assert_eq!(hits[0].0, new_id);
    assert!(hits.iter().all(|(id, _)| *id != ids[0]));
    assert_eq!(hits.len(), 5, "tombstones must not underfill k");
    Ok(())
}
