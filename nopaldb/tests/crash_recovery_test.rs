// tests/crash_recovery_test.rs

use nopaldb::{Graph, Node, PropertyValue};

#[tokio::test]
async fn test_crash_recovery() {
    // Setup: directorio temporal
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path(); // ← Sin .to_path_buf()

    // ═══════════════════════════════════════════════════════
    // FASE 1: Operación normal + CRASH simulado
    // ═══════════════════════════════════════════════════════

    let node_id = {
        let graph = Graph::open(&db_path).await.unwrap();

        // Transaction 1: Commiteada
        let mut tx1 = graph.begin_transaction().await.unwrap();

        let node = Node::new("Person")
            .with_property("name", PropertyValue::String("Alice".into()))
            .with_property("age", PropertyValue::Int(30));

        let id = tx1.add_node(node).await.unwrap();
        tx1.commit().await.unwrap();

        println!("✅ Transaction 1 committed (Alice)");

        // Transaction 2: NO commiteada (simula crash)
        let mut tx2 = graph.begin_transaction().await.unwrap();

        let node2 = Node::new("Person").with_property("name", PropertyValue::String("Bob".into()));

        tx2.add_node(node2).await.unwrap();
        // NO COMMIT! Simula crash

        println!("💥 CRASH! Transaction 2 not committed (Bob lost)");

        id
    }; // ← Graph dropped = simula crash

    // ═══════════════════════════════════════════════════════
    // FASE 2: Recovery - reabrir DB
    // ═══════════════════════════════════════════════════════

    println!("\n🔄 Recovering from WAL...");

    let graph = Graph::open(&db_path).await.unwrap();

    // Verificar: Alice debe estar (commiteada)
    let alice = graph.get_node(node_id).await.unwrap();
    assert_eq!(alice.label, "Person");
    assert_eq!(
        alice.properties.get("name"),
        Some(&PropertyValue::String("Alice".into()))
    );
    println!("✅ Alice recovered successfully");

    // Verificar: Bob NO debe estar (no commiteada)
    let all_nodes = graph
        .find_nodes_by_property("name", &PropertyValue::String("Bob".into()))
        .await
        .unwrap();

    assert!(all_nodes.is_empty(), "Bob should not exist (not committed)");
    println!("✅ Bob not recovered (correct - was not committed)");

    println!("\n🎉 Crash recovery test PASSED!");
}
