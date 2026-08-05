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
/// (allow(dead_code) fuera de tests: los consumidores llegan con el rewire
/// de los paths de dominio en F5.3/F5.4 y entonces el allow se retira.)
#[cfg_attr(not(test), allow(dead_code))]
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
    /// versiones de otros nodos.
    pub(crate) fn history_version_prefix(id: NodeId) -> [u8; 17] {
        let mut key = [0u8; 17];
        key[0] = HISTORY_VERSION_TAG;
        key[1..17].copy_from_slice(id.as_bytes());
        key
    }

    /// Puntero a la versión current de un nodo: `c` + uuid16 (17 bytes).
    pub(crate) fn history_current_key(id: NodeId) -> [u8; 17] {
        let mut key = [0u8; 17];
        key[0] = HISTORY_CURRENT_TAG;
        key[1..17].copy_from_slice(id.as_bytes());
        key
    }

    /// Lista de versiones de un nodo: `l` + uuid16 (17 bytes).
    pub(crate) fn history_versions_key(id: NodeId) -> [u8; 17] {
        let mut key = [0u8; 17];
        key[0] = HISTORY_VERSIONS_TAG;
        key[1..17].copy_from_slice(id.as_bytes());
        key
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
    /// benchmark, sin N+1.
    pub(crate) fn adj_out_typed_prefix(node: NodeId, etype: u32) -> [u8; 21] {
        let mut key = [0u8; 21];
        key[..17].copy_from_slice(&adj_out_prefix(node));
        key[17..21].copy_from_slice(&etype.to_be_bytes());
        key
    }

    /// Espejo entrante de `adj_out_typed_prefix` (21 bytes).
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
    pub(crate) fn ts_prefix(ts: u64) -> [u8; 9] {
        let mut key = [0u8; 9];
        key[0] = TS_TAG;
        key[1..9].copy_from_slice(&ts.to_be_bytes());
        key
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
    /// escribe al ACTIVAR la migración F5, ANTES de limpiar el legacy).
    pub(crate) const META_LAYOUT_FORMAT: &str = "layout_format";
    /// Nombre meta de la máquina de estados de la migración de layout.
    pub(crate) const META_LAYOUT_MIGRATION_STATE: &str = "layout_migration_state";
    /// Nombre meta de la marca de limpieza idempotente del legacy.
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
