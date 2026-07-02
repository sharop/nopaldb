use std::time::Duration;

use nopaldb::{AutoGcConfig, Graph, Node, PropertyValue};
use nopaldb::mvcc::GCConfig;

async fn seed_versioned_person(graph: &Graph) -> nopaldb::Result<nopaldb::NodeId> {
    let mut tx1 = graph.begin_transaction().await?;
    let node_id = tx1
        .add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String("Alice".into()))
                .with_property("balance", PropertyValue::Int(100)),
        )
        .await?;
    tx1.commit().await?;

    let mut tx2 = graph.begin_transaction().await?;
    let mut v2 = Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()))
        .with_property("balance", PropertyValue::Int(200));
    v2.id = node_id;
    tx2.add_node(v2).await?;
    tx2.commit().await?;

    let mut tx3 = graph.begin_transaction().await?;
    let mut v3 = Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()))
        .with_property("balance", PropertyValue::Int(300));
    v3.id = node_id;
    tx3.add_node(v3).await?;
    tx3.commit().await?;

    Ok(node_id)
}

#[tokio::test]
async fn test_p2_auto_gc_scheduler_runs_and_stops() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;
    let node_id = seed_versioned_person(&graph).await?;

    let before = graph.history(node_id).await?;
    assert_eq!(before.len(), 3);

    graph
        .start_auto_gc(AutoGcConfig {
            interval_secs: 1,
            gc_config: GCConfig {
                cutoff_timestamp: u64::MAX,
                min_versions_to_keep: 1,
                max_nodes_per_cycle: 0,
                dry_run: false,
                use_active_horizon: false,
            },
        })
        .await?;

    // Wait for scheduler to run at least once.
    tokio::time::sleep(Duration::from_millis(1800)).await;

    let running = graph.auto_gc_status().await;
    assert!(running.running, "Auto GC should be running");

    let after = graph.history(node_id).await?;
    assert_eq!(after.len(), 1, "Auto GC should prune old versions");

    let stopped = graph.stop_auto_gc().await?;
    assert!(stopped, "stop_auto_gc should report active scheduler");
    assert!(!graph.auto_gc_status().await.running);

    Ok(())
}

#[tokio::test]
async fn test_p2_auto_gc_scheduler_dry_run_keeps_versions() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;
    let node_id = seed_versioned_person(&graph).await?;

    graph
        .start_auto_gc(AutoGcConfig {
            interval_secs: 1,
            gc_config: GCConfig {
                cutoff_timestamp: u64::MAX,
                min_versions_to_keep: 1,
                max_nodes_per_cycle: 0,
                dry_run: true,
                use_active_horizon: false,
            },
        })
        .await?;

    tokio::time::sleep(Duration::from_millis(1800)).await;

    let after = graph.history(node_id).await?;
    assert_eq!(after.len(), 3, "Dry-run scheduler must not delete versions");

    let _ = graph.stop_auto_gc().await?;

    Ok(())
}
