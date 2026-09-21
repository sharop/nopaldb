//! #150: the WAL is truncated automatically once it passes
//! `StorageOptions::wal_checkpoint_bytes`, on `checkpoint()` and on
//! `close()`; every write acknowledged before a truncation is still there
//! after reopening. Engine: the build's default (redb; sled in the sled-only
//! CI step).

use nopaldb::{Graph, Node, PropertyValue, StorageOptions};

fn wal_len(dir: &std::path::Path) -> u64 {
    std::fs::metadata(dir.join("nopal.wal")).map(|m| m.len()).unwrap_or(0)
}

async fn add_rows(g: &Graph, from: usize, n: usize) {
    for i in from..from + n {
        g.add_node(
            Node::new("Fila")
                .with_property("i", PropertyValue::Int(i as i64))
                .with_property("texto", PropertyValue::String(format!("fila número {i} con algo de texto"))),
        )
        .await
        .unwrap();
    }
}

async fn count(g: &Graph) -> usize {
    g.get_all_nodes().await.unwrap().len()
}

#[tokio::test]
async fn the_wal_stays_bounded_under_a_write_load_and_nothing_is_lost() {
    let dir = tempfile::tempdir().unwrap();
    let opts = StorageOptions { wal_checkpoint_bytes: 16 * 1024, ..Default::default() };
    let g = Graph::open_with_options(dir.path(), opts).await.unwrap();

    add_rows(&g, 0, 400).await; // ~250 B each: ~100 KB without truncation
    let after_load = g.wal_bytes().await;
    assert!(
        after_load < 2 * 16 * 1024,
        "WAL should have been truncated on the way ({after_load} bytes)"
    );
    assert_eq!(count(&g).await, 400);

    // The last records may still be in the WAL: reopen without close (as a
    // crash would) and check nothing acknowledged is missing.
    drop(g);
    let g = Graph::open(dir.path()).await.unwrap();
    assert_eq!(count(&g).await, 400, "rows written before and after truncations");
    let ids: Vec<i64> = g
        .get_all_nodes()
        .await
        .unwrap()
        .iter()
        .filter_map(|n| match n.properties.get("i") {
            Some(PropertyValue::Int(i)) => Some(*i),
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), 400);
    assert_eq!(ids.iter().max(), Some(&399));
}

#[tokio::test]
async fn explicit_checkpoint_empties_the_wal_and_close_leaves_nothing_to_replay() {
    let dir = tempfile::tempdir().unwrap();
    // Automatic checkpoint off: only the explicit calls truncate.
    let opts = StorageOptions { wal_checkpoint_bytes: 0, ..Default::default() };
    let g = Graph::open_with_options(dir.path(), opts).await.unwrap();

    add_rows(&g, 0, 50).await;
    let mut tx = g.begin_transaction().await.unwrap();
    tx.add_node(Node::new("Fila").with_property("i", PropertyValue::Int(50))).await.unwrap();
    tx.commit().await.unwrap();
    let grown = g.wal_bytes().await;
    assert!(grown > 10 * 1024, "no automatic truncation with 0 ({grown} bytes)");

    g.checkpoint().await.unwrap();
    let after = g.wal_bytes().await;
    assert!(after < 256, "only the Checkpoint record remains ({after} bytes)");
    assert_eq!(count(&g).await, 51);

    add_rows(&g, 100, 10).await;
    assert!(g.wal_bytes().await > after);
    g.close().await.unwrap();
    let on_disk = wal_len(dir.path());
    assert!(on_disk < 256, "close() checkpoints: {on_disk} bytes left on disk");
    drop(g);

    let g = Graph::open(dir.path()).await.unwrap();
    assert_eq!(count(&g).await, 61);
    assert_eq!(g.wal_bytes().await, on_disk, "nothing was replayed or re-logged on open");
}

#[tokio::test]
async fn a_read_only_graph_refuses_to_checkpoint_and_closes_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    {
        let g = Graph::open(dir.path()).await.unwrap();
        add_rows(&g, 0, 3).await;
        g.close().await.unwrap();
    }
    let g = Graph::open_read_only(dir.path()).await.unwrap();
    assert!(g.checkpoint().await.is_err());
    assert_eq!(count(&g).await, 3);
    g.close().await.unwrap();
}
