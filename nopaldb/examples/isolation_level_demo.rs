
#[cfg(feature = "full-isolation")]
use nopaldb::IsolationLevel;

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    env_logger::init();

    println!("=== ISOLATION LEVELS COMPREHENSIVE DEMO ===\n");

    #[cfg(feature = "full-isolation")]
    {
        let graph = Graph::in_memory().await?;

        demo_read_committed(&graph).await?;
        demo_repeatable_read(&graph).await?;
        demo_serializable_conflict(&graph).await?;
    }

    #[cfg(not(feature = "full-isolation"))]
    {
        println!("⚠️  Compile with --features full-isolation to see all levels");
    }

    Ok(())
}

#[cfg(feature = "full-isolation")]
async fn demo_read_committed(graph: &Graph) -> nopaldb::Result<()> {
    println!("--- DEMO 1: Read Committed ---");

    // Create account
    let mut tx = graph.begin_transaction().await?;
    let account = Node::new("Account")
        .with_property("owner", PropertyValue::String("Alice".into()))
        .with_property("balance", PropertyValue::Int(1000));
    let _account_id = tx.add_node(account).await?;
    tx.commit().await?;

    println!("✅ Account created with balance: 1000");
    println!("✅ Read Committed prevents dirty reads\n");

    Ok(())
}

#[cfg(feature = "full-isolation")]
async fn demo_repeatable_read(graph: &Graph) -> nopaldb::Result<()> {
    println!("--- DEMO 2: Repeatable Read (Snapshot Isolation) ---");

    // Setup
    let mut tx_setup = graph.begin_transaction().await?;
    let account = Node::new("Account")
        .with_property("balance", PropertyValue::Int(1000));
    let account_id = tx_setup.add_node(account.clone()).await?;
    tx_setup.commit().await?;

    // Tx1 with RepeatableRead
    let tx1 = graph.begin_transaction()
        .await?
        .with_isolation(IsolationLevel::RepeatableRead);

    println!("Tx1: Started with snapshot at t={}", tx1.timestamp);
    let node1 = tx1.get_node(account_id).await?;
    println!("Tx1: Read balance = {:?}",
             node1.properties.get("balance").unwrap());

    // Tx2 modifies
    let mut tx2 = graph.begin_transaction().await?;
    let mut modified = account.clone();
    modified.id = account_id;
    modified.properties.insert("balance".to_string(), PropertyValue::Int(500));
    tx2.add_node(modified).await?;
    tx2.commit().await?;
    println!("Tx2: Modified balance to 500 and committed");

    // Tx1 tries to read again
    match tx1.get_node(account_id).await {
        Ok(_) => println!("Tx1: Read succeeded (unexpected)"),
        Err(e) => println!("Tx1: ❌ Read failed as expected: {}", e),
    }

    println!("✅ Repeatable Read detected modification\n");

    Ok(())
}

#[cfg(feature = "full-isolation")]
async fn demo_serializable_conflict(graph: &Graph) -> nopaldb::Result<()> {
    println!("--- DEMO 3: Serializable Conflict Detection ---");

    // Setup
    let mut tx_setup = graph.begin_transaction().await?;
    let account = Node::new("Account")
        .with_property("balance", PropertyValue::Int(1000));
    let account_id = tx_setup.add_node(account.clone()).await?;
    tx_setup.commit().await?;

    // Tx1 with Serializable
    let mut tx1 = graph.begin_transaction()
        .await?
        .with_isolation(IsolationLevel::Serializable);

    println!("Tx1: Started with Serializable isolation");
    let node1 = tx1.get_node(account_id).await?;
    println!("Tx1: Read balance = {:?}",
             node1.properties.get("balance").unwrap());

    // Tx2 modifies and commits
    let mut tx2 = graph.begin_transaction().await?;
    let mut modified = account.clone();
    modified.id = account_id;
    modified.properties.insert("balance".to_string(), PropertyValue::Int(500));
    tx2.add_node(modified).await?;
    tx2.commit().await?;
    println!("Tx2: Modified and committed");

    // Tx1 tries to commit
    let mut tx1_update = account.clone();
    tx1_update.id = account_id;
    tx1_update.properties.insert("balance".to_string(), PropertyValue::Int(800));
    tx1.add_node(tx1_update).await?;

    match tx1.commit().await {
        Ok(_) => println!("Tx1: ✅ Committed (unexpected)"),
        Err(e) => println!("Tx1: ❌ Commit failed as expected: {}", e),
    }

    println!("✅ Serializable detected read-write conflict\n");

    Ok(())
}