// examples/gnn_graph_embeddings.rs
//
// Graph Embeddings usando Random Walks (Node2Vec-style)
// Dataset: Citation Network
// Task: Learn node embeddings, compute similarity

use nopaldb::{Graph, Node, Edge, PropertyValue, Direction};
use std::collections::HashMap;
use rand::Rng;

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║          GRAPH EMBEDDINGS - RANDOM WALKS                  ║");
    println!("║             (Node2Vec-style for Citation Network)         ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");

    let graph = Graph::in_memory().await?;

    // 1. Create citation network
    println!("📊 Creating citation network...");
    let papers = create_citation_network(&graph).await?;
    println!("   ✓ {} papers created\n", papers.len());

    // 2. Generate random walks
    println!("🚶 Generating random walks...");
    let walks = generate_random_walks(&graph, &papers, 5, 10).await?;
    println!("   ✓ Generated {} walks", walks.len());

    // 3. Learn embeddings (simplified - using co-occurrence)
    println!("\n🧠 Learning embeddings from walks...");
    let embeddings = learn_embeddings(&walks, &papers);
    println!("   ✓ Learned {}-dimensional embeddings", embeddings.values().next().unwrap().len());

    // 4. Compute similarities
    println!("\n📐 Computing paper similarities:");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    for (i, &paper_id) in papers.iter().enumerate() {
        let paper = graph.get_node(paper_id).await?;
        let title = paper.properties.get("title").unwrap();

        println!("\n  Paper {}: {:?}", i, title);

        // Find most similar papers
        let mut similarities = Vec::new();

        for (j, &other_id) in papers.iter().enumerate() {
            if i != j {
                let sim = cosine_similarity(
                    embeddings.get(&paper_id).unwrap(),
                    embeddings.get(&other_id).unwrap(),
                );
                similarities.push((j, sim));
            }
        }

        similarities.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        for (j, sim) in similarities.iter().take(2) {
            let other = graph.get_node(papers[*j]).await?;
            let other_title = other.properties.get("title").unwrap();
            println!("    → {:?} (similarity: {:.3})", other_title, sim);
        }
    }

    println!();
    Ok(())
}

/// Create citation network
async fn create_citation_network(graph: &Graph) -> nopaldb::Result<Vec<uuid::Uuid>> {
    // Papers about different topics
    let ml_basics = graph.add_node(Node::new("Paper")
        .with_property("title", PropertyValue::String("Machine Learning Basics".into()))
        .with_property("topic", PropertyValue::String("ML".into()))).await?;

    let deep_learning = graph.add_node(Node::new("Paper")
        .with_property("title", PropertyValue::String("Deep Learning".into()))
        .with_property("topic", PropertyValue::String("ML".into()))).await?;

    let neural_nets = graph.add_node(Node::new("Paper")
        .with_property("title", PropertyValue::String("Neural Networks".into()))
        .with_property("topic", PropertyValue::String("ML".into()))).await?;

    let graph_theory = graph.add_node(Node::new("Paper")
        .with_property("title", PropertyValue::String("Graph Theory".into()))
        .with_property("topic", PropertyValue::String("Math".into()))).await?;

    let graph_algorithms = graph.add_node(Node::new("Paper")
        .with_property("title", PropertyValue::String("Graph Algorithms".into()))
        .with_property("topic", PropertyValue::String("CS".into()))).await?;

    let gnn = graph.add_node(Node::new("Paper")
        .with_property("title", PropertyValue::String("Graph Neural Networks".into()))
        .with_property("topic", PropertyValue::String("ML+Graph".into()))).await?;

    // Citations - ML cluster
    graph.add_edge(Edge::new(deep_learning, ml_basics, "CITES")).await?;
    graph.add_edge(Edge::new(neural_nets, ml_basics, "CITES")).await?;
    graph.add_edge(Edge::new(neural_nets, deep_learning, "CITES")).await?;

    // Citations - Graph cluster
    graph.add_edge(Edge::new(graph_algorithms, graph_theory, "CITES")).await?;

    // Citations - Bridge (GNN connects both)
    graph.add_edge(Edge::new(gnn, neural_nets, "CITES")).await?;
    graph.add_edge(Edge::new(gnn, deep_learning, "CITES")).await?;
    graph.add_edge(Edge::new(gnn, graph_algorithms, "CITES")).await?;
    graph.add_edge(Edge::new(gnn, graph_theory, "CITES")).await?;

    Ok(vec![ml_basics, deep_learning, neural_nets, graph_theory, graph_algorithms, gnn])
}

/// Generate random walks from each node
async fn generate_random_walks(
    graph: &Graph,
    nodes: &[uuid::Uuid],
    walks_per_node: usize,
    walk_length: usize,
) -> nopaldb::Result<Vec<Vec<uuid::Uuid>>> {
    let mut all_walks = Vec::new();
    let mut rng = rand::thread_rng();

    for &start_node in nodes {
        for _ in 0..walks_per_node {
            let mut walk = vec![start_node];
            let mut current = start_node;

            for _ in 0..walk_length {
                // Get outgoing edges
                let edges = graph.edges_of(current, Direction::Outgoing).await?;

                if edges.is_empty() {
                    // Dead end - try incoming (bidirectional walk)
                    let incoming = graph.edges_of(current, Direction::Incoming).await?;
                    if incoming.is_empty() {
                        break;
                    }
                    let idx = rng.gen_range(0..incoming.len());
                    current = incoming[idx].source;
                } else {
                    // Random outgoing neighbor
                    let idx = rng.gen_range(0..edges.len());
                    current = edges[idx].target;
                }

                walk.push(current);
            }

            all_walks.push(walk);
        }
    }

    Ok(all_walks)
}

/// Learn embeddings from walks (simplified co-occurrence)
/// Learn embeddings from walks (Skip-Gram style)
/// Learn embeddings with negative sampling
fn learn_embeddings(
    walks: &[Vec<uuid::Uuid>],
    nodes: &[uuid::Uuid],
) -> HashMap<uuid::Uuid, Vec<f64>> {
    let embedding_dim = 8; // Aumentar dimensión
    let mut embeddings = HashMap::new();
    let mut rng = rand::thread_rng();

    // Initialize embeddings (different per node)
    for (idx, &node_id) in nodes.iter().enumerate() {
        let embedding: Vec<f64> = (0..embedding_dim)
            .map(|i| {
                let angle = (idx as f64 * 2.5 + i as f64) * 0.5;
                angle.sin() * 0.3
            })
            .collect();
        embeddings.insert(node_id, embedding);
    }

    // Create training pairs (positive)
    let mut positive_pairs = Vec::new();
    let window_size = 2;

    for walk in walks {
        for (i, &center) in walk.iter().enumerate() {
            let start = i.saturating_sub(window_size);
            let end = (i + window_size + 1).min(walk.len());

            for j in start..end {
                if i != j {
                    positive_pairs.push((center, walk[j]));
                }
            }
        }
    }

    // Training with negative sampling
    let learning_rate = 0.1;
    let epochs = 100;
    let neg_samples = 3;

    for _ in 0..epochs {
        // Shuffle pairs
        use rand::seq::SliceRandom;
        let mut pairs = positive_pairs.clone();
        pairs.shuffle(&mut rng);

        for &(center, context) in &pairs {
            // Positive sample
            let center_emb = embeddings.get(&center).unwrap().clone();
            let context_emb = embeddings.get(&context).unwrap().clone();

            let dot: f64 = center_emb.iter()
                .zip(context_emb.iter())
                .map(|(a, b)| a * b)
                .sum();

            // Sigmoid
            let prob = 1.0 / (1.0 + (-dot).exp());
            let grad = prob - 1.0; // Target = 1

            // Update embeddings
            let mut new_center = center_emb.clone();
            let mut new_context = context_emb.clone();

            for i in 0..embedding_dim {
                new_center[i] -= learning_rate * grad * context_emb[i];
                new_context[i] -= learning_rate * grad * center_emb[i];
            }

            // Negative samples
            for _ in 0..neg_samples {
                let neg_idx = rng.gen_range(0..nodes.len());
                let neg_node = nodes[neg_idx];

                if neg_node == center || neg_node == context {
                    continue;
                }

                let neg_emb = embeddings.get(&neg_node).unwrap().clone();

                let neg_dot: f64 = new_center.iter()
                    .zip(neg_emb.iter())
                    .map(|(a, b)| a * b)
                    .sum();

                let neg_prob = 1.0 / (1.0 + (-neg_dot).exp());
                let neg_grad = neg_prob; // Target = 0

                for i in 0..embedding_dim {
                    new_center[i] -= learning_rate * neg_grad * neg_emb[i];
                }
            }

            embeddings.insert(center, new_center);
            embeddings.insert(context, new_context);
        }
    }

    // Normalize
    for embedding in embeddings.values_mut() {
        let norm: f64 = embedding.iter().map(|x| x * x).sum::<f64>().sqrt();
        if norm > 0.001 {
            for val in embedding.iter_mut() {
                *val /= norm;
            }
        }
    }

    embeddings
}

/// Compute cosine similarity between two vectors
fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    dot // Already normalized
}