// tests/nql_similar_to_test.rs
//
// Pruebas de integración para similar_to() en WHERE:
//   similar_to(n, "reference_name", "model") — búsqueda ANN via HNSW
//
// similar_to pre-computa los k vecinos más cercanos al nodo de referencia
// y filtra el stream a ese set. k viene del LIMIT de la query.

#[cfg(all(feature = "embeddings", feature = "embeddings-index"))]
mod tests {
    use nopaldb::Graph;
    use nopaldb::types::{Node, PropertyValue};

    /// Crea un grafo con 6 nodos Company con embeddings unitarios en R³.
    /// Los vectores están distribuidos angularmente para que la distancia coseno
    /// produzca un ranking claro.
    async fn setup_company_graph() -> (Graph, Vec<uuid::Uuid>) {
        let graph = Graph::in_memory().await.unwrap();

        // Vectores diseñados para ranking claro con distancia coseno:
        // "Atlas" apunta al eje X. Los demás rotan progresivamente.
        let data: Vec<(&str, Vec<f32>)> = vec![
            ("Atlas Fiduciary Group", vec![1.0, 0.0, 0.0]), // referencia
            ("Shell Corp A", vec![0.95, 0.31, 0.0]),        // cercano (~18°)
            ("Shell Corp B", vec![0.80, 0.60, 0.0]),        // medio (~37°)
            ("Legit Corp", vec![0.50, 0.87, 0.0]),          // lejano (~60°)
            ("Bank Nordic", vec![0.0, 1.0, 0.0]),           // ortogonal (~90°)
            ("Fund Pacific", vec![0.0, 0.0, 1.0]),          // ortogonal (~90°)
        ];

        let mut ids = vec![];
        for (name, vec) in data {
            let node =
                Node::new("Company").with_property("name", PropertyValue::String(name.to_string()));
            graph.add_node(node.clone()).await.unwrap();
            graph
                .add_node_embedding(node.id, vec, "minilm")
                .await
                .unwrap();
            ids.push(node.id);
        }
        (graph, ids)
    }

    #[tokio::test]
    async fn test_similar_to_basic() -> nopaldb::Result<()> {
        let (graph, _ids) = setup_company_graph().await;

        // Buscar las 3 empresas más similares a "Atlas Fiduciary Group"
        let nql = r#"
            FIND n.name
            FROM (n:Company)
            WHERE similar_to(n, "Atlas Fiduciary Group", "minilm")
            LIMIT 3
        "#;
        let result = graph.execute_nql(nql).await?;

        assert_eq!(
            result.rows.len(),
            3,
            "should return top 3 similar companies"
        );

        // Los resultados deben incluir Atlas Fiduciary Group (self), Shell Corp A, Shell Corp B
        let names: Vec<String> = result
            .rows
            .iter()
            .filter_map(|r| r.get("n.name"))
            .filter_map(|v| {
                if let PropertyValue::String(s) = v {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect();

        assert!(
            names.contains(&"Atlas Fiduciary Group".to_string()),
            "self should be in results"
        );
        assert!(
            names.contains(&"Shell Corp A".to_string()),
            "closest should be in results"
        );
        // Shell Corp B debería ser el tercero más cercano
        assert!(
            names.contains(&"Shell Corp B".to_string()),
            "second closest should be in results"
        );

        // Las empresas lejanas NO deben aparecer
        assert!(
            !names.contains(&"Bank Nordic".to_string()),
            "orthogonal node should not be in top 3"
        );
        assert!(
            !names.contains(&"Fund Pacific".to_string()),
            "orthogonal node should not be in top 3"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_similar_to_combined_with_property_filter() -> nopaldb::Result<()> {
        let (graph, _ids) = setup_company_graph().await;

        // Agregar propiedad "offshore" a algunos
        // (update via NQL o directamente)
        let nql_update =
            r#"UPDATE (n:Company) SET n.sector = "offshore" WHERE n.name = "Shell Corp A""#;
        graph.execute_nql(nql_update).await?;
        let nql_update2 =
            r#"UPDATE (n:Company) SET n.sector = "offshore" WHERE n.name = "Shell Corp B""#;
        graph.execute_nql(nql_update2).await?;
        let nql_update3 =
            r#"UPDATE (n:Company) SET n.sector = "legal" WHERE n.name = "Atlas Fiduciary Group""#;
        graph.execute_nql(nql_update3).await?;

        // similar_to con filtro adicional: solo offshore
        let nql = r#"
            FIND n.name
            FROM (n:Company)
            WHERE similar_to(n, "Atlas Fiduciary Group", "minilm") AND n.sector = "offshore"
            LIMIT 5
        "#;
        let result = graph.execute_nql(nql).await?;

        let names: Vec<String> = result
            .rows
            .iter()
            .filter_map(|r| r.get("n.name"))
            .filter_map(|v| {
                if let PropertyValue::String(s) = v {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect();

        // Solo Shell Corp A y B son offshore y similares
        assert!(names.contains(&"Shell Corp A".to_string()));
        assert!(names.contains(&"Shell Corp B".to_string()));
        // Atlas Fiduciary Group es "legal", no "offshore"
        assert!(!names.contains(&"Atlas Fiduciary Group".to_string()));

        Ok(())
    }

    #[tokio::test]
    async fn test_similar_to_reference_not_found() -> nopaldb::Result<()> {
        let (graph, _ids) = setup_company_graph().await;

        let nql = r#"
            FIND n.name
            FROM (n:Company)
            WHERE similar_to(n, "Nonexistent Corp", "minilm")
            LIMIT 5
        "#;
        let result = graph.execute_nql(nql).await;

        // Debe retornar error porque el nodo referencia no existe
        assert!(
            result.is_err(),
            "should error when reference node not found"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_similar_to_no_embedding_for_model() -> nopaldb::Result<()> {
        let (graph, _ids) = setup_company_graph().await;

        let nql = r#"
            FIND n.name
            FROM (n:Company)
            WHERE similar_to(n, "Atlas Fiduciary Group", "nonexistent-model")
            LIMIT 5
        "#;
        let result = graph.execute_nql(nql).await;

        // Debe retornar error porque no hay embeddings para ese modelo
        assert!(result.is_err(), "should error when model has no embeddings");

        Ok(())
    }

    #[tokio::test]
    async fn test_similar_to_default_limit() -> nopaldb::Result<()> {
        let (graph, _ids) = setup_company_graph().await;

        // Sin LIMIT explícito, default es 10 (más que los 6 nodos)
        let nql = r#"
            FIND n.name
            FROM (n:Company)
            WHERE similar_to(n, "Atlas Fiduciary Group", "minilm")
        "#;
        let result = graph.execute_nql(nql).await?;

        // Debería retornar todos los 6 nodos (k=10 > 6)
        assert_eq!(
            result.rows.len(),
            6,
            "default k=10 should return all 6 nodes"
        );

        Ok(())
    }
}
