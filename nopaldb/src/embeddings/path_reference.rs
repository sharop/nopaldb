// src/embeddings/path_reference.rs
//
// PathReferenceEmbedding — referencia persistida de path embedding para E-8.
// Clave unica por (name, node_model, edge_model).

use serde::{Deserialize, Serialize};
use crate::error::{NopalError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathReferenceEmbedding {
    pub name: String,
    pub node_model: String,
    pub edge_model: String,
    pub vector: Vec<f32>,
    pub created_at: u64,
}

impl PathReferenceEmbedding {
    pub fn new(
        name: String,
        node_model: String,
        edge_model: String,
        vector: Vec<f32>,
    ) -> Self {
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self { name, node_model, edge_model, vector, created_at }
    }

    /// Clave de almacenamiento: "name\x00node_model\x00edge_model"
    /// El separador \x00 evita colisiones entre nombres con guiones o puntos.
    pub fn storage_key(name: &str, node_model: &str, edge_model: &str) -> String {
        format!("{}\x00{}\x00{}", name, node_model, edge_model)
    }

    /// Valida el contrato semantico minimo de E-8 para referencias persistidas.
    pub fn validate(&self) -> Result<()> {
        if self.vector.is_empty() {
            return Err(NopalError::QueryExecutionError(
                "PathReferenceEmbedding E-8 cannot persist an empty vector".into(),
            ));
        }
        let norm_sq: f32 = self.vector.iter().map(|x| x * x).sum();
        if norm_sq == 0.0 {
            return Err(NopalError::QueryExecutionError(
                "PathReferenceEmbedding E-8 cannot persist a zero-norm vector".into(),
            ));
        }
        Ok(())
    }
}
