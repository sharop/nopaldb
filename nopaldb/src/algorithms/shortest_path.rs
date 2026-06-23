// src/algorithms/shortest_path.rs
//
// Shortest Path algorithms (Dijkstra, BFS)

use crate::error::Result;
use crate::graph::Graph;
use crate::types::NodeId;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};

/// Path result
#[derive(Debug, Clone)]
pub struct PathResult {
    /// Sequence of node IDs from source to target
    pub path: Vec<NodeId>,

    /// Total distance/cost
    pub distance: f64,
}

/// Shortest Path configuration
#[derive(Debug, Clone, Default)]
pub struct ShortestPathConfig {
    /// Edge weight property name (if None, use BFS with weight=1)
    pub weight_property: Option<String>,

    /// Maximum path length to search
    pub max_length: Option<usize>,
}

/// Shortest Path algorithms
pub struct ShortestPath {
    config: ShortestPathConfig,
}

impl ShortestPath {
    /// Create new Shortest Path instance
    pub fn new(config: ShortestPathConfig) -> Self {
        ShortestPath { config }
    }

    /// Create with default configuration (unweighted BFS)
    pub fn with_defaults() -> Self {
        ShortestPath {
            config: ShortestPathConfig::default(),
        }
    }

    /// Find shortest path between two nodes
    pub async fn find_path(
        &self,
        graph: &Graph,
        source: NodeId,
        target: NodeId,
    ) -> Result<Option<PathResult>> {
        if source == target {
            return Ok(Some(PathResult {
                path: vec![source],
                distance: 0.0,
            }));
        }

        let edges = graph.get_all_edges().await?;
        let config = self.config.clone();
        tokio::task::spawn_blocking(move || Self::find_path_cpu(edges, source, target, config))
            .await
            .map_err(|e| {
                crate::error::NopalError::custom(format!("shortest path join error: {e}"))
            })?
    }

    fn find_path_cpu(
        edges: Vec<crate::types::Edge>,
        source: NodeId,
        target: NodeId,
        config: ShortestPathConfig,
    ) -> Result<Option<PathResult>> {
        if config.weight_property.is_some() {
            Self::dijkstra_cpu(edges, source, target, &config)
        } else {
            Self::bfs_cpu(edges, source, target)
        }
    }

    fn bfs_cpu(
        edges: Vec<crate::types::Edge>,
        source: NodeId,
        target: NodeId,
    ) -> Result<Option<PathResult>> {
        let mut adjacency: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for edge in &edges {
            adjacency.entry(edge.source).or_default().push(edge.target);
        }

        let mut queue = VecDeque::new();
        let mut visited = HashSet::new();
        let mut parent: HashMap<NodeId, NodeId> = HashMap::new();

        queue.push_back(source);
        visited.insert(source);

        while let Some(current) = queue.pop_front() {
            if current == target {
                let path = reconstruct_path(&parent, source, target);
                let distance = (path.len() - 1) as f64;
                return Ok(Some(PathResult { path, distance }));
            }

            if let Some(neighbors) = adjacency.get(&current) {
                for &neighbor in neighbors {
                    if !visited.contains(&neighbor) {
                        visited.insert(neighbor);
                        parent.insert(neighbor, current);
                        queue.push_back(neighbor);
                    }
                }
            }
        }

        Ok(None)
    }

    fn dijkstra_cpu(
        edges: Vec<crate::types::Edge>,
        source: NodeId,
        target: NodeId,
        config: &ShortestPathConfig,
    ) -> Result<Option<PathResult>> {
        let weight_prop = config.weight_property.as_ref().ok_or_else(|| {
            crate::error::NopalError::Custom(
                "weight_property is required for weighted shortest path".into(),
            )
        })?;

        let mut adjacency: HashMap<NodeId, Vec<(NodeId, f64)>> = HashMap::new();
        for edge in &edges {
            let weight = edge
                .properties
                .get(weight_prop)
                .and_then(|v| match v {
                    crate::types::PropertyValue::Int(i) => Some(*i as f64),
                    crate::types::PropertyValue::Float(f) => Some(*f),
                    _ => None,
                })
                .unwrap_or(1.0);

            adjacency
                .entry(edge.source)
                .or_default()
                .push((edge.target, weight));
        }

        let mut distances: HashMap<NodeId, f64> = HashMap::new();
        let mut parent: HashMap<NodeId, NodeId> = HashMap::new();
        let mut heap = BinaryHeap::new();

        distances.insert(source, 0.0);
        heap.push(State {
            cost: 0.0,
            node: source,
        });

        while let Some(State { cost, node }) = heap.pop() {
            if node == target {
                let path = reconstruct_path(&parent, source, target);
                return Ok(Some(PathResult {
                    path,
                    distance: cost,
                }));
            }

            if cost > *distances.get(&node).unwrap_or(&f64::INFINITY) {
                continue;
            }

            if let Some(neighbors) = adjacency.get(&node) {
                for &(neighbor, weight) in neighbors {
                    let next_cost = cost + weight;
                    let current_dist = *distances.get(&neighbor).unwrap_or(&f64::INFINITY);

                    if next_cost < current_dist {
                        distances.insert(neighbor, next_cost);
                        parent.insert(neighbor, node);
                        heap.push(State {
                            cost: next_cost,
                            node: neighbor,
                        });
                    }
                }
            }
        }

        Ok(None)
    }

    /// Find all shortest paths from a single source
    pub async fn single_source_shortest_paths(
        &self,
        graph: &Graph,
        source: NodeId,
    ) -> Result<HashMap<NodeId, PathResult>> {
        let nodes = graph.get_all_nodes().await?;
        let mut results = HashMap::new();

        for node in &nodes {
            if node.id != source
                && let Some(result) = self.find_path(graph, source, node.id).await?
            {
                results.insert(node.id, result);
            }
        }

        Ok(results)
    }

    /// Compute average shortest path length for the graph
    pub async fn average_shortest_path_length(&self, graph: &Graph) -> Result<f64> {
        let nodes = graph.get_all_nodes().await?;

        if nodes.len() < 2 {
            return Ok(0.0);
        }

        let mut total_distance = 0.0;
        let mut path_count = 0;

        for i in 0..nodes.len() {
            for j in (i + 1)..nodes.len() {
                if let Some(result) = self.find_path(graph, nodes[i].id, nodes[j].id).await? {
                    total_distance += result.distance;
                    path_count += 1;
                }
            }
        }

        if path_count > 0 {
            Ok(total_distance / path_count as f64)
        } else {
            Ok(0.0)
        }
    }
}

/// Reconstruct path from parent map
fn reconstruct_path(
    parent: &HashMap<NodeId, NodeId>,
    source: NodeId,
    target: NodeId,
) -> Vec<NodeId> {
    let mut path = Vec::new();
    let mut current = target;

    while current != source {
        path.push(current);
        if let Some(&prev) = parent.get(&current) {
            current = prev;
        } else {
            break;
        }
    }

    path.push(source);
    path.reverse();
    path
}

/// State for Dijkstra's priority queue
#[derive(Copy, Clone)]
struct State {
    cost: f64,
    node: NodeId,
}

// Custom ordering for min-heap (reverse of natural ordering)
impl Eq for State {}

impl PartialEq for State {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost
    }
}

impl Ord for State {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering for min-heap
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for State {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Graph;
    use crate::types::{Edge, Node, PropertyValue};
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_shortest_path_unweighted() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();

        // Create linear path: A -> B -> C -> D
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

        let d = tx
            .add_node(Node {
                id: uuid::Uuid::new_v4(),
                label: "Node".to_string(),
                properties: HashMap::new(),
                kind: Default::default(),
            })
            .await
            .unwrap();

        tx.add_edge(Edge {
            id: uuid::Uuid::new_v4(),
            source: a,
            target: b,
            edge_type: "CONNECTS".to_string(),
            properties: HashMap::new(),
        })
        .unwrap();

        tx.add_edge(Edge {
            id: uuid::Uuid::new_v4(),
            source: b,
            target: c,
            edge_type: "CONNECTS".to_string(),
            properties: HashMap::new(),
        })
        .unwrap();

        tx.add_edge(Edge {
            id: uuid::Uuid::new_v4(),
            source: c,
            target: d,
            edge_type: "CONNECTS".to_string(),
            properties: HashMap::new(),
        })
        .unwrap();

        tx.commit().await.unwrap();

        // Find path A -> D
        let sp = ShortestPath::with_defaults();
        let result = sp.find_path(&graph, a, d).await.unwrap();

        assert!(result.is_some());
        let path_result = result.unwrap();
        assert_eq!(path_result.path, vec![a, b, c, d]);
        assert_eq!(path_result.distance, 3.0);
    }

    #[tokio::test]
    async fn test_shortest_path_weighted() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();

        // Create graph with weights
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

        // A -> B (weight 1)
        let mut props1 = HashMap::new();
        props1.insert("weight".to_string(), PropertyValue::Float(1.0));
        tx.add_edge(Edge {
            id: uuid::Uuid::new_v4(),
            source: a,
            target: b,
            edge_type: "CONNECTS".to_string(),
            properties: props1,
        })
        .unwrap();

        // B -> C (weight 1)
        let mut props2 = HashMap::new();
        props2.insert("weight".to_string(), PropertyValue::Float(1.0));
        tx.add_edge(Edge {
            id: uuid::Uuid::new_v4(),
            source: b,
            target: c,
            edge_type: "CONNECTS".to_string(),
            properties: props2,
        })
        .unwrap();

        // A -> C (weight 10) - longer direct path
        let mut props3 = HashMap::new();
        props3.insert("weight".to_string(), PropertyValue::Float(10.0));
        tx.add_edge(Edge {
            id: uuid::Uuid::new_v4(),
            source: a,
            target: c,
            edge_type: "CONNECTS".to_string(),
            properties: props3,
        })
        .unwrap();

        tx.commit().await.unwrap();

        // Find weighted shortest path A -> C
        let config = ShortestPathConfig {
            weight_property: Some("weight".to_string()),
            max_length: None,
        };
        let sp = ShortestPath::new(config);
        let result = sp.find_path(&graph, a, c).await.unwrap();

        assert!(result.is_some());
        let path_result = result.unwrap();

        // Should take A -> B -> C (distance 2) instead of A -> C (distance 10)
        assert_eq!(path_result.path, vec![a, b, c]);
        assert_eq!(path_result.distance, 2.0);
    }

    #[tokio::test]
    async fn test_no_path() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();

        // Create two disconnected nodes
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

        tx.commit().await.unwrap();

        let sp = ShortestPath::with_defaults();
        let result = sp.find_path(&graph, a, b).await.unwrap();

        assert!(result.is_none());
    }
}
