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

// ═════════════════════════════════════════════════════════════════════════════
// ⚠️ LEGACY layout v1 (de aquí hasta el módulo `v2`) — claves string de la
// sopa del keyspace `default` (`node:`/`idx:`/`ts:`/`meta:`). Tras F5.3/F5.4
// quedan SOLO para la migración de layout; ningún path nuevo debe usarlas.
// El layout v2 (binario, order-preserving, keyspaces separados) vive en el
// módulo `v2` de abajo.
// ═════════════════════════════════════════════════════════════════════════════

// ─── Prefijos de scan ────────────────────────────────────────────────────────

/// Prefijo del namespace de nodos en el keyspace default. LEGACY: lo usa la
/// migración de layout (F5.5) para copiar/limpiar bases v1; los nodos de
/// runtime viven en `entities` con claves de `v2` desde F5.4.
pub(crate) const NODE_PREFIX: &str = "node:";
/// Prefijo común de los índices de adyacencia (`idx:out:` / `idx:in:`).
/// LEGACY sin consumidores desde F5.3 (la adyacencia vive en el keyspace
/// `adjacency` con claves de `v2`). La migración F5.5 NO lo usa a propósito:
/// el borrado del legacy va por los prefijos exactos `idx:out:`/`idx:in:`
/// — el prefijo genérico pisaría `idx:prop:` (la sub-migración prop-idx).
#[allow(dead_code)]
pub(crate) const ADJ_PREFIX: &[u8] = b"idx:";
/// Prefijo del índice por timestamp. LEGACY: la migración F5.5 lo borra sin
/// copiarlo (el índice ts v2 se reconstruye desde `history`, des-blobeado).
pub(crate) const TS_PREFIX: &str = "ts:";
/// Prefijo del índice de propiedades LEGADO v1 (solo migración/limpieza; el
/// v2 vive en su propio keyspace con claves de `encode_property_index_key`).
pub(crate) const LEGACY_PROP_IDX_PREFIX: &[u8] = b"idx:prop:";

/// Prefijo de adyacencia saliente. LEGACY: la migración F5.5 lo borra sin
/// copiarlo (la adyacencia v2 se reconstruye desde `edges`, la fuente de
/// verdad — las claves huérfanas de nodos borrados mueren aquí).
pub(crate) const ADJ_OUT_PREFIX: &str = "idx:out:";
/// Prefijo de adyacencia entrante (espejo de `ADJ_OUT_PREFIX`).
pub(crate) const ADJ_IN_PREFIX: &str = "idx:in:";

// ─── Nombres meta LEGACY del tree default ────────────────────────────────────
// Las bases nuevas guardan estas metas en el keyspace `catalog`
// (`v2::catalog_meta_key` con los nombres SIN prefijo que exporta
// `storage::META_*`); estos nombres `meta:*` solo los lee la migración de
// layout (F5.5) sobre bases v1.

/// Prefijo del namespace meta del tree default (v1). La migración lo copia
/// a `catalog` (nombres sin prefijo) y luego lo borra — salvo la marca de
/// diagnóstico `LEGACY_META_LAYOUT_MIGRATED`.
pub(crate) const LEGACY_META_PREFIX: &str = "meta:";
#[allow(dead_code)]
pub(crate) const LEGACY_META_NEXT_TIMESTAMP: &str = "meta:next_timestamp";
#[allow(dead_code)]
pub(crate) const LEGACY_META_NEXT_TX_ID: &str = "meta:next_tx_id";
#[allow(dead_code)]
pub(crate) const LEGACY_META_PROP_IDX_FORMAT: &str = "meta:prop_idx_format";

/// Marca de diagnóstico que la migración F5.5 deja EN el tree default
/// (valor: u64 BE = 2). Los binarios ≤0.5.2 la ignoran (solo leen los tres
/// nombres meta conocidos); sirve a soporte para distinguir "base v1 vacía"
/// de "base migrada a v2 abierta con un binario viejo" — el downgrade
/// agravado del diseño F5 (invariante 8). Es la ÚNICA clave del default que
/// la limpieza del legacy conserva.
pub(crate) const LEGACY_META_LAYOUT_MIGRATED: &str = "meta:layout_migrated_to";

// ─── Constructores (keyspace default) ────────────────────────────────────────
// LEGACY todos desde F5.4: los reutilizará la migración de layout (F5.5);
// ningún path de runtime los llama ya.

/// Registro base de un nodo: `node:{uuid}`. LEGACY (F5.5).
#[allow(dead_code)]
pub(crate) fn node_key(id: NodeId) -> String {
    format!("{NODE_PREFIX}{id}")
}

/// Versión MVCC de un nodo: `node:{uuid}:v{n}` (n en decimal, sin padding).
/// LEGACY (F5.5).
#[allow(dead_code)]
pub(crate) fn node_version_key(id: NodeId, version: u64) -> String {
    format!("{NODE_PREFIX}{id}:v{version}")
}

/// Puntero a la versión current de un nodo: `node:{uuid}:current`.
/// LEGACY (F5.5).
#[allow(dead_code)]
pub(crate) fn node_current_key(id: NodeId) -> String {
    format!("{NODE_PREFIX}{id}:current")
}

/// Lista de versiones de un nodo: `node:{uuid}:versions`. LEGACY (F5.5).
#[allow(dead_code)]
pub(crate) fn node_versions_key(id: NodeId) -> String {
    format!("{NODE_PREFIX}{id}:versions")
}

/// Índice por timestamp: `ts:{n}` (n en decimal, sin padding — el orden
/// lexicográfico NO es el numérico; los consumidores parsean, no ordenan).
/// LEGACY (F5.5): el v2 des-blobea a `v2::ts_index_key` en `indexes`.
#[allow(dead_code)]
pub(crate) fn ts_key(ts: u64) -> String {
    format!("{TS_PREFIX}{ts}")
}

/// Adyacencia saliente de un nodo: `idx:out:{uuid}`. LEGACY sin
/// consumidores desde F5.3; lo usará la migración de layout (F5.5).
#[allow(dead_code)]
pub(crate) fn adjacency_out_key(id: NodeId) -> String {
    format!("{ADJ_OUT_PREFIX}{id}")
}

/// Adyacencia entrante de un nodo: `idx:in:{uuid}`. LEGACY sin
/// consumidores desde F5.3; lo usará la migración de layout (F5.5).
#[allow(dead_code)]
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

// ─── Clasificador estructural del namespace `node:` (LEGACY, F5.5) ──────────
// El namespace tiene exactamente cuatro formas: `node:{uuid}` (base),
// `node:{uuid}:v{n}` (versión MVCC), `node:{uuid}:current` y
// `node:{uuid}:versions`. La clasificación es por ESTRUCTURA (prefijo + UUID
// parseado + sufijo exacto) y no por substring: una blacklist tipo
// `contains(":v")` funciona hoy por accidente (un UUID no contiene `:`)
// pero se rompe en silencio el día que se agregue un sufijo nuevo.

/// Una clave del namespace `node:` del tree default, clasificada por
/// estructura. La migración de layout (F5.5) copia cada forma a su destino
/// v2 (entities/history) y FALLA FUERTE ante una forma desconocida — mejor
/// no migrar que perder datos en silencio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegacyNodeKey {
    /// `node:{uuid}` → `entities: n|{uuid16}`
    Base(NodeId),
    /// `node:{uuid}:v{n}` → `history: v|{uuid16}|{n BE}`
    Version(NodeId, u64),
    /// `node:{uuid}:current` → `history: c|{uuid16}`
    Current(NodeId),
    /// `node:{uuid}:versions` → `history: l|{uuid16}`
    Versions(NodeId),
}

/// Clasifica una clave del namespace `node:`; `None` si no es ninguna de las
/// cuatro formas conocidas (incluye claves fuera del namespace).
pub(crate) fn classify_legacy_node_key(key: &[u8]) -> Option<LegacyNodeKey> {
    let s = std::str::from_utf8(key).ok()?;
    let rest = s.strip_prefix(NODE_PREFIX)?;
    match rest.split_once(':') {
        None => Some(LegacyNodeKey::Base(uuid::Uuid::parse_str(rest).ok()?)),
        Some((uuid_part, suffix)) => {
            let id = uuid::Uuid::parse_str(uuid_part).ok()?;
            match suffix {
                "current" => Some(LegacyNodeKey::Current(id)),
                "versions" => Some(LegacyNodeKey::Versions(id)),
                _ => {
                    let digits = suffix.strip_prefix('v')?;
                    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                        return None;
                    }
                    Some(LegacyNodeKey::Version(id, digits.parse().ok()?))
                }
            }
        }
    }
}

/// Clave de nodo base: `node:{uuid}` exacto. (Azúcar sobre el clasificador;
/// pinneado por tests, sin consumidor de runtime — la migración usa
/// `classify_legacy_node_key` directo.)
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn is_base_node_key(key: &[u8]) -> bool {
    matches!(classify_legacy_node_key(key), Some(LegacyNodeKey::Base(_)))
}

/// Clave de versión MVCC: `node:{uuid}:v{n}` exacto (n = dígitos). Ver
/// `is_base_node_key`.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn is_version_node_key(key: &[u8]) -> bool {
    matches!(classify_legacy_node_key(key), Some(LegacyNodeKey::Version(..)))
}

/// ¿La clave pertenece al layout v1 y debe morir en la limpieza post-
/// migración (fase E de F5.5)?
///
/// Predicados EXACTOS, jamás el prefijo genérico `idx:`: `idx:prop:*` NO es
/// del layout — pertenece a la sub-migración del índice de propiedades, que
/// corre después y lo borra ella misma. La marca de diagnóstico
/// `LEGACY_META_LAYOUT_MIGRATED` tampoco muere (es post-migración). Una
/// clave que no clasifica (p. ej. un sufijo `node:` desconocido) se
/// CONSERVA: la limpieza es conservadora por diseño.
pub(crate) fn is_legacy_layout_key(key: &[u8]) -> bool {
    if key == LEGACY_META_LAYOUT_MIGRATED.as_bytes() {
        return false;
    }
    if key.starts_with(LEGACY_META_PREFIX.as_bytes())
        || key.starts_with(TS_PREFIX.as_bytes())
        || key.starts_with(ADJ_OUT_PREFIX.as_bytes())
        || key.starts_with(ADJ_IN_PREFIX.as_bytes())
    {
        return true;
    }
    classify_legacy_node_key(key).is_some()
}

// ═════════════════════════════════════════════════════════════════════════════
// Layout v2 (F5) — claves binarias order-preserving
// ═════════════════════════════════════════════════════════════════════════════

/// Codec del layout v2: claves binarias de los keyspaces `catalog` /
/// `entities` / `history` / `adjacency` / `indexes` (F5). Todos los enteros
/// van big-endian: el orden lexicográfico de bytes ES el orden numérico
/// (invariante de orden del contrato KV, pinneada en `conformance.rs`).
///
/// ⚠️ FORMATO EN DISCO — misma advertencia que la cabecera del archivo:
/// cambiar cualquier constructor exige bump de formato + migración.
///
/// Todos los namespaces tienen consumidores de runtime desde F5.4; los
/// allow(dead_code) que quedan son puntuales (sentinels y prefijos tipados
/// que estrenan F5.5 y los reads por tipo post-F5).
pub(crate) mod v2 {
    use crate::types::{EdgeId, NodeId};

    // ─── Keyspace `entities`: `n|{uuid16}` ──────────────────────────────────

    /// Discriminador del registro base de un nodo.
    pub(crate) const ENTITY_TAG: u8 = b'n';
    /// Longitud fija de la clave de entidad: tag + uuid16.
    pub(crate) const ENTITY_KEY_LEN: usize = 17;

    /// Registro base de un nodo: `n` + uuid16 (17 bytes).
    pub(crate) fn node_key_v2(id: NodeId) -> [u8; ENTITY_KEY_LEN] {
        let mut key = [0u8; ENTITY_KEY_LEN];
        key[0] = ENTITY_TAG;
        key[1..17].copy_from_slice(id.as_bytes());
        key
    }

    /// Parser estricto de `node_key_v2`: rechaza longitud ≠ 17 y tag ajeno.
    /// (Sin consumidor de runtime: los reads de `entities` deserializan el
    /// valor y el id viaja dentro del Node; lo usará la verificación de la
    /// migración F5.5. Pinneado por tests.)
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn parse_node_key_v2(key: &[u8]) -> Option<NodeId> {
        if key.len() != ENTITY_KEY_LEN || key[0] != ENTITY_TAG {
            return None;
        }
        Some(NodeId::from_bytes(key[1..17].try_into().ok()?))
    }

    // ─── Keyspace `history`: `v|{uuid16}|{ver8 BE}` · `c|{uuid16}` · `l|{uuid16}` ──

    /// Discriminador de versión MVCC de un nodo.
    pub(crate) const HISTORY_VERSION_TAG: u8 = b'v';
    /// Discriminador del puntero a la versión current.
    pub(crate) const HISTORY_CURRENT_TAG: u8 = b'c';
    /// Discriminador de la lista de versiones de un nodo.
    pub(crate) const HISTORY_VERSIONS_TAG: u8 = b'l';

    /// Versión MVCC de un nodo: `v` + uuid16 + versión u64 BE (25 bytes).
    /// El scan por `history_version_prefix` recorre las versiones en orden
    /// numérico (BE).
    pub(crate) fn history_version_key(id: NodeId, version: u64) -> [u8; 25] {
        let mut key = [0u8; 25];
        key[0] = HISTORY_VERSION_TAG;
        key[1..17].copy_from_slice(id.as_bytes());
        key[17..25].copy_from_slice(&version.to_be_bytes());
        key
    }

    /// Prefijo de scan de TODAS las versiones de un nodo: `v` + uuid16
    /// (17 bytes). El uuid es de longitud fija, así que el prefijo no captura
    /// versiones de otros nodos. (Sin consumidor de runtime: los reads de
    /// historia van por la lista `l|`; candidato natural del purge por nodo
    /// de F5.5. Pinneado por tests.)
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn history_version_prefix(id: NodeId) -> [u8; 17] {
        let mut key = [0u8; 17];
        key[0] = HISTORY_VERSION_TAG;
        key[1..17].copy_from_slice(id.as_bytes());
        key
    }

    /// Decodifica una clave de versión MVCC (`v` + uuid16 + ver u64 BE) como
    /// `(nodo, versión)`. RECHAZA longitud ≠ 25 y tag ajeno. Consumidores:
    /// el rebuild del índice ts y la verificación de la migración F5.5.
    pub(crate) fn parse_history_version_key(key: &[u8]) -> Option<(NodeId, u64)> {
        if key.len() != 25 || key[0] != HISTORY_VERSION_TAG {
            return None;
        }
        let id = NodeId::from_bytes(key[1..17].try_into().ok()?);
        let version = u64::from_be_bytes(key[17..25].try_into().ok()?);
        Some((id, version))
    }

    /// Puntero a la versión current de un nodo: `c` + uuid16 (17 bytes).
    pub(crate) fn history_current_key(id: NodeId) -> [u8; 17] {
        let mut key = [0u8; 17];
        key[0] = HISTORY_CURRENT_TAG;
        key[1..17].copy_from_slice(id.as_bytes());
        key
    }

    /// Parser estricto de `history_current_key` (`c` + uuid16): rechaza
    /// longitud ≠ 17 y tag ajeno. Consumidor: la verificación de la
    /// migración F5.5 ("todo `c|` apunta a una versión existente").
    pub(crate) fn parse_history_current_key(key: &[u8]) -> Option<NodeId> {
        if key.len() != 17 || key[0] != HISTORY_CURRENT_TAG {
            return None;
        }
        Some(NodeId::from_bytes(key[1..17].try_into().ok()?))
    }

    /// Lista de versiones de un nodo: `l` + uuid16 (17 bytes).
    pub(crate) fn history_versions_key(id: NodeId) -> [u8; 17] {
        let mut key = [0u8; 17];
        key[0] = HISTORY_VERSIONS_TAG;
        key[1..17].copy_from_slice(id.as_bytes());
        key
    }

    /// Parser estricto de `history_versions_key` (`l` + uuid16): rechaza
    /// longitud ≠ 17 y tag ajeno. El scan de "todos los nodos con historia"
    /// (GC) recorre el prefijo `[HISTORY_VERSIONS_TAG]` y decodifica con
    /// esto — en v1 eso era filtrar sufijos `:versions` de la sopa.
    pub(crate) fn parse_history_versions_key(key: &[u8]) -> Option<NodeId> {
        if key.len() != 17 || key[0] != HISTORY_VERSIONS_TAG {
            return None;
        }
        Some(NodeId::from_bytes(key[1..17].try_into().ok()?))
    }

    // ─── Keyspace `adjacency`: contrato de 53 bytes ─────────────────────────
    //
    // `O|{node16}|{etype4 BE}|{other16}|{edge16}` y espejo `I|...`:
    //   byte 0      dir: 0x4F ('O', saliente) / 0x49 ('I', entrante)
    //   bytes 1-16  nodo dueño de la lista (src en O, tgt en I)
    //   bytes 17-20 tipo de arista internado, u32 BE (EdgeTypeInterner)
    //   bytes 21-36 el otro extremo (tgt en O, src en I)
    //   bytes 37-52 EdgeId (multi-aristas permitidas: el id desambigua)
    // Valor: vacío — toda la información viaja en la clave. Los campos son de
    // longitud FIJA: cada prefijo formal es frontera exacta, sin escapes.

    /// Longitud fija de una clave de adyacencia. Los parsers RECHAZAN
    /// cualquier otra longitud.
    pub(crate) const ADJ_KEY_LEN: usize = 53;
    /// Discriminador de adyacencia saliente (`'O'`).
    pub(crate) const ADJ_DIR_OUT: u8 = 0x4F;
    /// Discriminador de adyacencia entrante (`'I'`).
    pub(crate) const ADJ_DIR_IN: u8 = 0x49;

    /// Dirección de una entrada de adyacencia (decodificada del byte 0).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum AdjDir {
        Out,
        In,
    }

    fn adj_key(dir: u8, node: NodeId, etype: u32, other: NodeId, edge: EdgeId) -> [u8; ADJ_KEY_LEN] {
        let mut key = [0u8; ADJ_KEY_LEN];
        key[0] = dir;
        key[1..17].copy_from_slice(node.as_bytes());
        key[17..21].copy_from_slice(&etype.to_be_bytes());
        key[21..37].copy_from_slice(other.as_bytes());
        key[37..53].copy_from_slice(edge.as_bytes());
        key
    }

    /// Entrada saliente: `O|src|etype|tgt|edge`.
    pub(crate) fn adj_out_key(src: NodeId, etype: u32, tgt: NodeId, edge: EdgeId) -> [u8; ADJ_KEY_LEN] {
        adj_key(ADJ_DIR_OUT, src, etype, tgt, edge)
    }

    /// Espejo entrante: `I|tgt|etype|src|edge`. Misma firma que `adj_out_key`
    /// a propósito — el constructor voltea los extremos, el call site pasa
    /// siempre `(src, etype, tgt, edge)` y no puede equivocarse de lado.
    pub(crate) fn adj_in_key(src: NodeId, etype: u32, tgt: NodeId, edge: EdgeId) -> [u8; ADJ_KEY_LEN] {
        adj_key(ADJ_DIR_IN, tgt, etype, src, edge)
    }

    /// Prefijo de TODA la adyacencia saliente de un nodo: `O` + uuid16 (17).
    pub(crate) fn adj_out_prefix(node: NodeId) -> [u8; 17] {
        let mut key = [0u8; 17];
        key[0] = ADJ_DIR_OUT;
        key[1..17].copy_from_slice(node.as_bytes());
        key
    }

    /// Prefijo de TODA la adyacencia entrante de un nodo: `I` + uuid16 (17).
    pub(crate) fn adj_in_prefix(node: NodeId) -> [u8; 17] {
        let mut key = [0u8; 17];
        key[0] = ADJ_DIR_IN;
        key[1..17].copy_from_slice(node.as_bytes());
        key
    }

    /// Prefijo de la adyacencia saliente de un nodo POR TIPO:
    /// `O` + uuid16 + etype u32 BE (21 bytes) — el neighbors-por-tipo del
    /// benchmark, sin N+1. (Consumidor de runtime: los reads por tipo
    /// post-F5; hoy lo pinnean los tests de frontera.)
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn adj_out_typed_prefix(node: NodeId, etype: u32) -> [u8; 21] {
        let mut key = [0u8; 21];
        key[..17].copy_from_slice(&adj_out_prefix(node));
        key[17..21].copy_from_slice(&etype.to_be_bytes());
        key
    }

    /// Espejo entrante de `adj_out_typed_prefix` (21 bytes).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn adj_in_typed_prefix(node: NodeId, etype: u32) -> [u8; 21] {
        let mut key = [0u8; 21];
        key[..17].copy_from_slice(&adj_in_prefix(node));
        key[17..21].copy_from_slice(&etype.to_be_bytes());
        key
    }

    /// Decodifica una clave de adyacencia como `(dir, nodo, etype, otro,
    /// edge)` — `nodo` es el dueño de la lista (src en Out, tgt en In) y
    /// `otro` el extremo opuesto. RECHAZA longitud ≠ 53 (truncada o con
    /// bytes extra) y discriminador desconocido.
    pub(crate) fn parse_adj_key(key: &[u8]) -> Option<(AdjDir, NodeId, u32, NodeId, EdgeId)> {
        if key.len() != ADJ_KEY_LEN {
            return None;
        }
        let dir = match key[0] {
            ADJ_DIR_OUT => AdjDir::Out,
            ADJ_DIR_IN => AdjDir::In,
            _ => return None,
        };
        let node = NodeId::from_bytes(key[1..17].try_into().ok()?);
        let etype = u32::from_be_bytes(key[17..21].try_into().ok()?);
        let other = NodeId::from_bytes(key[21..37].try_into().ok()?);
        let edge = EdgeId::from_bytes(key[37..53].try_into().ok()?);
        Some((dir, node, etype, other, edge))
    }

    // ─── Keyspace `indexes`: `t|{ts8 BE}|{uuid16}|{ver8 BE}` ────────────────
    //
    // La versión va EN la clave (invariante 6 del diseño F5): el reloj lógico
    // no garantiza unicidad por nodo, así que (ts, nodo) solo no basta.

    /// Discriminador del índice por timestamp.
    pub(crate) const TS_TAG: u8 = b't';

    /// Entrada del índice ts: `t` + ts u64 BE + uuid16 + versión u64 BE
    /// (33 bytes). Valor vacío (se des-blobea el `ts:{n}` → Vec<NodeId>).
    pub(crate) fn ts_index_key(ts: u64, id: NodeId, version: u64) -> [u8; 33] {
        let mut key = [0u8; 33];
        key[0] = TS_TAG;
        key[1..9].copy_from_slice(&ts.to_be_bytes());
        key[9..25].copy_from_slice(id.as_bytes());
        key[25..33].copy_from_slice(&version.to_be_bytes());
        key
    }

    /// Prefijo de scan de un timestamp exacto: `t` + ts u64 BE (9 bytes).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn ts_prefix(ts: u64) -> [u8; 9] {
        let mut key = [0u8; 9];
        key[0] = TS_TAG;
        key[1..9].copy_from_slice(&ts.to_be_bytes());
        key
    }

    /// Decodifica una entrada del índice ts como `(ts, nodo, versión)`.
    /// RECHAZA longitud ≠ 33 y tag ajeno.
    pub(crate) fn parse_ts_index_key(key: &[u8]) -> Option<(u64, NodeId, u64)> {
        if key.len() != 33 || key[0] != TS_TAG {
            return None;
        }
        let ts = u64::from_be_bytes(key[1..9].try_into().ok()?);
        let id = NodeId::from_bytes(key[9..25].try_into().ok()?);
        let version = u64::from_be_bytes(key[25..33].try_into().ok()?);
        Some((ts, id, version))
    }

    // ─── Keyspace `catalog`: `m|{nombre}` · `et|{u32 BE}` · `etn|{nombre}` ──

    /// Prefijo de las entradas meta del catalog (sustituyen a `meta:*`).
    pub(crate) const CATALOG_META_TAG: u8 = b'm';
    /// Prefijo id→nombre del interning de edge-types (clave de 6 bytes:
    /// `et` + u32 BE). También es el prefijo de scan del namespace completo
    /// del interning (`et*`: ids, nombres y contador).
    pub(crate) const EDGE_TYPE_ID_PREFIX: &[u8] = b"et";
    /// Longitud fija de una clave id→nombre: `et` + u32 BE.
    pub(crate) const EDGE_TYPE_ID_KEY_LEN: usize = 6;
    /// Prefijo nombre→id del interning (`etn` + bytes UTF-8 exactos).
    pub(crate) const EDGE_TYPE_NAME_PREFIX: &[u8] = b"etn";
    /// Clave del contador de ids de edge-type (valor: u32 BE = último id
    /// asignado; el próximo es contador+1; jamás decrece ni se reciclan ids).
    pub(crate) const EDGE_TYPE_COUNTER_KEY: &[u8] = b"etc";

    /// Cota superior de los ids de edge-type. 0x6DFF_FFFF mantiene el tercer
    /// byte de `et|{id BE}` fuera de `'n'` (0x6E): con un id mayor, la clave
    /// `et` + u32 BE sería byte a byte indistinguible de `etn` + nombre de
    /// 3 bytes y el load del interner no podría clasificarla. 1.8e9 tipos de
    /// arista quedan dentro del cap; `EdgeTypeInterner::intern` lo impone.
    pub(crate) const MAX_EDGE_TYPE_ID: u32 = 0x6DFF_FFFF;

    /// Nombre meta del sentinel de formato de layout (`layout_format=2` se
    /// escribe al ACTIVAR la migración F5.5, ANTES de limpiar el legacy:
    /// significa "layout v2 completo y verificado", no "cero bytes legacy").
    pub(crate) const META_LAYOUT_FORMAT: &str = "layout_format";
    /// Nombre meta de la máquina de estados de la migración de layout
    /// (`copying`/`rebuilding`/`verified`/`complete`, reanudable).
    pub(crate) const META_LAYOUT_MIGRATION_STATE: &str = "layout_migration_state";
    /// Nombre meta de la marca de limpieza idempotente del legacy. Si falta
    /// con `layout_format=2`, el próximo open solo reanuda la limpieza.
    pub(crate) const META_LEGACY_CLEANUP_DONE: &str = "legacy_cleanup_done";

    /// Entrada meta del catalog: `m` + nombre UTF-8 (sin separador: el
    /// namespace `m` solo contiene metas y el nombre es el resto de la clave).
    pub(crate) fn catalog_meta_key(name: &str) -> Vec<u8> {
        let mut key = Vec::with_capacity(1 + name.len());
        key.push(CATALOG_META_TAG);
        key.extend_from_slice(name.as_bytes());
        key
    }

    /// Interning id→nombre: `et` + u32 BE (6 bytes). Valor: bytes UTF-8
    /// exactos del nombre.
    pub(crate) fn edge_type_id_key(id: u32) -> [u8; EDGE_TYPE_ID_KEY_LEN] {
        let mut key = [0u8; EDGE_TYPE_ID_KEY_LEN];
        key[..2].copy_from_slice(EDGE_TYPE_ID_PREFIX);
        key[2..6].copy_from_slice(&id.to_be_bytes());
        key
    }

    /// Interning nombre→id: `etn` + bytes UTF-8 EXACTOS del nombre (sin
    /// normalización Unicode — ver el doc de `EdgeTypeInterner`). Valor:
    /// u32 BE.
    pub(crate) fn edge_type_name_key(name: &str) -> Vec<u8> {
        let mut key = Vec::with_capacity(EDGE_TYPE_NAME_PREFIX.len() + name.len());
        key.extend_from_slice(EDGE_TYPE_NAME_PREFIX);
        key.extend_from_slice(name.as_bytes());
        key
    }
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

    #[test]
    fn test_classify_legacy_node_key() {
        let uuid = "6f9619ff-8b86-d011-b42d-00c04fc964ff";
        let id = uuid::Uuid::parse_str(uuid).unwrap();

        assert_eq!(
            classify_legacy_node_key(format!("node:{uuid}").as_bytes()),
            Some(LegacyNodeKey::Base(id))
        );
        assert_eq!(
            classify_legacy_node_key(format!("node:{uuid}:v42").as_bytes()),
            Some(LegacyNodeKey::Version(id, 42))
        );
        assert_eq!(
            classify_legacy_node_key(format!("node:{uuid}:current").as_bytes()),
            Some(LegacyNodeKey::Current(id))
        );
        assert_eq!(
            classify_legacy_node_key(format!("node:{uuid}:versions").as_bytes()),
            Some(LegacyNodeKey::Versions(id))
        );
        // Formas desconocidas: None (la migración falla fuerte, no adivina).
        assert_eq!(classify_legacy_node_key(format!("node:{uuid}:vector").as_bytes()), None);
        assert_eq!(classify_legacy_node_key(format!("node:{uuid}:v").as_bytes()), None);
        assert_eq!(classify_legacy_node_key(format!("node:{uuid}:v1x").as_bytes()), None);
        assert_eq!(classify_legacy_node_key(b"node:not-a-uuid"), None);
        assert_eq!(classify_legacy_node_key(b"ts:42"), None);
    }

    #[test]
    fn test_is_legacy_layout_key_predicados_exactos() {
        let uuid = "6f9619ff-8b86-d011-b42d-00c04fc964ff";

        // Muere: las 4 formas node:, adyacencia out/in, ts:, meta:*.
        for k in [
            format!("node:{uuid}"),
            format!("node:{uuid}:v3"),
            format!("node:{uuid}:current"),
            format!("node:{uuid}:versions"),
            format!("idx:out:{uuid}"),
            format!("idx:in:{uuid}"),
            "ts:12345".to_string(),
            "meta:next_timestamp".to_string(),
            "meta:next_tx_id".to_string(),
            "meta:prop_idx_format".to_string(),
        ] {
            assert!(is_legacy_layout_key(k.as_bytes()), "{k} debe morir en el cleanup");
        }

        // Sobrevive: idx:prop:* (lo borra la sub-migración prop-idx, no el
        // layout), la marca de diagnóstico, y formas node: desconocidas.
        for k in [
            "idx:prop:edad:1".to_string(),
            LEGACY_META_LAYOUT_MIGRATED.to_string(),
            format!("node:{uuid}:vector"),
            "idx:otra_cosa".to_string(),
        ] {
            assert!(!is_legacy_layout_key(k.as_bytes()), "{k} debe sobrevivir al cleanup");
        }
    }

    // ─── Layout v2 ───────────────────────────────────────────────────────────

    use uuid::Uuid;

    const U00: Uuid = Uuid::from_bytes([0x00; 16]);
    const UFF: Uuid = Uuid::from_bytes([0xFF; 16]);

    fn mid_uuid() -> Uuid {
        Uuid::parse_str("6f9619ff-8b86-d011-b42d-00c04fc964ff").unwrap()
    }

    /// `sorted` byte a byte == orden de construcción (que es el numérico).
    fn assert_estrictamente_ordenadas(keys: &[Vec<u8>]) {
        for pair in keys.windows(2) {
            assert!(
                pair[0] < pair[1],
                "orden de bytes != orden numérico: {:02x?} !< {:02x?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn test_v2_node_key_roundtrip_y_rechazos() {
        for id in [U00, mid_uuid(), UFF] {
            let key = v2::node_key_v2(id);
            assert_eq!(key.len(), 17);
            assert_eq!(key[0], v2::ENTITY_TAG);
            assert_eq!(v2::parse_node_key_v2(&key), Some(id));
        }
        let key = v2::node_key_v2(mid_uuid());
        assert_eq!(v2::parse_node_key_v2(&key[..16]), None, "truncada");
        assert_eq!(v2::parse_node_key_v2(&[&key[..], &[0u8]].concat()), None, "bytes extra");
        let mut ajena = key;
        ajena[0] = b'x';
        assert_eq!(v2::parse_node_key_v2(&ajena), None, "tag ajeno");
    }

    #[test]
    fn test_v2_history_orden_be_y_prefijo() {
        let id = mid_uuid();
        // Fronteras de carry BE: el orden de bytes debe ser el numérico.
        let versions = [0u64, 1, 255, 256, 65535, 65536, u64::MAX - 1, u64::MAX];
        let keys: Vec<Vec<u8>> = versions
            .iter()
            .map(|v| v2::history_version_key(id, *v).to_vec())
            .collect();
        assert_estrictamente_ordenadas(&keys);

        // El prefijo v+uuid captura todas las versiones del nodo…
        let prefix = v2::history_version_prefix(id);
        assert_eq!(prefix.len(), 17);
        for key in &keys {
            assert!(key.starts_with(&prefix));
        }
        // …y NINGUNA clave de otro nodo ni de otro namespace del keyspace.
        assert!(!v2::history_version_key(UFF, 0).starts_with(&prefix));
        assert!(!v2::history_current_key(id).starts_with(&prefix));
        assert!(!v2::history_versions_key(id).starts_with(&prefix));

        // Fronteras con UUID extremos: el uuid es fijo de 16 bytes, así que
        // el prefijo de todo-0xFF no captura a nadie más.
        let pff = v2::history_version_prefix(UFF);
        assert!(v2::history_version_key(UFF, u64::MAX).starts_with(&pff));
        assert!(!v2::history_version_key(U00, u64::MAX).starts_with(&pff));

        // Los tres namespaces del keyspace history no colisionan.
        assert_ne!(v2::history_current_key(id)[0], v2::history_versions_key(id)[0]);
        assert_ne!(v2::history_current_key(id)[0], prefix[0]);
    }

    #[test]
    fn test_v2_history_version_y_current_key_roundtrip_y_rechazos() {
        for (id, ver) in [(U00, 0u64), (mid_uuid(), 42), (UFF, u64::MAX)] {
            let key = v2::history_version_key(id, ver);
            assert_eq!(v2::parse_history_version_key(&key), Some((id, ver)));
            let ckey = v2::history_current_key(id);
            assert_eq!(v2::parse_history_current_key(&ckey), Some(id));
        }
        let key = v2::history_version_key(mid_uuid(), 7);
        assert_eq!(v2::parse_history_version_key(&key[..24]), None, "truncada");
        assert_eq!(v2::parse_history_version_key(&[&key[..], &[0u8]].concat()), None, "26 bytes");
        // Tags cruzados: c| no parsea como v| ni al revés.
        assert_eq!(v2::parse_history_version_key(&v2::history_current_key(mid_uuid())), None);
        assert_eq!(v2::parse_history_current_key(&v2::history_version_key(mid_uuid(), 1)), None);
        assert_eq!(v2::parse_history_current_key(&v2::history_versions_key(mid_uuid())), None);
    }

    #[test]
    fn test_v2_history_versions_key_roundtrip_y_rechazos() {
        for id in [U00, mid_uuid(), UFF] {
            let key = v2::history_versions_key(id);
            assert_eq!(v2::parse_history_versions_key(&key), Some(id));
        }
        let key = v2::history_versions_key(mid_uuid());
        assert_eq!(v2::parse_history_versions_key(&key[..16]), None, "truncada");
        assert_eq!(
            v2::parse_history_versions_key(&[&key[..], &[0u8]].concat()),
            None,
            "bytes extra"
        );
        // Los otros dos namespaces del keyspace NO clasifican como lista.
        assert_eq!(v2::parse_history_versions_key(&v2::history_current_key(mid_uuid())), None);
        assert_eq!(
            v2::parse_history_versions_key(&v2::history_version_key(mid_uuid(), 1)),
            None
        );
    }

    #[test]
    fn test_v2_adj_contrato_53_bytes_roundtrip() {
        // etype en las fronteras del rango completo, UUIDs extremos.
        for (src, tgt, edge) in [(U00, UFF, mid_uuid()), (UFF, U00, U00), (UFF, UFF, UFF)] {
            for etype in [0u32, 1, 255, 256, u32::MAX - 1, u32::MAX] {
                let out = v2::adj_out_key(src, etype, tgt, edge);
                assert_eq!(out.len(), v2::ADJ_KEY_LEN);
                assert_eq!(out[0], v2::ADJ_DIR_OUT);
                assert_eq!(
                    v2::parse_adj_key(&out),
                    Some((v2::AdjDir::Out, src, etype, tgt, edge)),
                    "en Out el dueño es src y el otro extremo tgt"
                );

                let inn = v2::adj_in_key(src, etype, tgt, edge);
                assert_eq!(inn.len(), v2::ADJ_KEY_LEN);
                assert_eq!(inn[0], v2::ADJ_DIR_IN);
                assert_eq!(
                    v2::parse_adj_key(&inn),
                    Some((v2::AdjDir::In, tgt, etype, src, edge)),
                    "en In el dueño es tgt y el otro extremo src (espejo)"
                );
            }
        }
    }

    #[test]
    fn test_v2_adj_parse_rechaza_malformadas() {
        let key = v2::adj_out_key(mid_uuid(), 7, UFF, U00);
        assert!(v2::parse_adj_key(&key).is_some());
        assert_eq!(v2::parse_adj_key(&key[..52]), None, "truncada (52)");
        assert_eq!(v2::parse_adj_key(&key[..17]), None, "solo el prefijo");
        assert_eq!(v2::parse_adj_key(&[&key[..], &[0u8]].concat()), None, "54 bytes");
        assert_eq!(v2::parse_adj_key(b""), None, "vacía");
        let mut mal_dir = key;
        mal_dir[0] = b'X';
        assert_eq!(v2::parse_adj_key(&mal_dir), None, "discriminador desconocido");
    }

    #[test]
    fn test_v2_adj_orden_be_y_fronteras_de_prefijo() {
        let node = mid_uuid();

        // Orden: mismo nodo, etypes crecientes (con carrys BE) → bytes crecientes.
        let etypes = [0u32, 1, 255, 256, 65536, u32::MAX];
        let keys: Vec<Vec<u8>> = etypes
            .iter()
            .map(|et| v2::adj_out_key(node, *et, U00, U00).to_vec())
            .collect();
        assert_estrictamente_ordenadas(&keys);

        // adj_out_prefix captura TODO el abanico del nodo (etype 0..=MAX,
        // extremos y edges en 0x00/0xFF)…
        let p_node = v2::adj_out_prefix(node);
        assert_eq!(p_node.len(), 17);
        for key in &keys {
            assert!(key.starts_with(&p_node));
        }
        assert!(v2::adj_out_key(node, u32::MAX, UFF, UFF).starts_with(&p_node));
        // …y NO las claves de otro nodo ni del espejo I (aunque el nodo sea el mismo).
        assert!(!v2::adj_out_key(UFF, 0, U00, U00).starts_with(&p_node));
        assert!(!v2::adj_in_key(U00, 0, node, U00).starts_with(&p_node), "espejo I del mismo dueño");

        // typed_prefix es prefijo REAL de sus claves…
        for et in etypes {
            let p = v2::adj_out_typed_prefix(node, et);
            assert_eq!(p.len(), 21);
            assert!(v2::adj_out_key(node, et, U00, U00).starts_with(&p));
            assert!(v2::adj_out_key(node, et, UFF, UFF).starts_with(&p));
            // …y NO de las del tipo siguiente (incluye el carry 255→256).
            if et < u32::MAX {
                assert!(!v2::adj_out_key(node, et + 1, U00, U00).starts_with(&p));
            }
            if et > 0 {
                assert!(!v2::adj_out_key(node, et - 1, UFF, UFF).starts_with(&p));
            }
        }

        // Frontera dura: nodo todo-0xFF + etype MAX — nada ajeno cae dentro.
        let p_max = v2::adj_out_typed_prefix(UFF, u32::MAX);
        assert!(v2::adj_out_key(UFF, u32::MAX, UFF, UFF).starts_with(&p_max));
        assert!(!v2::adj_out_key(UFF, u32::MAX - 1, UFF, UFF).starts_with(&p_max));
        // Frontera baja: nodo todo-0x00 + etype 0.
        let p_min = v2::adj_in_typed_prefix(U00, 0);
        assert!(v2::adj_in_key(UFF, 0, U00, U00).starts_with(&p_min), "dueño U00 en el lado In");
        assert!(!v2::adj_in_key(UFF, 1, U00, U00).starts_with(&p_min));

        // Los espejos in son estructuralmente idénticos módulo discriminador.
        assert_eq!(v2::adj_in_prefix(node)[1..], v2::adj_out_prefix(node)[1..17]);
        assert_eq!(v2::adj_in_typed_prefix(node, 9)[1..], v2::adj_out_typed_prefix(node, 9)[1..21]);
    }

    #[test]
    fn test_v2_ts_orden_be_y_prefijo() {
        let id = mid_uuid();
        let tss = [0u64, 1, 255, 256, 65536, u64::MAX];
        let keys: Vec<Vec<u8>> = tss.iter().map(|ts| v2::ts_index_key(*ts, id, 1).to_vec()).collect();
        assert_estrictamente_ordenadas(&keys);

        // Con el MISMO (ts, nodo), versiones crecientes ordenan numérico.
        let vers: Vec<Vec<u8>> =
            [0u64, 255, 256, u64::MAX].iter().map(|v| v2::ts_index_key(7, id, *v).to_vec()).collect();
        assert_estrictamente_ordenadas(&vers);

        for ts in tss {
            let p = v2::ts_prefix(ts);
            assert_eq!(p.len(), 9);
            assert!(v2::ts_index_key(ts, U00, 0).starts_with(&p));
            assert!(v2::ts_index_key(ts, UFF, u64::MAX).starts_with(&p));
            // Frontera: ni el ts siguiente con uuid 0x00 ni el anterior con 0xFF.
            if ts < u64::MAX {
                assert!(!v2::ts_index_key(ts + 1, U00, 0).starts_with(&p));
            }
            if ts > 0 {
                assert!(!v2::ts_index_key(ts - 1, UFF, u64::MAX).starts_with(&p));
            }
        }
    }

    #[test]
    fn test_v2_ts_index_key_roundtrip_y_rechazos() {
        for (ts, id, ver) in [(0u64, U00, 0u64), (7, mid_uuid(), 42), (u64::MAX, UFF, u64::MAX)] {
            let key = v2::ts_index_key(ts, id, ver);
            assert_eq!(v2::parse_ts_index_key(&key), Some((ts, id, ver)));
        }
        let key = v2::ts_index_key(7, mid_uuid(), 42);
        assert_eq!(v2::parse_ts_index_key(&key[..32]), None, "truncada");
        assert_eq!(v2::parse_ts_index_key(&[&key[..], &[0u8]].concat()), None, "34 bytes");
        let mut ajena = key;
        ajena[0] = b'x';
        assert_eq!(v2::parse_ts_index_key(&ajena), None, "tag ajeno");
    }

    #[test]
    fn test_v2_catalog_keys() {
        // Meta: `m` + nombre exacto.
        assert_eq!(v2::catalog_meta_key(v2::META_LAYOUT_FORMAT), b"mlayout_format".to_vec());

        // et|id ordena numéricamente y el roundtrip del id es por BE.
        let ids = [1u32, 2, 255, 256, v2::MAX_EDGE_TYPE_ID];
        let keys: Vec<Vec<u8>> = ids.iter().map(|id| v2::edge_type_id_key(*id).to_vec()).collect();
        assert_estrictamente_ordenadas(&keys);
        for (id, key) in ids.iter().zip(&keys) {
            assert_eq!(key.len(), v2::EDGE_TYPE_ID_KEY_LEN);
            assert_eq!(u32::from_be_bytes(key[2..6].try_into().unwrap()), *id);
        }

        // etn|nombre lleva los bytes UTF-8 EXACTOS, sin normalizar.
        assert_eq!(v2::edge_type_name_key("amigo-de"), b"etnamigo-de".to_vec());
        assert_eq!(v2::edge_type_name_key("caf\u{e9}"), b"etncaf\xc3\xa9".to_vec());
        assert_ne!(v2::edge_type_name_key("caf\u{e9}"), v2::edge_type_name_key("cafe\u{301}"));

        // El namespace et* es disjunto por estructura: contador (3 bytes),
        // ids (6 bytes con byte[2] < 'n' por el cap), nombres (etn + ≥0).
        assert_eq!(v2::EDGE_TYPE_COUNTER_KEY.len(), 3);
        assert!(v2::edge_type_id_key(v2::MAX_EDGE_TYPE_ID)[2] < b'n');
        // El primer id FUERA del cap es exactamente el que colisionaría con
        // `etn` + nombre de 3 bytes — el hazard que MAX_EDGE_TYPE_ID impide:
        assert!(v2::edge_type_id_key(v2::MAX_EDGE_TYPE_ID + 1).starts_with(v2::EDGE_TYPE_NAME_PREFIX));

        // Todos cuelgan del prefijo de scan del interning.
        for k in [
            v2::edge_type_id_key(1).to_vec(),
            v2::edge_type_name_key("x"),
            v2::EDGE_TYPE_COUNTER_KEY.to_vec(),
        ] {
            assert!(k.starts_with(v2::EDGE_TYPE_ID_PREFIX));
        }
        // …y las metas NO (namespace `m`).
        assert!(!v2::catalog_meta_key(v2::META_LAYOUT_MIGRATION_STATE).starts_with(v2::EDGE_TYPE_ID_PREFIX));
        assert!(!v2::catalog_meta_key(v2::META_LEGACY_CLEANUP_DONE).starts_with(v2::EDGE_TYPE_ID_PREFIX));
    }
}
