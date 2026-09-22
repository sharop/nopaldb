// tests/open_lock_same_process_test.rs
//
// #120: reopening a path this process still holds fails immediately and says
// how to get out (drop the Graph); after the drop it opens; another engine
// handle on the same path gets the same answer. Both engines.

use std::time::{Duration, Instant};

use nopaldb::{Graph, StorageEngine, StorageOptions};

fn opts(engine: StorageEngine) -> StorageOptions {
    StorageOptions { engine, ..StorageOptions::default() }
}

fn engines() -> Vec<StorageEngine> {
    vec![StorageEngine::Sled, StorageEngine::Redb]
}

#[tokio::test]
async fn reopen_after_close_without_drop_fails_fast_and_says_this_process() {
    for engine in engines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let graph = Graph::open_with_options(&path, opts(engine)).await.unwrap();
        graph.close().await.unwrap();

        let t = Instant::now();
        let err = match Graph::open_with_options(&path, opts(engine)).await {
            Ok(_) => panic!("{engine:?}: the second open must fail"),
            Err(e) => e.to_string(),
        };
        let elapsed = t.elapsed();
        assert!(err.contains("ya está abierta en este proceso"), "{engine:?}: {err}");
        assert!(err.contains("drop"), "{engine:?}: must say how to get out: {err}");
        assert!(!err.contains("otro proceso"), "{engine:?}: {err}");
        assert!(elapsed < Duration::from_millis(200), "{engine:?}: no retry for the same process: {elapsed:?}");
        drop(graph);
    }
}

#[tokio::test]
async fn reopen_after_drop_works() {
    for engine in engines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let graph = Graph::open_with_options(&path, opts(engine)).await.unwrap();
        let id = graph.add_node(nopaldb::Node::new("Doc")).await.unwrap();
        graph.close().await.unwrap();
        drop(graph);

        let again = Graph::open_with_options(&path, opts(engine)).await.unwrap();
        assert_eq!(again.get_node(id).await.unwrap().label, "Doc");
    }
}

#[tokio::test]
async fn two_handles_on_the_same_path_in_one_process_get_the_same_answer() {
    for engine in engines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let first = Graph::open_with_options(&path, opts(engine)).await.unwrap();
        let err = match Graph::open_with_options(&path, opts(engine)).await {
            Ok(_) => panic!("{engine:?}: the second open must fail"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("este proceso"), "{engine:?}: {err}");
        drop(first);
        // Released: the third open succeeds.
        Graph::open_with_options(&path, opts(engine)).await.unwrap();
    }
}

#[tokio::test]
async fn a_clone_keeps_the_lock_until_every_clone_is_gone() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let graph = Graph::open_with_options(&path, opts(StorageEngine::Sled)).await.unwrap();
    let twin = graph.clone();
    drop(graph);
    let err = match Graph::open_with_options(&path, opts(StorageEngine::Sled)).await {
        Ok(_) => panic!("the open with a live clone must fail"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("este proceso"), "a live clone still holds the lock: {err}");
    drop(twin);
    Graph::open_with_options(&path, opts(StorageEngine::Sled)).await.unwrap();
}

/// 0.6.5: soltar el handle nada más recibir el ack de un commit y reabrir la
/// misma ruta en el acto debe funcionar. Hasta 0.6.4 el applier enviaba el
/// ack con sus clones de `Graph` todavía vivos (marca del redo, relojes,
/// checkpoint automático) y la reapertura inmediata fallaba con "ya está
/// abierta en este proceso"; en Python (`del g` + `Graph.open`) era
/// sistemático. Con la forma de los bindings: runtime multi-hilo y
/// `block_on` desde fuera, drop fuera del runtime, sin esperas.
#[test]
fn drop_right_after_a_commit_ack_and_reopen_immediately() {
    use nopaldb::{Node, PropertyValue};
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    for engine in engines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        for round in 0..20 {
            let graph = rt.block_on(Graph::open_with_options(&path, opts(engine))).unwrap();
            rt.block_on(async {
                let mut tx = graph.begin_transaction().await.unwrap();
                tx.add_node(Node::new("P").with_property("r", PropertyValue::Int(round)))
                    .await
                    .unwrap();
                tx.commit().await.unwrap();
                // Y una escritura directa, que también pasa por el applier.
                graph.add_node(Node::new("Q")).await.unwrap();
            });
            drop(graph);
            // La siguiente vuelta reabre sin ninguna espera.
        }
        let graph = rt.block_on(Graph::open_with_options(&path, opts(engine))).unwrap();
        assert_eq!(rt.block_on(graph.get_label_count("P")).unwrap(), 20, "{engine:?}");
        assert_eq!(rt.block_on(graph.get_label_count("Q")).unwrap(), 20, "{engine:?}");
    }
}
