// src/storage/keys.rs
//
// Codec centralizado de las claves compuestas de los keyspaces `default`
// (namespaces `node:` / `idx:` / `ts:`) y `versioned_edges`.
//
// ⚠️ FORMATO EN DISCO — cambiar cualquier constructor de este módulo requiere
// bump de formato + migración (patrón `meta:prop_idx_format` de 0.4.36). Una
// clave compuesta jamás se arma con `format!` fuera de este módulo.
//
// El encode del índice de propiedades v2 (`encode_property_index_key`) es el
// mismo patrón — clave en disco, única fn de encode — y vive junto al índice
// en `storage/mod.rs`; NO se muda aquí.

use crate::types::{EdgeId, NodeId};

// ─── Prefijos de scan ────────────────────────────────────────────────────────

/// Prefijo del namespace de nodos en el keyspace default.
pub(crate) const NODE_PREFIX: &str = "node:";
/// Prefijo común de los índices de adyacencia (`idx:out:` / `idx:in:`).
pub(crate) const ADJ_PREFIX: &[u8] = b"idx:";
/// Prefijo del índice por timestamp.
pub(crate) const TS_PREFIX: &str = "ts:";
/// Prefijo del índice de propiedades LEGADO v1 (solo migración/limpieza; el
/// v2 vive en su propio keyspace con claves de `encode_property_index_key`).
pub(crate) const LEGACY_PROP_IDX_PREFIX: &[u8] = b"idx:prop:";

/// Prefijo de adyacencia saliente (subconjunto de `ADJ_PREFIX`).
pub(crate) const ADJ_OUT_PREFIX: &str = "idx:out:";
/// Prefijo de adyacencia entrante (subconjunto de `ADJ_PREFIX`).
pub(crate) const ADJ_IN_PREFIX: &str = "idx:in:";

// ─── Constructores (keyspace default) ────────────────────────────────────────

/// Registro base de un nodo: `node:{uuid}`.
pub(crate) fn node_key(id: NodeId) -> String {
    format!("{NODE_PREFIX}{id}")
}

/// Versión MVCC de un nodo: `node:{uuid}:v{n}` (n en decimal, sin padding).
pub(crate) fn node_version_key(id: NodeId, version: u64) -> String {
    format!("{NODE_PREFIX}{id}:v{version}")
}

/// Puntero a la versión current de un nodo: `node:{uuid}:current`.
pub(crate) fn node_current_key(id: NodeId) -> String {
    format!("{NODE_PREFIX}{id}:current")
}

/// Lista de versiones de un nodo: `node:{uuid}:versions`.
pub(crate) fn node_versions_key(id: NodeId) -> String {
    format!("{NODE_PREFIX}{id}:versions")
}

/// Índice por timestamp: `ts:{n}` (n en decimal, sin padding — el orden
/// lexicográfico NO es el numérico; los consumidores parsean, no ordenan).
pub(crate) fn ts_key(ts: u64) -> String {
    format!("{TS_PREFIX}{ts}")
}

/// Adyacencia saliente de un nodo: `idx:out:{uuid}`.
pub(crate) fn adjacency_out_key(id: NodeId) -> String {
    format!("{ADJ_OUT_PREFIX}{id}")
}

/// Adyacencia entrante de un nodo: `idx:in:{uuid}`.
pub(crate) fn adjacency_in_key(id: NodeId) -> String {
    format!("{ADJ_IN_PREFIX}{id}")
}

// ─── Constructores (keyspace versioned_edges) ────────────────────────────────

/// Versión MVCC de una arista: `{edge_id}:v{n:020}` (padding a 20 dígitos:
/// el orden lexicográfico del scan ES el orden numérico de versión).
pub(crate) fn edge_version_key(id: EdgeId, version: u64) -> String {
    format!("{id}:v{version:020}")
}

/// Prefijo de scan del historial de una arista: `{edge_id}:v`.
pub(crate) fn edge_versions_prefix(id: EdgeId) -> String {
    format!("{id}:v")
}

// ─── Predicados estructurales del namespace `node:` ─────────────────────────
// El namespace tiene exactamente cuatro formas: `node:{uuid}` (base),
// `node:{uuid}:v{n}` (versión MVCC), `node:{uuid}:current` y
// `node:{uuid}:versions`. Los filtros de scan clasifican por ESTRUCTURA
// (prefijo + UUID parseado + sufijo exacto) y no por substring: una blacklist
// tipo `contains(":v")` funciona hoy por accidente (un UUID no contiene `:`)
// pero se rompe en silencio el día que se agregue un sufijo nuevo.

/// Clave de nodo base: `node:{uuid}` exacto.
pub(crate) fn is_base_node_key(key: &[u8]) -> bool {
    match std::str::from_utf8(key) {
        Ok(s) => s
            .strip_prefix(NODE_PREFIX)
            .is_some_and(|rest| uuid::Uuid::parse_str(rest).is_ok()),
        Err(_) => false,
    }
}

/// Clave de versión MVCC: `node:{uuid}:v{n}` exacto (n = dígitos).
pub(crate) fn is_version_node_key(key: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(key) else {
        return false;
    };
    let Some(rest) = s.strip_prefix(NODE_PREFIX) else {
        return false;
    };
    let Some((uuid_part, suffix)) = rest.split_once(':') else {
        return false;
    };
    uuid::Uuid::parse_str(uuid_part).is_ok()
        && suffix
            .strip_prefix('v')
            .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_key_predicates() {
        let uuid = "6f9619ff-8b86-d011-b42d-00c04fc964ff";

        assert!(is_base_node_key(format!("node:{uuid}").as_bytes()));
        assert!(!is_base_node_key(format!("node:{uuid}:v1").as_bytes()));
        assert!(!is_base_node_key(format!("node:{uuid}:current").as_bytes()));
        assert!(!is_base_node_key(format!("node:{uuid}:versions").as_bytes()));
        assert!(!is_base_node_key(b"node:not-a-uuid"));
        assert!(!is_base_node_key(b"edge:whatever"));
        // Un sufijo futuro no clasifica como base (la blacklist vieja lo
        // habría dejado pasar o no según sus letras)
        assert!(!is_base_node_key(format!("node:{uuid}:vector").as_bytes()));

        assert!(is_version_node_key(format!("node:{uuid}:v1").as_bytes()));
        assert!(is_version_node_key(format!("node:{uuid}:v42").as_bytes()));
        assert!(!is_version_node_key(format!("node:{uuid}").as_bytes()));
        assert!(!is_version_node_key(format!("node:{uuid}:versions").as_bytes()));
        assert!(!is_version_node_key(format!("node:{uuid}:current").as_bytes()));
        // El caso que la whitelist vieja aceptaba y reventaba el export:
        assert!(!is_version_node_key(format!("node:{uuid}:vector").as_bytes()));
        assert!(!is_version_node_key(format!("node:{uuid}:v").as_bytes()));
    }
}
