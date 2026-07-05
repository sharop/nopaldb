// examples/benchmarks.rs

use nopaldb::{Graph, Node, Edge, PropertyValue};

use std::time::Instant;

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║           NOPALDB - PERFORMANCE BENCHMARKS                ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");

    benchmark_node_insertion().await?;
    benchmark_edge_insertion().await?;
    benchmark_property_lookup().await?;
    benchmark_traversal().await?;
    benchmark_transactions().await?;

    println!("\n");
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║                 🌵 BENCHMARK COMPLETE 🌵                   ║");
    println!("╚════════════════════════════════════════════════════════════╝");

    // Easter egg
    nopaldb::easter_eggs::fun_facts();



    Ok(())
}

async fn benchmark_node_insertion() -> nopaldb::Result<()> {
    println!("📊 BENCHMARK 1: Node Insertion");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let graph = Graph::in_memory().await?;
    let iterations = [100, 1_000, 10_000];

    for &n in &iterations {
        let start = Instant::now();

        for i in 0..n {
            let node = Node::new("Person")
                .with_property("id", PropertyValue::Int(i as i64))
                .with_property("name", PropertyValue::String(format!("User{}", i)));

            graph.add_node(node).await?;
        }

        let elapsed = start.elapsed();
        let ops_per_sec = n as f64 / elapsed.as_secs_f64();

        println!("  {:>6} nodes: {:>8.2?}  ({:>10.0} ops/sec)",
                 n, elapsed, ops_per_sec);
    }

    println!();
    Ok(())
}

async fn benchmark_edge_insertion() -> nopaldb::Result<()> {
    println!("📊 BENCHMARK 2: Edge Insertion");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let graph = Graph::in_memory().await?;

    // Setup: Create 1000 nodes
    let mut node_ids = Vec::new();
    for i in 0..1000 {
        let node = Node::new("Node")
            .with_property("id", PropertyValue::Int(i));
        let id = graph.add_node(node).await?;
        node_ids.push(id);
    }

    let iterations = [100, 1_000, 5_000];

    for &n in &iterations {
        let start = Instant::now();

        for i in 0..n {
            let source = node_ids[i % 1000];
            let target = node_ids[(i + 1) % 1000];

            let edge = Edge::new(source, target, "CONNECTS");
            graph.add_edge(edge).await?;
        }

        let elapsed = start.elapsed();
        let ops_per_sec = n as f64 / elapsed.as_secs_f64();

        println!("  {:>6} edges: {:>8.2?}  ({:>10.0} ops/sec)",
                 n, elapsed, ops_per_sec);
    }

    println!();
    Ok(())
}

async fn benchmark_property_lookup() -> nopaldb::Result<()> {
    println!("📊 BENCHMARK 3: Property Index Lookup");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let graph = Graph::in_memory().await?;

    // Setup: 10,000 nodes with indexed properties
    for i in 0..10_000 {
        let node = Node::new("User")
            .with_property("email", PropertyValue::String(format!("user{}@example.com", i)))
            .with_property("age", PropertyValue::Int((i % 100) as i64));

        graph.add_node(node).await?;
    }

    let iterations = 1_000;
    let start = Instant::now();

    for i in 0..iterations {
        let email = format!("user{}@example.com", i % 10_000);
        let _ = graph.get_node_by_property("email", &email).await?;
    }

    let elapsed = start.elapsed();
    let ops_per_sec = iterations as f64 / elapsed.as_secs_f64();

    println!("  {:>6} lookups: {:>8.2?}  ({:>10.0} ops/sec)",
             iterations, elapsed, ops_per_sec);
    println!();
    Ok(())
}

async fn benchmark_traversal() -> nopaldb::Result<()> {
    println!("📊 BENCHMARK 4: Graph Traversal (BFS)");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let graph = Graph::in_memory().await?;

    // Setup: Create chain of 1000 nodes
    let mut prev_id = None;
    let mut start_id = None;

    for i in 0..1_000 {
        let node = Node::new("Node")
            .with_property("id", PropertyValue::Int(i));
        let id = graph.add_node(node).await?;

        if i == 0 {
            start_id = Some(id);
        }

        if let Some(prev) = prev_id {
            graph.add_edge(Edge::new(prev, id, "NEXT")).await?;
        }

        prev_id = Some(id);
    }

    let start = Instant::now();

    // ✅ USAR API DIRECTO DE GRAPH (no TraverseBuilder)
    use nopaldb::TraversalConfig;
    let config = TraversalConfig::new();
    let result = graph.bfs(start_id.unwrap(), config).await?;

    let elapsed = start.elapsed();

    println!("  Traversed {} nodes in {:?}", result.nodes.len(), elapsed);
    println!();
    Ok(())
}

async fn benchmark_transactions() -> nopaldb::Result<()> {
    println!("📊 BENCHMARK 5: Transactions");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let graph = Graph::in_memory().await?;
    let iterations = [10, 100, 500];

    for &n in &iterations {
        let start = Instant::now();

        for i in 0..n {
            let mut tx = graph.begin_transaction().await?;

            let node = Node::new("Account")
                .with_property("balance", PropertyValue::Int(i as i64));

            tx.add_node(node).await?;
            tx.commit().await?;
        }

        let elapsed = start.elapsed();
        let ops_per_sec = n as f64 / elapsed.as_secs_f64();

        println!("  {:>6} txs: {:>8.2?}  ({:>10.0} tx/sec)",
                 n, elapsed, ops_per_sec);
    }

    println!();
    Ok(())
}