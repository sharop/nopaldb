// src/query/builder.rs

use crate::error::Result;
use crate::graph::{Direction, Graph};
use crate::query::filter::NodePredicate;
use crate::query::step::{TraversalState, TraversalStep};
use crate::types::{Node, NodeId};
use std::sync::Arc;

/// Builder para traversals fluidos
pub struct TraverseBuilder {
    graph: Arc<Graph>,
    steps: Vec<TraversalStep>,
    predicates: Vec<NodePredicate>,
    state: TraversalState,
}

impl TraverseBuilder {
    /// Crea un nuevo builder desde un nodo inicial
    pub fn new(graph: Arc<Graph>, start: NodeId) -> Self {
        Self {
            graph,
            steps: Vec::new(),
            predicates: Vec::new(),
            state: TraversalState::new(start),
        }
    }

    /// Crea un builder desde múltiples nodos
    pub fn from_nodes(graph: Arc<Graph>, nodes: Vec<NodeId>) -> Self {
        Self {
            graph,
            steps: Vec::new(),
            predicates: Vec::new(),
            state: TraversalState::with_nodes(nodes),
        }
    }

    /// Sigue aristas salientes de cualquier tipo
    pub fn out(mut self) -> Self {
        self.steps.push(TraversalStep::FollowEdge {
            edge_type: None,
            direction: Direction::Outgoing,
        });
        self
    }

    /// Sigue aristas salientes de un tipo específico
    pub fn out_e(mut self, edge_type: impl Into<String>) -> Self {
        self.steps.push(TraversalStep::FollowEdge {
            edge_type: Some(edge_type.into()),
            direction: Direction::Outgoing,
        });
        self
    }

    /// Sigue aristas entrantes de cualquier tipo
    pub fn in_(mut self) -> Self {
        self.steps.push(TraversalStep::FollowEdge {
            edge_type: None,
            direction: Direction::Incoming,
        });
        self
    }

    /// Sigue aristas entrantes de un tipo específico
    pub fn in_e(mut self, edge_type: impl Into<String>) -> Self {
        self.steps.push(TraversalStep::FollowEdge {
            edge_type: Some(edge_type.into()),
            direction: Direction::Incoming,
        });
        self
    }

    /// Sigue aristas en ambas direcciones
    pub fn both(mut self) -> Self {
        self.steps.push(TraversalStep::FollowEdge {
            edge_type: None,
            direction: Direction::Both,
        });
        self
    }

    /// Filtra nodos con un predicado custom
    pub fn filter<F>(mut self, predicate: F) -> Self
    where
        F: Fn(&Node) -> bool + Send + Sync + 'static,
    {
        self.predicates.push(Arc::new(predicate));
        self.steps.push(TraversalStep::Filter {
            predicate: format!("custom_filter_{}", self.predicates.len()),
        });
        self
    }

    /// Filtra por label
    pub fn has_label(mut self, label: impl Into<String>) -> Self {
        let label = label.into();
        let label_clone = label.clone();
        self.predicates
            .push(Arc::new(move |node: &Node| node.label == label_clone));
        self.steps.push(TraversalStep::Filter {
            predicate: format!("label:{}", label),
        });
        self
    }

    /// Limita el número de resultados
    pub fn limit(mut self, count: usize) -> Self {
        self.state.limit = Some(count);
        self.steps.push(TraversalStep::Limit { count });
        self
    }

    /// Salta N resultados
    pub fn skip(mut self, count: usize) -> Self {
        self.state.skip = count;
        self.steps.push(TraversalStep::Skip { count });
        self
    }

    /// Ejecuta el traversal y retorna los nodos
    pub async fn execute(self) -> Result<Vec<NodeId>> {
        let mut current_nodes = self.state.current_nodes.clone();
        let mut predicate_idx = 0;

        for step in &self.steps {
            match step {
                TraversalStep::FollowEdge {
                    edge_type,
                    direction,
                } => {
                    current_nodes = self
                        .follow_edges(current_nodes, edge_type.as_deref(), *direction)
                        .await?;
                }

                TraversalStep::Filter { .. } => {
                    if predicate_idx < self.predicates.len() {
                        current_nodes = self
                            .apply_filter(current_nodes, &self.predicates[predicate_idx])
                            .await?;
                        predicate_idx += 1;
                    }
                }

                TraversalStep::Limit { count } => {
                    current_nodes.truncate(*count);
                }

                TraversalStep::Skip { count } => {
                    if *count < current_nodes.len() {
                        current_nodes = current_nodes[*count..].to_vec();
                    } else {
                        current_nodes.clear();
                    }
                }
            }

            // Si no quedan nodos, terminar
            if current_nodes.is_empty() {
                break;
            }
        }

        Ok(current_nodes)
    }

    /// Ejecuta y retorna los nodos completos (no solo IDs)
    pub async fn nodes(self) -> Result<Vec<Node>> {
        let graph = self.graph.clone();
        let node_ids = self.execute().await?;

        let mut nodes = Vec::new();
        for id in node_ids {
            nodes.push(graph.get_node(id).await?);
        }

        Ok(nodes)
    }

    /// Cuenta cuántos nodos resultan del traversal
    pub async fn count(self) -> Result<usize> {
        let node_ids = self.execute().await?;
        Ok(node_ids.len())
    }

    // === Helpers privados ===

    async fn follow_edges(
        &self,
        from_nodes: Vec<NodeId>,
        edge_type: Option<&str>,
        direction: Direction,
    ) -> Result<Vec<NodeId>> {
        let mut result = Vec::new();

        for node_id in from_nodes {
            // Obtener aristas del nodo
            let edges = self.graph.edges_of(node_id, direction).await?;

            for edge in edges {
                // Filtrar por tipo si se especificó
                if let Some(et) = edge_type
                    && edge.edge_type != et
                {
                    continue;
                }

                // Agregar el nodo destino
                let target = match direction {
                    Direction::Outgoing => edge.target,
                    Direction::Incoming => edge.source,
                    Direction::Both => {
                        if edge.source == node_id {
                            edge.target
                        } else {
                            edge.source
                        }
                    }
                };

                result.push(target);
            }
        }

        Ok(result)
    }

    async fn apply_filter(
        &self,
        nodes: Vec<NodeId>,
        predicate: &NodePredicate,
    ) -> Result<Vec<NodeId>> {
        let mut result = Vec::new();

        for node_id in nodes {
            let node = self.graph.get_node(node_id).await?;
            if predicate(&node) {
                result.push(node_id);
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Edge, Node, PropertyValue};

    async fn create_social_graph() -> (Graph, NodeId) {
        let graph = Graph::in_memory().await.unwrap();

        // Alice -> Bob -> Charlie
        //   |       |
        //   v       v
        // David   Eve

        let alice = graph
            .add_node(
                Node::new("Person")
                    .with_property("name", PropertyValue::String("Alice".into()))
                    .with_property("age", PropertyValue::Int(30)),
            )
            .await
            .unwrap();

        let bob = graph
            .add_node(
                Node::new("Person")
                    .with_property("name", PropertyValue::String("Bob".into()))
                    .with_property("age", PropertyValue::Int(25)),
            )
            .await
            .unwrap();

        let charlie = graph
            .add_node(
                Node::new("Person")
                    .with_property("name", PropertyValue::String("Charlie".into()))
                    .with_property("age", PropertyValue::Int(35)),
            )
            .await
            .unwrap();

        let david = graph
            .add_node(
                Node::new("Person")
                    .with_property("name", PropertyValue::String("David".into()))
                    .with_property("age", PropertyValue::Int(20)),
            )
            .await
            .unwrap();

        let eve = graph
            .add_node(
                Node::new("Person")
                    .with_property("name", PropertyValue::String("Eve".into()))
                    .with_property("age", PropertyValue::Int(28)),
            )
            .await
            .unwrap();

        graph
            .add_edge(Edge::new(alice, bob, "KNOWS"))
            .await
            .unwrap();
        graph
            .add_edge(Edge::new(alice, david, "KNOWS"))
            .await
            .unwrap();
        graph
            .add_edge(Edge::new(bob, charlie, "KNOWS"))
            .await
            .unwrap();
        graph.add_edge(Edge::new(bob, eve, "KNOWS")).await.unwrap();

        (graph, alice)
    }

    #[tokio::test]
    async fn test_simple_traverse() {
        let (graph, alice_id) = create_social_graph().await;

        let result = graph.traverse(alice_id).out().execute().await.unwrap();

        assert_eq!(result.len(), 2); // Bob y David
    }
}
