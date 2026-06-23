// src/query/step.rs

use crate::graph::Direction;
use crate::types::NodeId;

/// Un paso en el traversal
#[derive(Debug, Clone)]
pub enum TraversalStep {
    /// Moverse a vecinos siguiendo aristas de un tipo
    FollowEdge {
        edge_type: Option<String>, // None = cualquier tipo
        direction: Direction,
    },

    /// Filtrar nodos por predicado
    Filter {
        predicate: String, // Descripción para debug
    },

    /// Limitar resultados
    Limit { count: usize },

    /// Saltar N resultados
    Skip { count: usize },
}

/// Estado de ejecución del traversal
#[derive(Debug)]
pub struct TraversalState {
    /// Nodos actuales en el traversal
    pub current_nodes: Vec<NodeId>,

    /// Total de nodos visitados
    pub visited_count: usize,

    /// Máximo de nodos a retornar
    pub limit: Option<usize>,

    /// Nodos a saltar
    pub skip: usize,
}

impl TraversalState {
    pub fn new(start: NodeId) -> Self {
        Self {
            current_nodes: vec![start],
            visited_count: 0,
            limit: None,
            skip: 0,
        }
    }

    pub fn with_nodes(nodes: Vec<NodeId>) -> Self {
        Self {
            current_nodes: nodes,
            visited_count: 0,
            limit: None,
            skip: 0,
        }
    }
}
