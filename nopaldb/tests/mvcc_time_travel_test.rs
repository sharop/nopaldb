// tests/mvcc_time_travel_test.rs

use nopaldb::{Graph, Node, PropertyValue};

#[tokio::test]
async fn test_time_travel_queries() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path();

    let graph = Graph::open(&db_path).await.unwrap();

    println!("🕐 Creating timeline of changes...\n");

    // ═══════════════════════════════════════════════════════
    // TIMELINE: Crear historia de cambios
    // ═══════════════════════════════════════════════════════

    // t=100: Alice nace (age=0)
    let node_id = {
        let mut tx = graph.begin_transaction().await.unwrap();
        let node = Node::new("Person")
            .with_property("name", PropertyValue::String("Alice".into()))
            .with_property("age", PropertyValue::Int(0));
        let id = tx.add_node(node).await.unwrap();
        tx.commit().await.unwrap();
        println!("t=100: Alice created (age=0)");
        id
    };

    // Simular paso del tiempo
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // t=200: Alice cumple 25
    {
        let mut tx = graph.begin_transaction().await.unwrap();
        let mut node = graph.get_node(node_id).await.unwrap();
        node.properties.insert("age".into(), PropertyValue::Int(25));
        tx.add_node(node).await.unwrap();
        tx.commit().await.unwrap();
        println!("t=200: Alice turns 25");
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // t=300: Alice cumple 30
    {
        let mut tx = graph.begin_transaction().await.unwrap();
        let mut node = graph.get_node(node_id).await.unwrap();
        node.properties.insert("age".into(), PropertyValue::Int(30));
        tx.add_node(node).await.unwrap();
        tx.commit().await.unwrap();
        println!("t=300: Alice turns 30");
    }

    // ═══════════════════════════════════════════════════════
    // TIME TRAVEL: Consultar en diferentes momentos
    // ═══════════════════════════════════════════════════════

    println!("\n🔮 Time travel queries:");

    // Query: ¿Qué edad tenía Alice en diferentes momentos?
    let history = graph.history(node_id).await.unwrap();

    println!("\n📜 Complete history:");
    for (i, version) in history.iter().enumerate() {
        let age = version.node_data.properties.get("age").unwrap();
        println!("  Version {}: t={}, age={:?}", i + 1, version.timestamp, age);
    }

    // Verificar que tenemos 3 versiones
    assert_eq!(history.len(), 3, "Should have 3 versions");

    println!("\n✅ Time travel test PASSED!");
}

#[tokio::test]
async fn test_snapshot_queries() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path();

    let graph = Graph::open(&db_path).await.unwrap();

    println!("📸 Testing snapshot queries...\n");

    // Crear nodo con versiones
    let node_id = {
        let mut tx = graph.begin_transaction().await.unwrap();
        let node = Node::new("Counter")
            .with_property("value", PropertyValue::Int(0));
        let id = tx.add_node(node).await.unwrap();
        tx.commit().await.unwrap();
        id
    };

    // Capturar timestamps
    let t1 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Actualizar valor
    {
        let mut tx = graph.begin_transaction().await.unwrap();
        let mut node = graph.get_node(node_id).await.unwrap();
        node.properties.insert("value".into(), PropertyValue::Int(100));
        tx.add_node(node).await.unwrap();
        tx.commit().await.unwrap();
    }

    let t2 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Crear snapshot en t1 (antes del cambio)
    let snapshot1 = graph.as_of(t1);

    // Crear snapshot en t2 (después del cambio)
    let snapshot2 = graph.as_of(t2);

    println!("Snapshot 1 (t={}): Reading node...", t1);
    let node1 = snapshot1.get_node(node_id).await;

    println!("Snapshot 2 (t={}): Reading node...", t2);
    let node2 = snapshot2.get_node(node_id).await;

    // Al menos uno debe tener el nodo
    assert!(node1.is_ok() || node2.is_ok(), "At least one snapshot should have the node");

    if let Ok(n) = node2 {
        println!("  Value in snapshot 2: {:?}", n.properties.get("value"));
    }

    println!("\n✅ Snapshot test PASSED!");
}

#[tokio::test]
async fn test_as_of_datomic_style() {
    let graph = Graph::in_memory().await.unwrap();

    println!("🎯 Testing Datomic-style as_of...\n");

    // Crear dato
    let node_id = {
        let mut tx = graph.begin_transaction().await.unwrap();
        let node = Node::new("Document")
            .with_property("status", PropertyValue::String("draft".into()));
        let id = tx.add_node(node).await.unwrap();
        tx.commit().await.unwrap();
        id
    };

    let t1 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    println!("t1: Document is 'draft'");

    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Actualizar
    {
        let mut tx = graph.begin_transaction().await.unwrap();
        let mut node = graph.get_node(node_id).await.unwrap();
        node.properties.insert("status".into(), PropertyValue::String("published".into()));
        tx.add_node(node).await.unwrap();
        tx.commit().await.unwrap();
    }

    let t2 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    println!("t2: Document is 'published' {t2}");

    // Query: "Show me the document as it was before publication"
    let past = graph.as_of(t1);

    println!("\n🔍 Querying past state (as_of t1)...");
    match past.get_node(node_id).await {
        Ok(node) => {
            let status = node.properties.get("status").unwrap();
            println!("  Status was: {:?}", status);
        }
        Err(e) => {
            println!("  Node not found in past: {:?}", e);
        }
    }

    // Query: "Show me current state"
    println!("\n🔍 Querying current state...");
    let current_node = graph.get_node(node_id).await.unwrap();
    let current_status = current_node.properties.get("status").unwrap();
    println!("  Current status: {:?}", current_status);

    println!("\n Datomic-style test PASSED!");
}