// src/schema/mod.rs
//! Schema inspection and management for NopalDB

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::RwLock;
use serde::{Serialize, Deserialize};

use crate::error::Result;
use crate::graph::Graph;

/// Complete schema information for a graph
#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct SchemaInfo {
    /// All unique node labels in the graph
    pub node_labels: Vec<String>,

    /// All unique edge types in the graph
    pub edge_types: Vec<String>,

    /// Properties per node label
    pub node_properties: HashMap<String, HashSet<String>>,

    /// Properties per edge type
    pub edge_properties: HashMap<String, HashSet<String>>,

    /// Node count per label
    pub node_counts: HashMap<String, usize>,

    /// Edge count per type
    pub edge_counts: HashMap<String, usize>,

    /// Total nodes
    pub total_nodes: usize,

    /// Total edges
    pub total_edges: usize,
}


impl SchemaInfo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a node label to the schema
    pub fn add_node_label(&mut self, label: String) {
        if !self.node_labels.contains(&label) {
            self.node_labels.push(label.clone());
            self.node_properties.insert(label.clone(), HashSet::new());
            self.node_counts.insert(label, 0);
        }
    }

    /// Add a property to a node label
    pub fn add_node_property(&mut self, label: &str, property: String) {
        self.node_properties
            .entry(label.to_string())
            .or_default()
            .insert(property);
    }

    /// Add an edge type to the schema
    pub fn add_edge_type(&mut self, edge_type: String) {
        if !self.edge_types.contains(&edge_type) {
            self.edge_types.push(edge_type.clone());
            self.edge_properties.insert(edge_type.clone(), HashSet::new());
            self.edge_counts.insert(edge_type, 0);
        }
    }

    /// Add a property to an edge type
    pub fn add_edge_property(&mut self, edge_type: &str, property: String) {
        self.edge_properties
            .entry(edge_type.to_string())
            .or_default()
            .insert(property);
    }

    /// Increment node count for a label
    pub fn increment_node_count(&mut self, label: &str) {
        *self.node_counts.entry(label.to_string()).or_insert(0) += 1;
        self.total_nodes += 1;
    }

    /// Increment edge count for a type
    pub fn increment_edge_count(&mut self, edge_type: &str) {
        *self.edge_counts.entry(edge_type.to_string()).or_insert(0) += 1;
        self.total_edges += 1;
    }
}

/// Schema manager with caching
pub struct SchemaManager {
    info: Arc<RwLock<SchemaInfo>>,
    dirty: Arc<AtomicBool>,
}

impl SchemaManager {
    pub fn new() -> Self {
        Self {
            info: Arc::new(RwLock::new(SchemaInfo::new())),
            dirty: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Check if schema needs rebuilding
    pub fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::SeqCst)
    }

    /// Mark schema as dirty (needs rebuild)
    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::SeqCst);
    }

    /// Rebuild schema from graph (expensive operation)
    pub async fn rebuild(&self, graph: &Graph) -> Result<()> {
        log::info!("Rebuilding schema from graph...");

        let mut info = SchemaInfo::new();

        // Scan all nodes
        let nodes = graph.get_all_nodes().await?;
        log::debug!("Scanning {} nodes for schema", nodes.len());

        for node in nodes {
            let label = node.label.clone();
            info.add_node_label(label.clone());
            info.increment_node_count(&label);

            // Add all properties
            for key in node.properties.keys() {
                info.add_node_property(&label, key.clone());
            }
        }

        // Scan all edges
        let edges = graph.get_all_edges().await?;
        log::debug!("Scanning {} edges for schema", edges.len());

        for edge in edges {
            let edge_type = edge.edge_type.clone();
            info.add_edge_type(edge_type.clone());
            info.increment_edge_count(&edge_type);

            // Add all properties
            for key in edge.properties.keys() {
                info.add_edge_property(&edge_type, key.clone());
            }
        }

        // Update cached schema
        *self.info.write().await = info;
        self.dirty.store(false, Ordering::SeqCst);

        log::info!("Schema rebuilt successfully");

        Ok(())
    }

    /// Get cached schema info (rebuilds if dirty)
    pub async fn get_info(&self, graph: &Graph) -> Result<SchemaInfo> {
        if self.is_dirty() {
            self.rebuild(graph).await?;
        }
        Ok(self.info.read().await.clone())
    }

    /// Get cached schema without rebuilding
    pub async fn get_info_cached(&self) -> SchemaInfo {
        self.info.read().await.clone()
    }
}

impl Default for SchemaManager {
    fn default() -> Self {
        Self::new()
    }
}