// tests/nql_path_similarity_e8_test.rs
//
// Integration tests for E-8 PathSimilarity:
// path_embedding_similarity(ref_name, node_model, edge_model) — cosine similarity
// del path E-7 actual contra una PathReferenceEmbedding persistida.

#[cfg(feature = "embeddings")]
mod tests {
    use nopaldb::{Edge, Graph, Node, PropertyValue, Result};

    fn str_val(s: &str) -> PropertyValue {
        PropertyValue::String(s.to_string())
    }

    fn get_float(result: &nopaldb::query::nql::QueryResult, row: usize, col: &str) -> f64 {
        match result.rows()[row].get(col) {
            Some(PropertyValue::Float(f)) => *f,
            other => panic!("expected Float for '{}', got {:?}", col, other),
        }
    }

    /// Crea un grafo con dos nodos A y B conectados por una arista TX.
    /// Agrega embeddings de nodo y arista, y una referencia de path.
    /// Retorna (graph, node_a_id, node_b_id, edge_ab_id).
    async fn setup_simple_path(ref_name: &str) -> Result<Graph> {
        let graph = Graph::in_memory().await?;

        let mut tx = graph.begin_transaction().await?;
        let a = tx
            .add_node(Node::new("Account").with_property("name", str_val("A")))
            .await?;
        let b = tx
            .add_node(Node::new("Account").with_property("name", str_val("B")))
            .await?;
        let rel = tx.add_edge(Edge::new(a, b, "TX"))?;
        tx.commit().await?;

        // node embeddings: A=[1,0], B=[0,1] → mean=[0.5, 0.5]
        // edge embedding: TX=[1,1]
        // path E-7 vector: [0.5, 0.5, 1.0, 1.0]
        graph
            .add_node_embedding(a, vec![1.0, 0.0], "node-m")
            .await?;
        graph
            .add_node_embedding(b, vec![0.0, 1.0], "node-m")
            .await?;
        graph
            .add_edge_embedding(rel, vec![1.0, 1.0], "edge-m")
            .await?;

        // referencia = mismo vector que el path → cosine debe ser ~1.0
        graph
            .add_path_reference_embedding(
                ref_name.to_string(),
                "node-m".to_string(),
                "edge-m".to_string(),
                vec![0.5, 0.5, 1.0, 1.0],
            )
            .await?;

        Ok(graph)
    }

    // ────────────────────────────────────────────────────────────
    // Test 1: self-similarity ≈ 1.0
    // ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_self_similarity_is_one() -> Result<()> {
        let graph = setup_simple_path("fraud_ring_v1").await?;

        let result = graph
            .execute_nql(
                r#"
                find path_embedding_similarity("fraud_ring_v1", "node-m", "edge-m") as score
                from (a:Account {name: "A"})-[:TX]->(b:Account {name: "B"})
            "#,
            )
            .await?;

        assert_eq!(result.len(), 1);
        let score = get_float(&result, 0, "score");
        assert!(
            (score - 1.0).abs() < 1e-5,
            "expected self-similarity ~1.0, got {}",
            score
        );
        Ok(())
    }

    // ────────────────────────────────────────────────────────────
    // Test 2: projected score in FIND returns Float
    // ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_similarity_in_find_returns_float() -> Result<()> {
        let graph = setup_simple_path("ref1").await?;

        let result = graph
            .execute_nql(
                r#"
                find b.name,
                     path_embedding_similarity("ref1", "node-m", "edge-m") as score
                from (a:Account {name: "A"})-[:TX]->(b:Account)
            "#,
            )
            .await?;

        assert_eq!(result.len(), 1);
        let score = get_float(&result, 0, "score");
        assert!(
            score > 0.0 && score <= 1.0 + 1e-5,
            "score out of range: {}",
            score
        );
        Ok(())
    }

    // ────────────────────────────────────────────────────────────
    // Test 3: WHERE filter keeps/rejects by threshold
    // ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_where_filter_by_similarity() -> Result<()> {
        let graph = setup_simple_path("ref_where").await?;

        // threshold 0.99 — self-similarity should pass
        let result_pass = graph
            .execute_nql(
                r#"
                find b.name
                from (a:Account {name: "A"})-[:TX]->(b:Account)
                where path_embedding_similarity("ref_where", "node-m", "edge-m") > 0.99
            "#,
            )
            .await?;
        assert_eq!(
            result_pass.len(),
            1,
            "expected 1 row to pass threshold 0.99"
        );

        // threshold 1.01 — nothing should pass
        let result_reject = graph
            .execute_nql(
                r#"
                find b.name
                from (a:Account {name: "A"})-[:TX]->(b:Account)
                where path_embedding_similarity("ref_where", "node-m", "edge-m") > 1.01
            "#,
            )
            .await?;
        assert_eq!(
            result_reject.len(),
            0,
            "expected 0 rows above threshold 1.01"
        );
        Ok(())
    }

    // ────────────────────────────────────────────────────────────
    // Test 4: alias + ORDER BY works
    // ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_alias_and_order_by() -> Result<()> {
        // Dos paths: AB (self-sim=1.0) y una referencia ortogonal para el mismo nodo
        let graph = Graph::in_memory().await?;

        let mut tx = graph.begin_transaction().await?;
        let a = tx
            .add_node(Node::new("Account").with_property("name", str_val("Root")))
            .await?;
        let b = tx
            .add_node(Node::new("Account").with_property("name", str_val("B")))
            .await?;
        let c = tx
            .add_node(Node::new("Account").with_property("name", str_val("C")))
            .await?;
        let rel_ab = tx.add_edge(Edge::new(a, b, "TX"))?;
        let rel_ac = tx.add_edge(Edge::new(a, c, "TX"))?;
        tx.commit().await?;

        // A=[1,0], B=[1,0], C=[0,1]; edge-AB=[1,0], edge-AC=[0,1]
        graph.add_node_embedding(a, vec![1.0, 0.0], "nm").await?;
        graph.add_node_embedding(b, vec![1.0, 0.0], "nm").await?;
        graph.add_node_embedding(c, vec![0.0, 1.0], "nm").await?;
        graph
            .add_edge_embedding(rel_ab, vec![1.0, 0.0], "em")
            .await?;
        graph
            .add_edge_embedding(rel_ac, vec![0.0, 1.0], "em")
            .await?;

        // Referencia = path AB: node_mean=[1,0], edge_mean=[1,0] → [1,0,1,0]
        // path AB: score ≈ 1.0; path AC: node_mean=[0.5,0.5], edge_mean=[0,1] → [0.5,0.5,0,1]
        graph
            .add_path_reference_embedding(
                "ref_order".to_string(),
                "nm".to_string(),
                "em".to_string(),
                vec![1.0, 0.0, 1.0, 0.0],
            )
            .await?;

        let result = graph
            .execute_nql(
                r#"
                find n.name,
                     path_embedding_similarity("ref_order", "nm", "em") as score
                from (a:Account {name: "Root"})-[:TX]->(n:Account)
                order by score desc
            "#,
            )
            .await?;

        assert_eq!(result.len(), 2, "expected 2 paths");
        let score0 = get_float(&result, 0, "score");
        let score1 = get_float(&result, 1, "score");
        assert!(
            score0 >= score1,
            "expected descending order: {} >= {}",
            score0,
            score1
        );
        Ok(())
    }

    // ────────────────────────────────────────────────────────────
    // Test 5: dimension mismatch → QueryExecutionError
    // ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_dimension_mismatch_fails() -> Result<()> {
        let graph = Graph::in_memory().await?;

        let mut tx = graph.begin_transaction().await?;
        let a = tx
            .add_node(Node::new("Account").with_property("name", str_val("A")))
            .await?;
        let b = tx
            .add_node(Node::new("Account").with_property("name", str_val("B")))
            .await?;
        let rel = tx.add_edge(Edge::new(a, b, "TX"))?;
        tx.commit().await?;

        graph.add_node_embedding(a, vec![1.0, 0.0], "nm").await?;
        graph.add_node_embedding(b, vec![0.0, 1.0], "nm").await?;
        graph.add_edge_embedding(rel, vec![1.0], "em").await?;

        // path vector dim = 2 (node_mean) + 1 (edge_mean) = 3
        // referencia tiene dim = 10 → mismatch
        graph
            .add_path_reference_embedding(
                "wrong_dim".to_string(),
                "nm".to_string(),
                "em".to_string(),
                vec![1.0; 10],
            )
            .await?;

        let err = graph
            .execute_nql(
                r#"
                find path_embedding_similarity("wrong_dim", "nm", "em") as score
                from (a:Account {name: "A"})-[:TX]->(b:Account {name: "B"})
            "#,
            )
            .await;

        assert!(err.is_err(), "expected error on dimension mismatch");
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.contains("dimension mismatch"),
            "expected 'dimension mismatch' in error, got: {}",
            msg
        );
        Ok(())
    }

    // ────────────────────────────────────────────────────────────
    // Test 6: missing reference → QueryExecutionError with name
    // ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_missing_reference_fails() -> Result<()> {
        let graph = Graph::in_memory().await?;

        let mut tx = graph.begin_transaction().await?;
        let a = tx
            .add_node(Node::new("Account").with_property("name", str_val("A")))
            .await?;
        let b = tx
            .add_node(Node::new("Account").with_property("name", str_val("B")))
            .await?;
        let rel = tx.add_edge(Edge::new(a, b, "TX"))?;
        tx.commit().await?;

        graph.add_node_embedding(a, vec![1.0], "nm").await?;
        graph.add_node_embedding(b, vec![0.0], "nm").await?;
        graph.add_edge_embedding(rel, vec![1.0], "em").await?;

        let err = graph
            .execute_nql(
                r#"
                find path_embedding_similarity("nonexistent_ref", "nm", "em") as score
                from (a:Account {name: "A"})-[:TX]->(b:Account {name: "B"})
            "#,
            )
            .await;

        assert!(err.is_err(), "expected error for missing reference");
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.contains("nonexistent_ref"),
            "expected ref name in error message, got: {}",
            msg
        );
        Ok(())
    }

    // ────────────────────────────────────────────────────────────
    // Test 7: zero-norm reference rejected at persist time
    // ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_zero_norm_reference_rejected_on_persist() -> Result<()> {
        let graph = Graph::in_memory().await?;

        let err = graph
            .add_path_reference_embedding(
                "zero_ref".to_string(),
                "nm".to_string(),
                "em".to_string(),
                vec![0.0, 0.0, 0.0],
            )
            .await
            .expect_err("zero-norm reference must be rejected");

        assert!(
            err.to_string().contains("zero-norm vector"),
            "expected zero-norm validation error, got: {}",
            err
        );
        Ok(())
    }

    // ────────────────────────────────────────────────────────────
    // Test 8: zero-norm path rejected during similarity evaluation
    // ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_zero_norm_path_rejected_during_similarity() -> Result<()> {
        let graph = Graph::in_memory().await?;

        let mut tx = graph.begin_transaction().await?;
        let a = tx
            .add_node(Node::new("Account").with_property("name", str_val("A")))
            .await?;
        let b = tx
            .add_node(Node::new("Account").with_property("name", str_val("B")))
            .await?;
        let rel = tx.add_edge(Edge::new(a, b, "TX"))?;
        tx.commit().await?;

        graph.add_node_embedding(a, vec![0.0, 0.0], "nm").await?;
        graph.add_node_embedding(b, vec![0.0, 0.0], "nm").await?;
        graph.add_edge_embedding(rel, vec![0.0, 0.0], "em").await?;

        graph
            .add_path_reference_embedding(
                "nonzero_ref".to_string(),
                "nm".to_string(),
                "em".to_string(),
                vec![1.0, 0.0, 1.0, 0.0],
            )
            .await?;

        let err = graph
            .execute_nql(
                r#"
                find path_embedding_similarity("nonzero_ref", "nm", "em") as score
                from (a:Account {name: "A"})-[:TX]->(b:Account {name: "B"})
            "#,
            )
            .await
            .expect_err("zero-norm path must fail similarity evaluation");

        assert!(
            err.to_string().contains("zero-norm vectors"),
            "expected zero-norm similarity error, got: {}",
            err
        );
        Ok(())
    }

    // ────────────────────────────────────────────────────────────
    // Test 9: ORDER BY direct (not alias) → SemanticError
    // ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_order_by_direct_rejected() -> Result<()> {
        let graph = Graph::in_memory().await?;

        let err = graph
            .execute_nql(
                r#"
                find a.name
                from (a:Account)-[:TX]->(b:Account)
                order by path_embedding_similarity("ref", "nm", "em") desc
            "#,
            )
            .await;

        assert!(err.is_err(), "expected SemanticError for direct ORDER BY");
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.contains("ORDER BY") || msg.contains("alias"),
            "expected ORDER BY rejection message, got: {}",
            msg
        );
        Ok(())
    }

    // ────────────────────────────────────────────────────────────
    // Test 10: old 2-argument form rejected
    // ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_old_two_arg_form_rejected() -> Result<()> {
        let graph = Graph::in_memory().await?;

        let err = graph
            .execute_nql(
                r#"
                find path_embedding_similarity("ref", "model") as score
                from (a:Account)-[:TX]->(b:Account)
            "#,
            )
            .await;

        assert!(err.is_err(), "expected error for 2-arg form");
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.contains("3 arguments") || msg.contains("E-8"),
            "expected arity/migration error, got: {}",
            msg
        );
        Ok(())
    }
}
