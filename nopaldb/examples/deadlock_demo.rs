// examples/deadlock_demo.rs


#[cfg(feature = "full-isolation")]
use nopaldb::IsolationLevel;


#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    env_logger::init();

    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║        NOPALDB - DEADLOCK DETECTION DEMO                  ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");

    #[cfg(feature = "full-isolation")]
    {
        demo_no_deadlock().await?;
        println!("\n{}\n", "═".repeat(60));

        demo_deadlock_detection().await?;
        println!("\n{}\n", "═".repeat(60));

        demo_lock_waiting().await?;
    }

    #[cfg(not(feature = "full-isolation"))]
    {
        println!("⚠️  Compile with --features full-isolation to see deadlock detection");
    }

    Ok(())
}

#[cfg(feature = "full-isolation")]
async fn demo_no_deadlock() -> nopaldb::Result<()> {
    println!("📗 DEMO 1: No Deadlock (ReadCommitted)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let graph = Graph::in_memory().await?;

    // Con ReadCommitted no hay locks
    let mut tx1 = graph.begin_transaction()
        .await?
        .with_isolation(IsolationLevel::ReadCommitted);

    let alice = Node::new("Account")
        .with_property("balance", PropertyValue::Int(1000));

    let alice_id = tx1.add_node(alice).await?;
    tx1.commit().await?;

    println!("✅ Tx1 committed with ReadCommitted");

    // Múltiples tx pueden leer sin problema
    let tx2 = graph.begin_transaction()
        .await?
        .with_isolation(IsolationLevel::ReadCommitted);

    let node = tx2.get_node(alice_id).await?;
    println!("✅ Tx2 read node: {:?}", node.properties.get("balance"));

    println!("\n💡 ReadCommitted: No locks = No deadlocks");

    Ok(())
}

#[cfg(feature = "full-isolation")]
async fn demo_deadlock_detection() -> nopaldb::Result<()> {
    println!("📕 DEMO 2: Deadlock Detection (Serializable)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let graph = Arc::new(Graph::in_memory().await?);

    // Setup: Dos cuentas bancarias
    let mut setup = graph.begin_transaction().await?;

    let alice = Node::new("Account")
        .with_property("owner", PropertyValue::String("Alice".into()))
        .with_property("balance", PropertyValue::Int(1000));

    let bob = Node::new("Account")
        .with_property("owner", PropertyValue::String("Bob".into()))
        .with_property("balance", PropertyValue::Int(500));

    let alice_id = setup.add_node(alice).await?;
    let bob_id = setup.add_node(bob).await?;
    setup.commit().await?;

    println!("Setup: Alice (balance=1000), Bob (balance=500)");
    println!();

    let g1 = graph.clone();
    let g2 = graph.clone();

    println!("🔄 Starting concurrent transactions...\n");

    // Tx1: Transferencia Alice → Bob
    let handle1 = tokio::spawn(async move {
        let mut tx1 = g1.begin_transaction()
            .await.unwrap()
            .with_isolation(IsolationLevel::Serializable);

        println!("  Tx1: 🔒 Locking Alice...");
        let mut alice_mod = Node::new("Account")
            .with_property("balance", PropertyValue::Int(800));
        alice_mod.id = alice_id;
        tx1.add_node(alice_mod).await.unwrap();

        println!("  Tx1: ✓ Got Alice, waiting 200ms...");
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        println!("  Tx1: 🔒 Trying to lock Bob...");
        let mut bob_mod = Node::new("Account")
            .with_property("balance", PropertyValue::Int(700));
        bob_mod.id = bob_id;

        match tx1.add_node(bob_mod).await {
            Ok(_) => {
                tx1.commit().await.unwrap();
                println!("  Tx1: ✅ SUCCESS - Transferred $200 to Bob");
                Ok(())
            }
            Err(e) => {
                println!("  Tx1: ❌ ABORTED - {}", e);
                Err(e)
            }
        }
    });

    // Tx2: Transferencia Bob → Alice (DEADLOCK!)
    let handle2 = tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let mut tx2 = g2.begin_transaction()
            .await.unwrap()
            .with_isolation(IsolationLevel::Serializable);

        println!("  Tx2: 🔒 Locking Bob...");
        let mut bob_mod = Node::new("Account")
            .with_property("balance", PropertyValue::Int(400));
        bob_mod.id = bob_id;
        tx2.add_node(bob_mod).await.unwrap();

        println!("  Tx2: ✓ Got Bob, waiting 200ms...");
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        println!("  Tx2: 🔒 Trying to lock Alice...");
        let mut alice_mod = Node::new("Account")
            .with_property("balance", PropertyValue::Int(1100));
        alice_mod.id = alice_id;

        match tx2.add_node(alice_mod).await {
            Ok(_) => {
                tx2.commit().await.unwrap();
                println!("  Tx2: ✅ SUCCESS - Transferred $100 to Alice");
                Ok(())
            }
            Err(e) => {
                println!("  Tx2: ❌ ABORTED - {}", e);
                Err(e)
            }
        }
    });

    let r1 = handle1.await.unwrap();
    let r2 = handle2.await.unwrap();

    println!();

    if r1.is_err() || r2.is_err() {
        println!("🎯 Deadlock detected and resolved!");
        println!("   - One transaction aborted (victim)");
        println!("   - Other transaction completed successfully");
    }

    println!("\n💡 Serializable: Detects deadlocks automatically");
    println!("   Victim selection: Most recent transaction aborted");

    Ok(())
}

#[cfg(feature = "full-isolation")]
async fn demo_lock_waiting() -> nopaldb::Result<()> {
    println!("📘 DEMO 3: Lock Waiting & Wake-up");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let graph = Arc::new(Graph::in_memory().await?);

    // Setup
    let mut setup = graph.begin_transaction().await?;
    let resource_id = setup.add_node(Node::new("SharedResource")).await?;
    setup.commit().await?;

    println!("Setup: Shared resource created");
    println!();

    let g1 = graph.clone();
    let g2 = graph.clone();

    println!("🔄 Starting transactions...\n");

    // Tx1: Holds lock for 1 second
    let handle1 = tokio::spawn(async move {
        let mut tx = g1.begin_transaction()
            .await.unwrap()
            .with_isolation(IsolationLevel::Serializable);

        println!("  Tx1: 🔒 Acquiring lock...");
        let mut node = Node::new("SharedResource");
        node.id = resource_id;
        tx.add_node(node).await.unwrap();

        println!("  Tx1: ✓ Lock acquired, working for 1.5s...");
        tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;

        println!("  Tx1: 💾 Committing (releasing lock)...");
        tx.commit().await.unwrap();
        println!("  Tx1: ✅ Done");
    });

    // Tx2: Waits for lock
    let handle2 = tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;

        let mut tx = g2.begin_transaction()
            .await.unwrap()
            .with_isolation(IsolationLevel::Serializable);

        println!("  Tx2: 🔒 Trying to acquire lock...");
        println!("  Tx2: ⏳ Waiting for Tx1 to release...");

        let start = std::time::Instant::now();

        let mut node = Node::new("SharedResource");
        node.id = resource_id;
        tx.add_node(node).await.unwrap();

        let waited = start.elapsed();

        println!("  Tx2: ✓ Lock acquired after {:?}", waited);
        tx.commit().await.unwrap();
        println!("  Tx2: ✅ Done");
    });

    handle1.await.unwrap();
    handle2.await.unwrap();

    println!("\n💡 Lock waiting: Tx2 automatically waited for Tx1");
    println!("   Wake-up: Tx2 resumed immediately after Tx1 released lock");

    Ok(())
}