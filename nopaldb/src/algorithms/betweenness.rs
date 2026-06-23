// src/algorithms/betweenness.rs
//
// Betweenness Centrality algorithm implementation

use crate::error::Result;
use crate::graph::GraphView;
use crate::types::NodeId;
use std::collections::{HashMap, VecDeque};

/// Betweenness Centrality configuration
#[derive(Debug, Clone)]
pub struct BetweennessConfig {
    /// Normalize scores (divide by (n-1)(n-2)/2)
    pub normalize: bool,

    /// Use parallel computation
    pub parallel: bool,
}

impl Default for BetweennessConfig {
    fn default() -> Self {
        BetweennessConfig {
            normalize: true,
            parallel: false,
        }
    }
}

/// Betweenness Centrality algorithm
pub struct BetweennessCentrality {
    config: BetweennessConfig,
}

impl BetweennessCentrality {
    /// Create new Betweenness Centrality instance
    pub fn new(config: BetweennessConfig) -> Self {
        BetweennessCentrality { config }
    }

    /// Create with default configuration
    pub fn with_defaults() -> Self {
        BetweennessCentrality {
            config: BetweennessConfig::default(),
        }
    }

    /// Compute Betweenness Centrality for all nodes in the view
    /// Uses Brandes' algorithm (O(VE) for unweighted graphs)
    pub async fn compute<G: GraphView>(&self, graph: &G) -> Result<HashMap<NodeId, f64>> {
        let nodes = graph.get_all_nodes().await?;
        if nodes.is_empty() {
            return Ok(HashMap::new());
        }
        let edges = graph.get_all_edges().await?;
        let config = self.config.clone();
        tokio::task::spawn_blocking(move || Self::compute_cpu(nodes, edges, config))
            .await
            .map_err(|e| crate::error::NopalError::custom(format!("betweenness join error: {e}")))?
    }

    fn compute_cpu(
        nodes: Vec<crate::types::Node>,
        edges: Vec<crate::types::Edge>,
        config: BetweennessConfig,
    ) -> Result<HashMap<NodeId, f64>> {
        let n = nodes.len();
        let mut betweenness: HashMap<NodeId, f64> =
            nodes.iter().map(|node| (node.id, 0.0)).collect();

        let mut adjacency: HashMap<NodeId, Vec<NodeId>> = HashMap::new();

        for edge in &edges {
            adjacency.entry(edge.source).or_default().push(edge.target);
            // For undirected graph, add reverse edge
            adjacency.entry(edge.target).or_default().push(edge.source);
        }

        for source_node in &nodes {
            let source = source_node.id;

            let mut stack = Vec::new();
            let mut predecessors: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
            let mut sigma: HashMap<NodeId, f64> = HashMap::new();
            let mut distance: HashMap<NodeId, i32> = HashMap::new();

            for node in &nodes {
                sigma.insert(node.id, 0.0);
                distance.insert(node.id, -1);
                predecessors.insert(node.id, Vec::new());
            }

            sigma.insert(source, 1.0);
            distance.insert(source, 0);

            let mut queue = VecDeque::new();
            queue.push_back(source);

            while let Some(v) = queue.pop_front() {
                stack.push(v);

                if let Some(neighbors) = adjacency.get(&v) {
                    for &w in neighbors {
                        if distance[&w] < 0 {
                            queue.push_back(w);
                            distance.insert(w, distance[&v] + 1);
                        }

                        if distance[&w] == distance[&v] + 1 {
                            let sigma_w = sigma[&w] + sigma[&v];
                            sigma.insert(w, sigma_w);
                            if let Some(preds) = predecessors.get_mut(&w) {
                                preds.push(v);
                            }
                        }
                    }
                }
            }

            let mut delta: HashMap<NodeId, f64> = nodes.iter().map(|node| (node.id, 0.0)).collect();

            while let Some(w) = stack.pop() {
                if let Some(preds) = predecessors.get(&w) {
                    for &v in preds {
                        let contribution = (sigma[&v] / sigma[&w]) * (1.0 + delta[&w]);
                        delta.insert(v, delta[&v] + contribution);
                    }
                }

                if w != source {
                    betweenness.insert(w, betweenness[&w] + delta[&w]);
                }
            }
        }

        if config.normalize && n > 2 {
            let normalization = ((n - 1) * (n - 2)) as f64;
            for value in betweenness.values_mut() {
                *value /= normalization;
            }
        }

        Ok(betweenness)
    }

    /// Compute Betweenness Centrality for a subset of nodes within the view
    pub async fn compute_for_nodes<G: GraphView>(
        &self,
        graph: &G,
        node_ids: &[NodeId],
    ) -> Result<HashMap<NodeId, f64>> {
        // For now, compute for all and filter
        // TODO: Optimize to only compute for subset
        let all_betweenness = self.compute(graph).await?;

        Ok(node_ids
            .iter()
            .filter_map(|id| all_betweenness.get(id).map(|&bc| (*id, bc)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Graph;
    use crate::types::{Edge, Node};
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_betweenness_basic() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();

        // Create linear graph: A -- B -- C
        // B should have highest betweenness (it's the bridge)
        let a = tx
            .add_node(Node {
                id: uuid::Uuid::new_v4(),
                label: "Node".to_string(),
                properties: HashMap::new(),
                kind: Default::default(),
            })
            .await
            .unwrap();

        let b = tx
            .add_node(Node {
                id: uuid::Uuid::new_v4(),
                label: "Node".to_string(),
                properties: HashMap::new(),
                kind: Default::default(),
            })
            .await
            .unwrap();

        let c = tx
            .add_node(Node {
                id: uuid::Uuid::new_v4(),
                label: "Node".to_string(),
                properties: HashMap::new(),
                kind: Default::default(),
            })
            .await
            .unwrap();

        // A -- B
        tx.add_edge(Edge {
            id: uuid::Uuid::new_v4(),
            source: a,
            target: b,
            edge_type: "CONNECTS".to_string(),
            properties: HashMap::new(),
        })
        .unwrap();

        // B -- C
        tx.add_edge(Edge {
            id: uuid::Uuid::new_v4(),
            source: b,
            target: c,
            edge_type: "CONNECTS".to_string(),
            properties: HashMap::new(),
        })
        .unwrap();

        tx.commit().await.unwrap();

        // Compute Betweenness
        let bc = BetweennessCentrality::with_defaults();
        let scores = bc.compute(&graph).await.unwrap();

        assert_eq!(scores.len(), 3);

        // B should have highest betweenness (it's on all shortest paths between A and C)
        assert!(scores[&b] > scores[&a]);
        assert!(scores[&b] > scores[&c]);

        // A and C should have 0 betweenness (they're not on any shortest paths)
        assert_eq!(scores[&a], 0.0);
        assert_eq!(scores[&c], 0.0);
    }

    #[tokio::test]
    async fn test_betweenness_star() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();

        // Create star graph: center connected to 3 periphery nodes
        // Center should have betweenness = 1.0 (normalized)
        let center = tx
            .add_node(Node {
                id: uuid::Uuid::new_v4(),
                label: "Center".to_string(),
                properties: HashMap::new(),
                kind: Default::default(),
            })
            .await
            .unwrap();

        let mut periphery = Vec::new();
        for i in 0..3 {
            let p = tx
                .add_node(Node {
                    id: uuid::Uuid::new_v4(),
                    label: format!("P{}", i),
                    properties: HashMap::new(),
                    kind: Default::default(),
                })
                .await
                .unwrap();

            tx.add_edge(Edge {
                id: uuid::Uuid::new_v4(),
                source: center,
                target: p,
                edge_type: "CONNECTS".to_string(),
                properties: HashMap::new(),
            })
            .unwrap();

            periphery.push(p);
        }

        tx.commit().await.unwrap();

        // Compute Betweenness
        let bc = BetweennessCentrality::with_defaults();
        let scores = bc.compute(&graph).await.unwrap();

        // Center should have highest betweenness
        assert!(scores[&center] > 0.0);

        // All periphery nodes should have 0 betweenness
        for p in periphery {
            assert_eq!(scores[&p], 0.0);
        }
    }
}
