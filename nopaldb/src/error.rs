// src/error.rs
//
// Manejo centralizado de errores para NopalDB
//
// Refactorizado: 2026-01-12
// - Consolidado de 90+ variantes a ~15 variantes semánticas
// - Mensajes claros y en inglés para consistencia

/// Errores de NopalDB
///
/// Cada variante representa una categoría de error semánticamente distinta.
/// Para errores específicos, usa el campo `String` con contexto adicional.
#[derive(Debug, thiserror::Error)]
pub enum NopalError {
    // ═══════════════════════════════════════════════════════════════
    // ERRORES DE ENTIDADES (Nodos y Aristas)
    // ═══════════════════════════════════════════════════════════════

    /// Nodo no encontrado
    #[error("Node not found: {0}")]
    NodeNotFound(String),

    /// Arista no encontrada
    #[error("Edge not found: {0}")]
    EdgeNotFound(String),

    // ═══════════════════════════════════════════════════════════════
    // ERRORES DE STORAGE
    // ═══════════════════════════════════════════════════════════════

    /// Error del motor de storage (sled)
    #[error("Storage error: {0}")]
    StorageError(#[from] sled::Error),

    /// Error de I/O
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Error de serialización/deserialización
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Formato de clave inválido
    #[error("Invalid key format: {0}")]
    InvalidKey(String),

    // ═══════════════════════════════════════════════════════════════
    // ERRORES DE TRANSACCIONES
    // ═══════════════════════════════════════════════════════════════

    /// Transacción no está activa
    #[error("Transaction is not active")]
    TransactionNotActive,

    /// Conflicto de transacción (write-write conflict)
    #[error("Transaction conflict: {0}")]
    TransactionConflict(String),

    /// Deadlock detectado
    #[error("Deadlock detected: {0}")]
    Deadlock(String),

    /// Error de concurrencia genérico
    #[error("Concurrency error: {0}")]
    ConcurrencyError(String),

    // ═══════════════════════════════════════════════════════════════
    // ERRORES DE QUERIES (NQL)
    // ═══════════════════════════════════════════════════════════════

    /// Error al parsear query NQL
    #[error("Query parse error: {0}")]
    QueryParseError(String),

    /// Error al ejecutar query NQL
    #[error("Query execution error: {0}")]
    QueryExecutionError(String),

    /// Error al planificar query
    #[error("Query planning error: {0}")]
    QueryPlanningError(String),

    // ═══════════════════════════════════════════════════════════════
    // ERRORES DE SKETCH/COMMIT (NQL v0.2)
    // ═══════════════════════════════════════════════════════════════

    /// Sketch no encontrado
    #[error("Sketch not found: {0}")]
    SketchNotFound(String),

    /// Sketch inválido
    #[error("Invalid sketch: {0}")]
    InvalidSketch(String),

    /// Commit inválido
    #[error("Invalid commit: {0}")]
    InvalidCommit(String),

    /// Error de validación semántica
    #[error("Semantic validation error: {0}")]
    SemanticError(String),

    #[error("Index error: {0}")]
    IndexError(String),

    #[error("Ambiguous upsert key: {0}")]
    AmbiguousUpsertKey(String),

    // ═══════════════════════════════════════════════════════════════
    // ERRORES GENÉRICOS
    // ═══════════════════════════════════════════════════════════════

    /// Error personalizado (catch-all)
    ///
    /// Usar cuando ninguna otra variante aplica.
    /// Incluir contexto descriptivo en el mensaje.
    #[error("{0}")]
    Custom(String),
}

/// Result type alias para NopalDB
pub type Result<T> = std::result::Result<T, NopalError>;

// ═══════════════════════════════════════════════════════════════
// HELPERS PARA CONSTRUCCIÓN DE ERRORES
// ═══════════════════════════════════════════════════════════════

impl NopalError {
    /// Crea un error de nodo no encontrado
    pub fn node_not_found(id: impl std::fmt::Display) -> Self {
        NopalError::NodeNotFound(id.to_string())
    }

    /// Crea un error de arista no encontrada
    pub fn edge_not_found(id: impl std::fmt::Display) -> Self {
        NopalError::EdgeNotFound(id.to_string())
    }

    /// Crea un error de serialización
    pub fn serialization(msg: impl Into<String>) -> Self {
        NopalError::SerializationError(msg.into())
    }

    /// Crea un error personalizado
    pub fn custom(msg: impl Into<String>) -> Self {
        NopalError::Custom(msg.into())
    }

    /// Crea un error de query
    pub fn query_error(msg: impl Into<String>) -> Self {
        NopalError::QueryExecutionError(msg.into())
    }

    pub fn index_error(msg: impl Into<String>) -> Self {
        NopalError::IndexError(msg.into())
    }

    /// Crea un error de sketch no encontrado
    pub fn sketch_not_found(name: impl Into<String>) -> Self {
        NopalError::SketchNotFound(name.into())
    }

    /// Crea un error de sketch inválido
    pub fn invalid_sketch(msg: impl Into<String>) -> Self {
        NopalError::InvalidSketch(msg.into())
    }

    /// Crea un error de commit inválido
    pub fn invalid_commit(msg: impl Into<String>) -> Self {
        NopalError::InvalidCommit(msg.into())
    }

    /// Crea un error semántico
    pub fn semantic(msg: impl Into<String>) -> Self {
        NopalError::SemanticError(msg.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = NopalError::NodeNotFound("abc-123".to_string());
        assert!(err.to_string().contains("abc-123"));
    }

    #[test]
    fn test_error_helpers() {
        let err = NopalError::node_not_found("test-id");
        assert!(matches!(err, NopalError::NodeNotFound(_)));

        let err = NopalError::custom("something went wrong");
        assert!(matches!(err, NopalError::Custom(_)));
    }
}