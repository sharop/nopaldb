// src/mvcc/mod.rs
//
// Multi-Version Concurrency Control implementation
// Includes Garbage Collection for old versions

use serde::{Serialize, Deserialize};
use crate::types::{Node, Edge, NodeId, EdgeId};

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
///
/// Dos límites, ambos en la escala del RELOJ LÓGICO de la base (el de
/// `valid_from`/`valid_to`, no milisegundos Unix):
///
/// - `cutoff_timestamp`: corte lógico explícito ([`GCConfig::before_timestamp`]).
/// - `min_age_ms`: corte por antigüedad de pared ([`GCConfig::older_than_ms`]
///   y familia). El GC lo traduce a un corte lógico con las marcas
///   reloj-de-pared → reloj-lógico que la base registra al persistir sus
///   relojes; ver [`crate::graph::Graph::gc`].
///
/// Si están los dos, manda el más restrictivo (el menor).
#[derive(Debug, Clone)]
pub struct GCConfig {
    /// Corte lógico: versiones con `valid_to < cutoff_timestamp` son
    /// elegibles. Es un timestamp del reloj lógico de la base, NO un instante
    /// Unix. `u64::MAX` = sin límite lógico (el que ponen los constructores
    /// por antigüedad, que dejan el límite a `min_age_ms`).
    pub cutoff_timestamp: u64,

    /// Antigüedad mínima de pared (ms) que debe tener una versión cerrada
    /// para ser elegible. `None` = sin límite por antigüedad.
    ///
    /// Hasta 0.9.4 `older_than_ms` restaba la edad a `now()` en ms Unix y
    /// comparaba ese número con los `valid_to` LÓGICOS: el corte quedaba
    /// por encima de toda versión y `older_than_days(7)` borraba TODA la
    /// historia cerrada. Se descartó convertir la edad en timestamp lógico al
    /// construir la config (no hay base a mano para traducirla) y también
    /// quitar los cortes por edad (es lo que pide un scheduler de GC); la
    /// traducción se hace al ejecutar, con las marcas de reloj persistidas,
    /// y es conservadora: ante la duda, conserva.
    pub min_age_ms: Option<u64>,

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
            min_age_ms: None,
            min_versions_to_keep: 1,
            max_nodes_per_cycle: 0,
            dry_run: false,
            use_active_horizon: false,
        }
    }
}

impl GCConfig {
    /// Elimina versiones cerradas ANTES del timestamp lógico `ts`
    /// (`valid_to < ts`). `ts` es del reloj lógico de la base: por ejemplo,
    /// un valor obtenido con [`crate::graph::Graph::last_commit_timestamp`].
    pub fn before_timestamp(ts: u64) -> Self {
        Self {
            cutoff_timestamp: ts,
            ..Default::default()
        }
    }

    /// Elimina versiones que se cerraron hace al menos `age_ms` milisegundos
    /// de reloj de pared.
    ///
    /// La edad se traduce al reloj lógico al ejecutar el GC (ver
    /// [`GCConfig::min_age_ms`]). La traducción es conservadora: una versión
    /// solo se borra si la base puede probar que se cerró antes de
    /// `now - age_ms`; una base sin marcas de reloj suficientemente viejas
    /// (recién creada, o escrita por ≤ 0.9.4 hasta que pase `age_ms` desde
    /// la primera escritura con esta versión) no borra nada.
    pub fn older_than_ms(age_ms: u64) -> Self {
        Self {
            cutoff_timestamp: u64::MAX,
            min_age_ms: Some(age_ms),
            ..Default::default()
        }
    }

    /// Como [`GCConfig::older_than_ms`], en horas.
    pub fn older_than_hours(hours: u64) -> Self {
        Self::older_than_ms(hours.saturating_mul(60 * 60 * 1000))
    }

    /// Como [`GCConfig::older_than_ms`], en días.
    pub fn older_than_days(days: u64) -> Self {
        Self::older_than_ms(days.saturating_mul(24 * 60 * 60 * 1000))
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
        assert_eq!(config.min_age_ms, Some(24 * 60 * 60 * 1000));
        // Sin límite lógico propio: la edad se traduce al ejecutar.
        assert_eq!(config.cutoff_timestamp, u64::MAX);
        assert_eq!(config.min_versions_to_keep, 1);
        assert!(!config.dry_run);

        let config = GCConfig::older_than_days(7).dry_run().keep_at_least(2);
        assert!(config.dry_run);
        assert_eq!(config.min_versions_to_keep, 2);
    }
}