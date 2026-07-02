// examples/gnn_node_classification.rs

use nopaldb::{Graph, Node, Edge, PropertyValue, Direction};
use std::collections::HashMap;

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║        GRAPH NEURAL NETWORK - NODE CLASSIFICATION         ║");
    println!("║              (Karate Club - Community Detection)           ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");

    let graph = Graph::in_memory().await?;

    println!("📊 Creating Karate Club network...");
    let nodes = create_karate_club(&graph).await?;
    println!("   ✓ {} members created", nodes.len());
    println!("   ✓ Ground truth: Members 0-4 (Community A), 5-9 (Community B)\n");

    println!("🔢 Extracting node features (degree + community bias)...");
    let features = extract_features(&graph, &nodes).await?;

    println!("🧠 Running GCN layers (message passing)...");
    let embeddings1 = gcn_layer(&graph, &nodes, &features).await?;
    println!("   ✓ Layer 1 complete");

    let embeddings2 = gcn_layer(&graph, &nodes, &embeddings1).await?;
    println!("   ✓ Layer 2 complete");

    println!("\n👥 Community predictions:");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let mut correct = 0;
    let mut total = 0;

    for (i, node_id) in nodes.iter().enumerate() {
        let embedding = embeddings2.get(node_id).unwrap();

        let predicted = if *embedding > 0.0 { "A" } else { "B" };
        let actual = if i < 5 { "A" } else { "B" };
        let correct_mark = if predicted == actual { "✓" } else { "✗" };

        if predicted == actual {
            correct += 1;
        }
        total += 1;

        println!("  Member {}: {} → {} (embedding: {:.3}) {}",
                 i, actual, predicted, embedding, correct_mark);
    }

    let accuracy = (correct as f64 / total as f64) * 100.0;
    println!("\n📊 Accuracy: {}/{} ({:.1}%)", correct, total, accuracy);

    if accuracy >= 80.0 {
        println!("🎉 Great! The GCN successfully detected communities.");
    }

    println!();
    Ok(())
}

async fn create_karate_club(graph: &Graph) -> nopaldb::Result<Vec<uuid::Uuid>> {
    let mut members = Vec::new();

    for i in 0..10 {
        let community = if i < 5 { 0 } else { 1 };
        let node = graph.add_node(Node::new("Member")
            .with_property("id", PropertyValue::Int(i))
            .with_property("community", PropertyValue::Int(community))
        ).await?;
        members.push(node);
    }

    // Community A (dense internal connections)
    graph.add_edge(Edge::new(members[0], members[1], "FRIEND")).await?;
    graph.add_edge(Edge::new(members[0], members[2], "FRIEND")).await?;
    graph.add_edge(Edge::new(members[0], members[3], "FRIEND")).await?;
    graph.add_edge(Edge::new(members[1], members[2], "FRIEND")).await?;
    graph.add_edge(Edge::new(members[1], members[3], "FRIEND")).await?;
    graph.add_edge(Edge::new(members[2], members[3], "FRIEND")).await?;
    graph.add_edge(Edge::new(members[2], members[4], "FRIEND")).await?;
    graph.add_edge(Edge::new(members[3], members[4], "FRIEND")).await?;

    // Community B (dense internal connections)
    graph.add_edge(Edge::new(members[5], members[6], "FRIEND")).await?;
    graph.add_edge(Edge::new(members[5], members[7], "FRIEND")).await?;
    graph.add_edge(Edge::new(members[5], members[8], "FRIEND")).await?;
    graph.add_edge(Edge::new(members[6], members[7], "FRIEND")).await?;
    graph.add_edge(Edge::new(members[6], members[8], "FRIEND")).await?;
    graph.add_edge(Edge::new(members[7], members[8], "FRIEND")).await?;
    graph.add_edge(Edge::new(members[7], members[9], "FRIEND")).await?;
    graph.add_edge(Edge::new(members[8], members[9], "FRIEND")).await?;

    // Bridge edge
    graph.add_edge(Edge::new(members[4], members[5], "FRIEND")).await?;

    Ok(members)
}

async fn extract_features(
    graph: &Graph,
    nodes: &[uuid::Uuid],
) -> nopaldb::Result<HashMap<uuid::Uuid, f64>> {
    let mut features = HashMap::new();

    for (i, &node_id) in nodes.iter().enumerate() {
        let edges = graph.edges_of(node_id, Direction::Both).await?;
        let degree = edges.len() as f64;

        // Community A: positive bias, Community B: negative bias
        let community_bias = if i < 5 { 0.5 } else { -0.5 };
        let feature = (degree / 10.0) + community_bias;

        features.insert(node_id, feature);
    }

    Ok(features)
}

async fn gcn_layer(
    graph: &Graph,
    nodes: &[uuid::Uuid],
    features: &HashMap<uuid::Uuid, f64>,
) -> nopaldb::Result<HashMap<uuid::Uuid, f64>> {
    let mut new_features = HashMap::new();

    for &node_id in nodes {
        let edges = graph.edges_of(node_id, Direction::Both).await?;
        let self_degree = (edges.len() + 1) as f64;

        let mut aggregated = 0.0;

        for edge in &edges {
            let neighbor_id = if edge.source == node_id {
                edge.target
            } else {
                edge.source
            };

            if let Some(&neighbor_feat) = features.get(&neighbor_id) {
                let neighbor_edges = graph.edges_of(neighbor_id, Direction::Both).await?;
                let neighbor_degree = (neighbor_edges.len() + 1) as f64;

                let norm = 1.0 / (self_degree * neighbor_degree).sqrt();
                aggregated += neighbor_feat * norm;
            }
        }

        // Self-loop
        if let Some(&self_feat) = features.get(&node_id) {
            let norm = 1.0 / self_degree;
            aggregated += self_feat * norm;
        }

        let activated = aggregated.tanh();
        new_features.insert(node_id, activated);
    }

    Ok(new_features)
}