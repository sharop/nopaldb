// src/algorithms/pagerank.rs
//
// PageRank algorithm implementation

use crate::error::Result;
use crate::graph::GraphView;
use crate::types::NodeId;
use std::collections::HashMap;

//#[cfg(feature = "parallel")]
//use rayon::prelude::*;

/// PageRank configuration
#[derive(Debug, Clone)]
pub struct PageRankConfig {
    /// Damping factor (typically 0.85)
    pub damping: f64,

    /// Maximum number of iterations
    pub max_iterations: usize,

    /// Convergence tolerance
    pub tolerance: f64,

    /// Use parallel computation
    pub parallel: bool,
}

impl Default for PageRankConfig {
    fn default() -> Self {
        PageRankConfig {
            damping: 0.85,
            max_iterations: 100,
            tolerance: 1e-6,
            parallel: true,
        }
    }
}

/// PageRank algorithm
pub struct PageRank {
    config: PageRankConfig,
}

impl PageRank {
    /// Create new PageRank instance
    pub fn new(config: PageRankConfig) -> Self {
        PageRank { config }
    }

    /// Create with default configuration
    pub fn with_defaults() -> Self {
        PageRank {
            config: PageRankConfig::default(),
        }
    }

    /// Compute PageRank for all nodes in the view
    pub async fn compute<G: GraphView>(&self, graph: &G) -> Result<HashMap<NodeId, f64>> {
        let nodes = graph.get_all_nodes().await?;
        if nodes.is_empty() {
            return Ok(HashMap::new());
        }
        let edges = graph.get_all_edges().await?;
        let config = self.config.clone();
        tokio::task::spawn_blocking(move || Self::compute_cpu(nodes, edges, config))
            .await
            .map_err(|e| crate::error::NopalError::custom(format!("pagerank join error: {e}")))?
    }

    fn compute_cpu(
        nodes: Vec<crate::types::Node>,
        edges: Vec<crate::types::Edge>,
        config: PageRankConfig,
    ) -> Result<HashMap<NodeId, f64>> {
        let n = nodes.len();
        let initial_rank = 1.0 / n as f64;

        let mut ranks: HashMap<NodeId, f64> =
            nodes.iter().map(|node| (node.id, initial_rank)).collect();

        let mut out_degree: HashMap<NodeId, usize> = HashMap::new();
        for edge in &edges {
            *out_degree.entry(edge.source).or_insert(0) += 1;
        }

        let mut incoming: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for edge in &edges {
            incoming.entry(edge.target).or_default().push(edge.source);
        }

        for iteration in 0..config.max_iterations {
            let mut new_ranks = HashMap::new();
            let mut delta = 0.0;

            for node in &nodes {
                let node_id = node.id;

                let mut rank_sum = 0.0;
                if let Some(sources) = incoming.get(&node_id) {
                    for &source_id in sources {
                        let source_rank = ranks[&source_id];
                        let source_out_degree = *out_degree.get(&source_id).unwrap_or(&1);
                        rank_sum += source_rank / source_out_degree as f64;
                    }
                }

                let new_rank = (1.0 - config.damping) / n as f64 + config.damping * rank_sum;

                let old_rank = ranks[&node_id];
                delta += (new_rank - old_rank).abs();

                new_ranks.insert(node_id, new_rank);
            }

            ranks = new_ranks;

            if delta < config.tolerance {
                log::info!("PageRank converged after {} iterations", iteration + 1);
                break;
            }
        }

        Ok(ranks)
    }

    /// Compute PageRank for a subset of nodes within the given view
    pub async fn compute_for_nodes<G: GraphView>(
        &self,
        graph: &G,
        node_ids: &[NodeId],
    ) -> Result<HashMap<NodeId, f64>> {
        // For now, compute for all and filter
        // TODO: Optimize to only compute subgraph
        let all_ranks = self.compute(graph).await?;

        Ok(node_ids
            .iter()
            .filter_map(|id| all_ranks.get(id).map(|&rank| (*id, rank)))
            .collect())
    }

    /// Personalized PageRank (random walk from specific nodes) within the given view
    pub async fn personalized<G: GraphView>(
        &self,
        graph: &G,
        source_nodes: &[NodeId],
    ) -> Result<HashMap<NodeId, f64>> {
        let nodes = graph.get_all_nodes().await?;
        if nodes.is_empty() {
            return Ok(HashMap::new());
        }
        let edges = graph.get_all_edges().await?;
        let config = self.config.clone();
        let source_nodes: Vec<NodeId> = source_nodes.to_vec();
        tokio::task::spawn_blocking(move || {
            Self::personalized_cpu(nodes, edges, source_nodes, config)
        })
        .await
        .map_err(|e| {
            crate::error::NopalError::custom(format!("personalized pagerank join error: {e}"))
        })?
    }

    fn personalized_cpu(
        nodes: Vec<crate::types::Node>,
        edges: Vec<crate::types::Edge>,
        source_nodes: Vec<NodeId>,
        config: PageRankConfig,
    ) -> Result<HashMap<NodeId, f64>> {
        let source_set: std::collections::HashSet<_> = source_nodes.iter().copied().collect();

        let mut ranks: HashMap<NodeId, f64> = nodes
            .iter()
            .map(|node| {
                let rank = if source_set.contains(&node.id) {
                    1.0 / source_nodes.len() as f64
                } else {
                    0.0
                };
                (node.id, rank)
            })
            .collect();

        let mut out_degree: HashMap<NodeId, usize> = HashMap::new();
        for edge in &edges {
            *out_degree.entry(edge.source).or_insert(0) += 1;
        }

        let mut incoming: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for edge in &edges {
            incoming.entry(edge.target).or_default().push(edge.source);
        }

        for _ in 0..config.max_iterations {
            let mut new_ranks = HashMap::new();
            let mut delta = 0.0;

            for node in &nodes {
                let node_id = node.id;

                let mut rank_sum = 0.0;
                if let Some(sources) = incoming.get(&node_id) {
                    for &source_id in sources {
                        let source_rank = ranks[&source_id];
                        let source_out_degree = *out_degree.get(&source_id).unwrap_or(&1);
                        rank_sum += source_rank / source_out_degree as f64;
                    }
                }

                // Personalized: teleport back to source nodes
                let teleport = if source_set.contains(&node_id) {
                    (1.0 - config.damping) / source_nodes.len() as f64
                } else {
                    0.0
                };

                let new_rank = teleport + config.damping * rank_sum;

                let old_rank = ranks[&node_id];
                delta += (new_rank - old_rank).abs();

                new_ranks.insert(node_id, new_rank);
            }

            ranks = new_ranks;

            if delta < config.tolerance {
                break;
            }
        }

        Ok(ranks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Graph;
    use crate::types::{Edge, Node};
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_pagerank_basic() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();

        // Create simple graph: A -> B -> C
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

        let edge_ab = Edge {
            id: uuid::Uuid::new_v4(),
            source: a,
            target: b,
            edge_type: "LINKS_TO".to_string(),
            properties: HashMap::new(),
        };

        let edge_bc = Edge {
            id: uuid::Uuid::new_v4(),
            source: b,
            target: c,
            edge_type: "LINKS_TO".to_string(),
            properties: HashMap::new(),
        };

        tx.add_edge(edge_ab).unwrap();
        tx.add_edge(edge_bc).unwrap();
        tx.commit().await.unwrap();

        // Compute PageRank
        let pr = PageRank::with_defaults();
        let ranks = pr.compute(&graph).await.unwrap();

        assert_eq!(ranks.len(), 3);

        // C should have highest rank (receives link from B)
        // B should have middle rank (receives from A, links to C)
        // A should have lowest rank (only gives links)
        assert!(ranks[&c] > ranks[&b]);
        assert!(ranks[&b] > ranks[&a]);
    }

    #[tokio::test]
    async fn test_scoped_pagerank() {
        use crate::graph::Subgraph;
        use std::collections::HashSet;

        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();

        // Create a graph: A -> B -> C -> D
        let a = tx
            .add_node(Node {
                id: uuid::Uuid::new_v4(),
                label: "L".into(),
                properties: HashMap::new(),
                kind: Default::default(),
            })
            .await
            .unwrap();
        let b = tx
            .add_node(Node {
                id: uuid::Uuid::new_v4(),
                label: "L".into(),
                properties: HashMap::new(),
                kind: Default::default(),
            })
            .await
            .unwrap();
        let c = tx
            .add_node(Node {
                id: uuid::Uuid::new_v4(),
                label: "L".into(),
                properties: HashMap::new(),
                kind: Default::default(),
            })
            .await
            .unwrap();
        let d = tx
            .add_node(Node {
                id: uuid::Uuid::new_v4(),
                label: "L".into(),
                properties: HashMap::new(),
                kind: Default::default(),
            })
            .await
            .unwrap();

        tx.add_edge(Edge {
            id: uuid::Uuid::new_v4(),
            source: a,
            target: b,
            edge_type: "L".into(),
            properties: HashMap::new(),
        })
        .unwrap();
        tx.add_edge(Edge {
            id: uuid::Uuid::new_v4(),
            source: b,
            target: c,
            edge_type: "L".into(),
            properties: HashMap::new(),
        })
        .unwrap();
        tx.add_edge(Edge {
            id: uuid::Uuid::new_v4(),
            source: c,
            target: d,
            edge_type: "L".into(),
            properties: HashMap::new(),
        })
        .unwrap();
        tx.commit().await.unwrap();

        // Global PageRank
        let pr = PageRank::with_defaults();
        let global_ranks = pr.compute(&graph).await.unwrap();

        // Scope the graph to just nodes B and C
        let mut allowed = HashSet::new();
        allowed.insert(b);
        allowed.insert(c);
        let subgraph = Subgraph::new(&graph, allowed);

        // Subgraph PageRank
        let scoped_ranks = pr.compute(&subgraph).await.unwrap();

        assert_eq!(
            scoped_ranks.len(),
            2,
            "Scoped rank should only contain B and C"
        );

        // Inside the subgraph B -> C, C should have higher rank than B,
        // and both should have drastically different normalized scores than in the global graph.
        assert!(scoped_ranks[&c] > scoped_ranks[&b]);
        assert_ne!(scoped_ranks[&c], global_ranks[&c]); // Different because context changed!
        assert!(!scoped_ranks.contains_key(&a));
        assert!(!scoped_ranks.contains_key(&d));
    }
}
