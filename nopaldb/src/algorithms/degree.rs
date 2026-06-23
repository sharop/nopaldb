// src/algorithms/degree.rs
//
// Degree Centrality algorithm implementation

use crate::error::Result;
use crate::graph::GraphView;
use crate::types::NodeId;
use std::collections::HashMap;

/// Degree type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegreeType {
    /// Total degree (in + out)
    Total,
    /// In-degree (incoming edges)
    In,
    /// Out-degree (outgoing edges)
    Out,
}

/// Degree Centrality configuration
#[derive(Debug, Clone)]
pub struct DegreeConfig {
    /// Type of degree to compute
    pub degree_type: DegreeType,

    /// Normalize by (n-1) where n is number of nodes
    pub normalize: bool,
}

impl Default for DegreeConfig {
    fn default() -> Self {
        DegreeConfig {
            degree_type: DegreeType::Total,
            normalize: false,
        }
    }
}

/// Degree Centrality algorithm
pub struct DegreeCentrality {
    config: DegreeConfig,
}

impl DegreeCentrality {
    /// Create new Degree Centrality instance
    pub fn new(config: DegreeConfig) -> Self {
        DegreeCentrality { config }
    }

    /// Create with default configuration
    pub fn with_defaults() -> Self {
        DegreeCentrality {
            config: DegreeConfig::default(),
        }
    }

    /// Compute degree centrality for all nodes in the view
    pub async fn compute<G: GraphView>(&self, graph: &G) -> Result<HashMap<NodeId, f64>> {
        let nodes = graph.get_all_nodes().await?;
        if nodes.is_empty() {
            return Ok(HashMap::new());
        }
        let edges = graph.get_all_edges().await?;
        let config = self.config.clone();
        tokio::task::spawn_blocking(move || Self::compute_cpu(nodes, edges, config))
            .await
            .map_err(|e| crate::error::NopalError::custom(format!("degree join error: {e}")))?
    }

    fn compute_cpu(
        nodes: Vec<crate::types::Node>,
        edges: Vec<crate::types::Edge>,
        config: DegreeConfig,
    ) -> Result<HashMap<NodeId, f64>> {
        let n = nodes.len();
        let mut degrees: HashMap<NodeId, f64> = nodes.iter().map(|node| (node.id, 0.0)).collect();

        match config.degree_type {
            DegreeType::Total => {
                for edge in &edges {
                    *degrees.entry(edge.source).or_insert(0.0) += 1.0;
                    *degrees.entry(edge.target).or_insert(0.0) += 1.0;
                }
            }
            DegreeType::In => {
                for edge in &edges {
                    *degrees.entry(edge.target).or_insert(0.0) += 1.0;
                }
            }
            DegreeType::Out => {
                for edge in &edges {
                    *degrees.entry(edge.source).or_insert(0.0) += 1.0;
                }
            }
        }

        if config.normalize && n > 1 {
            let normalization = (n - 1) as f64;
            for value in degrees.values_mut() {
                *value /= normalization;
            }
        }

        Ok(degrees)
    }

    /// Compute in-degree centrality within the view
    pub async fn compute_in_degree<G: GraphView>(&self, graph: &G) -> Result<HashMap<NodeId, f64>> {
        let mut config = self.config.clone();
        config.degree_type = DegreeType::In;
        let dc = DegreeCentrality::new(config);
        dc.compute(graph).await
    }

    /// Compute out-degree centrality within the view
    pub async fn compute_out_degree<G: GraphView>(
        &self,
        graph: &G,
    ) -> Result<HashMap<NodeId, f64>> {
        let mut config = self.config.clone();
        config.degree_type = DegreeType::Out;
        let dc = DegreeCentrality::new(config);
        dc.compute(graph).await
    }

    /// Compute degree centrality for a subset of nodes within the view
    pub async fn compute_for_nodes<G: GraphView>(
        &self,
        graph: &G,
        node_ids: &[NodeId],
    ) -> Result<HashMap<NodeId, f64>> {
        let all_degrees = self.compute(graph).await?;

        Ok(node_ids
            .iter()
            .filter_map(|id| all_degrees.get(id).map(|&deg| (*id, deg)))
            .collect())
    }

    /// Get statistics about degrees in the graph view
    pub async fn compute_stats<G: GraphView>(&self, graph: &G) -> Result<DegreeStats> {
        let degrees = self.compute(graph).await?;

        if degrees.is_empty() {
            return Ok(DegreeStats {
                min: 0.0,
                max: 0.0,
                mean: 0.0,
                median: 0.0,
                total_edges: 0,
            });
        }

        let mut values: Vec<f64> = degrees.values().copied().collect();
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let min = values[0];
        let max = values[values.len() - 1];
        let sum: f64 = values.iter().sum();
        let mean = sum / values.len() as f64;

        let median = if values.len().is_multiple_of(2) {
            let mid = values.len() / 2;
            (values[mid - 1] + values[mid]) / 2.0
        } else {
            values[values.len() / 2]
        };

        // Total edges (for undirected, divide total degree by 2)
        let total_edges = match self.config.degree_type {
            DegreeType::Total => (sum / 2.0) as usize,
            DegreeType::In | DegreeType::Out => sum as usize,
        };

        Ok(DegreeStats {
            min,
            max,
            mean,
            median,
            total_edges,
        })
    }
}

/// Statistics about degree distribution
#[derive(Debug, Clone)]
pub struct DegreeStats {
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub median: f64,
    pub total_edges: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Graph;
    use crate::types::{Edge, Node};
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_degree_basic() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();

        // Create star: center with degree 3, periphery with degree 1 each
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

        // Compute degree
        let dc = DegreeCentrality::with_defaults();
        let degrees = dc.compute(&graph).await.unwrap();

        // Center should have total degree = 3
        assert_eq!(degrees[&center], 3.0);

        // Periphery nodes should have total degree = 1
        for p in &periphery {
            assert_eq!(degrees[p], 1.0);
        }

        // Test out-degree
        let out_degrees = dc.compute_out_degree(&graph).await.unwrap();
        assert_eq!(out_degrees[&center], 3.0); // Center has 3 outgoing edges
        for p in &periphery {
            assert_eq!(out_degrees[p], 0.0); // Periphery has no outgoing edges
        }

        // Test in-degree
        let in_degrees = dc.compute_in_degree(&graph).await.unwrap();
        assert_eq!(in_degrees[&center], 0.0); // Center has no incoming edges
        for p in &periphery {
            assert_eq!(in_degrees[p], 1.0); // Each periphery has 1 incoming edge
        }
    }

    #[tokio::test]
    async fn test_degree_normalized() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();

        // Create complete graph with 4 nodes
        let mut nodes = Vec::new();
        for i in 0..4 {
            let node = tx
                .add_node(Node {
                    id: uuid::Uuid::new_v4(),
                    label: format!("Node{}", i),
                    properties: HashMap::new(),
                    kind: Default::default(),
                })
                .await
                .unwrap();
            nodes.push(node);
        }

        // Connect all pairs
        for i in 0..nodes.len() {
            for j in (i + 1)..nodes.len() {
                tx.add_edge(Edge {
                    id: uuid::Uuid::new_v4(),
                    source: nodes[i],
                    target: nodes[j],
                    edge_type: "CONNECTS".to_string(),
                    properties: HashMap::new(),
                })
                .unwrap();
            }
        }

        tx.commit().await.unwrap();

        // Compute normalized degree
        let config = DegreeConfig {
            degree_type: DegreeType::Total,
            normalize: true,
        };
        let dc = DegreeCentrality::new(config);
        let degrees = dc.compute(&graph).await.unwrap();

        // In a complete graph, each node connects to all others
        // Normalized degree should be 1.0 for all nodes
        for node in &nodes {
            assert!((degrees[node] - 1.0).abs() < 1e-6);
        }
    }

    #[tokio::test]
    async fn test_degree_stats() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();

        // Create varied network
        let a = tx
            .add_node(Node {
                id: uuid::Uuid::new_v4(),
                label: "A".to_string(),
                properties: HashMap::new(),
                kind: Default::default(),
            })
            .await
            .unwrap();

        let b = tx
            .add_node(Node {
                id: uuid::Uuid::new_v4(),
                label: "B".to_string(),
                properties: HashMap::new(),
                kind: Default::default(),
            })
            .await
            .unwrap();

        let _c = tx
            .add_node(Node {
                id: uuid::Uuid::new_v4(),
                label: "C".to_string(),
                properties: HashMap::new(),
                kind: Default::default(),
            })
            .await
            .unwrap();

        // A -- B (both have degree 1), C isolated (degree 0)
        tx.add_edge(Edge {
            id: uuid::Uuid::new_v4(),
            source: a,
            target: b,
            edge_type: "CONNECTS".to_string(),
            properties: HashMap::new(),
        })
        .unwrap();

        tx.commit().await.unwrap();

        let dc = DegreeCentrality::with_defaults();
        let stats = dc.compute_stats(&graph).await.unwrap();

        assert_eq!(stats.min, 0.0);
        assert_eq!(stats.max, 1.0); // Changed from 2.0
        assert_eq!(stats.total_edges, 1);
    }
}
