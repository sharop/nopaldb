// src/embeddings/node.rs

use crate::types::NodeId;
use serde::{Deserialize, Serialize};

/// Embedding de un nodo (vector denso)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Embedding {
    pub node_id: NodeId,
    pub vector: Vec<f32>, // Vector denso (ej: 768 dimensiones)
    pub model: String,    // Modelo que lo generó (ej: "bert-base", "openai-ada-002")
    pub version: u32,     // Versión del embedding (para invalidar cache)
}

impl Embedding {
    pub fn new(node_id: NodeId, vector: Vec<f32>, model: impl Into<String>) -> Self {
        Self {
            node_id,
            vector,
            model: model.into(),
            version: 1,
        }
    }

    /// Calcula similitud coseno con otro embedding
    pub fn cosine_similarity(&self, other: &Embedding) -> f32 {
        if self.vector.len() != other.vector.len() {
            return 0.0;
        }

        let dot_product: f32 = self
            .vector
            .iter()
            .zip(other.vector.iter())
            .map(|(a, b)| a * b)
            .sum();

        let norm_a: f32 = self.vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = other.vector.iter().map(|x| x * x).sum::<f32>().sqrt();

        if norm_a == 0.0 || norm_b == 0.0 {
            return 0.0;
        }

        dot_product / (norm_a * norm_b)
    }

    /// Distancia euclidiana
    pub fn euclidean_distance(&self, other: &Embedding) -> f32 {
        self.vector
            .iter()
            .zip(other.vector.iter())
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f32>()
            .sqrt()
    }
}
