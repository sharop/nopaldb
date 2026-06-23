use nopaldb::{Graph, Node, PropertyValue};

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    env_logger::init();

    let graph = Graph::in_memory().await?;

    println!("=== DEMO 1: Transacción exitosa ===\n");

    {
        let mut tx = graph.begin_transaction().await?;

        let alice = Node::new("Person")
            .with_property("name", PropertyValue::String("Alice".into()))
            .with_property("balance", PropertyValue::Int(1000));

        let bob = Node::new("Person")
            .with_property("name", PropertyValue::String("Bob".into()))
            .with_property("balance", PropertyValue::Int(500));

        let _alice_id = tx.add_node(alice).await?;
        let _bob_id = tx.add_node(bob).await?;

        println!("Agregados Alice (balance: 1000) y Bob (balance: 500)");

        tx.commit().await?;
        println!("✅ Transacción committed\n");
    }

    println!("=== DEMO 2: Transacción con rollback ===\n");

    {
        let mut tx = graph.begin_transaction().await?;

        let charlie =
            Node::new("Person").with_property("name", PropertyValue::String("Charlie".into()));

        tx.add_node(charlie).await?;
        println!("Agregado Charlie en transacción");

        tx.rollback()?;
        println!("❌ Transacción abortada - Charlie no existe\n");
    }

    println!("=== DEMO 3: Auto-rollback (Drop) ===\n");

    {
        let mut tx = graph.begin_transaction().await?;

        let diana =
            Node::new("Person").with_property("name", PropertyValue::String("Diana".into()));

        tx.add_node(diana).await?;
        println!("Agregada Diana en transacción");

        // No hacemos commit ni rollback - se auto-aborta al salir del scope
    }
    println!("⚠️  Transacción auto-abortada (Drop) - Diana no existe\n");

    println!("=== DEMO 4: Transferencia bancaria (atomicidad) ===\n");

    {
        let mut tx = graph.begin_transaction().await?;

        let alice = Node::new("Account")
            .with_property("owner", PropertyValue::String("Alice".into()))
            .with_property("balance", PropertyValue::Int(1000));

        let bob = Node::new("Account")
            .with_property("owner", PropertyValue::String("Bob".into()))
            .with_property("balance", PropertyValue::Int(500));

        let alice_id = tx.add_node(alice.clone()).await?;
        let bob_id = tx.add_node(bob.clone()).await?;

        println!("Estado inicial:");
        println!("  Alice: 1000");
        println!("  Bob:   500");

        // Transferir 200 de Alice a Bob
        let mut alice_updated = tx.get_node(alice_id).await?;
        if let Some(PropertyValue::Int(balance)) = alice_updated.properties.get_mut("balance") {
            *balance -= 200;
        }
        tx.add_node(alice_updated).await?; // Update

        let mut bob_updated = tx.get_node(bob_id).await?;
        if let Some(PropertyValue::Int(balance)) = bob_updated.properties.get_mut("balance") {
            *balance += 200;
        }
        tx.add_node(bob_updated).await?; // Update

        println!("\nDespués de transferir 200:");
        println!("  Alice: 800");
        println!("  Bob:   700");

        tx.commit().await?;
        println!("\n✅ Transferencia committed - atomicidad garantizada\n");
    }

    println!("✅ Demo completado");

    Ok(())
}
