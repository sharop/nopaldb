// tests/checkpoint_test.rs

use nopaldb::{Graph, Node, PropertyValue};

#[tokio::test]
async fn test_checkpoint_truncates_wal() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path();

    let graph = Graph::open(&db_path).await.unwrap();

    // ═══════════════════════════════════════════════════════
    // FASE 1: Crear varios nodos
    // ═══════════════════════════════════════════════════════

    let mut node_ids = Vec::new();

    for i in 0..10 {
        let mut tx = graph.begin_transaction().await.unwrap();

        let node = Node::new("Person")
            .with_property("name", PropertyValue::String(format!("User{}", i)))
            .with_property("id", PropertyValue::Int(i));

        let id = tx.add_node(node).await.unwrap();
        tx.commit().await.unwrap();

        node_ids.push(id);
    }

    println!("✅ Created 10 nodes");

    // ═══════════════════════════════════════════════════════
    // FASE 2: Checkpoint (debe truncar WAL)
    // ═══════════════════════════════════════════════════════

    graph.checkpoint().await.unwrap();
    println!("✅ Checkpoint created");

    // ═══════════════════════════════════════════════════════
    // FASE 3: Crear más nodos después del checkpoint
    // ═══════════════════════════════════════════════════════

    for i in 10..15 {
        let mut tx = graph.begin_transaction().await.unwrap();

        let node = Node::new("Person")
            .with_property("name", PropertyValue::String(format!("User{}", i)))
            .with_property("id", PropertyValue::Int(i));

        let id = tx.add_node(node).await.unwrap();
        tx.commit().await.unwrap();

        node_ids.push(id);
    }

    println!("✅ Created 5 more nodes after checkpoint");

    // ═══════════════════════════════════════════════════════
    // FASE 4: Reabrir DB - recovery desde checkpoint
    // ═══════════════════════════════════════════════════════

    drop(graph);

    println!("\n🔄 Reopening database...");
    let graph = Graph::open(&db_path).await.unwrap();

    // Verificar: TODOS los nodos deben existir
    for (i, &node_id) in node_ids.iter().enumerate() {
        let node = graph.get_node(node_id).await.unwrap();
        assert_eq!(
            node.properties.get("name"),
            Some(&PropertyValue::String(format!("User{}", i)))
        );
    }

    println!("✅ All 15 nodes recovered successfully");
    println!("🎉 Checkpoint test PASSED!");
}

#[tokio::test]
async fn test_checkpoint_wal_size() {
    use std::fs;

    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path();
    let wal_path = db_path.join("nopal.wal");

    let graph = Graph::open(&db_path).await.unwrap();

    // Crear 100 nodos
    for i in 0..100 {
        let mut tx = graph.begin_transaction().await.unwrap();
        let node = Node::new("Test").with_property("id", PropertyValue::Int(i));
        tx.add_node(node).await.unwrap();
        tx.commit().await.unwrap();
    }

    let size_before = fs::metadata(&wal_path).unwrap().len();
    println!("WAL size before checkpoint: {} bytes", size_before);

    // Checkpoint
    graph.checkpoint().await.unwrap();

    let size_after = fs::metadata(&wal_path).unwrap().len();
    println!("WAL size after checkpoint: {} bytes", size_after);

    // WAL debe ser mucho más pequeño
    assert!(
        size_after < size_before / 2,
        "WAL should be significantly smaller after checkpoint"
    );

    println!(
        "✅ WAL size reduced by {}%",
        ((size_before - size_after) * 100) / size_before
    );
}
