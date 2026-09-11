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
