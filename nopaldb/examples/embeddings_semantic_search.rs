//! # Semantic document search with NopalDB embeddings
//!
//! This example shows the basic embeddings workflow:
//!   1. Create nodes that represent documents
//!   2. Store a dense vector per node (simulating an external embedding model)
//!   3. Rank nodes by cosine similarity against a query vector
//!   4. Combine graph filters with vector ranking for precision
//!
//! Run with:
//!   cargo run --example embeddings_semantic_search --features embeddings
//!
//! In a real pipeline you would replace the hard-coded vectors with calls to a
//! model such as `sentence-transformers/all-MiniLM-L6-v2` (Python) or
//! `fastembed-rs` (Rust).

#[cfg(feature = "embeddings")]
mod example {
    use nopaldb::Graph;
    use nopaldb::embeddings::Embedding;
    use nopaldb::types::{Node, PropertyValue};
    use uuid::Uuid;

    // ---------------------------------------------------------------------------
    // Toy dataset: 5 technical articles, each with a 4-dimensional mock vector.
    // The dimensions loosely represent topic affinity:
    //   [systems/Rust, databases/graphs, ML/embeddings, data-formats/Arrow]
    // ---------------------------------------------------------------------------
    const ARTICLES: &[(&str, [f32; 4])] = &[
        (
            "Rust ownership model and memory safety",
            [0.92, 0.10, 0.08, 0.15],
        ),
        ("Introduction to graph databases", [0.10, 0.95, 0.15, 0.12]),
        ("Sentence embeddings with BERT", [0.12, 0.18, 0.96, 0.10]),
        (
            "Apache Arrow: zero-copy columnar memory",
            [0.70, 0.25, 0.22, 0.94],
        ),
        (
            "Fraud detection with graph algorithms",
            [0.20, 0.88, 0.30, 0.18],
        ),
    ];

    pub async fn run() -> nopaldb::Result<()> {
        let graph = Graph::in_memory().await?;

        // ── 1. Populate the graph ────────────────────────────────────────────
        println!("Loading articles into the graph...\n");

        let mut node_ids: Vec<(Uuid, &str)> = Vec::new();

        for (title, vector) in ARTICLES {
            let node = Node::new("Article")
                .with_property("title", PropertyValue::String(title.to_string()));

            graph.add_node(node.clone()).await?;
            graph
                .add_node_embedding(node.id, vector.to_vec(), "mock-model-v1")
                .await?;

            println!("  [+] {}", title);
            node_ids.push((node.id, title));
        }

        // ── 2. Connect related articles with edges ───────────────────────────
        // "Fraud detection" cites both "Graph databases" and "Rust"
        let fraud_id = node_ids[4].0;
        let graphs_id = node_ids[1].0;
        let rust_id = node_ids[0].0;

        graph
            .add_edge(nopaldb::types::Edge::new(fraud_id, graphs_id, "CITES"))
            .await?;
        graph
            .add_edge(nopaldb::types::Edge::new(fraud_id, rust_id, "CITES"))
            .await?;

        // ── 3. Plain vector search ───────────────────────────────────────────
        println!("\n--- Plain similarity search ---");
        println!("Query: \"Rust performance and low-level systems\"");
        println!("Query vector ≈ [0.88, 0.05, 0.05, 0.20]\n");

        let query_emb = Embedding::new(
            Uuid::new_v4(),
            vec![0.88, 0.05, 0.05, 0.20],
            "mock-model-v1",
        );

        let mut ranked = rank_all(&graph, &node_ids, &query_emb).await?;
        print_ranked(&ranked);

        // ── 4. Graph-filtered search ─────────────────────────────────────────
        // "Show only articles that are cited by the fraud-detection article,
        //  ranked by similarity to a query about data pipelines."
        println!("\n--- Graph-filtered similarity search ---");
        println!("Filter:  articles cited by \"Fraud detection with graph algorithms\"");
        println!("Query:   \"data engineering and performance\"\n");

        let cited_ids: Vec<(Uuid, &str)> = node_ids
            .iter()
            .filter(|(id, _)| [graphs_id, rust_id].contains(id))
            .copied()
            .collect();

        let pipeline_query = Embedding::new(
            Uuid::new_v4(),
            vec![0.65, 0.20, 0.15, 0.90], // closer to Arrow/data-formats
            "mock-model-v1",
        );

        ranked = rank_all(&graph, &cited_ids, &pipeline_query).await?;
        println!("Candidates after graph filter: {}", cited_ids.len());
        print_ranked(&ranked);

        // ── 5. Similarity between two specific nodes ─────────────────────────
        println!("\n--- Direct embedding comparison ---");
        let emb_rust = graph
            .get_node_embedding(node_ids[0].0, "mock-model-v1")
            .await?;
        let emb_fraud = graph
            .get_node_embedding(node_ids[4].0, "mock-model-v1")
            .await?;

        println!(
            "cosine_similarity(\"Rust\", \"Fraud detection\") = {:.4}",
            emb_rust.cosine_similarity(&emb_fraud)
        );
        println!(
            "euclidean_distance(\"Rust\", \"Fraud detection\") = {:.4}",
            emb_rust.euclidean_distance(&emb_fraud)
        );

        Ok(())
    }

    // ── Helpers ─────────────────────────────────────────────────────────────

    async fn rank_all(
        graph: &Graph,
        candidates: &[(Uuid, &str)],
        query: &Embedding,
    ) -> nopaldb::Result<Vec<(f32, String)>> {
        let mut results: Vec<(f32, String)> = Vec::new();

        for (node_id, title) in candidates {
            if let Ok(emb) = graph.get_node_embedding(*node_id, "mock-model-v1").await {
                let score = query.cosine_similarity(&emb);
                results.push((score, title.to_string()));
            }
        }


        results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        Ok(results)
    }

    fn print_ranked(results: &[(f32, String)]) {
        for (rank, (score, title)) in results.iter().enumerate() {
            println!("  #{rank}  {score:.4}  {title}");
        }
    }
}

// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    #[cfg(feature = "embeddings")]
    {
        if let Err(e) = example::run().await {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }

    #[cfg(not(feature = "embeddings"))]
    eprintln!(
        "This example requires the `embeddings` feature.\n\
         Run with: cargo run --example embeddings_semantic_search --features embeddings"
    );
}
