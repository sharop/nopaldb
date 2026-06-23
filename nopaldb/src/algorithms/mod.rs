// src/algorithms/mod.rs
//
// Graph algorithms for NopalDB

pub mod betweenness;
pub mod clustering;
pub mod community;
pub mod degree;
pub mod pagerank;
pub mod shortest_path;

pub use betweenness::BetweennessCentrality;
pub use clustering::ClusteringCoefficient;
pub use community::{LeidenCommunity, LeidenConfig, LouvainCommunity};
pub use degree::DegreeCentrality;
pub use pagerank::PageRank;
pub use shortest_path::ShortestPath;

use crate::types::NodeId;
use std::collections::HashMap;

/// Algorithm configuration
#[derive(Debug, Clone)]
pub struct AlgorithmConfig {
    pub max_iterations: usize,
    pub tolerance: f64,
    pub parallel: bool,
}

impl Default for AlgorithmConfig {
    fn default() -> Self {
        AlgorithmConfig {
            max_iterations: 100,
            tolerance: 1e-6,
            parallel: true,
        }
    }
}

/// Common result type for algorithms
pub type AlgorithmResult = HashMap<NodeId, f64>;
