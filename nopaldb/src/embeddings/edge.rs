// src/embeddings/edge.rs

use crate::types::EdgeId;
use serde::{Deserialize, Serialize};

/// Embedding de una arista (vector denso)
///
/// Los embeddings de aristas capturan la semántica de una relación entre dos nodos.
/// La forma más sencilla de generarlos es por concatenación o promedio de los
/// embeddings de los nodos extremo (head/tail), sin requerir entrenamiento adicional.
///
/// # Ejemplo
/// ```
/// use nopaldb::embeddings::EdgeEmbedding;
/// use uuid::Uuid;
///
/// // Vector generado externamente (ej. concatenación head + tail, 128 dims)
/// let edge_id = Uuid::new_v4();
/// let vector = vec![0.1_f32; 128];
/// let emb = EdgeEmbedding::new(edge_id, vector, "concat-minilm");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeEmbedding {
    pub edge_id: EdgeId,
    pub vector: Vec<f32>, // Vector denso (dimensión depende del modelo)
    pub model: String,    // Modelo que lo generó (ej: "concat-minilm", "transe")
    pub version: u32,     // Versión del embedding (para invalidar caché)
}

impl EdgeEmbedding {
    pub fn new(edge_id: EdgeId, vector: Vec<f32>, model: impl Into<String>) -> Self {
        Self {
            edge_id,
            vector,
            model: model.into(),
            version: 1,
        }
    }

    /// Similitud coseno con otro EdgeEmbedding.
    /// Retorna 0.0 si las dimensiones no coinciden o algún vector es cero.
    pub fn cosine_similarity(&self, other: &EdgeEmbedding) -> f32 {
        if self.vector.len() != other.vector.len() {
            return 0.0;
        }

        let dot: f32 = self
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

        dot / (norm_a * norm_b)
    }

    /// Distancia euclidiana con otro EdgeEmbedding.
    pub fn euclidean_distance(&self, other: &EdgeEmbedding) -> f32 {
        self.vector
            .iter()
            .zip(other.vector.iter())
            .map(|(a, b)| (a - b).powi(2))
            .sum::<f32>()
            .sqrt()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn dummy_edge_id() -> EdgeId {
        Uuid::new_v4()
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let id = dummy_edge_id();
        let v = vec![1.0_f32, 0.0, 0.0];
        let a = EdgeEmbedding::new(id, v.clone(), "test");
        let b = EdgeEmbedding::new(id, v, "test");
        let sim = a.cosine_similarity(&b);
        assert!((sim - 1.0).abs() < 1e-6, "vectores idénticos → sim = 1.0");
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let id = dummy_edge_id();
        let a = EdgeEmbedding::new(id, vec![1.0, 0.0], "test");
        let b = EdgeEmbedding::new(id, vec![0.0, 1.0], "test");
        let sim = a.cosine_similarity(&b);
        assert!(sim.abs() < 1e-6, "vectores ortogonales → sim = 0.0");
    }

    #[test]
    fn test_cosine_similarity_dim_mismatch() {
        let id = dummy_edge_id();
        let a = EdgeEmbedding::new(id, vec![1.0, 0.0], "test");
        let b = EdgeEmbedding::new(id, vec![1.0, 0.0, 0.0], "test");
        assert_eq!(a.cosine_similarity(&b), 0.0, "dimensiones distintas → 0.0");
    }

    #[test]
    fn test_euclidean_distance_zero() {
        let id = dummy_edge_id();
        let v = vec![3.0_f32, 4.0];
        let a = EdgeEmbedding::new(id, v.clone(), "test");
        let b = EdgeEmbedding::new(id, v, "test");
        assert!((a.euclidean_distance(&b)).abs() < 1e-6);
    }

    #[test]
    fn test_euclidean_distance_known() {
        let id = dummy_edge_id();
        let a = EdgeEmbedding::new(id, vec![0.0, 0.0], "test");
        let b = EdgeEmbedding::new(id, vec![3.0, 4.0], "test");
        assert!((a.euclidean_distance(&b) - 5.0).abs() < 1e-5);
    }
}
