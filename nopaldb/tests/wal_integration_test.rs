// tests/wal_integration_test.rs

use nopaldb::{Graph, Node, PropertyValue};

#[tokio::test]
async fn test_transaction_writes_to_wal() {
    // Create graph with WAL
    let graph = Graph::in_memory().await.unwrap();

    // Begin transaction
    let mut tx = graph.begin_transaction().await.unwrap();

    // Add node
    let node = Node::new("Person").with_property("name", PropertyValue::String("Alice".into()));

    let node_id = tx.add_node(node).await.unwrap();

    // Commit (should write to WAL)
    tx.commit().await.unwrap();

    // Verify node was persisted
    let retrieved = graph.get_node(node_id).await.unwrap();
    assert_eq!(retrieved.label, "Person");

    // TODO: Verify WAL contains records (in recovery test)
}
