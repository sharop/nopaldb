// src/traversal/mod.rs

use crate::graph::Direction;
use crate::types::{Node, NodeId};

/// Resultado de un traversal
#[derive(Debug, Clone)]
pub struct TraversalResult {
    /// Nodos visitados en orden
    pub nodes: Vec<NodeId>,
    /// Distancias desde el nodo inicial (para BFS)
    pub distances: Option<Vec<usize>>,
    /// Camino específico (si se buscaba uno)
    pub path: Option<Vec<NodeId>>,
}

/// Condición para filtrar nodos durante el traversal
pub type NodeFilter = Box<dyn Fn(&Node) -> bool + Send + Sync>;

/// Configuración de traversal
pub struct TraversalConfig {
    /// Dirección de las aristas a seguir
    pub direction: Direction,
    /// Máxima profundidad (None = ilimitado)
    pub max_depth: Option<usize>,
    /// Filtro de nodos (None = todos)
    pub filter: Option<NodeFilter>,
    /// Máximo de nodos a visitar (previene loops infinitos)
    pub max_nodes: Option<usize>,
}

impl Default for TraversalConfig {
    fn default() -> Self {
        Self {
            direction: Direction::Outgoing,
            max_depth: None,
            filter: None,
            max_nodes: Some(10_000), // Límite de seguridad
        }
    }
}

impl TraversalConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn direction(mut self, direction: Direction) -> Self {
        self.direction = direction;
        self
    }

    pub fn max_depth(mut self, depth: usize) -> Self {
        self.max_depth = Some(depth);
        self
    }

    pub fn max_nodes(mut self, max: usize) -> Self {
        self.max_nodes = Some(max);
        self
    }

    pub fn filter<F>(mut self, f: F) -> Self
    where
        F: Fn(&Node) -> bool + Send + Sync + 'static,
    {
        self.filter = Some(Box::new(f));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Graph;
    use crate::types::{Edge, Node, PropertyValue};

    // Helper para crear grafo de prueba
    async fn create_test_graph() -> (Graph, NodeId) {
        let graph = Graph::in_memory().await.unwrap();

        // Crear nodos: A -> B -> C
        //              |    |
        //              v    v
        //              D -> E

        let a = graph
            .add_node(Node::new("Node").with_property("name", PropertyValue::String("A".into())))
            .await
            .unwrap();

        let b = graph
            .add_node(Node::new("Node").with_property("name", PropertyValue::String("B".into())))
            .await
            .unwrap();

        let c = graph
            .add_node(Node::new("Node").with_property("name", PropertyValue::String("C".into())))
            .await
            .unwrap();

        let d = graph
            .add_node(Node::new("Node").with_property("name", PropertyValue::String("D".into())))
            .await
            .unwrap();

        let e = graph
            .add_node(Node::new("Node").with_property("name", PropertyValue::String("E".into())))
            .await
            .unwrap();

        graph.add_edge(Edge::new(a, b, "CONNECTS")).await.unwrap();
        graph.add_edge(Edge::new(a, d, "CONNECTS")).await.unwrap();
        graph.add_edge(Edge::new(b, c, "CONNECTS")).await.unwrap();
        graph.add_edge(Edge::new(b, e, "CONNECTS")).await.unwrap();
        graph.add_edge(Edge::new(d, e, "CONNECTS")).await.unwrap();

        (graph, a)
    }

    #[tokio::test]
    async fn test_bfs() {
        let (graph, start) = create_test_graph().await;

        let result = graph.bfs(start, TraversalConfig::new()).await.unwrap();

        // BFS debe visitar en orden de nivel
        assert_eq!(result.nodes.len(), 5);
        // A está primero
        assert_eq!(result.nodes[0], start);
    }

    #[tokio::test]
    async fn test_bfs_with_max_depth() {
        let (graph, start) = create_test_graph().await;

        let result = graph
            .bfs(start, TraversalConfig::new().max_depth(1))
            .await
            .unwrap();

        // Solo debe visitar A, B, D (profundidad 0 y 1)
        assert!(result.nodes.len() <= 3);
    }
}
