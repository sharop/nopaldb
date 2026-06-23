#[cfg(feature = "embeddings")]
mod tests {
    use nopaldb::{Edge, Graph, Node, PropertyValue, Result};

    fn str_val(value: &str) -> PropertyValue {
        PropertyValue::String(value.to_string())
    }

    fn float_list(row: &nopaldb::query::nql::Row, key: &str) -> Vec<f64> {
        match row.get(key) {
            Some(PropertyValue::List(values)) => values
                .iter()
                .map(|value| match value {
                    PropertyValue::Float(f) => *f,
                    other => panic!(
                        "expected Float inside pattern embedding list, got {:?}",
                        other
                    ),
                })
                .collect(),
            other => panic!("expected List for '{}', got {:?}", key, other),
        }
    }

    #[tokio::test]
    async fn test_pattern_embedding_single_hop_concatenates_node_edge_node() -> Result<()> {
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

        graph
            .add_node_embedding(a, vec![1.0, 2.0], "node-minilm")
            .await?;
        graph
            .add_node_embedding(b, vec![3.0, 4.0], "node-minilm")
            .await?;
        graph
            .add_edge_embedding(rel, vec![9.0], "edge-relbert")
            .await?;

        let result = graph
            .execute_nql(
                r#"
                find pattern_embedding("node-minilm", "edge-relbert") as pat
                from (a:Account {name: "A"})-[:TX]->(b:Account {name: "B"})
            "#,
            )
            .await?;

        assert_eq!(result.len(), 1);
        assert_eq!(
            float_list(&result.rows()[0], "pat"),
            vec![1.0, 2.0, 9.0, 3.0, 4.0]
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_pattern_embedding_multi_hop_concatenates_full_binding() -> Result<()> {
        let graph = Graph::in_memory().await?;

        let mut tx = graph.begin_transaction().await?;
        let a = tx
            .add_node(Node::new("Account").with_property("name", str_val("A")))
            .await?;
        let b = tx
            .add_node(Node::new("Account").with_property("name", str_val("B")))
            .await?;
        let c = tx
            .add_node(Node::new("Account").with_property("name", str_val("C")))
            .await?;
        let rel_ab = tx.add_edge(Edge::new(a, b, "TX"))?;
        let rel_bc = tx.add_edge(Edge::new(b, c, "TX"))?;
        tx.commit().await?;

        graph
            .add_node_embedding(a, vec![1.0], "node-minilm")
            .await?;
        graph
            .add_node_embedding(b, vec![2.0], "node-minilm")
            .await?;
        graph
            .add_node_embedding(c, vec![3.0], "node-minilm")
            .await?;
        graph
            .add_edge_embedding(rel_ab, vec![10.0, 11.0], "edge-relbert")
            .await?;
        graph
            .add_edge_embedding(rel_bc, vec![20.0, 21.0], "edge-relbert")
            .await?;

        let result = graph
            .execute_nql(
                r#"
                find c.name, pattern_embedding("node-minilm", "edge-relbert") as pat
                from (a:Account {name: "A"})-[:TX]->(b:Account)-[:TX]->(c:Account {name: "C"})
            "#,
            )
            .await?;

        assert_eq!(result.len(), 1);
        assert_eq!(
            float_list(&result.rows()[0], "pat"),
            vec![1.0, 10.0, 11.0, 2.0, 20.0, 21.0, 3.0]
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_pattern_embedding_quantified_uses_materialized_binding() -> Result<()> {
        let graph = Graph::in_memory().await?;

        let mut tx = graph.begin_transaction().await?;
        let a = tx
            .add_node(Node::new("Account").with_property("name", str_val("A")))
            .await?;
        let b = tx
            .add_node(Node::new("Account").with_property("name", str_val("B")))
            .await?;
        let c = tx
            .add_node(Node::new("Account").with_property("name", str_val("C")))
            .await?;
        let rel_ab = tx.add_edge(Edge::new(a, b, "TX"))?;
        let rel_bc = tx.add_edge(Edge::new(b, c, "TX"))?;
        tx.commit().await?;

        graph
            .add_node_embedding(a, vec![1.0], "node-minilm")
            .await?;
        graph
            .add_node_embedding(b, vec![2.0], "node-minilm")
            .await?;
        graph
            .add_node_embedding(c, vec![3.0], "node-minilm")
            .await?;
        graph
            .add_edge_embedding(rel_ab, vec![10.0], "edge-relbert")
            .await?;
        graph
            .add_edge_embedding(rel_bc, vec![20.0], "edge-relbert")
            .await?;

        let result = graph
            .execute_nql(
                r#"
                find n.name, pattern_embedding("node-minilm", "edge-relbert") as pat
                from (a:Account {name: "A"})-[:TX]->{1,2}(n:Account)
                where pattern_has_embeddings("node-minilm", "edge-relbert")
            "#,
            )
            .await?;

        assert_eq!(result.len(), 2);
        let mut rows: Vec<_> = result.rows().iter().collect();
        rows.sort_by_key(|row| match row.get("n.name") {
            Some(PropertyValue::String(name)) => name.clone(),
            _ => String::new(),
        });

        assert_eq!(float_list(rows[0], "pat"), vec![1.0, 10.0, 2.0]);
        assert_eq!(float_list(rows[1], "pat"), vec![1.0, 10.0, 2.0, 20.0, 3.0]);
        Ok(())
    }

    #[tokio::test]
    async fn test_pattern_has_embeddings_returns_false_when_any_edge_embedding_is_missing()
    -> Result<()> {
        let graph = Graph::in_memory().await?;

        let mut tx = graph.begin_transaction().await?;
        let a = tx
            .add_node(Node::new("Account").with_property("name", str_val("A")))
            .await?;
        let b = tx
            .add_node(Node::new("Account").with_property("name", str_val("B")))
            .await?;
        let c = tx
            .add_node(Node::new("Account").with_property("name", str_val("C")))
            .await?;
        let rel_ab = tx.add_edge(Edge::new(a, b, "TX"))?;
        let _rel_bc = tx.add_edge(Edge::new(b, c, "TX"))?;
        tx.commit().await?;

        graph
            .add_node_embedding(a, vec![1.0], "node-minilm")
            .await?;
        graph
            .add_node_embedding(b, vec![2.0], "node-minilm")
            .await?;
        graph
            .add_node_embedding(c, vec![3.0], "node-minilm")
            .await?;
        graph
            .add_edge_embedding(rel_ab, vec![10.0], "edge-relbert")
            .await?;

        let result = graph
            .execute_nql(
                r#"
                find n.name
                from (a:Account {name: "A"})-[:TX]->{1,2}(n:Account)
                where pattern_has_embeddings("node-minilm", "edge-relbert")
            "#,
            )
            .await?;

        assert_eq!(result.len(), 1);
        assert_eq!(
            result.rows()[0].get("n.name"),
            Some(&PropertyValue::String("B".to_string()))
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_pattern_embedding_errors_when_node_embedding_is_missing() -> Result<()> {
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

        graph
            .add_node_embedding(a, vec![1.0, 2.0], "node-minilm")
            .await?;
        graph
            .add_edge_embedding(rel, vec![9.0], "edge-relbert")
            .await?;

        let err = graph
            .execute_nql(
                r#"
                find pattern_embedding("node-minilm", "edge-relbert") as pat
                from (a:Account {name: "A"})-[:TX]->(b:Account {name: "B"})
            "#,
            )
            .await
            .expect_err("missing node embedding should fail");

        assert!(err.to_string().contains("Embedding not found for node"));
        Ok(())
    }

    #[tokio::test]
    async fn test_pattern_embedding_errors_when_edge_embedding_is_missing() -> Result<()> {
        let graph = Graph::in_memory().await?;

        let mut tx = graph.begin_transaction().await?;
        let a = tx
            .add_node(Node::new("Account").with_property("name", str_val("A")))
            .await?;
        let b = tx
            .add_node(Node::new("Account").with_property("name", str_val("B")))
            .await?;
        tx.add_edge(Edge::new(a, b, "TX"))?;
        tx.commit().await?;

        graph
            .add_node_embedding(a, vec![1.0, 2.0], "node-minilm")
            .await?;
        graph
            .add_node_embedding(b, vec![3.0, 4.0], "node-minilm")
            .await?;

        let err = graph
            .execute_nql(
                r#"
                find pattern_embedding("node-minilm", "edge-relbert") as pat
                from (a:Account {name: "A"})-[:TX]->(b:Account {name: "B"})
            "#,
            )
            .await
            .expect_err("missing edge embedding should fail");

        assert!(err.to_string().contains("Embedding not found for edge"));
        Ok(())
    }

    #[tokio::test]
    async fn test_pattern_embedding_errors_on_inconsistent_node_dimensions() -> Result<()> {
        let graph = Graph::in_memory().await?;

        let mut tx = graph.begin_transaction().await?;
        let a = tx
            .add_node(Node::new("Account").with_property("name", str_val("A")))
            .await?;
        let b = tx
            .add_node(Node::new("Account").with_property("name", str_val("B")))
            .await?;
        let c = tx
            .add_node(Node::new("Account").with_property("name", str_val("C")))
            .await?;
        let rel_ab = tx.add_edge(Edge::new(a, b, "TX"))?;
        let rel_bc = tx.add_edge(Edge::new(b, c, "TX"))?;
        tx.commit().await?;

        graph
            .add_node_embedding(a, vec![1.0, 2.0], "node-minilm")
            .await?;
        graph
            .add_node_embedding(b, vec![3.0, 4.0, 5.0], "node-minilm")
            .await?;
        graph
            .add_node_embedding(c, vec![6.0, 7.0], "node-minilm")
            .await?;
        graph
            .add_edge_embedding(rel_ab, vec![10.0], "edge-relbert")
            .await?;
        graph
            .add_edge_embedding(rel_bc, vec![20.0], "edge-relbert")
            .await?;

        let err = graph
            .execute_nql(
                r#"
                find pattern_embedding("node-minilm", "edge-relbert") as pat
                from (a:Account {name: "A"})-[:TX]->(b:Account)-[:TX]->(c:Account {name: "C"})
            "#,
            )
            .await
            .expect_err("inconsistent node dimensions should fail");

        assert!(
            err.to_string()
                .contains("inconsistent node embedding dimensions")
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_pattern_embedding_errors_on_inconsistent_edge_dimensions() -> Result<()> {
        let graph = Graph::in_memory().await?;

        let mut tx = graph.begin_transaction().await?;
        let a = tx
            .add_node(Node::new("Account").with_property("name", str_val("A")))
            .await?;
        let b = tx
            .add_node(Node::new("Account").with_property("name", str_val("B")))
            .await?;
        let c = tx
            .add_node(Node::new("Account").with_property("name", str_val("C")))
            .await?;
        let rel_ab = tx.add_edge(Edge::new(a, b, "TX"))?;
        let rel_bc = tx.add_edge(Edge::new(b, c, "TX"))?;
        tx.commit().await?;

        graph
            .add_node_embedding(a, vec![1.0], "node-minilm")
            .await?;
        graph
            .add_node_embedding(b, vec![2.0], "node-minilm")
            .await?;
        graph
            .add_node_embedding(c, vec![3.0], "node-minilm")
            .await?;
        graph
            .add_edge_embedding(rel_ab, vec![10.0], "edge-relbert")
            .await?;
        graph
            .add_edge_embedding(rel_bc, vec![20.0, 21.0], "edge-relbert")
            .await?;

        let err = graph
            .execute_nql(
                r#"
                find pattern_embedding("node-minilm", "edge-relbert") as pat
                from (a:Account {name: "A"})-[:TX]->(b:Account)-[:TX]->(c:Account {name: "C"})
            "#,
            )
            .await
            .expect_err("inconsistent edge dimensions should fail");

        assert!(
            err.to_string()
                .contains("inconsistent edge embedding dimensions")
        );
        Ok(())
    }
}
