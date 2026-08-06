// src/storage/mod.rs

pub mod backend;

use std::sync::Arc;
use std::path::Path;
use std::collections::HashMap;
use crate::error::{NopalError, Result};
use crate::types::{Node, Edge, NodeId, EdgeId, PropertyValue};
use crate::mvcc::{VersionedNode, VersionedEdge};
pub use backend::{StorageEngine, StorageOptions, StorageProfile, StorageTuning};
pub use kv::migrate::MigrationReport;

/// Nombre meta con la cota superior persistida del reloj lógico de
/// timestamps. Vive en el keyspace `catalog` (clave `m|next_timestamp`,
/// F5.4); en bases v1 era `meta:next_timestamp` en el tree default
/// (`keys::LEGACY_META_NEXT_TIMESTAMP`, solo migración F5.5).
pub const META_NEXT_TIMESTAMP: &str = "next_timestamp";

/// Versión del formato del índice de propiedades en disco. Ausente = formato
/// legado v1 (`idx:prop:{name}:{value_str}` en el tree default); `2` = claves
/// tipadas order-preserving en el tree `prop_idx_v2`. La migración corre en
/// `Graph::open_with_options` (ver `migrate_property_index_if_needed`).
/// El sentinel vive en `catalog` para bases nuevas (F5.4); el legacy
/// `meta:prop_idx_format` del tree default lo leerá F5.5.
pub const META_PROP_IDX_FORMAT: &str = "prop_idx_format";

/// Valor actual de `META_PROP_IDX_FORMAT`.
pub const PROP_IDX_FORMAT_CURRENT: u64 = 2;

/// Nombre del keyspace del índice de propiedades v2.
const PROP_IDX_TREE: &str = "prop_idx_v2";
/// Nombre del keyspace de aristas (registro current por EdgeId).
const EDGES_TREE: &str = "edges";
/// Nombre del keyspace de adyacencia v2 (F5): dos claves de 53 bytes por
/// arista (`keys::v2`), valor vacío.
const ADJACENCY_TREE: &str = "adjacency";
/// Nombre del keyspace de entidades v2 (F5.4): registro base de nodos,
/// clave `n|{uuid16}` (`keys::v2::node_key_v2`).
const ENTITIES_TREE: &str = "entities";
/// Nombre del keyspace de historia MVCC de nodos v2 (F5.4): versiones
/// (`v|`), puntero current (`c|`) y lista de versiones (`l|`).
const HISTORY_TREE: &str = "history";
/// Nombre del keyspace de índices derivados v2 (F5.4): hoy solo el índice
/// ts des-blobeado (`t|{ts8 BE}|{uuid16}|{ver8 BE}`, valor vacío).
const INDEXES_TREE: &str = "indexes";
/// Nombre del keyspace de catálogo v2 (F5): metas (`m|`) + interning de
/// edge-types (`et*`).
const CATALOG_TREE: &str = "catalog";
/// Nombre meta con la cota superior persistida del contador de transaction
/// ids. Vive en `catalog` (F5.4); legacy: `meta:next_tx_id` (F5.5).
pub const META_NEXT_TX_ID: &str = "next_tx_id";
/// Marca de progreso del redo del WAL (#65): todo commit con timestamp
/// lógico ≤ este valor ya quedó totalmente aplicado (o abortado con registro
/// `Abort`), así que el replay del open solo procesa el sufijo posterior.
/// Vive en `catalog` con semántica de máximo (`put_meta_u64_max`).
///
/// Es un TIMESTAMP de commit y no una posición en bytes del archivo a
/// propósito: `truncate_after_checkpoint` reescribe el WAL desde cero (las
/// posiciones no sobreviven al truncado), mientras que los timestamps de
/// commit son estables y crecen en orden-de-log (el applier los asigna
/// FIFO, y orden-de-log == orden-de-apply). Bases sin la clave (WAL escrito
/// por binarios previos a la marca): replay completo con las guardas
/// heurísticas históricas de `replay_wal`.
pub const META_WAL_APPLIED_UPTO: &str = "wal_applied_upto";

/// Capa KV: el contrato `KvEngine`/`KvKeyspace` y las implementaciones por
/// motor de almacenamiento. Vive en su propio módulo para no sombrear los
/// crates de los motores (`mod sled` aquí ocultaría al crate `sled`).
mod kv;

/// Codec de claves compuestas en disco: el codec binario del layout v2 en
/// `keys::v2` (F5) + versiones de aristas + la sección LEGACY del keyspace
/// default (`node:`/`idx:`/`ts:`/`meta:`, solo para la migración F5.5). Ver
/// la advertencia de formato en la cabecera del módulo.
mod keys;

/// Interning de tipos de arista string↔u32 (layout v2, F5), persistido en el
/// keyspace `catalog`. Consumidor: la adyacencia v2 — el tipo viaja como
/// u32 BE dentro de la clave de 53 bytes (`keys::v2`).
mod interner;
pub(crate) use interner::EdgeTypeInterner;

/// Migración automática del layout v1→v2 (F5.5): mueve la fuente de verdad
/// del tree `default` a los keyspaces v2 con la máquina copy → verify →
/// rebuild → activate → cleanup. La orquesta `Graph::open_with_options`.
mod layout_migrate;

/// Capa de dominio del storage (MVCC, adyacencia, índices) sobre el contrato
/// KV (motor según feature).
///
/// Los motores embebidos soportados son thread-safe internamente
/// (Send + Sync). No requiere locking externo — todas las operaciones son
/// concurrentes.
///
/// # Layout v2 (F5.4): el keyspace `default` no tiene escritores
///
/// Para bases NUEVAS, cada dato vive en su keyspace con claves binarias de
/// `keys::v2`: nodos en `entities`, historia MVCC en `history`, índice ts en
/// `indexes`, metas + interning en `catalog`, adyacencia en `adjacency` (las
/// aristas ya vivían en `edges`/`versioned_edges*`). NINGÚN path de runtime
/// escribe ya en `default` — el único código que lo toca es
/// `clear_legacy_property_index` (solo borra claves v1) y lo tocará la
/// migración de layout F5.5 (sección legacy de `keys.rs`), que moverá las
/// bases v1 existentes a este layout.
pub struct Storage {
    engine: Arc<dyn kv::KvEngine>,
    /// Tree `default`: SIN escritores desde F5.4 (ver el doc del struct).
    /// Se conserva para leer/limpiar bases v1 (prop-idx legacy hoy, F5.5
    /// mañana).
    default_ks: Arc<dyn kv::KvKeyspace>,
    edges_ks: Arc<dyn kv::KvKeyspace>,
    versioned_edges_ks: Arc<dyn kv::KvKeyspace>,
    versioned_edges_current_ks: Arc<dyn kv::KvKeyspace>,
    prop_idx_ks: Arc<dyn kv::KvKeyspace>,
    // Keyspaces del layout v2 (F5) — todos con consumidores de runtime:
    // catalog/adjacency desde F5.3, entities/history/indexes desde F5.4.
    catalog_ks: Arc<dyn kv::KvKeyspace>,
    entities_ks: Arc<dyn kv::KvKeyspace>,
    history_ks: Arc<dyn kv::KvKeyspace>,
    adjacency_ks: Arc<dyn kv::KvKeyspace>,
    indexes_ks: Arc<dyn kv::KvKeyspace>,
    /// Interning nombre↔u32 de tipos de arista (RAM, respaldado por
    /// `catalog_ks`), cargado completo al abrir.
    interner: EdgeTypeInterner,
    profile: StorageProfile,
}

fn serialize<T: serde::Serialize + ?Sized>(value: &T) -> Result<Vec<u8>> {
    rmp_serde::to_vec(value)
        .map_err(|e| NopalError::SerializationError(format!("MessagePack serialize error: {}", e)))
}

fn deserialize<'a, T: serde::de::Deserialize<'a>>(bytes: &'a [u8]) -> Result<T> {
    rmp_serde::from_slice(bytes)
        .map_err(|e| NopalError::SerializationError(format!("MessagePack deserialize error: {}", e)))
}

/// Valor de las claves de adyacencia y del índice ts v2: vacío por contrato
/// (toda la información viaja en la clave).
const EMPTY_VALUE: &[u8] = &[];

/// Cota de operaciones por `WriteBatch` en el rebuild de adyacencia
/// (memoria acotada; el rebuild es reparación idempotente, no necesita ser
/// una sola transacción).
const REBUILD_BATCH_OPS: usize = 10_000;

/// Error de clave malformada dentro de un keyspace v2: nada más que este
/// módulo escribe ahí, así que un parser que rechaza es bug o corrupción
/// externa y falla FUERTE.
fn malformed_key(keyspace: &str, key: &[u8]) -> NopalError {
    crate::error::StorageError::new(
        crate::error::StorageErrorKind::InvalidData,
        format!(
            "clave malformada en el keyspace {keyspace} ({} bytes: {:02x?})",
            key.len(),
            key
        ),
    )
    .into()
}

/// Parser de clave de adyacencia con semántica de corrupción (ver
/// `malformed_key`).
fn parse_adj_key_strict(
    key: &[u8],
) -> Result<(keys::v2::AdjDir, NodeId, u32, NodeId, EdgeId)> {
    keys::v2::parse_adj_key(key).ok_or_else(|| malformed_key(ADJACENCY_TREE, key))
}

/// Parser estricto de una entrada del índice ts (keyspace `indexes`).
fn parse_ts_index_key_strict(key: &[u8]) -> Result<(u64, NodeId, u64)> {
    keys::v2::parse_ts_index_key(key).ok_or_else(|| malformed_key(INDEXES_TREE, key))
}

/// Parser estricto de una clave de lista de versiones (`l|{uuid16}`,
/// keyspace `history`).
fn parse_history_versions_key_strict(key: &[u8]) -> Result<NodeId> {
    keys::v2::parse_history_versions_key(key).ok_or_else(|| malformed_key(HISTORY_TREE, key))
}

// ─── Codificación de claves del índice de propiedades (formato v2) ──────────
//
// ⚠️ FORMATO EN DISCO — cambiarlo requiere bump de PROP_IDX_FORMAT_CURRENT y
// lógica de migración. Única fn de encode (el v1 tenía el match triplicado
// con drift entre las tres copias).
//
// Layout: [len(prop): u16 BE][prop utf8][type_tag: u8][valor canónico]
// - El length-prefix elimina la inyección de separador del v1 (prop `a` +
//   valor `b:c` colisionaba con prop `a:b` + valor `c`).
// - El type tag elimina las colisiones de tipo del v1 (Int(1), Float(1.0) y
//   String("1") compartían clave).
// - El valor se codifica order-preserving (orden numérico == orden de bytes),
//   dejando listos los range scans en disco sin costo extra hoy.

const TAG_NULL: u8 = 0x00;
const TAG_BOOL: u8 = 0x01;
const TAG_INT: u8 = 0x02;
const TAG_FLOAT: u8 = 0x03;
const TAG_STRING: u8 = 0x04;

/// Clave v2 para `(property, value)`, o `None` si la variante no se indexa
/// (Bytes/List/Object — decisión F2 conservada) o el nombre excede u16.
pub(crate) fn encode_property_index_key(property: &str, value: &PropertyValue) -> Option<Vec<u8>> {
    let prop_bytes = property.as_bytes();
    let prop_len = u16::try_from(prop_bytes.len()).ok()?;

    let mut key = Vec::with_capacity(2 + prop_bytes.len() + 1 + 8);
    key.extend_from_slice(&prop_len.to_be_bytes());
    key.extend_from_slice(prop_bytes);

    match value {
        PropertyValue::Null => key.push(TAG_NULL),
        PropertyValue::Bool(b) => {
            key.push(TAG_BOOL);
            key.push(u8::from(*b));
        }
        PropertyValue::Int(i) => {
            // BE con el bit de signo invertido: los negativos ordenan antes.
            key.push(TAG_INT);
            key.extend_from_slice(&((*i as u64) ^ (1u64 << 63)).to_be_bytes());
        }
        PropertyValue::Float(f) => {
            // Canonicalización: -0.0 == 0.0 y todo NaN colapsa a uno solo.
            let f = if *f == 0.0 {
                0.0
            } else if f.is_nan() {
                f64::NAN
            } else {
                *f
            };
            // Transform IEEE754 total-order: orden numérico == orden de bytes.
            let bits = f.to_bits();
            let ordered = if bits >> 63 == 1 { !bits } else { bits | (1u64 << 63) };
            key.push(TAG_FLOAT);
            key.extend_from_slice(&ordered.to_be_bytes());
        }
        PropertyValue::String(s) => {
            key.push(TAG_STRING);
            key.extend_from_slice(s.as_bytes());
        }
        PropertyValue::Bytes(_) | PropertyValue::List(_) | PropertyValue::Object(_) => {
            return None;
        }
    }

    Some(key)
}

impl Storage {
    #[cfg(feature = "embeddings")]
    fn open_embeddings_tree_sync(&self) -> Result<Arc<dyn kv::KvKeyspace>> {
        self.engine.keyspace("embeddings")
    }

    /// Construye la capa de dominio sobre un engine ya abierto, cacheando los
    /// handles de keyspace que se usan en caliente (los de embeddings se
    /// abren on-demand, igual que antes del rewire).
    fn from_engine(engine: Arc<dyn kv::KvEngine>, profile: StorageProfile) -> Result<Self> {
        let default_ks = engine.keyspace(kv::DEFAULT_KEYSPACE)?;
        let edges_ks = engine.keyspace(EDGES_TREE)?;
        let versioned_edges_ks = engine.keyspace("versioned_edges")?;
        let versioned_edges_current_ks = engine.keyspace("versioned_edges_current")?;
        let prop_idx_ks = engine.keyspace(PROP_IDX_TREE)?;
        let catalog_ks = engine.keyspace(CATALOG_TREE)?;
        let entities_ks = engine.keyspace(ENTITIES_TREE)?;
        let history_ks = engine.keyspace(HISTORY_TREE)?;
        let adjacency_ks = engine.keyspace(ADJACENCY_TREE)?;
        let indexes_ks = engine.keyspace(INDEXES_TREE)?;
        let interner = EdgeTypeInterner::load(&catalog_ks)?;

        Ok(Self {
            engine,
            default_ks,
            edges_ks,
            versioned_edges_ks,
            versioned_edges_current_ks,
            prop_idx_ks,
            catalog_ks,
            entities_ks,
            history_ks,
            adjacency_ks,
            indexes_ks,
            interner,
            profile,
        })
    }

    /// Copia una base COMPLETA entre motores (p. ej. sled → redb), verificada.
    ///
    /// Copia byte a byte TODOS los keyspaces (la lista canónica es
    /// `kv::migrate::ALL_KEYSPACES`: nodos, versiones MVCC, aristas,
    /// adyacencia, índices, relojes, embeddings, y los del layout v2 aunque
    /// estén vacíos) — el time-travel y los
    /// índices sobreviven intactos porque nada se reinterpreta. Verifica con
    /// conteo + checksum por keyspace re-escaneando el DESTINO; si la
    /// verificación falla devuelve error y el destino no debe usarse.
    ///
    /// Precondiciones: ambos directorios CERRADOS (los locks de motor lo
    /// imponen para otros procesos); el origen debe haberse abierto y
    /// cerrado limpio con `Graph` para aplicar su WAL. El destino debe
    /// estar vacío — la migración jamás mezcla bases. Requiere compilar las
    /// features de ambos motores involucrados.
    pub async fn copy_database(
        src_dir: impl AsRef<Path>,
        src_opts: StorageOptions,
        dst_dir: impl AsRef<Path>,
        dst_opts: StorageOptions,
    ) -> Result<MigrationReport> {
        kv::migrate::copy_database_dirs(src_dir.as_ref(), src_opts, dst_dir.as_ref(), dst_opts)
    }

    /// Crea una nueva instancia de storage
    pub async fn new(path: impl AsRef<Path>) -> Result<Self> {
        Self::new_with_options(path, StorageOptions::default()).await
    }

    /// Crea una nueva instancia de storage con opciones explícitas.
    pub async fn new_with_options(
        path: impl AsRef<Path>,
        options: StorageOptions,
    ) -> Result<Self> {
        let engine = kv::open_engine(path.as_ref(), options.profile, &options)?;
        Self::from_engine(engine, options.profile)
    }

    /// Crea una nueva instancia de storage con perfil de tuning.
    pub async fn new_with_profile(path: impl AsRef<Path>, profile: StorageProfile) -> Result<Self> {
        let options = StorageOptions {
            profile,
            ..StorageOptions::default()
        };
        Self::new_with_options(path, options).await
    }

    /// Crea storage en memoria (útil para tests)
    pub async fn in_memory() -> Result<Self> {
        Self::in_memory_with_options(StorageOptions::default()).await
    }

    /// Crea storage en memoria con opciones explícitas.
    pub async fn in_memory_with_options(options: StorageOptions) -> Result<Self> {
        let engine = kv::open_in_memory(options.profile, &options)?;
        Self::from_engine(engine, options.profile)
    }

    /// Crea storage en memoria con perfil de tuning.
    pub async fn in_memory_with_profile(profile: StorageProfile) -> Result<Self> {
        let options = StorageOptions {
            profile,
            ..StorageOptions::default()
        };
        Self::in_memory_with_options(options).await
    }

    pub fn backend_name(&self) -> &'static str {
        self.engine.engine_name()
    }

    /// Perfil de tuning con el que se abrió este storage. Los knobs derivados
    /// se consultan vía `StorageProfile::tuning()`.
    pub fn profile(&self) -> StorageProfile {
        self.profile
    }

    // ─── Relojes lógicos persistidos ─────────────────────────────────────────
    //
    // `next_timestamp` y `next_tx_id` viven como atomics en `Graph`, pero deben
    // sobrevivir reinicios: si se reinician, los `valid_from`/`valid_to` nuevos
    // colisionan con versiones ya guardadas y el time-travel deja de ser fiable.
    // Se persisten como cotas superiores en el keyspace `catalog` (claves
    // `m|{nombre}` de `keys::v2::catalog_meta_key`, F5.4) con semántica de
    // máximo (nunca retroceden), codificadas como u64 big-endian.

    fn decode_meta_u64(bytes: &[u8]) -> u64 {
        let mut buf = [0u8; 8];
        let n = bytes.len().min(8);
        buf[8 - n..].copy_from_slice(&bytes[..n]);
        u64::from_be_bytes(buf)
    }

    /// Registra `value` como cota del reloj `name` solo si supera la
    /// almacenada. Atómico (RMW del motor sobre `catalog`), seguro ante
    /// escritores concurrentes.
    fn bump_clock(&self, name: &str, value: u64) -> Result<()> {
        self.catalog_ks.rmw(&keys::v2::catalog_meta_key(name), &mut |old| {
            let current = old.map(Self::decode_meta_u64).unwrap_or(0);
            Some(current.max(value).to_be_bytes().to_vec())
        })?;
        Ok(())
    }

    /// Persiste `value` como cota del reloj lógico `key` (solo crece).
    pub async fn put_meta_u64_max(&self, key: &str, value: u64) -> Result<()> {
        self.bump_clock(key, value)
    }

    pub(crate) fn get_meta_u64_sync(&self, key: &str) -> Result<Option<u64>> {
        Ok(self
            .catalog_ks
            .get(&keys::v2::catalog_meta_key(key))?
            .map(|v| Self::decode_meta_u64(&v)))
    }

    /// Lee una cota de reloj lógico persistida.
    pub async fn get_meta_u64(&self, key: &str) -> Result<Option<u64>> {
        self.get_meta_u64_sync(key)
    }

    /// Elimina una key meta. Existe para pruebas de migración (simular una base
    /// creada antes de que los relojes se persistieran).
    pub async fn delete_meta(&self, key: &str) -> Result<()> {
        self.catalog_ks.remove(&keys::v2::catalog_meta_key(key))?;
        Ok(())
    }

    /// Escaneo de migración: máximo timestamp presente en versiones ya
    /// persistidas (nodos vía el índice ts del keyspace `indexes`, aristas
    /// vía el tree MVCC). Solo se usa al abrir una base que aún no tiene la
    /// meta del reloj.
    pub async fn max_persisted_timestamp(&self) -> Result<u64> {
        let mut max_ts = 0u64;

        // El ts va BE al inicio de la clave: bastaría la última entrada,
        // pero el contrato KV no expone reverse scan — se recorre completo
        // (path de migración, no de runtime).
        for item in self.indexes_ks.scan_prefix(&[keys::v2::TS_TAG]) {
            let (key, _) = item?;
            let (ts, _, _) = parse_ts_index_key_strict(&key)?;
            max_ts = max_ts.max(ts);
        }

        for item in self.versioned_edges_ks.iter() {
            let (_, value) = item?;
            if let Ok(versioned) = deserialize::<VersionedEdge>(&value) {
                max_ts = max_ts.max(versioned.timestamp);
                if let Some(valid_to) = versioned.valid_to {
                    max_ts = max_ts.max(valid_to);
                }
            }
        }

        Ok(max_ts)
    }

    /// Inserta un nodo (registro base, keyspace `entities`)
    pub async fn insert_node(&self, node: &Node) -> Result<()> {
        let key = keys::v2::node_key_v2(node.id);
        let value = serialize(node)?;

        self.entities_ks.insert(&key, &value)?;

        Ok(())
    }

    /// Obtiene un nodo por ID
    pub async fn get_node(&self, id: NodeId) -> Result<Node> {
        let key = keys::v2::node_key_v2(id);

        let value = self.entities_ks.get(&key)?
            .ok_or_else(|| NopalError::NodeNotFound(id.to_string()))?;

        let node: Node = deserialize(&value)?;

        Ok(node)
    }

    /// Elimina un nodo
    pub async fn delete_node(&self, id: NodeId) -> Result<()> {
        let key = keys::v2::node_key_v2(id);

        // El contrato KV no devuelve el valor previo en `remove`; la
        // existencia se verifica antes (mismo error observable que el
        // `remove` de sled devolviendo `None`).
        if !self.entities_ks.contains_key(&key)? {
            return Err(NopalError::NodeNotFound(id.to_string()));
        }
        self.entities_ks.remove(&key)?;

        Ok(())
    }

    /// Inserta una arista
    pub async fn insert_edge(&self, edge: &Edge) -> Result<()> {
        let key = edge.id.to_string();
        let value = serialize(edge)?;

        self.edges_ks.insert(key.as_bytes(), &value)?;

        Ok(())

    }

    /// Obtiene una arista por ID
    pub async fn get_edge(&self, id: EdgeId) -> Result<Edge> {
        let key = id.to_string();

        let value = self.edges_ks.get(key.as_bytes())?
            .ok_or_else(|| NopalError::EdgeNotFound(id.to_string()))?;

        let edge: Edge = deserialize(&value)?;
        Ok(edge)
    }

    pub async fn node_exists(&self, id: NodeId) -> Result<bool> {
        let key = keys::v2::node_key_v2(id);

        self.entities_ks.contains_key(&key)
    }

    /// Verifica si una arista existe
    pub async fn edge_exists(&self, id: EdgeId) -> Result<bool> {
        let key = id.to_string();

        self.edges_ks.contains_key(key.as_bytes())
    }

    /// Elimina una arista del storage
    pub async fn delete_edge(&self, id: EdgeId) -> Result<()> {
        let key = id.to_string();

        self.edges_ks.remove(key.as_bytes())?;

        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════════════
    // VERSIONED EDGES — MVCC para aristas
    // Tree "versioned_edges": key = "{edge_id}:v{version}", value = VersionedEdge (MessagePack)
    // Tree "versioned_edges_current": key = edge_id, value = current VersionedEdge
    // ═══════════════════════════════════════════════════════════════════════

    /// Inserta la primera versión de una arista en el historial MVCC.
    /// Debe llamarse justo después de `insert_edge()`.
    pub async fn insert_versioned_edge(&self, edge: &Edge, timestamp: u64) -> Result<()> {
        let versioned = VersionedEdge::new(edge.clone(), timestamp);
        let key = keys::edge_version_key(edge.id, versioned.version);
        let value = serialize(&versioned)?;
        let current_value = serialize(&versioned)?;

        self.versioned_edges_ks.insert(key.as_bytes(), &value)?;

        self.versioned_edges_current_ks
            .insert(edge.id.to_string().as_bytes(), &current_value)?;

        // Avanzar la cota persistida del reloj lógico (nunca retrocede)
        self.bump_clock(META_NEXT_TIMESTAMP, timestamp.saturating_add(1))?;

        Ok(())
    }

    /// Obtiene la versión actual de una arista del historial MVCC.
    pub async fn get_current_versioned_edge(&self, id: EdgeId) -> Result<VersionedEdge> {
        let value = self
            .versioned_edges_current_ks
            .get(id.to_string().as_bytes())?
            .ok_or_else(|| NopalError::EdgeNotFound(id.to_string()))?;
        let versioned: VersionedEdge = deserialize(&value)?;
        Ok(versioned)
    }

    /// Marca una arista como eliminada: cierra su valid_to en la versión actual.
    /// Debe llamarse justo antes de `delete_edge()`.
    pub async fn mark_edge_deleted(&self, id: EdgeId, timestamp: u64) -> Result<()> {
        let current = self.get_current_versioned_edge(id).await?;
        // Reescribir la entrada del historial con valid_to
        let closed = current.with_valid_to(timestamp);
        let key = keys::edge_version_key(id, closed.version);
        let value = serialize(&closed)?;

        self.versioned_edges_ks.insert(key.as_bytes(), &value)?;

        // Eliminar la entrada current (la arista ya no está activa)
        self.versioned_edges_current_ks
            .remove(id.to_string().as_bytes())?;

        Ok(())
    }

    /// Retorna todas las versiones de una arista, ordenadas de más antigua a más reciente.
    pub async fn get_edge_history(&self, id: EdgeId) -> Result<Vec<VersionedEdge>> {
        let prefix = keys::edge_versions_prefix(id);

        let mut versions: Vec<VersionedEdge> = self
            .versioned_edges_ks
            .scan_prefix(prefix.as_bytes())
            .filter_map(|r| r.ok())
            .filter_map(|(_, v)| deserialize::<VersionedEdge>(&v).ok())
            .collect();

        versions.sort_by_key(|v| v.version);
        Ok(versions)
    }

    /// Retorna todas las aristas de un tipo específico válidas en `timestamp`.
    /// Escanea el historial MVCC completo — O(total versioned edges).
    pub async fn get_versioned_edges_of_type_at(
        &self,
        edge_type: &str,
        timestamp: u64,
    ) -> Result<Vec<Edge>> {
        // Track seen edge_ids to only include the best (latest valid) version per edge
        let mut best: HashMap<EdgeId, VersionedEdge> = HashMap::new();

        for result in self.versioned_edges_ks.iter() {
            let (_, v) = result?;
            if let Ok(ve) = deserialize::<VersionedEdge>(&v)
                && ve.edge_data.edge_type == edge_type
                && ve.is_valid_at(timestamp)
            {
                let entry = best.entry(ve.id).or_insert_with(|| ve.clone());
                if ve.version > entry.version {
                    *entry = ve;
                }
            }
        }

        Ok(best.into_values().map(|ve| ve.edge_data).collect())
    }

    // ─── Adyacencia v2 (F5.3): arista-por-clave tipada, keyspace `adjacency` ──
    //
    // Cada arista son exactamente DOS claves de 53 bytes con valor vacío
    // (`O|src|etype|tgt|edge` + espejo `I|tgt|etype|src|edge`, codec en
    // `keys::v2`), escritas/borradas en el MISMO `apply_multi` que su
    // registro del keyspace `edges`. La invariante "jamás edges sin sus O/I,
    // ni O sin su espejo I" la garantiza la atomicidad cross-keyspace del
    // contrato KV (F5.1) — no una convención de callers. Costo por operación:
    // O(1) puts/removes (el v1 reescribía la lista completa del nodo: O(deg)).

    /// Interner nombre↔u32 de tipos de arista, cargado del catalog al abrir.
    pub(crate) fn edge_type_interner(&self) -> &EdgeTypeInterner {
        &self.interner
    }

    /// Id internado del tipo, asignando uno nuevo si no existía (batch
    /// atómico sobre el catalog). Toda ASIGNACIÓN debe correr bajo el
    /// write_gate del `Graph` — el mismo régimen serializado del resto de
    /// las escrituras; para tipos ya internados es un lookup RAM O(1).
    pub(crate) fn intern_edge_type(&self, name: &str) -> Result<u32> {
        self.edge_type_interner().intern(&self.catalog_ks, name)
    }

    /// Inserta una arista Y sus dos claves de adyacencia (O + espejo I) como
    /// UNA transacción cross-keyspace (`apply_multi`): o se ve todo, o nada.
    /// Idempotente (puts): el redo del WAL puede re-aplicarla sin duplicar.
    pub async fn insert_edge_with_adjacency(&self, edge: &Edge, etype_id: u32) -> Result<()> {
        let mut edges_batch = kv::WriteBatch::default();
        edges_batch.insert(edge.id.to_string().as_bytes(), serialize(edge)?);

        let mut adj_batch = kv::WriteBatch::default();
        adj_batch.insert(keys::v2::adj_out_key(edge.source, etype_id, edge.target, edge.id), EMPTY_VALUE);
        adj_batch.insert(keys::v2::adj_in_key(edge.source, etype_id, edge.target, edge.id), EMPTY_VALUE);

        self.engine.apply_multi(vec![
            (EDGES_TREE.to_string(), edges_batch),
            (ADJACENCY_TREE.to_string(), adj_batch),
        ])
    }

    /// Elimina una arista Y sus dos claves de adyacencia — el espejo exacto
    /// de `insert_edge_with_adjacency`: 3 removes en un `apply_multi`.
    pub async fn remove_edge_with_adjacency(&self, edge: &Edge, etype_id: u32) -> Result<()> {
        let mut edges_batch = kv::WriteBatch::default();
        edges_batch.remove(edge.id.to_string().as_bytes());

        let mut adj_batch = kv::WriteBatch::default();
        adj_batch.remove(keys::v2::adj_out_key(edge.source, etype_id, edge.target, edge.id));
        adj_batch.remove(keys::v2::adj_in_key(edge.source, etype_id, edge.target, edge.id));

        self.engine.apply_multi(vec![
            (EDGES_TREE.to_string(), edges_batch),
            (ADJACENCY_TREE.to_string(), adj_batch),
        ])
    }

    /// Adyacencia saliente de un nodo leída de DISCO: `(etype, target,
    /// edge)` en orden (etype, target, edge) — prefix scan O(deg del nodo).
    pub async fn scan_adjacency_out(&self, node: NodeId) -> Result<Vec<(u32, NodeId, EdgeId)>> {
        self.scan_adjacency_dir(keys::v2::adj_out_prefix(node))
    }

    /// Adyacencia entrante de un nodo leída de DISCO: `(etype, source, edge)`.
    pub async fn scan_adjacency_in(&self, node: NodeId) -> Result<Vec<(u32, NodeId, EdgeId)>> {
        self.scan_adjacency_dir(keys::v2::adj_in_prefix(node))
    }

    fn scan_adjacency_dir(&self, prefix: [u8; 17]) -> Result<Vec<(u32, NodeId, EdgeId)>> {
        let mut entries = Vec::new();
        for item in self.adjacency_ks.scan_prefix(&prefix) {
            let (key, _) = item?;
            let (_dir, _own, etype, other, edge) = parse_adj_key_strict(&key)?;
            entries.push((etype, other, edge));
        }
        Ok(entries)
    }

    /// Reconstruye los dos HashMaps RAM de adyacencia con un scan COMPLETO
    /// del keyspace `adjacency` (solo claves: el valor es vacío por
    /// contrato). Sustituye a `load_all_adjacency_indices` del layout v1.
    /// Un nodo sin aristas simplemente no aparece (v1 persistía listas
    /// vacías; en v2 ausencia == vacío).
    pub async fn load_all_adjacency_v2(&self) -> Result<(
        HashMap<NodeId, Vec<EdgeId>>,  // adjacency_out
        HashMap<NodeId, Vec<EdgeId>>,  // adjacency_in
    )> {
        let mut adjacency_out: HashMap<NodeId, Vec<EdgeId>> = HashMap::new();
        let mut adjacency_in: HashMap<NodeId, Vec<EdgeId>> = HashMap::new();

        for item in self.adjacency_ks.iter() {
            let (key, _) = item?;
            let (dir, own, _etype, _other, edge) = parse_adj_key_strict(&key)?;
            match dir {
                keys::v2::AdjDir::Out => adjacency_out.entry(own).or_default().push(edge),
                keys::v2::AdjDir::In => adjacency_in.entry(own).or_default().push(edge),
            }
        }

        Ok((adjacency_out, adjacency_in))
    }

    /// Purga hasta `max_edges` aristas incidentes a `node`: por cada una,
    /// su registro en `edges` + su clave O/I propia + el ESPEJO del otro
    /// extremo, todo en UN `apply_multi` atómico. Retorna `(saliente?, otro
    /// extremo, edge)` por arista purgada; vacío = no queda adyacencia del
    /// nodo en disco. Idempotente: cada chunk re-escanea desde el prefijo y
    /// las claves ya purgadas no reaparecen — un crash intermedio deja
    /// aristas completas de menos, jamás pares rotos.
    pub(crate) async fn purge_node_adjacency_chunk(
        &self,
        node: NodeId,
        max_edges: usize,
    ) -> Result<Vec<(bool, NodeId, EdgeId)>> {
        debug_assert!(max_edges > 0);
        // (saliente?, etype, otro, edge)
        let mut entries: Vec<(bool, u32, NodeId, EdgeId)> = Vec::new();
        for (is_out, prefix) in [
            (true, keys::v2::adj_out_prefix(node)),
            (false, keys::v2::adj_in_prefix(node)),
        ] {
            for item in self.adjacency_ks.scan_prefix(&prefix) {
                if entries.len() >= max_edges {
                    break;
                }
                let (key, _) = item?;
                let (_dir, _own, etype, other, edge) = parse_adj_key_strict(&key)?;
                entries.push((is_out, etype, other, edge));
            }
            if entries.len() >= max_edges {
                break;
            }
        }
        if entries.is_empty() {
            return Ok(Vec::new());
        }

        let mut edges_batch = kv::WriteBatch::default();
        let mut adj_batch = kv::WriteBatch::default();
        for &(is_out, etype, other, edge) in &entries {
            edges_batch.remove(edge.to_string().as_bytes());
            // Los constructores reciben SIEMPRE (src, etype, tgt, edge) y
            // voltean solos el lado I — aquí solo se decide quién es src.
            let (src, tgt) = if is_out { (node, other) } else { (other, node) };
            adj_batch.remove(keys::v2::adj_out_key(src, etype, tgt, edge));
            adj_batch.remove(keys::v2::adj_in_key(src, etype, tgt, edge));
        }
        self.engine.apply_multi(vec![
            (EDGES_TREE.to_string(), edges_batch),
            (ADJACENCY_TREE.to_string(), adj_batch),
        ])?;

        Ok(entries
            .into_iter()
            .map(|(is_out, _etype, other, edge)| (is_out, other, edge))
            .collect())
    }

    /// Reconstruye la adyacencia COMPLETA desde el keyspace `edges` (la
    /// fuente de verdad): interna los tipos, REESCRIBE las claves v2 en disco
    /// (clear + batches acotados) y devuelve los HashMaps para la RAM.
    /// Es el reparador canónico: cualquier clave O/I faltante o huérfana
    /// queda corregida. Idempotente.
    pub async fn rebuild_indices(&self) -> Result<(
        HashMap<NodeId, Vec<EdgeId>>,
        HashMap<NodeId, Vec<EdgeId>>,
    )> {
        let mut adjacency_out: HashMap<NodeId, Vec<EdgeId>> = HashMap::new();
        let mut adjacency_in: HashMap<NodeId, Vec<EdgeId>> = HashMap::new();

        self.adjacency_ks.clear()?;

        let mut batch = kv::WriteBatch::default();
        let mut pending_ops = 0usize;
        for item in self.edges_ks.iter() {
            let (_, value) = item?;
            let edge: Edge = deserialize(&value)?;
            let etype_id = self.intern_edge_type(&edge.edge_type)?;

            batch.insert(keys::v2::adj_out_key(edge.source, etype_id, edge.target, edge.id), EMPTY_VALUE);
            batch.insert(keys::v2::adj_in_key(edge.source, etype_id, edge.target, edge.id), EMPTY_VALUE);
            pending_ops += 2;
            if pending_ops >= REBUILD_BATCH_OPS {
                self.adjacency_ks.apply_batch(std::mem::take(&mut batch))?;
                pending_ops = 0;
            }

            adjacency_out.entry(edge.source).or_default().push(edge.id);
            adjacency_in.entry(edge.target).or_default().push(edge.id);
        }
        if pending_ops > 0 {
            self.adjacency_ks.apply_batch(batch)?;
        }

        Ok((adjacency_out, adjacency_in))
    }
    /// Guarda un índice de propiedad: clave -> valor -> lista de nodos
    ///
    /// ⚠️ FORMATO EN DISCO: la clave la define `encode_property_index_key`
    /// (v2, tipada). NO usar `Display`/`to_display_string` aquí.
    pub async fn save_property_index(&self, property: &str, value: &PropertyValue, node_id: NodeId) -> Result<()> {
        let Some(key) = encode_property_index_key(property, value) else {
            return Ok(()); // Variante no indexable (Bytes/List/Object, F2)
        };

        // RMW bajo el single-writer applier (igual que el v1)
        let mut nodes: Vec<NodeId> = match self.prop_idx_ks.get(&key)? {
            Some(v) => deserialize(&v)?,
            None => Vec::new(),
        };

        if !nodes.contains(&node_id) {
            nodes.push(node_id);
            self.prop_idx_ks.insert(&key, &serialize(&nodes)?)?;
        }

        Ok(())
    }

    /// Remueve un NodeId de un índice de propiedad
    pub async fn remove_from_property_index(
        &self,
        property: &str,
        value: &PropertyValue,
        node_id: NodeId,
    ) -> Result<()> {
        let Some(key) = encode_property_index_key(property, value) else {
            return Ok(());
        };

        let mut nodes: Vec<NodeId> = match self.prop_idx_ks.get(&key)? {
            Some(v) => deserialize(&v)?,
            None => return Ok(()),
        };

        nodes.retain(|&id| id != node_id);

        if nodes.is_empty() {
            self.prop_idx_ks.remove(&key)?;
        } else {
            self.prop_idx_ks.insert(&key, &serialize(&nodes)?)?;
        }

        Ok(())
    }

    /// Obtiene lista de nodos que tienen una propiedad con cierto valor.
    ///
    /// Lookup TIPADO: `Int(1)`, `Float(1.0)` y `String("1")` son claves
    /// distintas (en el v1 colisionaban en la misma entrada).
    pub async fn get_nodes_by_property(&self, property: &str, value: &PropertyValue) -> Result<Vec<NodeId>> {
        let Some(key) = encode_property_index_key(property, value) else {
            return Ok(Vec::new());
        };

        match self.prop_idx_ks.get(&key)? {
            Some(v) => {
                let nodes: Vec<NodeId> = deserialize(&v)?;
                Ok(nodes)
            }
            None => Ok(Vec::new()),
        }
    }

    /// Borra las claves del formato LEGADO v1 (`idx:prop:*` en el tree
    /// default). Idempotente; parte de la migración a v2.
    pub(crate) async fn clear_legacy_property_index(&self) -> Result<usize> {
        let mut removed = 0usize;
        let keys: Vec<Vec<u8>> = self
            .default_ks
            .scan_prefix(keys::LEGACY_PROP_IDX_PREFIX)
            .map(|item| item.map(|(k, _)| k))
            .collect::<Result<Vec<_>>>()?;
        for key in keys {
            self.default_ks.remove(&key)?;
            removed += 1;
        }
        Ok(removed)
    }

    /// Vacía el índice v2 completo (para rebuild). Idempotente.
    pub(crate) async fn clear_property_index_v2(&self) -> Result<()> {
        self.prop_idx_ks.clear()?;
        Ok(())
    }

    // ═════════════════════════════════════════════════════════
    // ✅ MÉTODOS DE EMBEDDINGS
    // ═════════════════════════════════════════════════════════

    /// Comprueba (de forma síncrona) si existe un embedding para `node_id` y `model`.
    #[cfg(feature = "embeddings")]
    pub fn node_embedding_exists_sync(&self, node_id: crate::types::NodeId, model: &str) -> bool {
        self.try_node_embedding_exists_sync(node_id, model).unwrap_or(false)
    }

    /// Comprueba (sync, con semántica estricta) si existe un embedding para `node_id` y `model`.
    #[cfg(feature = "embeddings")]
    pub fn try_node_embedding_exists_sync(
        &self,
        node_id: crate::types::NodeId,
        model: &str,
    ) -> Result<bool> {
        let key = format!("{}:{}", node_id, model);
        let tree = self.open_embeddings_tree_sync()?;
        tree.contains_key(key.as_bytes())
    }

    /// Carga (sync) el embedding de `node_id` y `model`.
    #[cfg(feature = "embeddings")]
    pub fn load_node_embedding_sync(
        &self,
        node_id: NodeId,
        model: &str,
    ) -> Result<crate::embeddings::Embedding> {
        let key = format!("{}:{}", node_id, model);
        let tree = self.open_embeddings_tree_sync()?;
        let value = tree
            .get(key.as_bytes())?
            .ok_or_else(|| NopalError::custom(format!("Embedding not found for node {} model {}", node_id, model)))?;
        let embedding: crate::embeddings::Embedding = deserialize(&value)?;
        Ok(embedding)
    }

    /// Comprueba (sync, con semántica estricta) si existe un embedding para `edge_id` y `model`.
    #[cfg(feature = "embeddings")]
    pub fn try_edge_embedding_exists_sync(
        &self,
        edge_id: EdgeId,
        model: &str,
    ) -> Result<bool> {
        let key = format!("e:{}:{}", edge_id, model);
        let tree = self.open_embeddings_tree_sync()?;
        tree.contains_key(key.as_bytes())
    }

    /// Carga (sync, estricta) el embedding de `edge_id` y `model`.
    #[cfg(feature = "embeddings")]
    pub fn load_edge_embedding_sync(
        &self,
        edge_id: EdgeId,
        model: &str,
    ) -> Result<crate::embeddings::EdgeEmbedding> {
        let key = format!("e:{}:{}", edge_id, model);
        let tree = self.open_embeddings_tree_sync()?;
        let value = tree
            .get(key.as_bytes())?
            .ok_or_else(|| NopalError::custom(format!("Embedding not found for edge {} model {}", edge_id, model)))?;
        let embedding: crate::embeddings::EdgeEmbedding = deserialize(&value)?;
        Ok(embedding)
    }

    #[cfg(feature = "embeddings")]
    pub async fn save_node_embedding(&self, embedding: &crate::embeddings::Embedding) -> Result<()> {
        let key = format!("{}:{}", embedding.node_id, embedding.model);
        let value = serialize(embedding)?;

        let tree = self.open_embeddings_tree_sync()?;
        tree.insert(key.as_bytes(), &value)?;
        Ok(())
    }

    #[cfg(feature = "embeddings")]
    pub async fn load_node_embedding(&self, node_id: NodeId, model: &str) -> Result<crate::embeddings::Embedding> {
        let key = format!("{}:{}", node_id, model);

        let tree = self.open_embeddings_tree_sync()?;
        let value = tree.get(key.as_bytes())?
            .ok_or_else(|| NopalError::custom(format!("Embedding not found for node {} model {}", node_id, model)))?;
        let embedding: crate::embeddings::Embedding = deserialize(&value)?;
        Ok(embedding)
    }

    #[cfg(feature = "embeddings")]
    pub async fn save_edge_embedding(&self, embedding: &crate::embeddings::EdgeEmbedding) -> Result<()> {
        // Prefijo "e:" distingue aristas de nodos en el mismo keyspace
        let key = format!("e:{}:{}", embedding.edge_id, embedding.model);
        let value = serialize(embedding)?;

        let tree = self.open_embeddings_tree_sync()?;
        tree.insert(key.as_bytes(), &value)?;
        Ok(())
    }

    #[cfg(feature = "embeddings")]
    pub async fn load_edge_embedding(&self, edge_id: EdgeId, model: &str) -> Result<crate::embeddings::EdgeEmbedding> {
        let key = format!("e:{}:{}", edge_id, model);

        let tree = self.open_embeddings_tree_sync()?;
        let value = tree.get(key.as_bytes())?
            .ok_or_else(|| NopalError::custom(format!("Embedding not found for edge {} model {}", edge_id, model)))?;
        let embedding: crate::embeddings::EdgeEmbedding = deserialize(&value)?;
        Ok(embedding)
    }

    // ───────────────────────────────────────────────────────────
    // E-8: PathReferenceEmbedding — árbol "path_ref_embeddings"
    // ───────────────────────────────────────────────────────────

    #[cfg(feature = "embeddings")]
    fn open_path_ref_tree_sync(&self) -> Result<Arc<dyn kv::KvKeyspace>> {
        self.engine.keyspace("path_ref_embeddings")
    }

    /// Persiste una referencia de path embedding (E-8).
    #[cfg(feature = "embeddings")]
    pub async fn save_path_reference_embedding(
        &self,
        emb: &crate::embeddings::PathReferenceEmbedding,
    ) -> Result<()> {
        emb.validate()?;
        let key = crate::embeddings::PathReferenceEmbedding::storage_key(
            &emb.name, &emb.node_model, &emb.edge_model,
        );
        let value = serialize(emb)?;

        let tree = self.open_path_ref_tree_sync()?;
        tree.insert(key.as_bytes(), &value)?;
        Ok(())
    }

    /// Carga (sync) una referencia de path embedding por (name, node_model, edge_model).
    #[cfg(feature = "embeddings")]
    pub fn load_path_reference_embedding_sync(
        &self,
        name: &str,
        node_model: &str,
        edge_model: &str,
    ) -> Result<crate::embeddings::PathReferenceEmbedding> {
        let key = crate::embeddings::PathReferenceEmbedding::storage_key(name, node_model, edge_model);
        let tree = self.open_path_ref_tree_sync()?;
        match tree.get(key.as_bytes())? {
            Some(bytes) => {
                let emb: crate::embeddings::PathReferenceEmbedding = deserialize(&bytes)?;
                Ok(emb)
            }
            None => Err(NopalError::QueryExecutionError(format!(
                "PathReferenceEmbedding '{}' (node_model={}, edge_model={}) not found",
                name, node_model, edge_model
            ))),
        }
    }

    /// Comprueba (sync) si existe una referencia de path embedding.
    #[cfg(feature = "embeddings")]
    pub fn path_reference_embedding_exists_sync(
        &self,
        name: &str,
        node_model: &str,
        edge_model: &str,
    ) -> Result<bool> {
        let key = crate::embeddings::PathReferenceEmbedding::storage_key(name, node_model, edge_model);
        let tree = self.open_path_ref_tree_sync()?;
        tree.contains_key(key.as_bytes())
    }

    /// Carga (sync) todas las PathReferenceEmbedding para el par (node_model, edge_model).
    /// Itera el árbol completo y filtra por la clave "name\x00node_model\x00edge_model".
    /// Retorna lista vacía si no hay referencias para ese par de modelos.
    #[cfg(feature = "embeddings")]
    pub fn load_all_path_references_for_models_sync(
        &self,
        node_model: &str,
        edge_model: &str,
    ) -> Result<Vec<crate::embeddings::PathReferenceEmbedding>> {
        let tree = self.open_path_ref_tree_sync()?;
        let mut results = Vec::new();
        for item in tree.iter() {
            let (key_bytes, val_bytes) = item?;
            let key = std::str::from_utf8(&key_bytes)
                .map_err(|e| NopalError::custom(e.to_string()))?;
            // Clave: "name\x00node_model\x00edge_model"
            let parts: Vec<&str> = key.splitn(3, '\x00').collect();
            if parts.len() == 3 && parts[1] == node_model && parts[2] == edge_model {
                let emb: crate::embeddings::PathReferenceEmbedding = deserialize(&val_bytes)?;
                results.push(emb);
            }
        }
        Ok(results)
    }

    /// Retorna todos los embeddings de nodo que pertenecen al modelo `model`.
    /// Las claves de nodo tienen formato `{uuid}:{model}` (sin prefijo `e:`).
    #[cfg(feature = "embeddings")]
    pub async fn load_all_node_embeddings_for_model(
        &self,
        model: &str,
    ) -> Result<Vec<crate::embeddings::Embedding>> {
        let suffix = format!(":{}", model);

        let tree = self.open_embeddings_tree_sync()?;
        let mut result = Vec::new();
        for item in tree.iter() {
            let (key_bytes, val_bytes) = item?;
            let key = std::str::from_utf8(&key_bytes)
                .map_err(|e| NopalError::custom(e.to_string()))?;
            // Excluir aristas (prefijo "e:") y filtrar por modelo
            if !key.starts_with("e:") && key.ends_with(&suffix) {
                let emb: crate::embeddings::Embedding = deserialize(&val_bytes)?;
                result.push(emb);
            }
        }
        Ok(result)
    }

    // ═════════════════════════════════════════════════════════
    // ✅ MÉTODOS MVCC
    // ═════════════════════════════════════════════════════════

    /// Inserta una versión de nodo (MVCC): versión + puntero current + lista
    /// de versiones en `history`, entrada del índice ts en `indexes`.
    pub async fn insert_node_version(&self, versioned: &VersionedNode) -> Result<()> {
        // 1. Guardar versión
        let version_key = keys::v2::history_version_key(versioned.id, versioned.version);
        let version_value = serialize(versioned)?;

        self.history_ks.insert(&version_key, &version_value)?;

        // 2. Actualizar puntero current (si es la versión más reciente)
        if versioned.valid_to.is_none() {
            let current_key = keys::v2::history_current_key(versioned.id);
            let version_bytes = versioned.version.to_le_bytes();
            self.history_ks.insert(&current_key, version_bytes.as_ref())?;
        }

        // 3. Agregar a lista de versiones
        let versions_key = keys::v2::history_versions_key(versioned.id);
        let mut versions: Vec<u64> = match self.history_ks.get(&versions_key)? {
            Some(v) => deserialize(&v)?,
            None => Vec::new(),
        };

        if !versions.contains(&versioned.version) {
            versions.push(versioned.version);
            versions.sort_unstable();
            versions.reverse(); // Más reciente primero

            let versions_value = serialize(&versions)?;
            self.history_ks.insert(&versions_key, &versions_value)?;
        }

        // 4. Indexar por timestamp — des-blobeado (F5.4): una clave
        // `t|ts|nodo|versión` con valor vacío por entrada, en vez del RMW
        // del blob `ts:{n}` → Vec<NodeId>. El put es idempotente por clave
        // (la versión desambigua; el dedup por nodo del v1 sobra).
        self.indexes_ks.insert(
            &keys::v2::ts_index_key(versioned.timestamp, versioned.id, versioned.version),
            EMPTY_VALUE,
        )?;

        // 5. Avanzar la cota persistida del reloj lógico (nunca retrocede)
        self.bump_clock(META_NEXT_TIMESTAMP, versioned.timestamp.saturating_add(1))?;

        log::debug!("Inserted node version: {} v{}", versioned.id, versioned.version);

        Ok(())
    }

    /// Aplica ATÓMICAMENTE el write-set de versión de un nodo commiteado:
    ///   - la versión anterior invalidada (si es update) — `history`
    ///   - la versión nueva + puntero current + lista de versiones — `history`
    ///   - la entrada del índice ts — `indexes`
    ///   - el registro base del nodo — `entities`
    ///
    /// Antes esto eran 5+ escrituras independientes: un crash a la mitad dejaba
    /// al nodo sin versión current o con la cadena rota. En v1 todo cabía en un
    /// `WriteBatch` del tree default; con el layout v2 el write-set CRUZA tres
    /// keyspaces, así que la atomicidad la da `apply_multi` (contrato F5.1):
    /// o se aplica todo o no se aplica nada (el WAL redo cubre el caso "nada").
    ///
    /// PRECONDICIÓN: el caller serializa los commits (commit lock); las listas
    /// se leen-modifican-escriben aquí sin coordinación adicional.
    pub async fn commit_node_version_atomic(
        &self,
        node: &Node,
        invalidated_prev: Option<&VersionedNode>,
        new_version: &VersionedNode,
    ) -> Result<()> {
        let id = new_version.id;
        let mut history_batch = kv::WriteBatch::default();
        let mut indexes_batch = kv::WriteBatch::default();
        let mut entities_batch = kv::WriteBatch::default();

        // 1. Versión anterior invalidada (update)
        if let Some(prev) = invalidated_prev {
            history_batch.insert(keys::v2::history_version_key(id, prev.version), serialize(prev)?);
        }

        // 2. Versión nueva
        history_batch
            .insert(keys::v2::history_version_key(id, new_version.version), serialize(new_version)?);

        // 3. Puntero current
        history_batch
            .insert(keys::v2::history_current_key(id), new_version.version.to_le_bytes().as_ref());

        // 4. Lista de versiones (RMW bajo commit lock)
        let versions_key = keys::v2::history_versions_key(id);
        let mut versions: Vec<u64> = match self.history_ks.get(&versions_key)? {
            Some(v) => deserialize(&v)?,
            None => Vec::new(),
        };
        if !versions.contains(&new_version.version) {
            versions.push(new_version.version);
            versions.sort_unstable();
            versions.reverse();
        }
        history_batch.insert(versions_key, serialize(&versions)?);

        // 5. Índice por timestamp — des-blobeado: un put con valor vacío
        // (adiós al RMW del Vec<NodeId> bajo commit lock).
        indexes_batch.insert(
            keys::v2::ts_index_key(new_version.timestamp, id, new_version.version),
            EMPTY_VALUE,
        );

        // 6. Registro base del nodo (keyspace `entities`)
        entities_batch.insert(keys::v2::node_key_v2(node.id), serialize(node)?);

        self.engine.apply_multi(vec![
            (HISTORY_TREE.to_string(), history_batch),
            (INDEXES_TREE.to_string(), indexes_batch),
            (ENTITIES_TREE.to_string(), entities_batch),
        ])?;

        // Cota del reloj: fuera del batch, con CAS-max (los escritores directos
        // concurrentes también la avanzan; un put plano podría retrocederla).
        // Si crasheamos antes de esto, el open la deriva del máximo del WAL.
        self.bump_clock(META_NEXT_TIMESTAMP, new_version.timestamp.saturating_add(1))?;

        log::debug!("Committed node {} v{} atomically", id, new_version.version);
        Ok(())
    }

    /// Obtiene la versión actual de un nodo
    pub async fn get_current_version(&self, id: NodeId) -> Result<u64> {
        let current_key = keys::v2::history_current_key(id);

        let value = self.history_ks.get(&current_key)?
            .ok_or_else(|| NopalError::NodeNotFound(id.to_string()))?;

        let version = u64::from_le_bytes(
            value.as_slice().try_into()
                .map_err(|_| NopalError::Custom("Invalid version format".into()))?
        );

        Ok(version)
    }

    /// Obtiene una versión específica de un nodo
    pub async fn get_node_version(&self, id: NodeId, version: u64) -> Result<VersionedNode> {
        let version_key = keys::v2::history_version_key(id, version);

        let value = self.history_ks.get(&version_key)?
            .ok_or_else(|| NopalError::NodeNotFound(
                format!("{}:v{}", id, version)
            ))?;

        let versioned: VersionedNode = deserialize(&value)?;

        Ok(versioned)
    }

    /// Obtiene nodo en un timestamp específico (MVCC as_of)
    pub async fn get_node_at_timestamp(&self, id: NodeId, timestamp: u64) -> Result<VersionedNode> {
        // Obtener lista de versiones
        let versions_key = keys::v2::history_versions_key(id);

        let versions: Vec<u64> = match self.history_ks.get(&versions_key)? {
            Some(v) => deserialize(&v)?,
            None => {
                log::debug!("No versions found for node {}", id);
                return Err(NopalError::NodeNotFound(id.to_string()));
            }
        };

        log::debug!(
            "Searching version for node {} at t={}, available versions: {:?}",
            id, timestamp, versions
        );

        // Buscar versión válida en timestamp (más reciente primero)
        for &version in &versions {
            let versioned = self.get_node_version(id, version).await?;

            log::debug!(
                "  Checking v{}: valid_from={}, valid_to={:?}, is_valid={}",
                version,
                versioned.valid_from,
                versioned.valid_to,
                versioned.is_valid_at(timestamp)
            );

            if versioned.is_valid_at(timestamp) {
                log::debug!("  ✓ Found valid version: v{}", version);
                return Ok(versioned);
            }
        }

        Err(NopalError::Custom(format!(
            "No version of node {} valid at timestamp {}",
            id, timestamp
        )))
    }

    /// Obtiene historial completo de un nodo
    pub async fn get_node_history(&self, id: NodeId) -> Result<Vec<VersionedNode>> {
        let versions_key = keys::v2::history_versions_key(id);

        let versions: Vec<u64> = match self.history_ks.get(&versions_key)? {
            Some(v) => deserialize(&v)?,
            None => return Ok(Vec::new()),
        };

        let mut history = Vec::new();

        for &version in &versions {
            let versioned = self.get_node_version(id, version).await?;
            history.push(versioned);
        }

        Ok(history)
    }

    /// Invalida la versión actual de un nodo
    pub async fn invalidate_current_version(&self, id: NodeId, timestamp: u64) -> Result<()> {
        let current_version = self.get_current_version(id).await?;
        let mut versioned = self.get_node_version(id, current_version).await?;

        versioned.invalidate(timestamp);

        // Guardar versión invalidada
        let version_key = keys::v2::history_version_key(id, current_version);
        let version_value = serialize(&versioned)?;

        self.history_ks.insert(&version_key, &version_value)?;

        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════════════
    // GARBAGE COLLECTION - Clean up old MVCC versions
    // ═══════════════════════════════════════════════════════════════════════

    /// Elimina versiones antiguas de nodos según la configuración de GC.
    ///
    /// # Arguments
    /// * `config` - Configuración del garbage collector
    ///
    /// # Returns
    /// Estadísticas de la operación de GC
    ///
    /// # Example
    /// ```ignore
    /// // Eliminar versiones más viejas de 7 días
    /// let config = GCConfig::older_than_days(7);
    /// let stats = storage.gc_old_versions(&config).await?;
    /// println!("Deleted {} versions", stats.versions_deleted);
    /// ```
    pub async fn gc_old_versions(&self, config: &crate::mvcc::GCConfig) -> Result<crate::mvcc::GCStats> {
        use crate::mvcc::GCStats;

        let start = std::time::Instant::now();
        let mut stats = GCStats::default();

        // 1. Encontrar todos los nodos con versiones: el namespace `l|` del
        // keyspace `history` ES esa lista (una clave `l|{uuid16}` por nodo
        // con historia). En v1 esto era filtrar sufijos `:versions` de la
        // sopa del tree default.
        let mut node_ids_with_versions: Vec<NodeId> = Vec::new();

        for item in self.history_ks.scan_prefix(&[keys::v2::HISTORY_VERSIONS_TAG]) {
            let (key, _) = item?;
            node_ids_with_versions.push(parse_history_versions_key_strict(&key)?);
        }


        log::debug!("GC: Found {} nodes with versions", node_ids_with_versions.len());

        // 2. Aplicar límite de nodos por ciclo
        let nodes_to_process = if config.max_nodes_per_cycle > 0 {
            node_ids_with_versions.into_iter()
                .take(config.max_nodes_per_cycle)
                .collect::<Vec<_>>()
        } else {
            node_ids_with_versions
        };

        // 3. Para cada nodo, identificar y eliminar versiones elegibles
        for node_id in nodes_to_process {
            stats.nodes_scanned += 1;

            let history = self.get_node_history(node_id).await?;

            if history.len() <= config.min_versions_to_keep {
                // No hay suficientes versiones para eliminar
                continue;
            }

            // Identificar versiones a eliminar (mantener las más recientes)
            let versions_to_keep = config.min_versions_to_keep;
            let mut versions_to_delete: Vec<u64> = Vec::new();

            for (idx, versioned) in history.iter().enumerate() {
                // Siempre mantener las N versiones más recientes
                if idx < versions_to_keep {
                    continue;
                }

                // Verificar si es elegible para GC
                if versioned.is_gc_eligible(config.cutoff_timestamp) {
                    versions_to_delete.push(versioned.version);
                }
            }

            if versions_to_delete.is_empty() {
                continue;
            }

            log::debug!(
                "GC: Node {} - deleting {} versions: {:?}",
                node_id, versions_to_delete.len(), versions_to_delete
            );

            if !config.dry_run {
                // Eliminar las versiones
                let (deleted, bytes_freed) =
                    self.delete_node_versions(node_id, &versions_to_delete).await?;
                stats.versions_deleted += deleted;
                stats.bytes_freed += bytes_freed;
            } else {
                stats.versions_deleted += versions_to_delete.len();
            }
        }

        stats.duration_ms = start.elapsed().as_millis() as u64;

        log::info!(
            "GC complete: scanned {} nodes, deleted {} versions in {}ms{}",
            stats.nodes_scanned,
            stats.versions_deleted,
            stats.duration_ms,
            if config.dry_run { " (DRY RUN)" } else { "" }
        );

        Ok(stats)
    }

    /// Elimina versiones específicas de un nodo.
    ///
    /// Dos fases: LECTURA (los `get` por versión para contabilizar
    /// deleted/bytes_freed — el contrato KV no devuelve el valor previo en
    /// `remove` — más el RMW de la lista `:versions`; el GC corre
    /// serializado, sin carrera) y ESCRITURA: un solo `WriteBatch` con todos
    /// los removes de versiones + el update/remove de la lista, aplicado de
    /// una vez. El borrado por versión anterior era N syscalls/N entradas de
    /// árbol y bloqueaba el write_gate más tiempo; el batch es una sola
    /// aplicación atómica — preparación honesta para medir GC entre engines
    /// (los removals masivos son la debilidad declarada de redb).
    async fn delete_node_versions(&self, node_id: NodeId, versions: &[u64]) -> Result<(usize, usize)> {
        let mut deleted = 0;
        let mut bytes_freed = 0;
        let mut batch = kv::WriteBatch::default();

        // Fase de lectura 1: versiones a borrar (solo cuentan las existentes)
        for &version in versions {
            let version_key = keys::v2::history_version_key(node_id, version);
            if let Some(value) = self.history_ks.get(&version_key)? {
                batch.remove(version_key);
                deleted += 1;
                bytes_freed += value.len();
            }
        }

        // Fase de lectura 2: lista de versiones filtrada. Si queda vacía se
        // borra la clave; si no, se reescribe (misma semántica que el borrado
        // por versión que reemplazó este batch).
        //
        // Las entradas `t|ts|nodo|versión` del índice ts NO se tocan — misma
        // semántica que el v1, que tampoco limpiaba su blob `ts:{n}` (su
        // único lector, `max_persisted_timestamp`, solo quiere el máximo).
        let versions_key = keys::v2::history_versions_key(node_id);
        if let Some(value) = self.history_ks.get(&versions_key)? {
            let mut version_list: Vec<u64> = deserialize(&value)?;

            version_list.retain(|v| !versions.contains(v));

            if version_list.is_empty() {
                batch.remove(versions_key);
            } else {
                batch.insert(versions_key, serialize(&version_list)?);
            }
        }

        // Fase de escritura: todo el write-set en una sola aplicación atómica.
        self.history_ks.apply_batch(batch)?;

        Ok((deleted, bytes_freed))
    }

    /// Get all edges (for query executor)
    pub async fn get_all_edges(&self) -> Result<Vec<Edge>> {
        let mut edges = Vec::new();

        for result in self.edges_ks.iter() {
            let (_, value) = result?;
            let edge: Edge = deserialize(&value)
                .map_err(|e| NopalError::SerializationError(format!("{}", e)))?;
            edges.push(edge);
        }

        Ok(edges)
    }
    /// Obtiene todos los nodos del storage (para export)
    ///
    /// Scan PLANO del keyspace `entities`: es homogéneo por construcción
    /// (solo registros base `n|{uuid16}`), así que el filtro estructural del
    /// v1 (`is_base_node_key` para apartar `:v{n}`/`:current`/`:versions` de
    /// la sopa del tree default) ya no existe — la separación la da el
    /// keyspace, no un predicado.
    pub async fn get_all_nodes(&self) -> Result<Vec<Node>> {
        let mut nodes = Vec::new();

        for item in self.entities_ks.iter() {
            let (_, value) = item?;
            let node: Node = deserialize(&value)?;

            nodes.push(node);
        }

        log::debug!("Retrieved {} nodes for export", nodes.len());

        Ok(nodes)
    }

    /// Scan nodes in key order using a cursor and bounded batch size.
    ///
    /// This enables pull-based execution without materializing all nodes in memory.
    /// Returns `(nodes, next_cursor)`.
    ///
    /// Cursor (opaco para los callers, que solo lo devuelven tal cual): el
    /// uuid en string del último nodo entregado — se traduce a su clave
    /// binaria `n|{uuid16}` y el `range_from` sobre `entities` arranca ahí
    /// (inclusivo; la propia clave del cursor se filtra). El loop corre
    /// hasta juntar `limit` MATCHES de label o agotar el keyspace;
    /// `next_cursor == None` significa scan completo, nunca corte.
    pub async fn scan_nodes_batch(
        &self,
        label: Option<&str>,
        start_after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Node>, Option<String>)> {
        if limit == 0 {
            return Ok((Vec::new(), start_after.map(|s| s.to_string())));
        }

        let start: Vec<u8> = match start_after {
            Some(cursor) => {
                let id = uuid::Uuid::parse_str(cursor).map_err(|_| {
                    crate::error::StorageError::new(
                        crate::error::StorageErrorKind::InvalidData,
                        format!("cursor de scan_nodes_batch inválido (se espera uuid): {cursor:?}"),
                    )
                })?;
                keys::v2::node_key_v2(id).to_vec()
            }
            // `n` (ENTITY_TAG) es <= que toda clave del keyspace homogéneo.
            None => vec![keys::v2::ENTITY_TAG],
        };

        let mut nodes = Vec::with_capacity(limit);
        let mut last_seen: Option<NodeId> = None;

        for item in self.entities_ks.range_from(&start) {
            let (key, value) = item?;

            // range_from es inclusivo: saltar la clave del propio cursor.
            if start_after.is_some() && key.as_slice() == start.as_slice() {
                continue;
            }

            let node: Node = deserialize(&value)?;
            last_seen = Some(node.id);

            if let Some(expected_label) = label
                && node.label != expected_label {
                    continue;
            }

            nodes.push(node);
            if nodes.len() >= limit {
                break;
            }
        }

        let next_cursor = if nodes.len() >= limit {
            last_seen.map(|id| id.to_string())
        } else {
            None
        };

        Ok((nodes, next_cursor))
    }

    /// Obtiene todos los nodos versionados del storage (para MVCC export)
    ///
    /// Scan del namespace `v|` del keyspace `history`: el tag discrimina por
    /// estructura (las listas `l|` y punteros `c|` viven bajo otros tags),
    /// así que el clasificador string del v1 (`is_version_node_key` sobre la
    /// sopa `node:*`) ya no hace falta.
    pub async fn get_all_versioned_nodes(&self) -> Result<Vec<crate::mvcc::VersionedNode>> {
        let mut versioned_nodes = Vec::new();

        for item in self.history_ks.scan_prefix(&[keys::v2::HISTORY_VERSION_TAG]) {
            let (_, value) = item?;
            let versioned: crate::mvcc::VersionedNode = deserialize(&value)?;

            versioned_nodes.push(versioned);
        }

        log::debug!("Retrieved {} versioned nodes for export", versioned_nodes.len());

        Ok(versioned_nodes)
    }

    // ═══════════════════════════════════════════════════════════════════════
    // BATCH OPERATIONS - High Performance Bulk Insert
    // ═══════════════════════════════════════════════════════════════════════

    /// Inserta múltiples nodos en una sola operación atómica.
    ///
    /// **IMPORTANTE**: Esta es la forma recomendada para cargas masivas.
    /// Es 100-1000x más rápido que insertar nodos uno por uno.
    pub async fn insert_nodes_batch(&self, nodes: &[Node]) -> Result<Vec<NodeId>> {
        if nodes.is_empty() {
            return Ok(Vec::new());
        }

        let mut batch = kv::WriteBatch::default();
        let mut ids = Vec::with_capacity(nodes.len());

        for node in nodes {
            let value = serialize(node)?;
            batch.insert(keys::v2::node_key_v2(node.id), value);
            ids.push(node.id);
        }

        // Una sola operación de disco para todos los nodos
        self.entities_ks.apply_batch(batch)?;

        log::debug!("Batch inserted {} nodes", ids.len());
        Ok(ids)
    }

    /// Inserta múltiples aristas CON su adyacencia v2 en UNA aplicación
    /// atómica cross-keyspace. Cierra el hueco histórico del bulk: antes las
    /// aristas entraban sin adyacencia persistida y dependían de un
    /// `flush_indices` posterior que alguien tenía que acordarse de llamar.
    ///
    /// Los tipos se internan ANTES del `apply_multi` (cada asignación es su
    /// propio batch atómico del catalog): un crash entre medias deja a lo
    /// sumo tipos internados sin aristas, que es inocuo — los ids jamás se
    /// reciclan. Debe llamarse bajo el write_gate (los bulk paths del Graph
    /// ya lo toman).
    pub async fn insert_edges_batch(&self, edges: &[Edge]) -> Result<Vec<EdgeId>> {
        if edges.is_empty() {
            return Ok(Vec::new());
        }

        let mut edges_batch = kv::WriteBatch::default();
        let mut adj_batch = kv::WriteBatch::default();
        let mut ids = Vec::with_capacity(edges.len());

        for edge in edges {
            let etype_id = self.intern_edge_type(&edge.edge_type)?;
            edges_batch.insert(edge.id.to_string().as_bytes(), serialize(edge)?);
            adj_batch.insert(keys::v2::adj_out_key(edge.source, etype_id, edge.target, edge.id), EMPTY_VALUE);
            adj_batch.insert(keys::v2::adj_in_key(edge.source, etype_id, edge.target, edge.id), EMPTY_VALUE);
            ids.push(edge.id);
        }

        self.engine.apply_multi(vec![
            (EDGES_TREE.to_string(), edges_batch),
            (ADJACENCY_TREE.to_string(), adj_batch),
        ])?;

        log::debug!("Batch inserted {} edges (+ adjacency v2) atomically", ids.len());
        Ok(ids)
    }

    /// Flush all pending writes to disk
    ///
    /// Forces the underlying storage engine to persist all buffered data.
    pub async fn flush(&self) -> Result<()> {
        self.engine.flush()
    }

    // ─── Seams crudas SOLO para tests de migración ──────────────────────────
    //
    // No son API: existen para que los tests de integración de la migración
    // de layout (tests/layout_v2_migration_test.rs) puedan FABRICAR bases v1
    // byte a byte y simular crashes/corrupción por fase — cosas imposibles
    // desde la API pública precisamente porque el runtime ya no escribe el
    // layout v1. Ocultas de la doc; pueden cambiar o desaparecer sin aviso.
    // (Mismo precedente que los easter eggs #[doc(hidden)] del Graph.)

    /// Escritura cruda en un keyspace por nombre. SOLO tests de migración.
    #[doc(hidden)]
    pub fn debug_raw_insert(&self, keyspace: &str, key: &[u8], value: &[u8]) -> Result<()> {
        self.engine.keyspace(keyspace)?.insert(key, value)
    }

    /// Borrado crudo en un keyspace por nombre. SOLO tests de migración.
    #[doc(hidden)]
    pub fn debug_raw_remove(&self, keyspace: &str, key: &[u8]) -> Result<()> {
        self.engine.keyspace(keyspace)?.remove(key)
    }

    /// Lectura cruda de un keyspace por nombre. SOLO tests de migración.
    #[doc(hidden)]
    pub fn debug_raw_get(&self, keyspace: &str, key: &[u8]) -> Result<Option<Vec<u8>>> {
        self.engine.keyspace(keyspace)?.get(key)
    }

    /// Todas las claves de un keyspace, en orden. SOLO tests de migración.
    #[doc(hidden)]
    pub fn debug_raw_keys(&self, keyspace: &str) -> Result<Vec<Vec<u8>>> {
        self.engine
            .keyspace(keyspace)?
            .iter()
            .map(|item| item.map(|(k, _)| k))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PropertyValue;
    use crate::mvcc::VersionedNode;

    #[test]
    fn test_encode_v2_type_tags_are_disjoint() {
        // Las colisiones de tipo del v1: Int(1), Float(1.0) y String("1")
        // compartían clave. En v2 son claves distintas.
        let k_int = encode_property_index_key("age", &PropertyValue::Int(1)).unwrap();
        let k_float = encode_property_index_key("age", &PropertyValue::Float(1.0)).unwrap();
        let k_str = encode_property_index_key("age", &PropertyValue::String("1".into())).unwrap();
        assert_ne!(k_int, k_float);
        assert_ne!(k_int, k_str);
        assert_ne!(k_float, k_str);

        // Ídem Bool(true) vs String("true") y Null vs String("null")
        assert_ne!(
            encode_property_index_key("x", &PropertyValue::Bool(true)).unwrap(),
            encode_property_index_key("x", &PropertyValue::String("true".into())).unwrap()
        );
        assert_ne!(
            encode_property_index_key("x", &PropertyValue::Null).unwrap(),
            encode_property_index_key("x", &PropertyValue::String("null".into())).unwrap()
        );
    }

    #[test]
    fn test_encode_v2_no_separator_injection() {
        // v1: prop `a` + valor `b:c` colisionaba con prop `a:b` + valor `c`
        let k1 = encode_property_index_key("a", &PropertyValue::String("b:c".into())).unwrap();
        let k2 = encode_property_index_key("a:b", &PropertyValue::String("c".into())).unwrap();
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_encode_v2_order_preserving() {
        let enc = |v: i64| encode_property_index_key("n", &PropertyValue::Int(v)).unwrap();
        assert!(enc(-5) < enc(-1));
        assert!(enc(-1) < enc(0));
        assert!(enc(0) < enc(3));
        assert!(enc(3) < enc(i64::MAX));
        assert!(enc(i64::MIN) < enc(-5));

        let encf = |v: f64| encode_property_index_key("x", &PropertyValue::Float(v)).unwrap();
        assert!(encf(-2.5) < encf(-0.5));
        assert!(encf(-0.5) < encf(0.5));
        assert!(encf(0.5) < encf(2.5));
        assert!(encf(f64::NEG_INFINITY) < encf(-2.5));
        assert!(encf(2.5) < encf(f64::INFINITY));
    }

    #[test]
    fn test_encode_v2_float_canonicalization() {
        let encf = |v: f64| encode_property_index_key("x", &PropertyValue::Float(v)).unwrap();
        // -0.0 y 0.0 son la MISMA clave (v1: "-0" ≠ "0")
        assert_eq!(encf(-0.0), encf(0.0));
        // Todo NaN colapsa a un NaN canónico único
        let nan_a = f64::NAN;
        let nan_b = f64::from_bits(0x7ff8_0000_0000_0001);
        assert_eq!(encf(nan_a), encf(nan_b));
    }

    #[test]
    fn test_encode_v2_non_indexable_variants() {
        assert!(encode_property_index_key("b", &PropertyValue::Bytes(vec![1])).is_none());
        assert!(encode_property_index_key("l", &PropertyValue::List(vec![])).is_none());
        assert!(encode_property_index_key("o", &PropertyValue::Object(vec![])).is_none());
    }

    #[tokio::test]
    async fn test_prop_index_migration_from_legacy() {
        // DB persistente con: nodos + claves LEGADAS v1 fabricadas + sin
        // sentinel → al abrir el Graph, la migración borra el legado,
        // reconstruye v2 desde los nodos y escribe el sentinel.
        //
        // Nota F5.4: fabricar `idx:prop:*` en el tree default sigue siendo
        // EXACTAMENTE lo que esta sub-migración limpia (una base v1 tiene el
        // legado ahí); lo que cambió es dónde vive el sentinel (catalog) y de
        // dónde se reconstruye el v2 (nodos en `entities`). La migración de
        // TODO el layout v1 (nodos/historia/meta en default) es F5.5.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mig_db");

        let node_id;
        {
            let storage = Storage::new(&path).await.unwrap();
            let node = Node::new("P")
                .with_property("name", PropertyValue::String("Ana".into()))
                .with_property("edad", PropertyValue::Int(1));
            node_id = node.id;
            storage.insert_node(&node).await.unwrap();

            // Claves v1 fabricadas, incluida la COLISIÓN clásica: Int(1) y
            // String("1") compartiendo entrada.
            let legacy = serialize(&vec![node_id]).unwrap();
            storage.default_ks.insert(b"idx:prop:edad:1", &legacy).unwrap();
            storage.default_ks.insert(b"idx:prop:name:Ana", &legacy).unwrap();
            storage.engine.flush().unwrap();
        }

        let graph = crate::Graph::open(&path).await.unwrap();

        // Sentinel escrito
        assert_eq!(
            graph.storage().get_meta_u64(META_PROP_IDX_FORMAT).await.unwrap(),
            Some(PROP_IDX_FORMAT_CURRENT)
        );
        // Legado eliminado
        assert_eq!(
            graph.storage().default_ks.scan_prefix(keys::LEGACY_PROP_IDX_PREFIX).count(),
            0
        );
        // Lookups tipados correctos desde el v2 reconstruido
        let hits = graph
            .storage()
            .get_nodes_by_property("edad", &PropertyValue::Int(1))
            .await
            .unwrap();
        assert_eq!(hits, vec![node_id]);
        // La colisión quedó resuelta: buscar el STRING "1" ya no encuentra al Int
        let hits = graph
            .storage()
            .get_nodes_by_property("edad", &PropertyValue::String("1".into()))
            .await
            .unwrap();
        assert!(hits.is_empty());

        // Reabrir: idempotente, sin re-migración destructiva
        drop(graph);
        let graph = crate::Graph::open(&path).await.unwrap();
        let hits = graph
            .storage()
            .get_nodes_by_property("name", &PropertyValue::String("Ana".into()))
            .await
            .unwrap();
        assert_eq!(hits, vec![node_id]);
    }

    #[tokio::test]
    async fn test_prop_index_migration_is_crash_safe() {
        // Simula un crash a mitad de migración: legado ya borrado, v2 a
        // medias, SIN sentinel. El próximo open debe completar el rebuild.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crash_db");

        let node_id;
        {
            let graph = crate::Graph::open(&path).await.unwrap();
            let node = Node::new("P").with_property("k", PropertyValue::Int(7));
            node_id = graph.add_node(node).await.unwrap();
            // Fabricar el estado post-crash: borrar sentinel y vaciar v2
            graph.storage().delete_meta(META_PROP_IDX_FORMAT).await.unwrap();
            graph.storage().clear_property_index_v2().await.unwrap();
        }

        let graph = crate::Graph::open(&path).await.unwrap();
        assert_eq!(
            graph.storage().get_meta_u64(META_PROP_IDX_FORMAT).await.unwrap(),
            Some(PROP_IDX_FORMAT_CURRENT)
        );
        let hits = graph
            .storage()
            .get_nodes_by_property("k", &PropertyValue::Int(7))
            .await
            .unwrap();
        assert_eq!(hits, vec![node_id]);
    }

    #[tokio::test]
    async fn test_scan_nodes_batch_sparse_label_loses_nothing() {
        // Regresión: un label esparcido entre muchos nodos de otro label
        // debe recuperarse COMPLETO paginando con el cursor (next_cursor ==
        // None significa scan terminado, nunca corte).
        let storage = Storage::in_memory().await.unwrap();

        let mut raros = 0;
        for i in 0..500 {
            let label = if i % 100 == 0 { "Raro" } else { "Comun" };
            if label == "Raro" {
                raros += 1;
            }
            storage
                .insert_node(&Node::new(label).with_property("i", PropertyValue::Int(i)))
                .await
                .unwrap();
        }

        let mut found = 0;
        let mut cursor: Option<String> = None;
        loop {
            let (batch, next) = storage
                .scan_nodes_batch(Some("Raro"), cursor.as_deref(), 2)
                .await
                .unwrap();
            found += batch.len();
            match next {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }
        assert_eq!(found, raros);
    }

    #[tokio::test]
    async fn test_insert_and_get_node() {
        let storage = Storage::in_memory().await.unwrap();

        let node = Node::new("Person")
            .with_property("name", PropertyValue::String("Alice".to_string()))
            .with_property("age", PropertyValue::Int(30));

        storage.insert_node(&node).await.unwrap();

        let retrieved = storage.get_node(node.id).await.unwrap();

        assert_eq!(retrieved.id, node.id);
        assert_eq!(retrieved.label, "Person");
        assert_eq!(retrieved.properties.get("name"), Some(&PropertyValue::String("Alice".to_string())));
    }

    #[tokio::test]
    async fn test_storage_profile_mobile_on_in_memory() {
        // Agnóstico al motor: usa el engine default del build (sled si está
        // compilado; redb si es el único backend) — el perfil es lo probado.
        let options = StorageOptions {
            profile: StorageProfile::Mobile,
            ..StorageOptions::default()
        };
        let storage = Storage::in_memory_with_options(options).await.unwrap();
        assert!(matches!(storage.backend_name(), "sled" | "redb"));
        assert_eq!(storage.profile(), StorageProfile::Mobile);
        assert_eq!(storage.profile().tuning().cache_capacity_bytes, Some(16 * 1024 * 1024));
    }

    #[tokio::test]
    async fn test_delete_node() {
        let storage = Storage::in_memory().await.unwrap();

        let node = Node::new("Test");
        storage.insert_node(&node).await.unwrap();

        storage.delete_node(node.id).await.unwrap();

        let result = storage.get_node(node.id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_insert_and_get_edge() {
        let storage = Storage::in_memory().await.unwrap();

        let node1 = Node::new("Person")
            .with_property("name", PropertyValue::String("German".to_string()))
            .with_property("rol", PropertyValue::String("Assasin".to_string()));

        let node2 = Node::new("Person")
            .with_property("name", PropertyValue::String("Volga".to_string()))
            .with_property("rol", PropertyValue::String("Deidad".to_string()));

        storage.insert_node(&node1).await.unwrap();
        storage.insert_node(&node2).await.unwrap();

        let edge = Edge::new(node1.id, node2.id, "Enemy of".to_string())
            .with_property("damage", PropertyValue::Int(10));

        storage.insert_edge(&edge).await.unwrap();

        let retrieved = storage.get_edge(edge.id).await.unwrap();

        assert_eq!(retrieved.id, edge.id);
        assert_eq!(retrieved.edge_type, "Enemy of".to_string());
        assert_eq!(retrieved.properties.get("damage"), Some(&PropertyValue::Int(10)));
    }

    #[tokio::test]
    async fn test_adjacency_v2_roundtrip_insert_scan_remove() {
        let storage = Storage::in_memory().await.unwrap();

        let a = uuid::Uuid::new_v4();
        let b = uuid::Uuid::new_v4();
        let edge = Edge::new(a, b, "LINKS");
        let et = storage.intern_edge_type("LINKS").unwrap();

        storage.insert_edge_with_adjacency(&edge, et).await.unwrap();

        // Registro + espejo O/I consistente: la O de `a` y la I de `b` ven la
        // MISMA tripleta; los lados que no tocan quedan vacíos.
        assert!(storage.edge_exists(edge.id).await.unwrap());
        assert_eq!(storage.scan_adjacency_out(a).await.unwrap(), vec![(et, b, edge.id)]);
        assert_eq!(storage.scan_adjacency_in(b).await.unwrap(), vec![(et, a, edge.id)]);
        assert!(storage.scan_adjacency_out(b).await.unwrap().is_empty());
        assert!(storage.scan_adjacency_in(a).await.unwrap().is_empty());

        // load_all reconstruye ambos mapas desde las claves.
        let (out, inn) = storage.load_all_adjacency_v2().await.unwrap();
        assert_eq!(out.get(&a), Some(&vec![edge.id]));
        assert_eq!(inn.get(&b), Some(&vec![edge.id]));
        assert_eq!(out.len(), 1);
        assert_eq!(inn.len(), 1);

        // Remove espejo: las 3 claves fuera, nada residual.
        storage.remove_edge_with_adjacency(&edge, et).await.unwrap();
        assert!(!storage.edge_exists(edge.id).await.unwrap());
        assert!(storage.scan_adjacency_out(a).await.unwrap().is_empty());
        assert!(storage.scan_adjacency_in(b).await.unwrap().is_empty());
        let (out, inn) = storage.load_all_adjacency_v2().await.unwrap();
        assert!(out.is_empty());
        assert!(inn.is_empty());
    }

    #[tokio::test]
    async fn test_adjacency_v2_rebuild_repara_clave_perdida() {
        // La invariante de pares la garantiza apply_multi (probado en la
        // conformance del contrato KV); esto simula corrupción EXTERNA:
        // borrar a mano UNA de las dos claves y verificar que rebuild —
        // que deriva de edges, la fuente de verdad — repara ambas.
        let storage = Storage::in_memory().await.unwrap();

        let a = uuid::Uuid::new_v4();
        let b = uuid::Uuid::new_v4();
        let edge = Edge::new(a, b, "LINKS");
        let et = storage.intern_edge_type("LINKS").unwrap();
        storage.insert_edge_with_adjacency(&edge, et).await.unwrap();

        let in_key = keys::v2::adj_in_key(a, et, b, edge.id);
        storage.adjacency_ks.remove(&in_key).unwrap();
        assert!(storage.scan_adjacency_in(b).await.unwrap().is_empty(), "clave I perdida");

        let (out, inn) = storage.rebuild_indices().await.unwrap();
        assert_eq!(out.get(&a), Some(&vec![edge.id]));
        assert_eq!(inn.get(&b), Some(&vec![edge.id]));
        assert_eq!(storage.scan_adjacency_out(a).await.unwrap(), vec![(et, b, edge.id)]);
        assert_eq!(storage.scan_adjacency_in(b).await.unwrap(), vec![(et, a, edge.id)]);
    }
    #[tokio::test]
    async fn test_mvcc_insert_and_get() {
        let storage = Storage::in_memory().await.unwrap();

        let node = Node::new("Person")
            .with_property("name", PropertyValue::String("Alice".into()))
            .with_property("age", PropertyValue::Int(25));

        let v1 = VersionedNode::new(node, 100);

        storage.insert_node_version(&v1).await.unwrap();

        // Get current version
        let current = storage.get_current_version(v1.id).await.unwrap();
        assert_eq!(current, 1);

        // Get specific version
        let retrieved = storage.get_node_version(v1.id, 1).await.unwrap();
        assert_eq!(retrieved.version, 1);
        assert_eq!(retrieved.timestamp, 100);
    }

    #[tokio::test]
    async fn test_mvcc_version_chain() {
        let storage = Storage::in_memory().await.unwrap();

        // Version 1
        let node1 = Node::new("Person")
            .with_property("age", PropertyValue::Int(25));
        let v1 = VersionedNode::new(node1, 100);
        storage.insert_node_version(&v1).await.unwrap();

        // Invalidate v1
        storage.invalidate_current_version(v1.id, 200).await.unwrap();

        // Version 2
        let node2 = Node::new("Person")
            .with_property("age", PropertyValue::Int(30));
        let v2 = VersionedNode::new_version(&v1, node2, 200);
        storage.insert_node_version(&v2).await.unwrap();

        // Get at different timestamps
        let at_150 = storage.get_node_at_timestamp(v1.id, 150).await.unwrap();
        assert_eq!(at_150.version, 1);

        let at_250 = storage.get_node_at_timestamp(v1.id, 250).await.unwrap();
        assert_eq!(at_250.version, 2);

        // Get history
        let history = storage.get_node_history(v1.id).await.unwrap();
        assert_eq!(history.len(), 2);
    }

    #[tokio::test]
    async fn test_mvcc_time_travel() {
        let storage = Storage::in_memory().await.unwrap();

        let node_id = uuid::Uuid::new_v4();

        // t=100: Create (age=25)
        let n1 = Node::with_id(node_id, "Person")
            .with_property("age", PropertyValue::Int(25));
        let v1 = VersionedNode::new(n1, 100);
        storage.insert_node_version(&v1).await.unwrap();

        // t=200: Update (age=30)
        storage.invalidate_current_version(node_id, 200).await.unwrap();
        let n2 = Node::with_id(node_id, "Person")
            .with_property("age", PropertyValue::Int(30));
        let v2 = VersionedNode::new_version(&v1, n2, 200);
        storage.insert_node_version(&v2).await.unwrap();

        // t=300: Update (age=35)
        storage.invalidate_current_version(node_id, 300).await.unwrap();
        let n3 = Node::with_id(node_id, "Person")
            .with_property("age", PropertyValue::Int(35));
        let v3 = VersionedNode::new_version(&v2, n3, 300);
        storage.insert_node_version(&v3).await.unwrap();

        // Time travel queries
        let at_150 = storage.get_node_at_timestamp(node_id, 150).await.unwrap();
        assert_eq!(
            at_150.node_data.properties.get("age"),
            Some(&PropertyValue::Int(25))
        );

        let at_250 = storage.get_node_at_timestamp(node_id, 250).await.unwrap();
        assert_eq!(
            at_250.node_data.properties.get("age"),
            Some(&PropertyValue::Int(30))
        );

        let at_350 = storage.get_node_at_timestamp(node_id, 350).await.unwrap();
        assert_eq!(
            at_350.node_data.properties.get("age"),
            Some(&PropertyValue::Int(35))
        );
    }

    #[tokio::test]
    async fn test_layout_v2_base_nueva_default_sin_escritores() {
        // Pin de F5.4: en una base NUEVA todo vive en los keyspaces v2 y el
        // tree default queda VACÍO. Si algún path vuelve a escribir claves
        // legacy (`node:`/`idx:`/`ts:`/`meta:`), el conteo final lo delata.
        // El reopen verifica además que nodos, historia, time-travel y
        // relojes se leen íntegros de entities/history/indexes/catalog.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v2_db");

        let node_id;
        let other_id;
        let (ts_v1, ts_v2);
        {
            let graph = crate::Graph::open(&path).await.unwrap();

            let mut tx = graph.begin_transaction().await.unwrap();
            node_id = tx
                .add_node(Node::new("Person").with_property("edad", PropertyValue::Int(25)))
                .await
                .unwrap();
            other_id = tx.add_node(Node::new("Person")).await.unwrap();
            tx.commit().await.unwrap();

            // Segunda versión (update) → historia + time-travel reales.
            let mut tx = graph.begin_transaction().await.unwrap();
            let mut node = graph.get_node(node_id).await.unwrap();
            node.properties.insert("edad".into(), PropertyValue::Int(30));
            tx.add_node(node).await.unwrap();
            tx.commit().await.unwrap();

            // Una arista: en v1 la adyacencia también ensuciaba el default.
            graph
                .add_edge(crate::types::Edge::new(node_id, other_id, "CONOCE"))
                .await
                .unwrap();

            let history = graph.storage().get_node_history(node_id).await.unwrap();
            assert_eq!(history.len(), 2);
            ts_v1 = history.iter().find(|v| v.version == 1).unwrap().timestamp;
            ts_v2 = history.iter().find(|v| v.version == 2).unwrap().timestamp;
            assert!(ts_v2 > ts_v1);
        }

        let graph = crate::Graph::open(&path).await.unwrap();
        let storage = graph.storage();

        // Nodo current desde `entities`.
        let node = storage.get_node(node_id).await.unwrap();
        assert_eq!(node.properties.get("edad"), Some(&PropertyValue::Int(30)));

        // Historia completa y puntero current desde `history`.
        assert_eq!(storage.get_current_version(node_id).await.unwrap(), 2);
        assert_eq!(storage.get_node_history(node_id).await.unwrap().len(), 2);

        // Time-travel intacto tras el reopen.
        let at_v1 = storage.get_node_at_timestamp(node_id, ts_v1).await.unwrap();
        assert_eq!(at_v1.node_data.properties.get("edad"), Some(&PropertyValue::Int(25)));
        let at_v2 = storage.get_node_at_timestamp(node_id, ts_v2).await.unwrap();
        assert_eq!(at_v2.node_data.properties.get("edad"), Some(&PropertyValue::Int(30)));

        // Relojes desde `catalog` (persistidos en cada commit) y el índice
        // ts des-blobeado desde `indexes`.
        let clock = storage.get_meta_u64(META_NEXT_TIMESTAMP).await.unwrap().unwrap();
        assert!(clock > ts_v2, "la cota del reloj supera el último ts escrito");
        assert!(storage.get_meta_u64(META_NEXT_TX_ID).await.unwrap().is_some());
        assert!(storage.max_persisted_timestamp().await.unwrap() >= ts_v2);

        // Cada keyspace v2 tiene lo suyo…
        assert!(storage.entities_ks.iter().count() >= 2, "entities: registros base");
        assert!(storage.history_ks.iter().count() > 0, "history: v|/c|/l|");
        assert!(storage.indexes_ks.iter().count() > 0, "indexes: t| des-blobeado");
        assert!(storage.catalog_ks.iter().count() > 0, "catalog: metas + interning");
        assert!(storage.adjacency_ks.iter().count() == 2, "adjacency: O + espejo I");

        // …y el default quedó VACÍO: cero escritores legacy en bases nuevas.
        assert_eq!(
            storage.default_ks.iter().count(),
            0,
            "el tree default no debe recibir NINGUNA clave en una base nueva"
        );
    }

    // Note: The 4 contention tests (test_try_node_embedding_exists_sync_reports_busy_*,
    // test_load_*_sync_reports_busy_under_contention) were removed as part of the
    // P0 RwLock removal. Sled is thread-safe internally and no longer needs an
    // external RwLock, so contention-based "busy" errors no longer occur.
}
