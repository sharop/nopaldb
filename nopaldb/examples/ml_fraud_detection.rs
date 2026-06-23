// examples/ml_fraud_detection.rs

use nopaldb::{Direction, Edge, Graph, Node, PropertyValue};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║          FRAUD DETECTION - TRANSACTION NETWORK             ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");

    let graph = Graph::in_memory().await?;

    println!("💰 Creando red de transacciones...");
    let accounts = create_transaction_network(&graph).await?;
    println!("   ✓ {} cuentas creadas\n", accounts.len());

    println!("🔍 Análisis de fraude:");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    for &account_id in &accounts {
        let risk_score = calculate_fraud_risk(&graph, account_id).await?;
        let account = graph.get_node(account_id).await?;
        let name = account.properties.get("name").unwrap();

        let risk_level = if risk_score > 70.0 {
            "🔴 HIGH"
        } else if risk_score > 40.0 {
            "🟡 MEDIUM"
        } else {
            "🟢 LOW"
        };

        println!("  {:?}: {} (score: {:.1})", name, risk_level, risk_score);
    }

    println!();
    Ok(())
}

async fn create_transaction_network(graph: &Graph) -> nopaldb::Result<Vec<uuid::Uuid>> {
    let alice = graph
        .add_node(
            Node::new("Account")
                .with_property("name", PropertyValue::String("Alice".into()))
                .with_property("balance", PropertyValue::Int(10000)),
        )
        .await?;

    let bob = graph
        .add_node(
            Node::new("Account")
                .with_property("name", PropertyValue::String("Bob".into()))
                .with_property("balance", PropertyValue::Int(5000)),
        )
        .await?;

    let charlie = graph
        .add_node(
            Node::new("Account")
                .with_property("name", PropertyValue::String("Charlie".into()))
                .with_property("balance", PropertyValue::Int(1000)),
        )
        .await?;

    let dana = graph
        .add_node(
            Node::new("Account")
                .with_property("name", PropertyValue::String("Dana".into()))
                .with_property("balance", PropertyValue::Int(1000)),
        )
        .await?;

    let eve = graph
        .add_node(
            Node::new("Account")
                .with_property("name", PropertyValue::String("Eve".into()))
                .with_property("balance", PropertyValue::Int(1000)),
        )
        .await?;

    // Transacciones normales
    graph
        .add_edge(
            Edge::new(alice, bob, "TRANSFER").with_property("amount", PropertyValue::Int(100)),
        )
        .await?;

    graph
        .add_edge(Edge::new(bob, alice, "TRANSFER").with_property("amount", PropertyValue::Int(50)))
        .await?;

    // Patrón sospechoso: Circular transfer
    graph
        .add_edge(
            Edge::new(charlie, dana, "TRANSFER").with_property("amount", PropertyValue::Int(500)),
        )
        .await?;

    graph
        .add_edge(Edge::new(dana, eve, "TRANSFER").with_property("amount", PropertyValue::Int(500)))
        .await?;

    graph
        .add_edge(
            Edge::new(eve, charlie, "TRANSFER").with_property("amount", PropertyValue::Int(500)),
        )
        .await?;

    Ok(vec![alice, bob, charlie, dana, eve])
}

async fn transaction_velocity(graph: &Graph, account_id: uuid::Uuid) -> nopaldb::Result<usize> {
    let outgoing = graph.edges_of(account_id, Direction::Outgoing).await?;
    let incoming = graph.edges_of(account_id, Direction::Incoming).await?;

    Ok(outgoing.len() + incoming.len())
}

async fn has_structuring_pattern(graph: &Graph, account_id: uuid::Uuid) -> nopaldb::Result<bool> {
    let edges = graph.edges_of(account_id, Direction::Outgoing).await?;

    let mut amounts: HashMap<i64, usize> = HashMap::new();

    for edge in edges {
        if let Some(PropertyValue::Int(amount)) = edge.properties.get("amount") {
            *amounts.entry(*amount).or_insert(0) += 1;
        }
    }

    Ok(amounts.values().any(|&count| count >= 3))
}

// Reemplazar calculate_fraud_risk:

async fn calculate_fraud_risk(graph: &Graph, account_id: uuid::Uuid) -> nopaldb::Result<f64> {
    let mut risk_score: f64 = 0.0;

    // FEATURE 1: Ciclos de 3+ nodos (money laundering)
    if has_suspicious_circular_transfers(graph, account_id).await? {
        risk_score += 50.0;
    }

    // FEATURE 2: High velocity
    let velocity = transaction_velocity(graph, account_id).await?;
    if velocity > 3 {
        risk_score += 20.0;
    }

    // FEATURE 3: Structuring
    if has_structuring_pattern(graph, account_id).await? {
        risk_score += 30.0;
    }

    Ok(risk_score.min(100.0))
}

/// Detecta ciclos sospechosos (3+ nodos)
async fn has_suspicious_circular_transfers(
    graph: &Graph,
    start_account: uuid::Uuid,
) -> nopaldb::Result<bool> {
    let mut visited = std::collections::HashSet::new();
    let mut path = Vec::new();

    detect_cycle_with_length(graph, start_account, &mut visited, &mut path).await
}

#[async_recursion::async_recursion]
async fn detect_cycle_with_length(
    graph: &Graph,
    current: uuid::Uuid,
    visited: &mut std::collections::HashSet<uuid::Uuid>,
    path: &mut Vec<uuid::Uuid>,
) -> nopaldb::Result<bool> {
    if let Some(pos) = path.iter().position(|&n| n == current) {
        // Encontramos ciclo
        let cycle_length = path.len() - pos;

        // Solo ciclos de 3+ nodos son sospechosos
        // (ciclos de 2 son normales: A→B→A es reciprocidad normal)
        return Ok(cycle_length >= 3);
    }

    if visited.contains(&current) {
        return Ok(false);
    }

    visited.insert(current);
    path.push(current);

    let edges = graph.edges_of(current, Direction::Outgoing).await?;

    for edge in edges {
        if edge.edge_type == "TRANSFER" {
            if detect_cycle_with_length(graph, edge.target, visited, path).await? {
                return Ok(true);
            }
        }
    }

    path.pop();
    Ok(false)
}
