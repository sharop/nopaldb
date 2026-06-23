//! # Path Embeddings for Fraud Detection (E-7, E-8, E-9, E-10)
//!
//! Demonstrates the full path embedding pipeline on a small synthetic fraud graph:
//!
//!   1. E-7: `path_has_embeddings` / `path_embedding` — vector del path materializado
//!   2. E-8: `path_embedding_similarity` — comparar un path contra una referencia persistida
//!   3. E-9: `path_knn_references` — top-k referencias más similares al path actual
//!   4. E-10: `path_anomaly_score` — score de anomalía vs. centroide de referencias
//!
//! Run with:
//!   cargo run --example path_embeddings_fraud --features embeddings
//!
//! The example uses 4-dimensional mock vectors:
//!   [transfer_volume, node_age, cross_border, night_activity]
//!
//! Two model namespaces are used:
//!   - "fraud-v1" / "tx-v1"   → E-8 / E-9: refs incluyen normal + fraud_ring (3 total)
//!   - "baseline-v1" / "tx-v1" → E-10: solo refs normales (2 total) para calcular centroide

#[cfg(feature = "embeddings")]
mod example {
    use nopaldb::Graph;
    use nopaldb::types::{Edge, Node, PropertyValue};

    fn str_val(s: &str) -> PropertyValue {
        PropertyValue::String(s.to_string())
    }

    pub async fn run() -> nopaldb::Result<()> {
        let graph = Graph::in_memory().await?;

        // ── 1. Build a small fraud graph ─────────────────────────────────────
        //
        // Topology:
        //   Root → Alice, Bob, Carol  (TX edges)
        //
        // Root = known entry node
        // Alice = typical transfer (low risk)
        // Bob   = typical transfer (low risk)
        // Carol = suspicious (high cross-border + night_activity)

        let mut tx = graph.begin_transaction().await?;

        let root = tx
            .add_node(Node::new("Account").with_property("name", str_val("Root")))
            .await?;
        let alice = tx
            .add_node(Node::new("Account").with_property("name", str_val("Alice")))
            .await?;
        let bob = tx
            .add_node(Node::new("Account").with_property("name", str_val("Bob")))
            .await?;
        let carol = tx
            .add_node(Node::new("Account").with_property("name", str_val("Carol")))
            .await?;

        let edge_ra = tx.add_edge(Edge::new(root, alice, "TX"))?;
        let edge_rb = tx.add_edge(Edge::new(root, bob, "TX"))?;
        let edge_rc = tx.add_edge(Edge::new(root, carol, "TX"))?;

        tx.commit().await?;

        // ── 2. Embeddings de nodos — modelo "fraud-v1" ───────────────────────
        // Dimensions: [transfer_volume, node_age, cross_border, night_activity]
        //
        // Root y Alice/Bob: doméstico, diurno
        // Carol: alto cross-border, alta actividad nocturna

        graph
            .add_node_embedding(root, vec![0.5, 0.8, 0.1, 0.1], "fraud-v1")
            .await?;
        graph
            .add_node_embedding(alice, vec![0.4, 0.9, 0.1, 0.1], "fraud-v1")
            .await?;
        graph
            .add_node_embedding(bob, vec![0.5, 0.7, 0.2, 0.1], "fraud-v1")
            .await?;
        graph
            .add_node_embedding(carol, vec![0.9, 0.3, 0.9, 0.8], "fraud-v1")
            .await?;

        // Modelo "baseline-v1" — mismos vectores, namespace separado para E-10
        graph
            .add_node_embedding(root, vec![0.5, 0.8, 0.1, 0.1], "baseline-v1")
            .await?;
        graph
            .add_node_embedding(alice, vec![0.4, 0.9, 0.1, 0.1], "baseline-v1")
            .await?;
        graph
            .add_node_embedding(bob, vec![0.5, 0.7, 0.2, 0.1], "baseline-v1")
            .await?;
        graph
            .add_node_embedding(carol, vec![0.9, 0.3, 0.9, 0.8], "baseline-v1")
            .await?;

        // ── 3. Embeddings de aristas — modelo "tx-v1" ────────────────────────
        // Transferencias normales: bajo volumen, doméstico, diurno
        // Transferencia sospechosa a Carol: alto volumen, cross-border, nocturno

        graph
            .add_edge_embedding(edge_ra, vec![0.2, 0.0, 0.1, 0.1], "tx-v1")
            .await?;
        graph
            .add_edge_embedding(edge_rb, vec![0.3, 0.0, 0.1, 0.1], "tx-v1")
            .await?;
        graph
            .add_edge_embedding(edge_rc, vec![0.9, 0.0, 0.9, 0.9], "tx-v1")
            .await?;

        // ── 4. PathReferenceEmbeddings ────────────────────────────────────────
        //
        // Namespace "fraud-v1"/"tx-v1" — para E-8 (similitud directa) y E-9 (kNN):
        //   Incluye referencias normales + patrón de fraude conocido.
        //
        //   normal_domestic_alice: path Root→Alice promediado
        //   normal_domestic_bob:   path Root→Bob  promediado
        //   fraud_ring_v1:         patrón de fraude conocido (alta sospecha)
        //
        // Namespace "baseline-v1"/"tx-v1" — SOLO para E-10 (anomaly score):
        //   Solo referencias normales → define el centroide del comportamiento esperado.
        //   Carol debería estar lejos de este centroide → alta anomalía.

        // -- E-8/E-9 references (fraud-v1 / tx-v1) ---------------------------
        graph
            .add_path_reference_embedding(
                "normal_domestic_alice".to_string(),
                "fraud-v1".to_string(),
                "tx-v1".to_string(),
                vec![0.475, 0.85, 0.10, 0.10, 0.20, 0.0, 0.10, 0.10],
            )
            .await?;

        graph
            .add_path_reference_embedding(
                "normal_domestic_bob".to_string(),
                "fraud-v1".to_string(),
                "tx-v1".to_string(),
                vec![0.50, 0.75, 0.15, 0.10, 0.30, 0.0, 0.10, 0.10],
            )
            .await?;

        graph
            .add_path_reference_embedding(
                "fraud_ring_v1".to_string(),
                "fraud-v1".to_string(),
                "tx-v1".to_string(),
                vec![0.90, 0.30, 0.90, 0.85, 0.90, 0.0, 0.90, 0.90],
            )
            .await?;

        // -- E-10 baseline (baseline-v1 / tx-v1) — solo patrones normales -----
        graph
            .add_path_reference_embedding(
                "baseline_alice".to_string(),
                "baseline-v1".to_string(),
                "tx-v1".to_string(),
                vec![0.475, 0.85, 0.10, 0.10, 0.20, 0.0, 0.10, 0.10],
            )
            .await?;

        graph
            .add_path_reference_embedding(
                "baseline_bob".to_string(),
                "baseline-v1".to_string(),
                "tx-v1".to_string(),
                vec![0.50, 0.75, 0.15, 0.10, 0.30, 0.0, 0.10, 0.10],
            )
            .await?;

        // ── 5. E-8: path_embedding_similarity ────────────────────────────────
        println!("=== E-8: path_embedding_similarity ===");
        let result = graph.execute_nql(r#"
            find n.name,
                 path_embedding_similarity("normal_domestic_alice", "fraud-v1", "tx-v1") as sim_normal,
                 path_embedding_similarity("fraud_ring_v1", "fraud-v1", "tx-v1") as sim_fraud
            from (r:Account {name: "Root"})-[:TX]->(n:Account)
            order by sim_fraud desc
        "#).await?;

        for row in result.rows() {
            let name = match row.get("n.name") {
                Some(PropertyValue::String(s)) => s.clone(),
                _ => "?".to_string(),
            };
            let sim_n = match row.get("sim_normal") {
                Some(PropertyValue::Float(f)) => *f,
                _ => 0.0,
            };
            let sim_f = match row.get("sim_fraud") {
                Some(PropertyValue::Float(f)) => *f,
                _ => 0.0,
            };
            println!("  {name:<8} sim_normal={sim_n:.3}  sim_fraud={sim_f:.3}");
        }

        // ── 6. E-9: path_knn_references ──────────────────────────────────────
        println!("\n=== E-9: path_knn_references (top-2, min_score=0.0) ===");
        let result = graph
            .execute_nql(
                r#"
            find n.name,
                 path_knn_references("fraud-v1", "tx-v1", 2, 0.0) as top_refs
            from (r:Account {name: "Root"})-[:TX]->(n:Account)
        "#,
            )
            .await?;

        for row in result.rows() {
            let name = match row.get("n.name") {
                Some(PropertyValue::String(s)) => s.clone(),
                _ => "?".to_string(),
            };
            let refs = match row.get("top_refs") {
                Some(PropertyValue::List(items)) => items
                    .iter()
                    .map(|item| match item {
                        PropertyValue::Object(fields) => {
                            let ref_name = fields
                                .iter()
                                .find(|(k, _)| k == "name")
                                .map(|(_, v)| match v {
                                    PropertyValue::String(s) => s.as_str(),
                                    _ => "?",
                                })
                                .unwrap_or("?");
                            let score = fields
                                .iter()
                                .find(|(k, _)| k == "score")
                                .map(|(_, v)| match v {
                                    PropertyValue::Float(f) => *f,
                                    _ => 0.0,
                                })
                                .unwrap_or(0.0);
                            format!("{}({:.3})", ref_name, score)
                        }
                        _ => "?".to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                _ => "[]".to_string(),
            };
            println!("  {name:<8} top_refs=[{refs}]");
        }

        // ── 7. E-10: path_anomaly_score ──────────────────────────────────────
        // Usa "baseline-v1"/"tx-v1": solo referencias normales (alice + bob).
        // El centroide representa el comportamiento doméstico esperado.
        // Carol, con alto cross-border y actividad nocturna, estará lejos del centroide.
        println!("\n=== E-10: path_anomaly_score (baseline normal: alice + bob) ===");
        let result = graph
            .execute_nql(
                r#"
            find n.name,
                 path_anomaly_score("baseline-v1", "tx-v1") as anomaly
            from (r:Account {name: "Root"})-[:TX]->(n:Account)
            order by anomaly desc
        "#,
            )
            .await?;

        for row in result.rows() {
            let name = match row.get("n.name") {
                Some(PropertyValue::String(s)) => s.clone(),
                _ => "?".to_string(),
            };
            let anom = match row.get("anomaly") {
                Some(PropertyValue::Float(f)) => *f,
                _ => 0.0,
            };
            let label = if anom > 0.5 { " ← HIGH ANOMALY" } else { "" };
            println!("  {name:<8} anomaly={anom:.3}{label}");
        }

        // ── 8. E-10: WHERE filter for anomalous paths ─────────────────────────
        println!("\n=== E-10: WHERE path_anomaly_score > 0.3 ===");
        let result = graph
            .execute_nql(
                r#"
            find n.name
            from (r:Account {name: "Root"})-[:TX]->(n:Account)
            where path_anomaly_score("baseline-v1", "tx-v1") > 0.3
        "#,
            )
            .await?;

        if result.is_empty() {
            println!("  (no anomalous paths found)");
        } else {
            for row in result.rows() {
                let name = match row.get("n.name") {
                    Some(PropertyValue::String(s)) => s.clone(),
                    _ => "?".to_string(),
                };
                println!("  ALERT: anomalous path to {name}");
            }
        }

        println!("\nDone. Carol should appear as the most anomalous path.");
        Ok(())
    }
}

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
    {
        println!(
            "This example requires the `embeddings` feature.\n\
             Run with: cargo run --example path_embeddings_fraud --features embeddings"
        );
    }
}
