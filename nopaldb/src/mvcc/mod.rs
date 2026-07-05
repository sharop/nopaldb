// src/mvcc/mod.rs
//
// Multi-Version Concurrency Control implementation
// Includes Garbage Collection for old versions

use serde::{Serialize, Deserialize};
use crate::types::{Node, Edge, NodeId, EdgeId};
use crate::error::Result;

/// Nodo versionado con metadata MVCC
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionedNode {
    /// ID del nodo (inmutable)
    pub id: NodeId,

    /// Número de versión (monotónico)
    pub version: u64,

    /// Timestamp de creación de esta versión
    pub timestamp: u64,

    /// Datos del nodo
    pub node_data: Node,

    /// Versión anterior (chain)
    pub prev_version: Option<u64>,

    /// Válido desde (inclusive)
    pub valid_from: u64,

    /// Válido hasta (exclusive, None = actual)
    pub valid_to: Option<u64>,
}

impl VersionedNode {
    /// Crea una nueva versión inicial
    pub fn new(node: Node, timestamp: u64) -> Self {
        Self {
            id: node.id,
            version: 1,
            timestamp,
            node_data: node,
            prev_version: None,
            valid_from: timestamp,
            valid_to: None,
        }
    }

    /// Crea una nueva versión desde una anterior
    pub fn new_version(
        previous: &VersionedNode,
        new_data: Node,
        timestamp: u64,
    ) -> Self {
        Self {
            id: previous.id,
            version: previous.version + 1,
            timestamp,
            node_data: new_data,
            prev_version: Some(previous.version),
            valid_from: timestamp,
            valid_to: None,
        }
    }

    /// Invalida esta versión (marca valid_to)
    pub fn invalidate(&mut self, timestamp: u64) {
        self.valid_to = Some(timestamp);
    }

    /// Verifica si esta versión es válida en un timestamp
    pub fn is_valid_at(&self, timestamp: u64) -> bool {
        timestamp >= self.valid_from
            && self.valid_to.map(|to| timestamp < to).unwrap_or(true)
    }

    /// Verifica si esta versión puede ser eliminada por GC
    /// Una versión es elegible para GC si:
    /// 1. Tiene valid_to (está invalidada)
    /// 2. valid_to es menor que el cutoff timestamp
    pub fn is_gc_eligible(&self, cutoff_timestamp: u64) -> bool {
        match self.valid_to {
            Some(valid_to) => valid_to < cutoff_timestamp,
            None => false, // Versión actual, no eliminar
        }
    }
}

/// Arista versionada con metadata MVCC (espejo de VersionedNode para aristas)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionedEdge {
    /// ID de la arista (inmutable)
    pub id: EdgeId,

    /// Número de versión por-arista (monotónico, empieza en 1)
    pub version: u64,

    /// Timestamp lógico del commit que creó esta versión
    pub timestamp: u64,

    /// Datos de la arista
    pub edge_data: Edge,

    /// Válido desde (inclusive)
    pub valid_from: u64,

    /// Válido hasta (exclusive, None = versión actual)
    pub valid_to: Option<u64>,

    /// Versión anterior en la cadena (None = primera versión)
    pub prev_version: Option<u64>,
}

impl VersionedEdge {
    /// Crea la versión inicial de una arista
    pub fn new(edge: Edge, timestamp: u64) -> Self {
        Self {
            id: edge.id,
            version: 1,
            timestamp,
            edge_data: edge,
            valid_from: timestamp,
            valid_to: None,
            prev_version: None,
        }
    }

    /// Marca esta versión como eliminada (establece valid_to)
    pub fn with_valid_to(mut self, ts: u64) -> Self {
        self.valid_to = Some(ts);
        self
    }

    /// Verifica si esta versión es válida en un timestamp dado
    pub fn is_valid_at(&self, timestamp: u64) -> bool {
        timestamp >= self.valid_from
            && self.valid_to.map(|to| timestamp < to).unwrap_or(true)
    }

    /// Verifica si esta versión puede ser eliminada por GC
    pub fn is_gc_eligible(&self, cutoff_timestamp: u64) -> bool {
        match self.valid_to {
            Some(valid_to) => valid_to < cutoff_timestamp,
            None => false,
        }
    }
}

/// Estadísticas de Garbage Collection
#[derive(Debug, Clone, Default)]
pub struct GCStats {
    /// Número de nodos escaneados
    pub nodes_scanned: usize,
    /// Número de versiones eliminadas
    pub versions_deleted: usize,
    /// Bytes liberados (estimado)
    pub bytes_freed: usize,
    /// Duración del GC
    pub duration_ms: u64,
}

/// Configuración de Garbage Collection
#[derive(Debug, Clone)]
pub struct GCConfig {
    /// Timestamp de corte: versiones invalidadas antes de este tiempo serán eliminadas
    pub cutoff_timestamp: u64,

    /// Número mínimo de versiones a mantener por nodo (incluso si son elegibles para GC)
    /// Default: 1 (siempre mantener al menos la versión actual)
    pub min_versions_to_keep: usize,

    /// Máximo de nodos a procesar por ciclo de GC (0 = sin límite)
    pub max_nodes_per_cycle: usize,

    /// Si es true, solo reporta qué se eliminaría sin hacerlo (dry run)
    pub dry_run: bool,

    /// Si es true, el GC limitará cutoff_timestamp al horizonte seguro
    /// (timestamp mínimo de transacciones activas) para no borrar versiones
    /// que alguna transacción en vuelo aún necesita.
    pub use_active_horizon: bool,
}

impl Default for GCConfig {
    fn default() -> Self {
        Self {
            cutoff_timestamp: 0,
            min_versions_to_keep: 1,
            max_nodes_per_cycle: 0,
            dry_run: false,
            use_active_horizon: false,
        }
    }
}

impl GCConfig {
    /// Crea configuración para eliminar versiones más viejas que `age_ms` milisegundos
    pub fn older_than_ms(age_ms: u64) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or(std::time::Duration::from_secs(0))
            .as_millis() as u64;

        Self {
            cutoff_timestamp: now.saturating_sub(age_ms),
            ..Default::default()
        }
    }

    /// Crea configuración para eliminar versiones más viejas que `hours` horas
    pub fn older_than_hours(hours: u64) -> Self {
        Self::older_than_ms(hours * 60 * 60 * 1000)
    }

    /// Crea configuración para eliminar versiones más viejas que `days` días
    pub fn older_than_days(days: u64) -> Self {
        Self::older_than_ms(days * 24 * 60 * 60 * 1000)
    }

    /// Modo dry-run (solo reportar, no eliminar)
    pub fn dry_run(mut self) -> Self {
        self.dry_run = true;
        self
    }

    /// Activa el respeto del horizonte activo de transacciones.
    /// El GC no borrará versiones visibles a transacciones en vuelo.
    pub fn with_active_horizon(mut self) -> Self {
        self.use_active_horizon = true;
        self
    }

    /// Establecer mínimo de versiones a mantener
    pub fn keep_at_least(mut self, n: usize) -> Self {
        self.min_versions_to_keep = n;
        self
    }
}

/// Version Manager - maneja versiones de nodos y garbage collection
pub struct VersionManager {
    // Placeholder para futuras extensiones
}

impl Default for VersionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl VersionManager {
    pub fn new() -> Self {
        Self {}
    }

    /// Obtiene la versión de un nodo en un timestamp específico
    pub async fn get_version_at(
        &self,
        _node_id: NodeId,
        _timestamp: u64,
    ) -> Result<Option<VersionedNode>> {
        // La lógica real está en Storage::get_node_at_timestamp
        Ok(None)
    }

    /// Obtiene el historial completo de un nodo
    pub async fn get_history(
        &self,
        _node_id: NodeId,
    ) -> Result<Vec<VersionedNode>> {
        // La lógica real está en Storage::get_node_history
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PropertyValue;

    #[test]
    fn test_versioned_node_creation() {
        let node = Node::new("Person")
            .with_property("name", PropertyValue::String("Alice".into()))
            .with_property("age", PropertyValue::Int(25));

        let v1 = VersionedNode::new(node, 100);

        assert_eq!(v1.version, 1);
        assert_eq!(v1.timestamp, 100);
        assert_eq!(v1.valid_from, 100);
        assert!(v1.valid_to.is_none());
        assert!(v1.is_valid_at(100));
        assert!(v1.is_valid_at(200));
    }

    #[test]
    fn test_version_chain() {
        let node1 = Node::new("Person")
            .with_property("age", PropertyValue::Int(25));

        let mut v1 = VersionedNode::new(node1, 100);

        // Update
        let node2 = Node::new("Person")
            .with_property("age", PropertyValue::Int(30));

        let v2 = VersionedNode::new_version(&v1, node2, 200);

        // Invalidar v1
        v1.invalidate(200);

        // Verificar
        assert_eq!(v2.version, 2);
        assert_eq!(v2.prev_version, Some(1));
        assert!(v1.is_valid_at(150));
        assert!(!v1.is_valid_at(200));
        assert!(v2.is_valid_at(200));
    }

    #[test]
    fn test_is_valid_at() {
        let node = Node::new("Test");
        let mut v = VersionedNode::new(node, 100);

        assert!(!v.is_valid_at(50));   // Antes de valid_from
        assert!(v.is_valid_at(100));   // En valid_from
        assert!(v.is_valid_at(150));   // Durante validez

        v.invalidate(200);

        assert!(v.is_valid_at(150));   // Todavía válido
        assert!(!v.is_valid_at(200));  // Invalidado en 200
        assert!(!v.is_valid_at(250));  // Después de invalidación
    }

    #[test]
    fn test_gc_eligibility() {
        let node = Node::new("Test");
        let mut v = VersionedNode::new(node, 100);

        // Versión actual (sin valid_to) no es elegible
        assert!(!v.is_gc_eligible(500));

        // Invalidar
        v.invalidate(200);

        // Ahora es elegible si cutoff > valid_to
        assert!(!v.is_gc_eligible(100)); // cutoff antes de valid_to
        assert!(!v.is_gc_eligible(200)); // cutoff == valid_to
        assert!(v.is_gc_eligible(201));  // cutoff > valid_to
        assert!(v.is_gc_eligible(500));  // cutoff >> valid_to
    }

    #[test]
    fn test_gc_config() {
        let config = GCConfig::older_than_hours(24);
        assert!(config.cutoff_timestamp > 0);
        assert_eq!(config.min_versions_to_keep, 1);
        assert!(!config.dry_run);

        let config = GCConfig::older_than_days(7).dry_run().keep_at_least(2);
        assert!(config.dry_run);
        assert_eq!(config.min_versions_to_keep, 2);
    }
}