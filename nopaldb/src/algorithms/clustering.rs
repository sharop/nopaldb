// src/algorithms/clustering.rs
//
// Clustering Coefficient algorithm implementation

use crate::error::Result;
use crate::graph::GraphView;
use crate::types::NodeId;
use std::collections::{HashMap, HashSet};

/// Clustering Coefficient configuration
#[derive(Debug, Clone, Default)]
pub struct ClusteringConfig {
    /// Use weighted version (if edge weights are available)
    pub weighted: bool,
}

/// Clustering Coefficient algorithm
pub struct ClusteringCoefficient {
    _config: ClusteringConfig,
}

impl ClusteringCoefficient {
    /// Create new Clustering Coefficient instance
    pub fn new(config: ClusteringConfig) -> Self {
        ClusteringCoefficient { _config : config }
    }

    /// Create with default configuration
    pub fn with_defaults() -> Self {
        ClusteringCoefficient {
            _config: ClusteringConfig::default(),
        }
    }

    /// Compute local clustering coefficient for all nodes in the view
    pub async fn compute<G: GraphView>(&self, graph: &G) -> Result<HashMap<NodeId, f64>> {
        let nodes = graph.get_all_nodes().await?;
        if nodes.is_empty() {
            return Ok(HashMap::new());
        }
        let edges = graph.get_all_edges().await?;
        tokio::task::spawn_blocking(move || Self::compute_cpu(nodes, edges))
            .await
            .map_err(|e| crate::error::NopalError::custom(format!("clustering join error: {e}")))?
    }

    fn compute_cpu(
        nodes: Vec<crate::types::Node>,
        edges: Vec<crate::types::Edge>,
    ) -> Result<HashMap<NodeId, f64>> {
        let mut clustering: HashMap<NodeId, f64> = HashMap::new();

        let mut adjacency: HashMap<NodeId, HashSet<NodeId>> = HashMap::new();

        for edge in &edges {
            adjacency.entry(edge.source).or_default().insert(edge.target);
            adjacency.entry(edge.target).or_default().insert(edge.source);
        }

        for node in &nodes {
            let node_id = node.id;

            if let Some(neighbors) = adjacency.get(&node_id) {
                let k = neighbors.len();

                if k < 2 {
                    clustering.insert(node_id, 0.0);
                    continue;
                }

                let mut triangles = 0;
                let neighbors_vec: Vec<_> = neighbors.iter().copied().collect();

                for i in 0..neighbors_vec.len() {
                    for j in (i + 1)..neighbors_vec.len() {
                        let n1 = neighbors_vec[i];
                        let n2 = neighbors_vec[j];

                        if let Some(n1_neighbors) = adjacency.get(&n1)
                            && n1_neighbors.contains(&n2)
                        {
                            triangles += 1;
                        }
                    }
                }

                let max_triangles = k * (k - 1) / 2;
                let coefficient = if max_triangles > 0 {
                    triangles as f64 / max_triangles as f64
                } else {
                    0.0
                };

                clustering.insert(node_id, coefficient);
            } else {
                clustering.insert(node_id, 0.0);
            }
        }

        Ok(clustering)
    }

    /// Compute average clustering coefficient for the graph view
    pub async fn compute_average<G: GraphView>(&self, graph: &G) -> Result<f64> {
        let coefficients = self.compute(graph).await?;

        if coefficients.is_empty() {
            return Ok(0.0);
        }

        let sum: f64 = coefficients.values().sum();
        Ok(sum / coefficients.len() as f64)
    }

    /// Compute clustering coefficient for a subset of nodes within the view
    pub async fn compute_for_nodes<G: GraphView>(
        &self,
        graph: &G,
        node_ids: &[NodeId],
    ) -> Result<HashMap<NodeId, f64>> {
        let all_clustering = self.compute(graph).await?;

        Ok(node_ids
            .iter()
            .filter_map(|id| all_clustering.get(id).map(|&cc| (*id, cc)))
            .collect())
    }

    /// Compute global clustering coefficient (transitivity)
    /// This is different from average local clustering:
    /// Global = 3 * (number of triangles) / (number of connected triples)
    pub async fn compute_global<G: GraphView>(&self, graph: &G) -> Result<f64> {
        let nodes = graph.get_all_nodes().await?;
        if nodes.is_empty() {
            return Ok(0.0);
        }
        let edges = graph.get_all_edges().await?;
        tokio::task::spawn_blocking(move || Self::compute_global_cpu(nodes, edges))
            .await
            .map_err(|e| crate::error::NopalError::custom(format!("clustering global join error: {e}")))?
    }

    fn compute_global_cpu(
        nodes: Vec<crate::types::Node>,
        edges: Vec<crate::types::Edge>,
    ) -> Result<f64> {
        let mut adjacency: HashMap<NodeId, HashSet<NodeId>> = HashMap::new();

        for edge in &edges {
            adjacency.entry(edge.source).or_default().insert(edge.target);
            adjacency.entry(edge.target).or_default().insert(edge.source);
        }

        let mut triangles = 0usize;
        let mut triples = 0usize;

        for node in &nodes {
            let node_id = node.id;

            if let Some(neighbors) = adjacency.get(&node_id) {
                let k = neighbors.len();

                if k < 2 {
                    continue;
                }

                triples += k * (k - 1) / 2;

                let neighbors_vec: Vec<_> = neighbors.iter().copied().collect();
                for i in 0..neighbors_vec.len() {
                    for j in (i + 1)..neighbors_vec.len() {
                        let n1 = neighbors_vec[i];
                        let n2 = neighbors_vec[j];

                        if let Some(n1_neighbors) = adjacency.get(&n1)
                            && n1_neighbors.contains(&n2)
                        {
                            triangles += 1;
                        }
                    }
                }
            }
        }

        // Each triangle is counted 3 times (once for each vertex)
        let actual_triangles = triangles / 3;

        if triples == 0 {
            Ok(0.0)
        } else {
            Ok(3.0 * actual_triangles as f64 / triples as f64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Graph;
    use crate::types::{Node, Edge};
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_clustering_triangle() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();

        // Create complete triangle: A -- B -- C -- A
        // All nodes should have clustering coefficient = 1.0
        let a = tx.add_node(Node {
            id: uuid::Uuid::new_v4(),
            label: "Node".to_string(),
            properties: HashMap::new(),
            kind: Default::default(),
        }).await.unwrap();

        let b = tx.add_node(Node {
            id: uuid::Uuid::new_v4(),
            label: "Node".to_string(),
            properties: HashMap::new(),
            kind: Default::default(),
        }).await.unwrap();

        let c = tx.add_node(Node {
            id: uuid::Uuid::new_v4(),
            label: "Node".to_string(),
            properties: HashMap::new(),
            kind: Default::default(),
        }).await.unwrap();

        // Create triangle
        tx.add_edge(Edge {
            id: uuid::Uuid::new_v4(),
            source: a,
            target: b,
            edge_type: "CONNECTS".to_string(),
            properties: HashMap::new(),
        }).unwrap();

        tx.add_edge(Edge {
            id: uuid::Uuid::new_v4(),
            source: b,
            target: c,
            edge_type: "CONNECTS".to_string(),
            properties: HashMap::new(),
        }).unwrap();

        tx.add_edge(Edge {
            id: uuid::Uuid::new_v4(),
            source: c,
            target: a,
            edge_type: "CONNECTS".to_string(),
            properties: HashMap::new(),
        }).unwrap();

        tx.commit().await.unwrap();

        // Compute clustering
        let cc = ClusteringCoefficient::with_defaults();
        let coefficients = cc.compute(&graph).await.unwrap();

        assert_eq!(coefficients.len(), 3);

        // All nodes in a complete triangle should have coefficient = 1.0
        assert!((coefficients[&a] - 1.0).abs() < 1e-6);
        assert!((coefficients[&b] - 1.0).abs() < 1e-6);
        assert!((coefficients[&c] - 1.0).abs() < 1e-6);

        // Average should also be 1.0
        let avg = cc.compute_average(&graph).await.unwrap();
        assert!((avg - 1.0).abs() < 1e-6);
    }

    #[tokio::test]
    async fn test_clustering_star() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();

        // Create star: center connected to 3 periphery nodes (no edges between periphery)
        // Center should have clustering = 0 (neighbors not connected)
        // Periphery nodes should have clustering = 0 (only 1 neighbor each)
        let center = tx.add_node(Node {
            id: uuid::Uuid::new_v4(),
            label: "Center".to_string(),
            properties: HashMap::new(),
            kind: Default::default(),
        }).await.unwrap();

        let mut periphery = Vec::new();
        for i in 0..3 {
            let p = tx.add_node(Node {
                id: uuid::Uuid::new_v4(),
                label: format!("P{}", i),
                properties: HashMap::new(),
                kind: Default::default(),
            }).await.unwrap();

            tx.add_edge(Edge {
                id: uuid::Uuid::new_v4(),
                source: center,
                target: p,
                edge_type: "CONNECTS".to_string(),
                properties: HashMap::new(),
            }).unwrap();

            periphery.push(p);
        }

        tx.commit().await.unwrap();

        // Compute clustering
        let cc = ClusteringCoefficient::with_defaults();
        let coefficients = cc.compute(&graph).await.unwrap();

        // Center has 3 neighbors but they're not connected -> coefficient = 0
        assert_eq!(coefficients[&center], 0.0);

        // Periphery nodes have only 1 neighbor each -> coefficient = 0
        for p in periphery {
            assert_eq!(coefficients[&p], 0.0);
        }
    }
}