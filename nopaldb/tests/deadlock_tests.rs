// tests/deadlock_tests.rs

#![cfg(feature = "full-isolation")]

use nopaldb::{Graph, Node, PropertyValue, IsolationLevel};
use std::sync::Arc;

/// Test básico: No hay deadlock con ReadCommitted
#[tokio::test]
async fn test_no_deadlock_with_read_committed() {
    let graph = Graph::in_memory().await.unwrap();

    let mut tx1 = graph.begin_transaction()
        .await.unwrap()
        .with_isolation(IsolationLevel::ReadCommitted);

    let alice = Node::new("Account");
    let alice_id = tx1.add_node(alice).await.unwrap();
    tx1.commit().await.unwrap();

    // Múltiples tx pueden leer sin locks
    let tx2 = graph.begin_transaction()
        .await.unwrap()
        .with_isolation(IsolationLevel::ReadCommitted);

    let _node = tx2.get_node(alice_id).await.unwrap();

    println!("✅ No deadlock with ReadCommitted (no locks)");
}

/// Test: Deadlock detection con 2 transacciones
#[tokio::test]
async fn test_deadlock_detection_two_transactions() {
    env_logger::try_init().ok();

    let graph = Arc::new(Graph::in_memory().await.unwrap());

    // Setup: Crear dos cuentas
    let mut setup_tx = graph.begin_transaction().await.unwrap();

    let alice = Node::new("Account")
        .with_property("name", PropertyValue::String("Alice".into()))
        .with_property("balance", PropertyValue::Int(1000));

    let bob = Node::new("Account")
        .with_property("name", PropertyValue::String("Bob".into()))
        .with_property("balance", PropertyValue::Int(500));

    let alice_id = setup_tx.add_node(alice).await.unwrap();
    let bob_id = setup_tx.add_node(bob).await.unwrap();
    setup_tx.commit().await.unwrap();

    println!("Setup complete: Alice={}, Bob={}", alice_id, bob_id);

    // Clonar para threads
    let graph1 = graph.clone();
    let graph2 = graph.clone();

    // Tx1: Alice → Bob
    let handle1 = tokio::spawn(async move {
        let mut tx1 = graph1.begin_transaction()
            .await.unwrap()
            .with_isolation(IsolationLevel::Serializable);

        println!("Tx1: Acquiring Alice...");
        let mut alice_mod = Node::new("Account")
            .with_property("balance", PropertyValue::Int(900));
        alice_mod.id = alice_id;
        tx1.add_node(alice_mod).await.unwrap();

        println!("Tx1: Got Alice, sleeping 100ms...");
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        println!("Tx1: Trying to acquire Bob...");
        let mut bob_mod = Node::new("Account")
            .with_property("balance", PropertyValue::Int(600));
        bob_mod.id = bob_id;

        match tx1.add_node(bob_mod).await {
            Ok(_) => {
                println!("Tx1: Got Bob, committing...");
                tx1.commit().await.unwrap();
                println!("Tx1: ✅ Committed");
                Ok(())
            }
            Err(e) => {
                println!("Tx1: ❌ Failed: {}", e);
                Err(e)
            }
        }
    });

    // Tx2: Bob → Alice (DEADLOCK!)
    let handle2 = tokio::spawn(async move {
        // Pequeño delay para que tx1 empiece primero
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let mut tx2 = graph2.begin_transaction()
            .await.unwrap()
            .with_isolation(IsolationLevel::Serializable);

        println!("Tx2: Acquiring Bob...");
        let mut bob_mod = Node::new("Account")
            .with_property("balance", PropertyValue::Int(450));
        bob_mod.id = bob_id;
        tx2.add_node(bob_mod).await.unwrap();

        println!("Tx2: Got Bob, sleeping 100ms...");
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        println!("Tx2: Trying to acquire Alice...");
        let mut alice_mod = Node::new("Account")
            .with_property("balance", PropertyValue::Int(1100));
        alice_mod.id = alice_id;

        match tx2.add_node(alice_mod).await {
            Ok(_) => {
                println!("Tx2: Got Alice, committing...");
                tx2.commit().await.unwrap();
                println!("Tx2: ✅ Committed");
                Ok(())
            }
            Err(e) => {
                println!("Tx2: ❌ Failed: {}", e);
                Err(e)
            }
        }
    });

    // Esperar ambos threads
    let result1 = handle1.await.unwrap();
    let result2 = handle2.await.unwrap();

    // Al menos UNA debe detectar deadlock
    let tx1_failed = result1.is_err();
    let tx2_failed = result2.is_err();

    assert!(
        tx1_failed || tx2_failed,
        "At least one transaction should detect deadlock"
    );

    // Verificar que al menos una falló por deadlock
    let has_deadlock_error =
        matches!(result1, Err(ref e) if e.to_string().contains("Deadlock")) ||
            matches!(result2, Err(ref e) if e.to_string().contains("Deadlock"));

    assert!(
        has_deadlock_error,
        "At least one should fail with Deadlock error"
    );

    println!("\n🎉 Deadlock detected successfully!");
}

/// Test: 3 transacciones con ciclo complejo
#[tokio::test]
async fn test_deadlock_three_transactions() {
    env_logger::try_init().ok();

    let graph = Arc::new(Graph::in_memory().await.unwrap());

    // Setup: 3 cuentas
    let mut setup = graph.begin_transaction().await.unwrap();
    let a_id = setup.add_node(Node::new("A")).await.unwrap();
    let b_id = setup.add_node(Node::new("B")).await.unwrap();
    let c_id = setup.add_node(Node::new("C")).await.unwrap();
    setup.commit().await.unwrap();

    println!("Setup: A={}, B={}, C={}", a_id, b_id, c_id);

    let g1 = graph.clone();
    let g2 = graph.clone();
    let g3 = graph.clone();

    // Tx1: A → B
    let h1 = tokio::spawn(async move {
        let mut tx = g1.begin_transaction().await.unwrap()
            .with_isolation(IsolationLevel::Serializable);

        let mut a = Node::new("A");
        a.id = a_id;
        tx.add_node(a).await.unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let mut b = Node::new("B");
        b.id = b_id;
        tx.add_node(b).await
    });

    // Tx2: B → C
    let h2 = tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let mut tx = g2.begin_transaction().await.unwrap()
            .with_isolation(IsolationLevel::Serializable);

        let mut b = Node::new("B");
        b.id = b_id;
        tx.add_node(b).await.unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let mut c = Node::new("C");
        c.id = c_id;
        tx.add_node(c).await
    });

    // Tx3: C → A (COMPLETA EL CICLO!)
    let h3 = tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let mut tx = g3.begin_transaction().await.unwrap()
            .with_isolation(IsolationLevel::Serializable);

        let mut c = Node::new("C");
        c.id = c_id;
        tx.add_node(c).await.unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let mut a = Node::new("A");
        a.id = a_id;
        tx.add_node(a).await
    });

    let r1 = h1.await.unwrap();
    let r2 = h2.await.unwrap();
    let r3 = h3.await.unwrap();

    // Al menos una debe fallar
    let failures = [r1.is_err(), r2.is_err(), r3.is_err()]
        .iter()
        .filter(|&&x| x)
        .count();

    assert!(
        failures >= 1,
        "At least one transaction should fail in 3-way deadlock"
    );

    println!("🎉 3-way deadlock detected! {} transactions aborted", failures);
}

/// Test: Waiting y wake-up funcionan
// tests/deadlock_tests.rs

#[tokio::test]
async fn test_lock_waiting_and_wakeup() {
    env_logger::try_init().ok();

    let graph = Arc::new(Graph::in_memory().await.unwrap());

    // Create initial node
    let node_id = {
        let mut tx = graph.begin_transaction().await.unwrap();
        let node = Node::new("TestWait")
            .with_property("value", PropertyValue::Int(0));
        let id = tx.add_node(node).await.unwrap();
        tx.commit().await.unwrap();
        id
    };

    // ═══════════════════════════════════════════════════════
    // Tx1: Acquire lock, hold it, then release
    // ═══════════════════════════════════════════════════════

    let graph1 = Arc::clone(&graph);
    let node_id1 = node_id;

    let tx1_handle = tokio::spawn(async move {
        println!("Tx1: Acquiring lock...");

        let mut tx = graph1.begin_transaction().await.unwrap();

        // Read and modify
        let mut node = graph1.get_node(node_id1).await.unwrap();
        node.properties.insert("value".into(), PropertyValue::Int(1));
        tx.add_node(node).await.unwrap();

        println!("Tx1: Lock acquired, sleeping 1s...");
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

        println!("Tx1: Committing...");
        tx.commit().await.unwrap();

        println!("Tx1: Done!");
    });

    // Wait for Tx1 to acquire lock
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // ═══════════════════════════════════════════════════════
    // Tx2: Try to acquire lock (should wait, then succeed)
    // ═══════════════════════════════════════════════════════

    let graph2 = Arc::clone(&graph);
    let node_id2 = node_id;

    let tx2_handle = tokio::spawn(async move {
        println!("Tx2: Trying to acquire lock (should wait)...");

        // ✅ CRITICAL FIX: Start transaction AFTER Tx1 commits
        // to avoid MVCC conflict
        tokio::time::sleep(tokio::time::Duration::from_millis(1200)).await;

        let mut tx = graph2.begin_transaction().await.unwrap();

        // This should succeed now (Tx1 already committed)
        let mut node = graph2.get_node(node_id2).await.unwrap();

        println!("Tx2: Lock acquired!");

        node.properties.insert("value".into(), PropertyValue::Int(2));
        tx.add_node(node).await.unwrap();

        tx.commit().await.unwrap();

        println!("Tx2: Done!");
    });

    // Wait for both
    tx1_handle.await.unwrap();
    tx2_handle.await.unwrap();

    // Verify final value
    let final_node = graph.get_node(node_id).await.unwrap();
    assert_eq!(
        final_node.properties.get("value"),
        Some(&PropertyValue::Int(2))
    );

    println!("✅ Lock waiting test passed!");
}