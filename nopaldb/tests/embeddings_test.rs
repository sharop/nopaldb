#[cfg(feature = "embeddings")]
mod tests {
    use nopaldb::types::{Node, Edge, PropertyValue};
    use nopaldb::{Graph, NopalError};
    use nopaldb::embeddings::{Embedding, EdgeEmbedding};

    #[tokio::test]
    async fn test_node_embedding_crud() -> nopaldb::Result<()> {
        let graph = Graph::in_memory().await?;

        let node = Node::new("Document").with_property("id", PropertyValue::String("doc1".to_string()));
        graph.add_node(node.clone()).await?;

        // Add embeddings for two different models on the same node
        let vector1 = vec![0.1, 0.2, 0.3];
        let vector2 = vec![0.9, 0.8, 0.7];
        
        graph.add_node_embedding(node.id, vector1.clone(), "minilm").await?;
        graph.add_node_embedding(node.id, vector2.clone(), "bert").await?;

        // Retrieve and verify
        let emb1 = graph.get_node_embedding(node.id, "minilm").await?;
        assert_eq!(emb1.node_id, node.id);
        assert_eq!(emb1.model, "minilm");
        assert_eq!(emb1.vector, vector1);
        
        let emb2 = graph.get_node_embedding(node.id, "bert").await?;
        assert_eq!(emb2.model, "bert");
        assert_eq!(emb2.vector, vector2);

        // Try retrieving a non-existent model
        let result = graph.get_node_embedding(node.id, "llama").await;
        assert!(result.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn test_embedding_on_non_existent_node() -> nopaldb::Result<()> {
        let graph = Graph::in_memory().await?;
        let random_id = uuid::Uuid::new_v4();
        
        let result = graph.add_node_embedding(random_id, vec![1.0, 2.0], "minilm").await;
        assert!(matches!(result, Err(NopalError::NodeNotFound(_))));

        Ok(())
    }

    #[tokio::test]
    async fn test_cosine_similarity() {
        let emb1 = Embedding::new(uuid::Uuid::new_v4(), vec![1.0, 0.0, 0.0], "test");
        let emb2 = Embedding::new(uuid::Uuid::new_v4(), vec![0.0, 1.0, 0.0], "test");
        let emb3 = Embedding::new(uuid::Uuid::new_v4(), vec![1.0, 1.0, 0.0], "test");

        assert_eq!(emb1.cosine_similarity(&emb2), 0.0);
        assert_eq!(emb1.cosine_similarity(&emb1), 1.0);
        let sim = emb1.cosine_similarity(&emb3);
        assert!((sim - 0.70710677).abs() < 1e-6);
    }

    // ── EdgeEmbedding tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_edge_embedding_crud() -> nopaldb::Result<()> {
        let graph = Graph::in_memory().await?;

        // Crear dos nodos y una arista entre ellos
        let src = Node::new("Person").with_property("name", PropertyValue::String("Alice".into()));
        let dst = Node::new("Person").with_property("name", PropertyValue::String("Bob".into()));
        graph.add_node(src.clone()).await?;
        graph.add_node(dst.clone()).await?;

        let edge = Edge::new(src.id, dst.id, "KNOWS");
        let edge_id = graph.add_edge(edge).await?;

        // Agregar embeddings de dos modelos distintos sobre la misma arista
        let v1 = vec![0.1_f32, 0.2, 0.3];
        let v2 = vec![0.9_f32, 0.8, 0.7];
        graph.add_edge_embedding(edge_id, v1.clone(), "concat-minilm").await?;
        graph.add_edge_embedding(edge_id, v2.clone(), "transe").await?;

        // Recuperar y verificar model 1
        let emb1 = graph.get_edge_embedding(edge_id, "concat-minilm").await?;
        assert_eq!(emb1.edge_id, edge_id);
        assert_eq!(emb1.model, "concat-minilm");
        assert_eq!(emb1.vector, v1);

        // Recuperar y verificar model 2
        let emb2 = graph.get_edge_embedding(edge_id, "transe").await?;
        assert_eq!(emb2.vector, v2);

        // Modelo inexistente devuelve error
        let miss = graph.get_edge_embedding(edge_id, "unknown-model").await;
        assert!(miss.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn test_edge_embedding_on_non_existent_edge() -> nopaldb::Result<()> {
        let graph = Graph::in_memory().await?;
        let random_id = uuid::Uuid::new_v4();

        let result = graph.add_edge_embedding(random_id, vec![1.0, 2.0], "minilm").await;
        assert!(matches!(result, Err(NopalError::EdgeNotFound(_))));

        Ok(())
    }

    #[tokio::test]
    async fn test_edge_embedding_cosine_similarity() {
        let id = uuid::Uuid::new_v4();
        let a = EdgeEmbedding::new(id, vec![1.0, 0.0], "test");
        let b = EdgeEmbedding::new(id, vec![1.0, 0.0], "test");
        let c = EdgeEmbedding::new(id, vec![0.0, 1.0], "test");

        assert!((a.cosine_similarity(&b) - 1.0).abs() < 1e-6);
        assert!(a.cosine_similarity(&c).abs() < 1e-6);
    }

    #[tokio::test]
    async fn test_node_and_edge_embeddings_do_not_collide() -> nopaldb::Result<()> {
        // Misma UUID usada como NodeId y EdgeId — las claves Sled deben ser distintas
        let graph = Graph::in_memory().await?;

        let node = Node::new("Item");
        graph.add_node(node.clone()).await?;

        let src = Node::new("A");
        let dst = Node::new("B");
        graph.add_node(src.clone()).await?;
        graph.add_node(dst.clone()).await?;
        let edge = Edge::new(src.id, dst.id, "REL");
        let edge_id = graph.add_edge(edge).await?;

        graph.add_node_embedding(node.id, vec![1.0, 0.0], "test").await?;
        graph.add_edge_embedding(edge_id, vec![0.0, 1.0], "test").await?;

        let n_emb = graph.get_node_embedding(node.id, "test").await?;
        let e_emb = graph.get_edge_embedding(edge_id, "test").await?;

        // Los vectores deben ser los originales, sin colisión
        assert_eq!(n_emb.vector, vec![1.0, 0.0]);
        assert_eq!(e_emb.vector, vec![0.0, 1.0]);

        Ok(())
    }
}

// ── EmbeddingIndex integration tests ──────────────────────────────────────────

#[cfg(feature = "embeddings-index")]
mod index_tests {
    use nopaldb::types::{Node, PropertyValue};
    use nopaldb::Graph;
    use nopaldb::embeddings::EmbeddingIndex;

    #[tokio::test]
    async fn test_build_embedding_index_basic_knn() -> nopaldb::Result<()> {
        let graph = Graph::in_memory().await?;

        // Insertar 4 nodos con embeddings en el espacio R²
        let nodes_and_vecs: Vec<(&str, Vec<f32>)> = vec![
            ("alpha",  vec![1.0, 0.0]),
            ("beta",   vec![0.0, 1.0]),
            ("gamma",  vec![1.0, 1.0]),
            ("delta",  vec![0.5, 0.5]),
        ];

        let mut node_ids = vec![];
        for (name, vec) in &nodes_and_vecs {
            let node = Node::new("Item")
                .with_property("name", PropertyValue::String(name.to_string()));
            graph.add_node(node.clone()).await?;
            graph.add_node_embedding(node.id, vec.clone(), "minilm").await?;
            node_ids.push(node.id);
        }

        // Construir índice HNSW
        let index: EmbeddingIndex = graph.build_embedding_index("minilm").await?;
        assert!(!index.is_empty());
        assert_eq!(index.len(), 4);

        // Query cerca de "alpha" [1.0, 0.0]
        let results = index.search_knn(&[0.95, 0.05], 1)?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, node_ids[0], "nearest to [0.95,0.05] should be alpha");

        Ok(())
    }

    #[tokio::test]
    async fn test_build_embedding_index_no_embeddings_returns_error() -> nopaldb::Result<()> {
        let graph = Graph::in_memory().await?;
        let result = graph.build_embedding_index("nonexistent-model").await;
        assert!(result.is_err(), "should fail when no embeddings exist for model");
        Ok(())
    }

    #[tokio::test]
    async fn test_build_embedding_index_top_3() -> nopaldb::Result<()> {
        let graph = Graph::in_memory().await?;

        // 10 nodos con vectores unitarios rotados (distancia coseno).
        // Ángulo = i * 10° respecto al eje X.
        let mut ids = vec![];
        for i in 0..10_u32 {
            let angle = (i as f32) * 0.1745; // ~10 grados en radianes
            let node = Node::new("Point")
                .with_property("i", PropertyValue::Int(i as i64));
            graph.add_node(node.clone()).await?;
            graph.add_node_embedding(node.id, vec![angle.cos(), angle.sin()], "line").await?;
            ids.push(node.id);
        }

        let index = graph.build_embedding_index("line").await?;
        // Query en [1.0, 0.0] (ángulo 0°) → los 3 más cercanos son i=0, i=1, i=2
        let results = index.search_knn(&[1.0, 0.0], 3)?;
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].0, ids[0], "closest should be i=0");

        Ok(())
    }
}
