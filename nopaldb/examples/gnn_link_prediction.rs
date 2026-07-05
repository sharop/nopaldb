// examples/gnn_link_prediction.rs
//
// Link Prediction usando Graph Embeddings
// Combina Random Walks + Embeddings + Similarity Scoring

use nopaldb::{Graph, Node, Edge, PropertyValue, Direction};
use std::collections::HashMap;
use rand::Rng;

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    println!("╔════════════════════════════════════════════════════════════╗");
    println!("║       LINK PREDICTION - GRAPH EMBEDDINGS APPROACH         ║");
    println!("║            (Citation Network - Predict Next Citation)     ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");

    let graph = Graph::in_memory().await?;

    // 1. Create citation network
    println!("📊 Creating citation network...");
    let papers = create_citation_network(&graph).await?;
    println!("   ✓ {} papers created\n", papers.len());

    // 2. Learn embeddings via random walks
    println!("🚶 Generating random walks for embeddings...");
    let all_walks = generate_all_walks(&graph, &papers).await?;
    println!("   ✓ Generated {} walks total", all_walks.len());

    println!("\n🧠 Learning embeddings...");
    let embeddings = learn_embeddings_from_walks(&all_walks, &papers);
    println!("   ✓ Learned {}-dimensional embeddings", embeddings.values().next().unwrap().len());

    // 3. Predict links for target paper
    let target_paper = papers[0]; // Paper A

    println!("\n🔮 Predicting next citations for Paper A:");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    let target = graph.get_node(target_paper).await?;
    println!("  Target: {:?}", target.properties.get("title").unwrap());

    // Get existing citations
    let existing = graph.edges_of(target_paper, Direction::Outgoing).await?;
    let existing_targets: Vec<_> = existing.iter().map(|e| e.target).collect();

    println!("  Current citations: {}", existing_targets.len());

    // 4. Score all candidates
    let mut predictions = Vec::new();

    for &candidate_id in &papers {
        if candidate_id == target_paper {
            continue;
        }

        // Skip if already cited
        if existing_targets.contains(&candidate_id) {
            continue;
        }

        // Compute link prediction score
        let score = link_prediction_score(
            &graph,
            &embeddings,
            target_paper,
            candidate_id,
        ).await?;

        predictions.push((candidate_id, score));
    }

    // Sort by score
    predictions.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    println!("\n  Top 3 predictions:");
    for (i, (paper_id, score)) in predictions.iter().take(3).enumerate() {
        let paper = graph.get_node(*paper_id).await?;
        let title = paper.properties.get("title").unwrap();
        println!("    {}. {:?} (score: {:.3})", i + 1, title, score);
    }

    println!();
    Ok(())
}

/// Create citation network
async fn create_citation_network(graph: &Graph) -> nopaldb::Result<Vec<uuid::Uuid>> {
    // Paper A - starting point
    let paper_a = graph.add_node(Node::new("Paper")
        .with_property("title", PropertyValue::String("Graph Databases Survey".into()))
        .with_property("topic", PropertyValue::String("Databases".into()))).await?;

    // Papers B, C - already cited by A
    let paper_b = graph.add_node(Node::new("Paper")
        .with_property("title", PropertyValue::String("NoSQL Systems".into()))
        .with_property("topic", PropertyValue::String("Databases".into()))).await?;

    let paper_c = graph.add_node(Node::new("Paper")
        .with_property("title", PropertyValue::String("Graph Algorithms".into()))
        .with_property("topic", PropertyValue::String("Algorithms".into()))).await?;

    // Papers D, E - candidates for prediction
    let paper_d = graph.add_node(Node::new("Paper")
        .with_property("title", PropertyValue::String("Network Analysis".into()))
        .with_property("topic", PropertyValue::String("Networks".into()))).await?;

    let paper_e = graph.add_node(Node::new("Paper")
        .with_property("title", PropertyValue::String("Machine Learning on Graphs".into()))
        .with_property("topic", PropertyValue::String("ML".into()))).await?;

    // Existing citations from A
    graph.add_edge(Edge::new(paper_a, paper_b, "CITES")).await?;
    graph.add_edge(Edge::new(paper_a, paper_c, "CITES")).await?;

    // B and C cite D (common neighbor pattern)
    graph.add_edge(Edge::new(paper_b, paper_d, "CITES")).await?;
    graph.add_edge(Edge::new(paper_c, paper_d, "CITES")).await?;

    // C cites E (less common)
    graph.add_edge(Edge::new(paper_c, paper_e, "CITES")).await?;

    Ok(vec![paper_a, paper_b, paper_c, paper_d, paper_e])
}

/// Generate random walks from all nodes
async fn generate_all_walks(
    graph: &Graph,
    nodes: &[uuid::Uuid],
) -> nopaldb::Result<Vec<Vec<uuid::Uuid>>> {
    let mut all_walks = Vec::new();

    for &start in nodes {
        let walks = random_walks(graph, start, 5, 10).await?;
        all_walks.extend(walks);
    }

    Ok(all_walks)
}

/// Random walks from a starting node (Node2Vec-style)
///
/// TEORÍA:
/// Random walk = secuencia de nodos visitados al seguir edges aleatoriamente
///
/// Ejemplo:
///   Start: A
///   Neighbors of A: [B, C]
///   Pick random: B
///   Neighbors of B: [C, D]
///   Pick random: D
///   Walk: [A, B, D]
///
/// Propósito:
/// - Capturar estructura local del grafo
/// - Nodos cercanos aparecen juntos en walks
/// - Base para aprender embeddings (como Word2Vec para palabras)
async fn random_walks(
    graph: &Graph,
    start: uuid::Uuid,
    num_walks: usize,
    walk_length: usize,
) -> nopaldb::Result<Vec<Vec<uuid::Uuid>>> {
    let mut walks = Vec::new();
    let mut rng = rand::thread_rng();

    for _ in 0..num_walks {
        let mut walk = vec![start];
        let mut current = start;

        for _ in 0..walk_length {
            // Get outgoing edges
            let edges = graph.edges_of(current, Direction::Outgoing).await?;

            if edges.is_empty() {
                // Dead end - try going backwards (bidirectional)
                let incoming = graph.edges_of(current, Direction::Incoming).await?;
                if incoming.is_empty() {
                    break; // Truly stuck
                }

                // Pick random incoming edge
                let idx = rng.gen_range(0..incoming.len());
                current = incoming[idx].source;
            } else {
                // Pick random outgoing neighbor
                let idx = rng.gen_range(0..edges.len());
                current = edges[idx].target;
            }

            walk.push(current);
        }

        walks.push(walk);
    }

    Ok(walks)
}

/// Learn embeddings from walks (simplified)
fn learn_embeddings_from_walks(
    walks: &[Vec<uuid::Uuid>],
    nodes: &[uuid::Uuid],
) -> HashMap<uuid::Uuid, Vec<f64>> {
    let embedding_dim = 4;
    let mut embeddings = HashMap::new();
    let mut rng = rand::thread_rng();

    // Initialize randomly
    for (idx, &node_id) in nodes.iter().enumerate() {
        let embedding: Vec<f64> = (0..embedding_dim)
            .map(|i| ((idx as f64 * 2.5 + i as f64) * 0.7).sin() * 0.4)
            .collect();
        embeddings.insert(node_id, embedding);
    }

    // Train with negative sampling
    let learning_rate = 0.1;
    let window_size = 2;
    let neg_samples = 2;

    for _ in 0..50 {
        for walk in walks {
            for (i, &center) in walk.iter().enumerate() {
                let start = i.saturating_sub(window_size);
                let end = (i + window_size + 1).min(walk.len());

                for j in start..end {
                    if i == j {
                        continue;
                    }

                    let context = walk[j];

                    // Positive pair
                    let center_emb = embeddings.get(&center).unwrap().clone();
                    let context_emb = embeddings.get(&context).unwrap().clone();

                    let dot: f64 = center_emb.iter()
                        .zip(context_emb.iter())
                        .map(|(a, b)| a * b)
                        .sum();

                    let prob = 1.0 / (1.0 + (-dot).exp());
                    let grad = prob - 1.0;

                    let mut new_center = center_emb;
                    for k in 0..embedding_dim {
                        new_center[k] -= learning_rate * grad * context_emb[k];
                    }

                    // Negative samples
                    for _ in 0..neg_samples {
                        let neg_idx = rng.gen_range(0..nodes.len());
                        let neg_node = nodes[neg_idx];

                        let neg_emb = embeddings.get(&neg_node).unwrap();
                        let neg_dot: f64 = new_center.iter()
                            .zip(neg_emb.iter())
                            .map(|(a, b)| a * b)
                            .sum();

                        let neg_prob = 1.0 / (1.0 + (-neg_dot).exp());
                        let neg_grad = neg_prob;

                        for k in 0..embedding_dim {
                            new_center[k] -= learning_rate * neg_grad * neg_emb[k];
                        }
                    }

                    embeddings.insert(center, new_center);
                }
            }
        }
    }

    // Normalize
    for emb in embeddings.values_mut() {
        let norm: f64 = emb.iter().map(|x| x * x).sum::<f64>().sqrt();
        if norm > 0.001 {
            for val in emb.iter_mut() {
                *val /= norm;
            }
        }
    }

    embeddings
}

/// Link prediction score combining multiple signals
///
/// TEORÍA:
/// Score = weighted combination of:
/// 1. Embedding similarity (learned from structure)
/// 2. Common neighbors (Adamic-Adar)
/// 3. Preferential attachment (degree product)
async fn link_prediction_score(
    graph: &Graph,
    embeddings: &HashMap<uuid::Uuid, Vec<f64>>,
    source: uuid::Uuid,
    target: uuid::Uuid,
) -> nopaldb::Result<f64> {
    let mut score = 0.0;

    // 1. Embedding similarity (40% weight)
    let emb_sim = cosine_similarity(
        embeddings.get(&source).unwrap(),
        embeddings.get(&target).unwrap(),
    );
    score += emb_sim * 0.4;

    // 2. Adamic-Adar (common neighbors) (40% weight)
    let aa_score = adamic_adar(graph, source, target).await?;
    score += aa_score * 0.4;

    // 3. Preferential attachment (20% weight)
    let pa_score = preferential_attachment(graph, source, target).await?;
    score += pa_score * 0.2;

    Ok(score)
}

/// Cosine similarity between embeddings
fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// Adamic-Adar index
async fn adamic_adar(
    graph: &Graph,
    source: uuid::Uuid,
    target: uuid::Uuid,
) -> nopaldb::Result<f64> {
    let source_neighbors = get_neighbors(graph, source).await?;
    let target_neighbors = get_neighbors(graph, target).await?;

    let mut score = 0.0;

    for common in source_neighbors.intersection(&target_neighbors) {
        let degree = graph.degree(*common, Direction::Both).await? as f64;
        if degree > 1.0 {
            score += 1.0 / degree.log2();
        } else {
            score += 1.0;
        }
    }

    Ok(score)
}

/// Preferential attachment
async fn preferential_attachment(
    graph: &Graph,
    source: uuid::Uuid,
    target: uuid::Uuid,
) -> nopaldb::Result<f64> {
    let source_degree = graph.degree(source, Direction::Both).await? as f64;
    let target_degree = graph.degree(target, Direction::Both).await? as f64;

    // Normalize by max possible
    Ok((source_degree * target_degree) / 100.0)
}

/// Get all neighbors of a node
async fn get_neighbors(
    graph: &Graph,
    node_id: uuid::Uuid,
) -> nopaldb::Result<std::collections::HashSet<uuid::Uuid>> {
    let edges = graph.edges_of(node_id, Direction::Both).await?;

    let neighbors = edges.iter()
        .map(|e| if e.source == node_id { e.target } else { e.source })
        .collect();

    Ok(neighbors)
}