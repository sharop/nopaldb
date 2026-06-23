// src/graph/view.rs
//
// Abstraction for running graph algorithms on the full graph or a subgraph.

use crate::error::Result;
use crate::graph::Graph;
use crate::types::{Edge, Node, NodeId};
use std::collections::HashSet;

/// A view over a graph topology.
/// Algorithms should accept `&impl GraphView` instead of `&Graph` to support isolated subgraph execution.
pub trait GraphView: Send + Sync {
    /// Returns the nodes present in this view.
    fn get_all_nodes(&self) -> impl std::future::Future<Output = Result<Vec<Node>>> + Send;

    /// Returns the edges present in this view.
    fn get_all_edges(&self) -> impl std::future::Future<Output = Result<Vec<Edge>>> + Send;
}

impl GraphView for Graph {
    async fn get_all_nodes(&self) -> Result<Vec<Node>> {
        self.get_all_nodes().await
    }

    async fn get_all_edges(&self) -> Result<Vec<Edge>> {
        self.get_all_edges().await
    }
}

/// A view over a subset of a graph, defined by a set of allowed `NodeId`s.
pub struct Subgraph<'a> {
    graph: &'a Graph,
    allowed_nodes: HashSet<NodeId>,
}

impl<'a> Subgraph<'a> {
    /// Creates a new `Subgraph` restricted to the specified nodes.
    pub fn new(graph: &'a Graph, allowed_nodes: HashSet<NodeId>) -> Self {
        Self {
            graph,
            allowed_nodes,
        }
    }

    /// Creates a new `Subgraph` from a list of nodes.
    pub fn from_nodes(graph: &'a Graph, nodes: &[Node]) -> Self {
        let allowed_nodes = nodes.iter().map(|n| n.id).collect();
        Self {
            graph,
            allowed_nodes,
        }
    }

    /// Gives access to the underlying Graph reference
    pub fn inner(&self) -> &Graph {
        self.graph
    }
}

impl<'a> GraphView for Subgraph<'a> {
    async fn get_all_nodes(&self) -> Result<Vec<Node>> {
        // Only return nodes that are explicitly allowed. We can load them individually or filter full scan.
        // Assuming we only allow nodes we ALREADY know exist, loading individually or from the slice is faster,
        // but `GraphView` doesn't enforce caching. To avoid O(N) full scan:
        let mut nodes = Vec::with_capacity(self.allowed_nodes.len());
        for &id in &self.allowed_nodes {
            match self.graph.get_node(id).await {
                Ok(node) => nodes.push(node),
                Err(_) => continue, // Graceful skip if missing
            }
        }
        Ok(nodes)
    }

    async fn get_all_edges(&self) -> Result<Vec<Edge>> {
        // Only return edges where BOTH source and target are in the allowed_nodes set.
        // The most efficient way without a custom index is to either scan all edges or check all allowed_nodes
        // out_edges and filter them. Iterating out_edges of allowed_nodes is likely faster for small subgraphs.

        let mut edges = Vec::new();
        // Use a set to avoid duplicates if returning from both directions, but outgoing is sufficient to cover all edges
        // that start in our subgraph. If target is also in subgraph, it's valid.
        for &node_id in &self.allowed_nodes {
            if let Ok(outgoing) = self.graph.get_outgoing_edges(node_id).await {
                for edge in outgoing {
                    if self.allowed_nodes.contains(&edge.target) {
                        edges.push(edge);
                    }
                }
            }
        }

        Ok(edges)
    }
}
