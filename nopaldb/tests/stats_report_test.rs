//! `Graph::stats()` y el evento de progreso (#158): el reporte dice lo que
//! de verdad pasó. Dos aperturas de la misma base — una con replay forzado
//! (WAL con commits sin checkpoint) y otra tras `checkpoint()` + `close()` —
//! deben contarse distinto; el progreso de un `upsert_batch` y de un `open`
//! con replay debe llegar entero al callback.

use std::sync::{Arc, Mutex};

use nopaldb::index::IndexType;
use nopaldb::{Graph, GraphSection, Node, Progress, PropertyValue, StorageOptions, UpsertRequest};

async fn commit_rows(g: &Graph, n: usize) {
    for i in 0..n {
        let mut tx = g.begin_transaction().await.unwrap();
        tx.add_node(
            Node::new("Planta")
                .with_property("nombre", PropertyValue::String(format!("planta-{i}")))
                .with_property("n", PropertyValue::Int(i as i64)),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }
}

fn collector() -> (Arc<Mutex<Vec<Progress>>>, nopaldb::ProgressCallback) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let cb: nopaldb::ProgressCallback = Arc::new(move |p| sink.lock().unwrap().push(p));
    (seen, cb)
}

#[tokio::test]
async fn a_forced_replay_and_a_clean_reopen_are_told_apart() {
    let dir = tempfile::tempdir().unwrap();

    // Sesión 1: 40 commits y NINGÚN close: el WAL se queda con todo.
    let g = Graph::open(dir.path()).await.unwrap();
    commit_rows(&g, 40).await;
    let s = g.stats().await.unwrap();
    assert_eq!(s.wal.checkpoints_this_session, 0);
    assert_eq!(s.wal.last_checkpoint_unix_ms, None);
    assert!(s.wal.bytes > 1000, "40 commits leave a real WAL: {}", s.wal.bytes);
    assert_eq!(s.wal.checkpoint_threshold_bytes, nopaldb::DEFAULT_WAL_CHECKPOINT_BYTES);
    assert_eq!(s.wal.direct_write_durability, "process_crash");
    assert_eq!(s.storage.engine, g.storage().backend_name());
    assert_eq!(s.storage.profile, "default");
    assert_eq!(s.storage.data_dir.as_deref(), Some(dir.path()));
    assert!(!s.storage.read_only);
    assert_eq!(s.recovery.operations_replayed, 0, "a brand-new database replays nothing");
    assert!(!s.recovery.crash_recovery);
    assert!(s.recovery.open_ms.total >= s.recovery.open_ms.wal_replay);
    drop(g);

    // Sesión 2: replay forzado, con progreso.
    let (seen, cb) = collector();
    let g = Graph::open_with_progress(dir.path(), StorageOptions::default(), Some(cb)).await.unwrap();
    let s = g.stats().await.unwrap();
    assert!(s.recovery.wal_records_read >= 40 * 3, "Begin + op + Commit per tx: {}", s.recovery.wal_records_read);
    // Cuántas hubo que RE-aplicar depende de cuánto había llegado al motor:
    // tras un `drop` en el mismo proceso el applier ya aplicó todo (la caché
    // del SO sobrevive), así que el redo las salta; un SIGKILL a mitad de
    // lote es lo que las deja pendientes y eso lo ejercita `crash_commit_test`.
    // Aquí se afirma la cota y que el reporte cuadra con el estado real.
    assert!(s.recovery.operations_replayed <= 40, "{}", s.recovery.operations_replayed);
    assert!(s.recovery.crash_recovery, "committed txs in the WAL without a checkpoint");
    assert!(s.recovery.adjacency_rebuilt);
    assert_eq!(s.recovery.uncommitted_txs_discarded, 0);
    assert_eq!(
        s.graph,
        GraphSection {
            total_nodes: 40,
            total_edges: 0,
            avg_degree: 0.0,
            nodes_per_label: [("Planta".to_string(), 40)].into_iter().collect(),
            edges_per_type: Default::default(),
        }
    );
    {
        let seen = seen.lock().unwrap();
        let replay: Vec<&Progress> = seen.iter().filter(|p| p.phase == "wal_replay").collect();
        assert!(replay.len() >= 2, "start + finish at least: {replay:?}");
        assert_eq!(replay.first().unwrap().done, 0);
        let last = replay.last().unwrap();
        assert_eq!(last.total, Some(40));
        assert_eq!(last.done, 40);
        assert!(seen.iter().any(|p| p.phase == "adjacency_rebuild"), "{seen:?}");
        assert!(seen.iter().any(|p| p.phase == "index_load"), "{seen:?}");
    }

    // checkpoint() se anota; close() es otro checkpoint.
    g.checkpoint().await.unwrap();
    let s = g.stats().await.unwrap();
    assert_eq!(s.wal.checkpoints_this_session, 1);
    assert!(s.wal.last_checkpoint_unix_ms.unwrap() > 1_600_000_000_000);
    assert!(s.wal.bytes < 256, "the WAL is one Checkpoint record: {}", s.wal.bytes);
    g.close().await.unwrap();
    assert_eq!(g.stats().await.unwrap().wal.checkpoints_this_session, 2);
    drop(g);

    // Sesión 3: reapertura limpia — solo el registro Checkpoint, nada que reproducir.
    let g = Graph::open(dir.path()).await.unwrap();
    let s = g.stats().await.unwrap();
    assert_eq!(s.recovery.wal_records_read, 1);
    assert_eq!(s.recovery.operations_replayed, 0);
    assert!(!s.recovery.crash_recovery);
    assert!(!s.recovery.adjacency_rebuilt);
    assert_eq!(s.graph.total_nodes, 40);
    assert_eq!(s.wal.checkpoints_this_session, 0);
}

#[tokio::test]
async fn indexes_come_with_size_and_analyzer_and_gc_leaves_a_last_run() {
    let dir = tempfile::tempdir().unwrap();
    let g = Graph::open(dir.path()).await.unwrap();
    commit_rows(&g, 12).await;

    let (seen, cb) = collector();
    g.set_progress_callback(cb);
    g.create_index("Planta", "nombre", IndexType::Hash).await.unwrap();
    g.clear_progress_callback();
    {
        let seen = seen.lock().unwrap();
        let build: Vec<&Progress> = seen.iter().filter(|p| p.phase == "index_build").collect();
        assert_eq!(build.last().map(|p| (p.done, p.total)), Some((12, Some(12))), "{seen:?}");
    }

    let s = g.stats().await.unwrap();
    assert_eq!(s.indexes.len(), 1);
    let ix = &s.indexes[0];
    assert_eq!((ix.name.as_str(), ix.label.as_str(), ix.property.as_str(), ix.kind.as_str()), ("Planta_nombre", "Planta", "nombre", "Hash"));
    assert_eq!(ix.size, 12);
    assert_eq!(ix.analyzer, None, "only full-text indexes have one");

    #[cfg(feature = "fulltext")]
    {
        use nopaldb::index::{FullTextAnalyzer, IndexOptions};
        g.create_index_with(
            "Nota",
            "cuerpo",
            IndexType::FullText,
            IndexOptions { analyzer: Some(FullTextAnalyzer::for_language("spanish")) },
        )
        .await
        .unwrap();
        let s = g.stats().await.unwrap();
        let ft = s.indexes.iter().find(|i| i.kind == "FullText").unwrap();
        assert_eq!(ft.analyzer.as_deref(), Some("spanish+stemming+stopwords+ascii_folding"));
    }

    assert_eq!(s.gc.last_run, None);
    assert!(!s.gc.auto_running);
    g.gc(nopaldb::mvcc::GCConfig::default().dry_run()).await.unwrap();
    let s = g.stats().await.unwrap();
    let run = s.gc.last_run.expect("gc() records its run");
    assert!(run.dry_run);
    assert_eq!(run.versions_removed, 0);
    assert!(run.unix_ms > 1_600_000_000_000);

    g.start_auto_gc(nopaldb::AutoGcConfig { interval_secs: 3600, gc_config: Default::default() }).await.unwrap();
    let s = g.stats().await.unwrap();
    assert!(s.gc.auto_running);
    assert_eq!(s.gc.auto.as_ref().map(|a| a.interval_secs), Some(3600));
    g.stop_auto_gc().await.unwrap();
    assert!(!g.stats().await.unwrap().gc.auto_running);
}

#[tokio::test]
async fn upsert_batch_and_bulk_loader_report_progress_and_memory_has_no_recovery() {
    let g = Graph::in_memory().await.unwrap();
    let (seen, cb) = collector();
    g.set_progress_callback(cb);

    let reqs: Vec<UpsertRequest> = (0..2500)
        .map(|i| UpsertRequest {
            label: "Fila".into(),
            key: "k".into(),
            props: [("k".to_string(), PropertyValue::Int(i))].into_iter().collect(),
            embedding: None,
            links: vec![],
        })
        .collect();
    g.upsert_batch(reqs).await.unwrap();

    let mut loader = g.bulk_loader(100);
    for i in 0..350 {
        loader.add_node(Node::new("Bulk").with_property("i", PropertyValue::Int(i))).await.unwrap();
    }
    loader.finish().await.unwrap();

    let seen = seen.lock().unwrap();
    let up: Vec<&Progress> = seen.iter().filter(|p| p.phase == "upsert_batch").collect();
    assert_eq!(up.first().map(|p| (p.done, p.total)), Some((0, Some(2500))));
    assert_eq!(up.last().map(|p| (p.done, p.total)), Some((2500, Some(2500))));
    assert!(up.iter().any(|p| p.done > 0 && p.done < 2500), "intermediate events: {up:?}");
    let bulk: Vec<&Progress> = seen.iter().filter(|p| p.phase == "bulk_load").collect();
    assert_eq!(bulk.last().map(|p| (p.done, p.total)), Some((350, Some(350))), "{bulk:?}");
    drop(seen);

    let s = g.stats().await.unwrap();
    assert_eq!(s.storage.data_dir, None);
    assert_eq!(s.recovery, Default::default(), "in memory there is no open to report");
    assert_eq!(s.graph.total_nodes, 2850);
    assert!(s.hnsw.is_empty());
}
