// tests/nql_embeddings_test.rs
//
// Pruebas de integración para las funciones NQL de embeddings:
//   - has_embedding(n, "model")    — predicado WHERE
//   - embedding_similarity(n, "ref-uuid", "model") — proyección FIND
//   - knn_nodes(n, k, "model")     — proyección FIND con HNSW

#[cfg(all(feature = "embeddings", feature = "embeddings-index"))]
mod tests {
    use nopaldb::Graph;
    use nopaldb::types::{Node, PropertyValue};

    /// Crea un grafo en memoria con 4 nodos "Article" con embeddings en R².
    /// Retorna (graph, node_ids) donde node_ids[i] corresponde al i-ésimo nodo.
    async fn setup_graph() -> (Graph, Vec<uuid::Uuid>) {
        let graph = Graph::in_memory().await.unwrap();

        let data: Vec<(&str, Vec<f32>)> = vec![
            ("alpha", vec![1.0, 0.0]), // eje X positivo
            ("beta", vec![0.0, 1.0]),  // eje Y positivo
            ("gamma", vec![1.0, 1.0]), // diagonal
            ("delta", vec![0.5, 0.5]), // centro
        ];

        let mut ids = vec![];
        for (name, vec) in data {
            let node = Node::new("Article")
                .with_property("title", PropertyValue::String(name.to_string()));
            graph.add_node(node.clone()).await.unwrap();
            graph
                .add_node_embedding(node.id, vec, "minilm")
                .await
                .unwrap();
            ids.push(node.id);
        }
        (graph, ids)
    }

    // ── has_embedding ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_has_embedding_filters_correctly() -> nopaldb::Result<()> {
        let (graph, ids) = setup_graph().await;

        // Agregar un nodo SIN embedding
        let orphan = Node::new("Article")
            .with_property("title", PropertyValue::String("orphan".to_string()));
        graph.add_node(orphan.clone()).await?;

        let nql = "FIND n.title FROM (n:Article) WHERE has_embedding(n, \"minilm\")";
        let result = graph.execute_nql(nql).await?;

        // Solo los 4 nodos con embedding deben aparecer
        assert_eq!(
            result.rows.len(),
            4,
            "should only return nodes with embedding"
        );

        // El nodo sin embedding no debe estar
        let titles: Vec<_> = result
            .rows
            .iter()
            .filter_map(|r| r.get("n.title"))
            .filter_map(|v| {
                if let PropertyValue::String(s) = v {
                    Some(s.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert!(
            !titles.contains(&"orphan"),
            "orphan (no embedding) should not appear"
        );

        // Todos los ids con embedding deben aparecer
        assert_eq!(titles.len(), 4);
        drop(ids); // usado para confirmar que setup_graph creó 4
        Ok(())
    }

    #[tokio::test]
    async fn test_has_embedding_false_for_different_model() -> nopaldb::Result<()> {
        let (graph, _ids) = setup_graph().await;

        // Buscar con un modelo que no existe — debe retornar 0 filas
        let nql = "FIND n.title FROM (n:Article) WHERE has_embedding(n, \"gpt4\")";
        let result = graph.execute_nql(nql).await?;
        assert_eq!(result.rows.len(), 0, "no nodes should have gpt4 embeddings");
        Ok(())
    }

    // ── embedding_similarity ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_embedding_similarity_self_is_one() -> nopaldb::Result<()> {
        let (graph, ids) = setup_graph().await;

        // embedding_similarity(n, alpha_uuid, "minilm") para el propio alpha
        let ref_uuid = ids[0].to_string();
        let nql = format!(
            "FIND n.title, embedding_similarity(n, \"{}\", \"minilm\") AS sim \
             FROM (n:Article) WHERE n.title = \"alpha\"",
            ref_uuid
        );
        let result = graph.execute_nql(&nql).await?;
        assert_eq!(result.rows.len(), 1);

        let sim = match result.rows[0].get("sim") {
            Some(PropertyValue::Float(f)) => *f,
            _ => panic!("expected Float for sim"),
        };
        // Similitud del nodo consigo mismo ≈ 1.0
        assert!(
            (sim - 1.0).abs() < 1e-5,
            "self-similarity should be ~1.0, got {}",
            sim
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_embedding_similarity_orthogonal_is_zero() -> nopaldb::Result<()> {
        let (graph, ids) = setup_graph().await;

        // alpha=[1,0] vs beta=[0,1]: ortogonales → similitud = 0
        let ref_uuid = ids[0].to_string(); // alpha
        let nql = format!(
            "FIND n.title, embedding_similarity(n, \"{}\", \"minilm\") AS sim \
             FROM (n:Article) WHERE n.title = \"beta\"",
            ref_uuid
        );
        let result = graph.execute_nql(&nql).await?;
        assert_eq!(result.rows.len(), 1);

        let sim = match result.rows[0].get("sim") {
            Some(PropertyValue::Float(f)) => *f,
            _ => panic!("expected Float for sim"),
        };
        assert!(
            sim.abs() < 1e-5,
            "orthogonal vectors should have sim ~0.0, got {}",
            sim
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_embedding_similarity_ordered_results() -> nopaldb::Result<()> {
        let (graph, ids) = setup_graph().await;

        // alpha=[1,0] — todos los nodos ordenados por similitud decreciente
        let ref_uuid = ids[0].to_string();
        let nql = format!(
            "FIND n.title, embedding_similarity(n, \"{}\", \"minilm\") AS sim \
             FROM (n:Article) \
             ORDER BY sim DESC",
            ref_uuid
        );
        let result = graph.execute_nql(&nql).await?;
        assert_eq!(result.rows.len(), 4);

        // El primer resultado debe ser alpha (sim ≈ 1.0)
        let first_title = match result.rows[0].get("n.title") {
            Some(PropertyValue::String(s)) => s.as_str(),
            _ => panic!("expected title string"),
        };
        assert_eq!(first_title, "alpha");

        // beta (ortogonal) debe ser el último
        let last_title = match result.rows[3].get("n.title") {
            Some(PropertyValue::String(s)) => s.as_str(),
            _ => panic!("expected title string"),
        };
        assert_eq!(last_title, "beta");
        Ok(())
    }

    // ── knn_nodes ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_knn_nodes_returns_json_array() -> nopaldb::Result<()> {
        let (graph, _ids) = setup_graph().await;

        // knn_nodes para el nodo alpha, top-2
        let nql = "FIND n.title, knn_nodes(n, 2, \"minilm\") AS neighbors \
                   FROM (n:Article) WHERE n.title = \"alpha\"";
        let result = graph.execute_nql(nql).await?;
        assert_eq!(result.rows.len(), 1);

        let neighbors = match result.rows[0].get("neighbors") {
            Some(PropertyValue::String(s)) => s.clone(),
            _ => panic!("expected String for neighbors"),
        };

        // Debe ser un JSON array válido con 2 UUIDs
        assert!(neighbors.starts_with('['), "should be JSON array");
        assert!(neighbors.ends_with(']'), "should be JSON array");
        let count = neighbors.matches("\"").count() / 2;
        assert_eq!(count, 2, "should contain 2 UUIDs, got: {}", neighbors);
        Ok(())
    }

    #[tokio::test]
    async fn test_knn_nodes_no_embedding_returns_empty_array() -> nopaldb::Result<()> {
        let (graph, _ids) = setup_graph().await;

        // Nodo sin embedding
        let orphan = Node::new("Article")
            .with_property("title", PropertyValue::String("orphan".to_string()));
        graph.add_node(orphan.clone()).await?;

        let nql = "FIND n.title, knn_nodes(n, 3, \"minilm\") AS neighbors \
                   FROM (n:Article) WHERE n.title = \"orphan\"";
        let result = graph.execute_nql(nql).await?;
        assert_eq!(result.rows.len(), 1);

        let neighbors = match result.rows[0].get("neighbors") {
            Some(PropertyValue::String(s)) => s.clone(),
            _ => panic!("expected String"),
        };
        assert_eq!(neighbors, "[]", "no embedding → empty array");
        Ok(())
    }
}
