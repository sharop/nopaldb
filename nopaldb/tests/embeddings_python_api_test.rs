// tests/embeddings_python_api_test.rs
//
// Tests de integración para la API Rust de embeddings que refleja lo expuesto en Python:
//   - add_node_embedding / get_node_embedding (Graph API)
//   - build_embedding_index + search_knn (knn_nodes equivalente)
//   - from_graph_with_embeddings (to_pyg con embeddings)
//
// Nota: los tests de PyO3 requieren cdylib (bloqueado por zstd_sys en macOS).
// Estos tests validan la lógica Rust subyacente que Python invoca.

#[cfg(feature = "embeddings")]
mod embedding_graph_api_tests {
    use nopaldb::Graph;
    use nopaldb::types::{Edge, Node, PropertyValue};

    async fn setup_graph_with_embeddings() -> Graph {
        let graph = Graph::in_memory().await.unwrap();

        let alice = Node::new("Article")
            .with_property("title", PropertyValue::String("Graph Databases".into()))
            .with_property("score", PropertyValue::Float(0.95));

        let bob = Node::new("Article")
            .with_property("title", PropertyValue::String("Machine Learning".into()))
            .with_property("score", PropertyValue::Float(0.87));

        let carol = Node::new("Article")
            .with_property("title", PropertyValue::String("Embedded Systems".into()))
            .with_property("score", PropertyValue::Float(0.72));

        for n in [&alice, &bob, &carol] {
            graph.add_node(n.clone()).await.unwrap();
        }

        graph
            .add_edge(Edge::new(alice.id, bob.id, "CITES"))
            .await
            .unwrap();

        // Embeddings: dim=3, modelo "minilm"
        graph
            .add_node_embedding(alice.id, vec![1.0, 0.0, 0.0], "minilm")
            .await
            .unwrap();
        graph
            .add_node_embedding(bob.id, vec![0.9, 0.1, 0.0], "minilm")
            .await
            .unwrap();
        graph
            .add_node_embedding(carol.id, vec![0.0, 0.0, 1.0], "minilm")
            .await
            .unwrap();

        graph
    }

    #[tokio::test]
    async fn test_add_and_get_node_embedding() {
        let graph = setup_graph_with_embeddings().await;

        let node = Node::new("Test");
        let node_id = graph.add_node(node).await.unwrap();

        graph
            .add_node_embedding(node_id, vec![0.5, 0.5, 0.5], "test-model")
            .await
            .unwrap();

        let emb = graph
            .get_node_embedding(node_id, "test-model")
            .await
            .unwrap();
        assert_eq!(emb.vector, vec![0.5, 0.5, 0.5]);
        assert_eq!(emb.model, "test-model");
    }

    #[tokio::test]
    async fn test_get_embedding_wrong_model_errors() {
        let graph = setup_graph_with_embeddings().await;
        let node = Node::new("Test");
        let node_id = graph.add_node(node).await.unwrap();
        graph
            .add_node_embedding(node_id, vec![1.0, 0.0], "modelA")
            .await
            .unwrap();

        // Modelo diferente → error
        let result = graph.get_node_embedding(node_id, "modelB").await;
        assert!(result.is_err(), "Se esperaba error para modelo inexistente");
    }

    #[cfg(feature = "embeddings-index")]
    #[tokio::test]
    async fn test_knn_nodes_returns_nearest() {
        let graph = setup_graph_with_embeddings().await;

        // Query cercano a alice [1.0, 0.0, 0.0]
        let idx = graph.build_embedding_index("minilm").await.unwrap();
        let results = idx.search_knn(&[0.95, 0.05, 0.0], 2).unwrap();

        assert_eq!(results.len(), 2, "Debe retornar 2 vecinos");
        // El más cercano debe tener distancia menor (más similar a alice)
        assert!(
            results[0].1 <= results[1].1,
            "Resultados deben estar ordenados por distancia"
        );
    }

    #[cfg(feature = "embeddings-index")]
    #[tokio::test]
    async fn test_knn_returns_at_most_k() {
        let graph = setup_graph_with_embeddings().await;

        let idx = graph.build_embedding_index("minilm").await.unwrap();
        // k=10 pero solo hay 3 nodos con embedding
        let results = idx.search_knn(&[1.0, 0.0, 0.0], 10).unwrap();
        assert!(
            results.len() <= 3,
            "No puede retornar más nodos de los que hay"
        );
        assert!(!results.is_empty(), "Debe retornar al menos 1 resultado");
    }
}

#[cfg(all(feature = "embeddings", feature = "ml"))]
mod to_pyg_with_embeddings_tests {
    use nopaldb::Graph;
    use nopaldb::ml::pyg::PyGData;
    use nopaldb::types::{Edge, Node, PropertyValue};

    async fn setup() -> Graph {
        let graph = Graph::in_memory().await.unwrap();

        let a = Node::new("Item").with_property("value", PropertyValue::Float(1.0));
        let b = Node::new("Item").with_property("value", PropertyValue::Float(2.0));
        let c = Node::new("Item").with_property("value", PropertyValue::Float(3.0));

        for n in [&a, &b, &c] {
            graph.add_node(n.clone()).await.unwrap();
        }
        graph.add_edge(Edge::new(a.id, b.id, "LINK")).await.unwrap();
        graph.add_edge(Edge::new(b.id, c.id, "LINK")).await.unwrap();

        // Solo a y b tienen embedding (c queda como ceros)
        graph
            .add_node_embedding(a.id, vec![1.0, 0.0, 0.0, 0.0], "bert")
            .await
            .unwrap();
        graph
            .add_node_embedding(b.id, vec![0.0, 1.0, 0.0, 0.0], "bert")
            .await
            .unwrap();

        graph
    }

    #[tokio::test]
    async fn test_from_graph_no_embeddings() {
        let graph = setup().await;
        let data = PyGData::from_graph(&graph, "Item", None).await.unwrap();
        assert_eq!(data.num_nodes, 3);
        assert_eq!(data.num_edges, 2);
    }

    #[tokio::test]
    async fn test_from_graph_with_embeddings_adds_tensor() {
        let graph = setup().await;
        let data = PyGData::from_graph_with_embeddings(&graph, "Item", None, Some("bert"))
            .await
            .unwrap();

        assert_eq!(data.num_nodes, 3);
        // El tensor de embeddings se agrega como último elemento de x
        let emb_tensor = data.x.last().expect("Debe haber tensor de embeddings");
        // shape: [3 nodos, 4 dims]
        assert_eq!(emb_tensor.shape, vec![3, 4]);
        // Tamaño en bytes: 3 * 4 floats * 4 bytes = 48
        assert_eq!(emb_tensor.data.len(), 3 * 4 * 4);
    }

    #[tokio::test]
    async fn test_from_graph_with_unknown_model_returns_base() {
        let graph = setup().await;
        // Modelo inexistente → ningún nodo tiene embeddings → retorna base sin tensor extra
        let base = PyGData::from_graph(&graph, "Item", None).await.unwrap();
        let with_emb =
            PyGData::from_graph_with_embeddings(&graph, "Item", None, Some("nonexistent"))
                .await
                .unwrap();

        // Mismo número de tensores en x (no se añade tensor vacío)
        assert_eq!(
            base.x.len(),
            with_emb.x.len(),
            "Con modelo inexistente no se debe agregar tensor de embeddings"
        );
    }

    #[tokio::test]
    async fn test_from_graph_no_embedding_model_returns_base() {
        let graph = setup().await;
        let base = PyGData::from_graph(&graph, "Item", None).await.unwrap();
        let same = PyGData::from_graph_with_embeddings(&graph, "Item", None, None)
            .await
            .unwrap();

        assert_eq!(base.x.len(), same.x.len());
        assert_eq!(base.num_nodes, same.num_nodes);
    }
}
