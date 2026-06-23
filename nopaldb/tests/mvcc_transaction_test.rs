// tests/mvcc_transaction_test.rs

use nopaldb::Result;
use nopaldb::{Graph, Node, PropertyValue};

#[tokio::test]
async fn test_transaction_creates_versions() {
    let graph = Graph::in_memory().await.unwrap();

    println!("🔄 Testing automatic version creation in transactions...\n");

    // ═══════════════════════════════════════════════════════
    // Transaction 1: CREATE
    // ═══════════════════════════════════════════════════════

    let node_id = {
        let mut tx = graph.begin_transaction().await.unwrap();
        let node = Node::new("Person")
            .with_property("name", PropertyValue::String("Alice".into()))
            .with_property("age", PropertyValue::Int(25));
        let id = tx.add_node(node).await.unwrap();
        tx.commit().await.unwrap();
        println!("✅ Transaction 1: Created Alice (age=25)");
        id
    };

    // Verificar: debe tener versión 1
    let history = graph.history(node_id).await.unwrap();
    assert_eq!(history.len(), 1, "Should have 1 version after create");
    assert_eq!(history[0].version, 1);
    println!("   Version 1 created");

    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // ═══════════════════════════════════════════════════════
    // Transaction 2: UPDATE
    // ═══════════════════════════════════════════════════════

    {
        let mut tx = graph.begin_transaction().await.unwrap();
        let mut node = graph.get_node(node_id).await.unwrap();
        node.properties.insert("age".into(), PropertyValue::Int(30));
        tx.add_node(node).await.unwrap();
        tx.commit().await.unwrap();
        println!("\n✅ Transaction 2: Updated Alice (age=30)");
    }

    // Verificar: debe tener versión 2
    let history = graph.history(node_id).await.unwrap();
    assert_eq!(history.len(), 2, "Should have 2 versions after update");
    assert_eq!(history[0].version, 2, "Latest should be v2");
    assert_eq!(history[1].version, 1, "Older should be v1");
    println!("   Version 2 created");

    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // ═══════════════════════════════════════════════════════
    // Transaction 3: UPDATE again
    // ═══════════════════════════════════════════════════════

    {
        let mut tx = graph.begin_transaction().await.unwrap();
        let mut node = graph.get_node(node_id).await.unwrap();
        node.properties.insert("age".into(), PropertyValue::Int(35));
        node.properties
            .insert("city".into(), PropertyValue::String("NYC".into()));
        tx.add_node(node).await.unwrap();
        tx.commit().await.unwrap();
        println!("\n✅ Transaction 3: Updated Alice (age=35, city=NYC)");
    }

    // Verificar: debe tener versión 3
    let history = graph.history(node_id).await.unwrap();
    assert_eq!(history.len(), 3, "Should have 3 versions");
    println!("   Version 3 created");

    // ═══════════════════════════════════════════════════════
    // Verificar version chain
    // ═══════════════════════════════════════════════════════

    println!("\n📜 Complete version chain:");
    for version in &history {
        let age = version.node_data.properties.get("age").unwrap();
        let city = version.node_data.properties.get("city");
        println!(
            "   v{}: age={:?}, city={:?} (t={})",
            version.version, age, city, version.timestamp
        );
    }

    // Verificar version links
    assert_eq!(history[0].version, 3);
    assert_eq!(history[0].prev_version, Some(2));

    assert_eq!(history[1].version, 2);
    assert_eq!(history[1].prev_version, Some(1));

    assert_eq!(history[2].version, 1);
    assert_eq!(history[2].prev_version, None);

    println!("\n✅ Version chain verified!");
}

// tests/mvcc_transaction_test.rs

#[tokio::test]
async fn test_mvcc_with_concurrent_reads() -> Result<()> {
    let graph = Graph::in_memory().await?;

    println!("🔀 Testing MVCC with concurrent reads...\n");

    // Crear nodo inicial
    let node_id = {
        let mut tx = graph.begin_transaction().await?;
        let node = Node::new("Counter").with_property("value", PropertyValue::Int(0));
        let id = tx.add_node(node).await?;
        tx.commit().await?;
        id
    };

    println!("t1: Counter created (value=0), waiting for logical MVCC timestamp...");

    // Update counter
    {
        let mut tx = graph.begin_transaction().await?;
        let mut node = graph.get_node(node_id).await?;
        node.properties
            .insert("value".into(), PropertyValue::Int(100));
        tx.add_node(node).await?;
        tx.commit().await?;
    }

    // Usar timestamps lógicos MVCC del historial (no reloj del sistema).
    let history = graph.history(node_id).await?;
    let v2 = history
        .iter()
        .find(|v| v.version == 2)
        .expect("v2 must exist");
    let v1 = history
        .iter()
        .find(|v| v.version == 1)
        .expect("v1 must exist");
    let t1 = v1.valid_from;
    let t2 = v2.valid_from;

    println!("t1: Counter created (value=0), logical timestamp {}", t1);
    println!("t2: Counter updated (value=100), logical timestamp {}", t2);

    // ═══════════════════════════════════════════════════════
    // Leer en paralelo desde diferentes snapshots
    // ═══════════════════════════════════════════════════════

    println!("\n📸 Reading from different snapshots:");
    println!("   snapshot_past at t={}", t1);
    println!("   snapshot_present at t={}", t2);

    let snapshot_past = graph.as_of(t1);
    let snapshot_present = graph.as_of(t2);

    // ✅ DEBUGGING: Verificar historial
    println!("\n📜 Version history:");
    for v in &history {
        println!(
            "   v{}: value={:?}, valid_from={}, valid_to={:?}",
            v.version,
            v.node_data.properties.get("value"),
            v.valid_from,
            v.valid_to
        );
    }

    // Read past (debe ver 0)
    println!("\n🔍 Querying past snapshot (t={})...", t1);
    match snapshot_past.get_node(node_id).await {
        Ok(past_node) => {
            let value = past_node.properties.get("value").unwrap();
            println!("   Past snapshot value: {:?}", value);
            assert_eq!(
                value,
                &PropertyValue::Int(0),
                "Past snapshot should see old value (0)"
            );
        }
        Err(e) => {
            println!("   ❌ Error reading past snapshot: {:?}", e);
            panic!("Should have found version at t={}", t1);
        }
    }

    // Read present (debe ver 100)
    println!("\n🔍 Querying present snapshot (t={})...", t2);
    let present_node = snapshot_present.get_node(node_id).await?;
    let value = present_node.properties.get("value").unwrap();
    println!("   Present snapshot value: {:?}", value);
    assert_eq!(
        value,
        &PropertyValue::Int(100),
        "Present snapshot should see new value (100)"
    );

    // Read current (debe ver 100)
    println!("\n🔍 Querying current state...");
    let current_node = graph.get_node(node_id).await?;
    let current_value = current_node.properties.get("value").unwrap();
    println!("   Current state value: {:?}", current_value);
    assert_eq!(current_value, &PropertyValue::Int(100));

    println!("\n✅ MVCC concurrent reads successful!");
    println!("   - Past snapshot (t={}) correctly shows value=0", t1);
    println!("   - Present snapshot (t={}) correctly shows value=100", t2);

    Ok(())
}

#[tokio::test]
async fn test_mvcc_rollback_no_version() -> Result<()> {
    let graph = Graph::in_memory().await?;

    println!("↩️  Testing rollback creates no versions...\n");

    // Transaction committed
    let node_id = {
        let mut tx = graph.begin_transaction().await?;
        let node =
            Node::new("Test").with_property("status", PropertyValue::String("committed".into()));
        let id = tx.add_node(node).await?;
        tx.commit().await?;
        println!("✅ Transaction 1 committed");
        id
    };

    // Verificar 1 versión
    let history1 = graph.history(node_id).await?;
    assert_eq!(history1.len(), 1);

    // Transaction rolled back
    {
        let mut tx = graph.begin_transaction().await?;
        let mut node = graph.get_node(node_id).await?;
        node.properties.insert(
            "status".into(),
            PropertyValue::String("should_not_exist".into()),
        );
        tx.add_node(node).await?;
        tx.rollback()?; // ← Rollback
        println!("↩️  Transaction 2 rolled back");
    }

    // Verificar: TODAVÍA 1 versión (rollback no crea versión)
    let history2 = graph.history(node_id).await?;
    assert_eq!(history2.len(), 1, "Rollback should not create version");

    // Verificar contenido
    let node = graph.get_node(node_id).await?;
    assert_eq!(
        node.properties.get("status"),
        Some(&PropertyValue::String("committed".into()))
    );

    println!("✅ Rollback correctly prevented version creation!");
    Ok(())
}
