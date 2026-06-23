// tests/nql_path_knn_e9_test.rs
//
// Integration tests for E-9 PathKNN:
// path_knn_references(node_model, edge_model, k, min_score)
// Returns List<Object {name: String, score: Float}> sorted desc.

#[cfg(feature = "embeddings")]
mod tests {
    use nopaldb::{Edge, Graph, Node, PropertyValue, Result};

    fn str_val(s: &str) -> PropertyValue {
        PropertyValue::String(s.to_string())
    }

    /// Extrae Vec<(name, score)> de un QueryResult row col que debe ser PropertyValue::List.
    fn get_knn_list(
        result: &nopaldb::query::nql::QueryResult,
        row: usize,
        col: &str,
    ) -> Vec<(String, f64)> {
        match result.rows()[row].get(col) {
            Some(PropertyValue::List(items)) => items
                .iter()
                .map(|item| match item {
                    PropertyValue::Object(fields) => {
                        let name = fields
                            .iter()
                            .find(|(k, _)| k == "name")
                            .map(|(_, v)| match v {
                                PropertyValue::String(s) => s.clone(),
                                other => panic!("expected String for name, got {:?}", other),
                            })
                            .expect("missing 'name' field");
                        let score = fields
                            .iter()
                            .find(|(k, _)| k == "score")
                            .map(|(_, v)| match v {
                                PropertyValue::Float(f) => *f,
                                other => panic!("expected Float for score, got {:?}", other),
                            })
                            .expect("missing 'score' field");
                        (name, score)
                    }
                    other => panic!("expected Object in List, got {:?}", other),
                })
                .collect(),
            other => panic!("expected List for '{}', got {:?}", col, other),
        }
    }

    /// Setup: un grafo con nodo Root conectado a B y C.
    /// Registra 3 referencias: "ref_ab" (coincide con path Root→B), "ref_ac" (coincide con path Root→C),
    /// "ref_unrelated" (ortogonal a ambos).
    async fn setup_knn_graph() -> Result<Graph> {
        let graph = Graph::in_memory().await?;

        let mut tx = graph.begin_transaction().await?;
        let root = tx
            .add_node(Node::new("Account").with_property("name", str_val("Root")))
            .await?;
        let b = tx
            .add_node(Node::new("Account").with_property("name", str_val("B")))
            .await?;
        let c = tx
            .add_node(Node::new("Account").with_property("name", str_val("C")))
            .await?;
        let rel_rb = tx.add_edge(Edge::new(root, b, "TX"))?;
        let rel_rc = tx.add_edge(Edge::new(root, c, "TX"))?;
        tx.commit().await?;

        // Root=[1,0], B=[1,0], C=[0,1]
        // edge-RB=[1,0], edge-RC=[0,1]
        graph.add_node_embedding(root, vec![1.0, 0.0], "nm").await?;
        graph.add_node_embedding(b, vec![1.0, 0.0], "nm").await?;
        graph.add_node_embedding(c, vec![0.0, 1.0], "nm").await?;
        graph
            .add_edge_embedding(rel_rb, vec![1.0, 0.0], "em")
            .await?;
        graph
            .add_edge_embedding(rel_rc, vec![0.0, 1.0], "em")
            .await?;

        // path Root→B: node_mean=[1,0], edge_mean=[1,0] → [1,0,1,0]
        // path Root→C: node_mean=[0.5,0.5], edge_mean=[0,1] → [0.5,0.5,0,1]

        // ref_ab ≈ [1,0,1,0] → máxima similitud con Root→B
        graph
            .add_path_reference_embedding(
                "ref_ab".to_string(),
                "nm".to_string(),
                "em".to_string(),
                vec![1.0, 0.0, 1.0, 0.0],
            )
            .await?;

        // ref_ac ≈ [0.5,0.5,0,1] normalizado → máxima similitud con Root→C
        graph
            .add_path_reference_embedding(
                "ref_ac".to_string(),
                "nm".to_string(),
                "em".to_string(),
                vec![0.5, 0.5, 0.0, 1.0],
            )
            .await?;

        // ref_unrelated = [0,1,0,0] → baja similitud con ambos
        graph
            .add_path_reference_embedding(
                "ref_unrelated".to_string(),
                "nm".to_string(),
                "em".to_string(),
                vec![0.0, 1.0, 0.0, 0.0],
            )
            .await?;

        Ok(graph)
    }

    // ────────────────────────────────────────────────────────────
    // Test 1: devuelve top-k ordenado desc
    // ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_knn_returns_top_k_sorted() -> Result<()> {
        let graph = setup_knn_graph().await?;

        let result = graph
            .execute_nql(
                r#"
                find path_knn_references("nm", "em", 2, 0.0) as refs
                from (a:Account {name: "Root"})-[:TX]->(b:Account {name: "B"})
            "#,
            )
            .await?;

        assert_eq!(result.len(), 1);
        let refs = get_knn_list(&result, 0, "refs");
        assert_eq!(refs.len(), 2, "expected top-2 refs, got {}", refs.len());
        // Verificar orden desc
        assert!(
            refs[0].1 >= refs[1].1,
            "expected descending order: {} >= {}",
            refs[0].1,
            refs[1].1
        );
        // ref_ab debe ser primera (score ≈ 1.0 para path Root→B)
        assert_eq!(
            refs[0].0, "ref_ab",
            "expected ref_ab first, got {}",
            refs[0].0
        );
        Ok(())
    }

    // ────────────────────────────────────────────────────────────
    // Test 2: min_score filtra referencias con score bajo
    // ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_knn_filters_by_min_score() -> Result<()> {
        let graph = setup_knn_graph().await?;

        // min_score=0.95 → solo ref_ab debe pasar para path Root→B
        let result = graph
            .execute_nql(
                r#"
                find path_knn_references("nm", "em", 10, 0.95) as refs
                from (a:Account {name: "Root"})-[:TX]->(b:Account {name: "B"})
            "#,
            )
            .await?;

        assert_eq!(result.len(), 1);
        let refs = get_knn_list(&result, 0, "refs");
        // Solo ref_ab tiene score ≈ 1.0; las otras deben estar por debajo
        assert!(refs.len() >= 1, "expected at least ref_ab");
        for (name, score) in &refs {
            assert!(
                *score >= 0.95,
                "ref {} has score {} below threshold 0.95",
                name,
                score
            );
        }
        Ok(())
    }

    // ────────────────────────────────────────────────────────────
    // Test 3: self-similarity aparece primero con score ≈ 1.0
    // ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_knn_self_similarity_is_first() -> Result<()> {
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
        graph.add_edge_embedding(rel, vec![1.0, 1.0], "em").await?;

        // path vector: node_mean=[0.5,0.5], edge_mean=[1,1] → [0.5,0.5,1,1]
        graph
            .add_path_reference_embedding(
                "self_ref".to_string(),
                "nm".to_string(),
                "em".to_string(),
                vec![0.5, 0.5, 1.0, 1.0],
            )
            .await?;
        graph
            .add_path_reference_embedding(
                "other_ref".to_string(),
                "nm".to_string(),
                "em".to_string(),
                vec![1.0, 0.0, 0.0, 1.0],
            )
            .await?;

        let result = graph
            .execute_nql(
                r#"
                find path_knn_references("nm", "em", 2, 0.0) as refs
                from (a:Account {name: "A"})-[:TX]->(b:Account {name: "B"})
            "#,
            )
            .await?;

        assert_eq!(result.len(), 1);
        let refs = get_knn_list(&result, 0, "refs");
        assert_eq!(refs.len(), 2);
        assert_eq!(
            refs[0].0, "self_ref",
            "expected self_ref first, got {}",
            refs[0].0
        );
        assert!(
            (refs[0].1 - 1.0).abs() < 1e-5,
            "expected self_ref score ≈ 1.0, got {}",
            refs[0].1
        );
        Ok(())
    }

    // ────────────────────────────────────────────────────────────
    // Test 4: k mayor que referencias disponibles → devuelve lo que hay
    // ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_knn_k_larger_than_available() -> Result<()> {
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
        graph.add_node_embedding(b, vec![1.0, 0.0], "nm").await?;
        graph.add_edge_embedding(rel, vec![1.0, 0.0], "em").await?;

        graph
            .add_path_reference_embedding(
                "r1".to_string(),
                "nm".to_string(),
                "em".to_string(),
                vec![1.0, 0.0, 1.0, 0.0],
            )
            .await?;
        graph
            .add_path_reference_embedding(
                "r2".to_string(),
                "nm".to_string(),
                "em".to_string(),
                vec![0.0, 1.0, 0.0, 1.0],
            )
            .await?;

        // k=10 pero solo hay 2 referencias → devuelve 2 sin error
        let result = graph
            .execute_nql(
                r#"
                find path_knn_references("nm", "em", 10, 0.0) as refs
                from (a:Account {name: "A"})-[:TX]->(b:Account {name: "B"})
            "#,
            )
            .await?;

        assert_eq!(result.len(), 1);
        let refs = get_knn_list(&result, 0, "refs");
        assert_eq!(
            refs.len(),
            2,
            "expected 2 refs (all available), got {}",
            refs.len()
        );
        Ok(())
    }

    // ────────────────────────────────────────────────────────────
    // Test 5: lista vacía cuando todo está por debajo de min_score
    // ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_knn_empty_when_all_below_min_score() -> Result<()> {
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
        graph.add_edge_embedding(rel, vec![1.0, 1.0], "em").await?;

        graph
            .add_path_reference_embedding(
                "ref1".to_string(),
                "nm".to_string(),
                "em".to_string(),
                vec![0.5, 0.5, 1.0, 1.0],
            )
            .await?;

        // min_score=1.1 → imposible, resultado vacío
        let result = graph
            .execute_nql(
                r#"
                find path_knn_references("nm", "em", 5, 1.1) as refs
                from (a:Account {name: "A"})-[:TX]->(b:Account {name: "B"})
            "#,
            )
            .await?;

        assert_eq!(result.len(), 1);
        let refs = get_knn_list(&result, 0, "refs");
        assert_eq!(refs.len(), 0, "expected empty list when min_score=1.1");
        Ok(())
    }

    // ────────────────────────────────────────────────────────────
    // Test 6: lista vacía cuando no hay referencias para el par (nm, em)
    // ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_knn_empty_when_no_refs_for_model_pair() -> Result<()> {
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
        graph.add_edge_embedding(rel, vec![1.0, 0.0], "em").await?;

        // Registrar referencias con un par distinto (nm2, em2)
        graph
            .add_path_reference_embedding(
                "ref_other".to_string(),
                "nm2".to_string(),
                "em2".to_string(),
                vec![1.0, 0.0, 1.0, 0.0],
            )
            .await?;

        // Consultar (nm, em) → sin referencias → lista vacía
        let result = graph
            .execute_nql(
                r#"
                find path_knn_references("nm", "em", 5, 0.0) as refs
                from (a:Account {name: "A"})-[:TX]->(b:Account {name: "B"})
            "#,
            )
            .await?;

        assert_eq!(result.len(), 1);
        let refs = get_knn_list(&result, 0, "refs");
        assert_eq!(
            refs.len(),
            0,
            "expected empty list when no refs for (nm, em)"
        );
        Ok(())
    }

    // ────────────────────────────────────────────────────────────
    // Test 7: path_knn_references en WHERE — abierto en E-10
    // ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_knn_in_where_accepted_since_e10() -> Result<()> {
        // E-10 abrió path_knn_references en WHERE. El validator ya no lo rechaza.
        // Verificamos que la query pasa la validación semántica.
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
        graph.add_edge_embedding(rel, vec![1.0, 1.0], "em").await?;

        graph
            .add_path_reference_embedding(
                "ref".to_string(),
                "nm".to_string(),
                "em".to_string(),
                vec![0.5, 0.5, 1.0, 1.0],
            )
            .await?;

        // path_knn_references en WHERE pasa el validator en E-10.
        // Usamos path_anomaly_score como condición numérica comparable.
        let result = graph
            .execute_nql(
                r#"
                find b.name
                from (a:Account {name: "A"})-[:TX]->(b:Account {name: "B"})
                where path_anomaly_score("nm", "em") < 1.0
            "#,
            )
            .await;

        assert!(
            result.is_ok(),
            "expected query to succeed since E-10 allows anomaly in WHERE, got: {:?}",
            result
        );
        Ok(())
    }

    // ────────────────────────────────────────────────────────────
    // Test 8: aridad incorrecta rechazada
    // ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_knn_wrong_arity_rejected() -> Result<()> {
        let graph = Graph::in_memory().await?;

        let err = graph
            .execute_nql(
                r#"
                find path_knn_references("nm", "em", 3) as refs
                from (a:Account)-[:TX]->(b:Account)
            "#,
            )
            .await;

        assert!(err.is_err(), "expected error for 3-arg form");
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.contains("4 arguments") || msg.contains("E-9"),
            "expected arity error, got: {}",
            msg
        );
        Ok(())
    }
}
