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

/// Key meta con la cota superior persistida del reloj lógico de timestamps.
pub const META_NEXT_TIMESTAMP: &str = "meta:next_timestamp";

/// Versión del formato del índice de propiedades en disco. Ausente = formato
/// legado v1 (`idx:prop:{name}:{value_str}` en el tree default); `2` = claves
/// tipadas order-preserving en el tree `prop_idx_v2`. La migración corre en
/// `Graph::open_with_options` (ver `migrate_property_index_if_needed`).
pub const META_PROP_IDX_FORMAT: &str = "meta:prop_idx_format";

/// Valor actual de `META_PROP_IDX_FORMAT`.
pub const PROP_IDX_FORMAT_CURRENT: u64 = 2;

/// Nombre del keyspace del índice de propiedades v2.
const PROP_IDX_TREE: &str = "prop_idx_v2";
/// Key meta con la cota superior persistida del contador de transaction ids.
pub const META_NEXT_TX_ID: &str = "meta:next_tx_id";

/// Capa KV: el contrato `KvEngine`/`KvKeyspace` y las implementaciones por
/// motor de almacenamiento. Vive en su propio módulo para no sombrear los
/// crates de los motores (`mod sled` aquí ocultaría al crate `sled`).
mod kv;

/// Codec de claves compuestas en disco (namespaces `node:`/`idx:`/`ts:` del
/// keyspace default y versiones de aristas) + el codec binario del layout
/// v2 en `keys::v2` (F5). Ver la advertencia de formato en la cabecera del
/// módulo.
mod keys;

/// Interning de tipos de arista string↔u32 (layout v2, F5), persistido en el
/// keyspace `catalog`. Sin consumidores hasta el rewire de adyacencia
/// (F5.3/F5.4).
#[cfg_attr(not(test), allow(dead_code))]
mod interner;
#[allow(unused_imports)] // el consumidor (Graph) llega en F5.3/F5.4
pub(crate) use interner::EdgeTypeInterner;

/// Capa de dominio del storage (MVCC, adyacencia, índices) sobre el contrato
/// KV (motor según feature).
///
/// Los motores embebidos soportados son thread-safe internamente
/// (Send + Sync). No requiere locking externo — todas las operaciones son
/// concurrentes.
pub struct Storage {
    engine: Arc<dyn kv::KvEngine>,
    default_ks: Arc<dyn kv::KvKeyspace>,
    edges_ks: Arc<dyn kv::KvKeyspace>,
    versioned_edges_ks: Arc<dyn kv::KvKeyspace>,
    versioned_edges_current_ks: Arc<dyn kv::KvKeyspace>,
    prop_idx_ks: Arc<dyn kv::KvKeyspace>,
    // Keyspaces del layout v2 (F5): abiertos y cacheados desde F5.2, AÚN sin
    // consumidores — el rewire de los paths de dominio llega en F5.3/F5.4 y
    // ahí se retiran estos allow(dead_code).
    #[allow(dead_code)]
    catalog_ks: Arc<dyn kv::KvKeyspace>,
    #[allow(dead_code)]
    entities_ks: Arc<dyn kv::KvKeyspace>,
    #[allow(dead_code)]
    history_ks: Arc<dyn kv::KvKeyspace>,
    #[allow(dead_code)]
    adjacency_ks: Arc<dyn kv::KvKeyspace>,
    #[allow(dead_code)]
    indexes_ks: Arc<dyn kv::KvKeyspace>,
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
        let edges_ks = engine.keyspace("edges")?;
        let versioned_edges_ks = engine.keyspace("versioned_edges")?;
        let versioned_edges_current_ks = engine.keyspace("versioned_edges_current")?;
        let prop_idx_ks = engine.keyspace(PROP_IDX_TREE)?;
        let catalog_ks = engine.keyspace("catalog")?;
        let entities_ks = engine.keyspace("entities")?;
        let history_ks = engine.keyspace("history")?;
        let adjacency_ks = engine.keyspace("adjacency")?;
        let indexes_ks = engine.keyspace("indexes")?;

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
    // Se persisten como cotas superiores bajo keys `meta:` con semántica de
    // máximo (nunca retroceden), codificadas como u64 big-endian.

    fn decode_meta_u64(bytes: &[u8]) -> u64 {
        let mut buf = [0u8; 8];
        let n = bytes.len().min(8);
        buf[8 - n..].copy_from_slice(&bytes[..n]);
        u64::from_be_bytes(buf)
    }

    /// Registra `value` como cota del reloj `key` solo si supera la almacenada.
    /// Atómico (RMW del motor), seguro ante escritores concurrentes.
    fn bump_clock(&self, key: &str, value: u64) -> Result<()> {
        self.default_ks.rmw(key.as_bytes(), &mut |old| {
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
        Ok(self.default_ks.get(key.as_bytes())?.map(|v| Self::decode_meta_u64(&v)))
    }

    /// Lee una cota de reloj lógico persistida.
    pub async fn get_meta_u64(&self, key: &str) -> Result<Option<u64>> {
        self.get_meta_u64_sync(key)
    }

    /// Elimina una key meta. Existe para pruebas de migración (simular una base
    /// creada antes de que los relojes se persistieran).
    pub async fn delete_meta(&self, key: &str) -> Result<()> {
        self.default_ks.remove(key.as_bytes())?;
        Ok(())
    }

    /// Escaneo de migración: máximo timestamp presente en versiones ya
    /// persistidas (nodos vía keys `ts:{n}`, aristas vía el tree MVCC).
    /// Solo se usa al abrir una base que aún no tiene keys `meta:`.
    pub async fn max_persisted_timestamp(&self) -> Result<u64> {
        let mut max_ts = 0u64;

        for item in self.default_ks.scan_prefix(keys::TS_PREFIX.as_bytes()) {
            let (key, _) = item?;
            if let Ok(s) = std::str::from_utf8(&key)
                && let Ok(ts) = s.trim_start_matches(keys::TS_PREFIX).parse::<u64>()
            {
                max_ts = max_ts.max(ts);
            }
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

    /// Inserta un nodo
    pub async fn insert_node(&self, node: &Node) -> Result<()> {
        let key = keys::node_key(node.id);
        let value = serialize(node)?;

        self.default_ks.insert(key.as_bytes(), &value)?;

        Ok(())
    }

    /// Obtiene un nodo por ID
    pub async fn get_node(&self, id: NodeId) -> Result<Node> {
        let key = keys::node_key(id);

        let value = self.default_ks.get(key.as_bytes())?
            .ok_or_else(|| NopalError::NodeNotFound(id.to_string()))?;

        let node: Node = deserialize(&value)?;

        Ok(node)
    }

    /// Elimina un nodo
    pub async fn delete_node(&self, id: NodeId) -> Result<()> {
        let key = keys::node_key(id);

        // El contrato KV no devuelve el valor previo en `remove`; la
        // existencia se verifica antes (mismo error observable que el
        // `remove` de sled devolviendo `None`).
        if !self.default_ks.contains_key(key.as_bytes())? {
            return Err(NopalError::NodeNotFound(id.to_string()));
        }
        self.default_ks.remove(key.as_bytes())?;

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
        let key = keys::node_key(id);

        self.default_ks.contains_key(key.as_bytes())
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

    /// Guarda el índice de adyacencia saliente de un nodo
    pub async fn save_adjacency_out(&self, node_id: NodeId, edges: &[EdgeId]) -> Result<()> {
        let key = keys::adjacency_out_key(node_id);
        let value = serialize(edges)?;

        self.default_ks.insert(key.as_bytes(), &value)?;

        Ok(())
    }

    /// Carga el índice de adyacencia saliente de un nodo
    pub async fn load_adjacency_out(&self, node_id: NodeId) -> Result<Vec<EdgeId>> {
        let key = keys::adjacency_out_key(node_id);

        let value = self.default_ks.get(key.as_bytes())?;

        match value {
            Some(v) => {
                let edges: Vec<EdgeId> = deserialize(&v)?;
                Ok(edges)
            }
            None => Ok(Vec::new()),
        }
    }

    /// Guarda el índice de adyacencia entrante de un nodo
    pub async fn save_adjacency_in(&self, node_id: NodeId, edges: &[EdgeId]) -> Result<()> {
        let key = keys::adjacency_in_key(node_id);
        let value = serialize(edges)?;

        self.default_ks.insert(key.as_bytes(), &value)?;

        Ok(())
    }

    /// Carga el índice de adyacencia entrante de un nodo
    pub async fn load_adjacency_in(&self, node_id: NodeId) -> Result<Vec<EdgeId>> {
        let key = keys::adjacency_in_key(node_id);

        let value = self.default_ks.get(key.as_bytes())?;

        match value {
            Some(v) => {
                let edges: Vec<EdgeId> = deserialize(&v)?;
                Ok(edges)
            }
            None => Ok(Vec::new()),
        }
    }

    /// Carga todos los índices de adyacencia (para reconstruir al abrir el grafo)
    pub async fn load_all_adjacency_indices(&self) -> Result<(
        HashMap<NodeId, Vec<EdgeId>>,  // adjacency_out
        HashMap<NodeId, Vec<EdgeId>>,  // adjacency_in
    )> {
        let mut adjacency_out = HashMap::new();
        let mut adjacency_in = HashMap::new();


        // Iterar sobre todas las keys que empiezan con "idx:"
        for item in self.default_ks.scan_prefix(keys::ADJ_PREFIX) {
            let (key, value) = item?;
            let key_str = String::from_utf8_lossy(&key);

            if key_str.starts_with(keys::ADJ_OUT_PREFIX) {
                if let Some(node_id_str) = key_str.strip_prefix(keys::ADJ_OUT_PREFIX)
                    && let Ok(node_id) = uuid::Uuid::parse_str(node_id_str) {
                        let edges: Vec<EdgeId> = deserialize(&value)?;
                        adjacency_out.insert(node_id, edges);
                }
            } else if key_str.starts_with(keys::ADJ_IN_PREFIX)
                && let Some(node_id_str) = key_str.strip_prefix(keys::ADJ_IN_PREFIX)
                && let Ok(node_id) = uuid::Uuid::parse_str(node_id_str) {
                    let edges: Vec<EdgeId> = deserialize(&value)?;
                    adjacency_in.insert(node_id, edges);
            }
        }

        Ok((adjacency_out, adjacency_in))
    }

    /// Reconstruye índices desde cero escaneando todas las aristas
    pub async fn rebuild_indices(&self) -> Result<(
        HashMap<NodeId, Vec<EdgeId>>,
        HashMap<NodeId, Vec<EdgeId>>,
    )> {
        let mut adjacency_out: HashMap<NodeId, Vec<EdgeId>> = HashMap::new();
        let mut adjacency_in: HashMap<NodeId, Vec<EdgeId>> = HashMap::new();

        // Escanear todas las aristas del keyspace "edges"
        for item in self.edges_ks.iter() {
            let (_, value) = item?;
            let edge: Edge = deserialize(&value)?;

            // Actualizar índice out
            adjacency_out
                .entry(edge.source)
                .or_default()
                .push(edge.id);

            // Actualizar índice in
            adjacency_in
                .entry(edge.target)
                .or_default()
                .push(edge.id);
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

    /// Inserta una versión de nodo (MVCC)
    pub async fn insert_node_version(&self, versioned: &VersionedNode) -> Result<()> {
        

        // 1. Guardar versión
        let version_key = keys::node_version_key(versioned.id, versioned.version);
        let version_value = serialize(versioned)?;

        self.default_ks.insert(version_key.as_bytes(), &version_value)?;

        // 2. Actualizar puntero current (si es la versión más reciente)
        if versioned.valid_to.is_none() {
            let current_key = keys::node_current_key(versioned.id);
            let version_bytes = versioned.version.to_le_bytes();
            self.default_ks.insert(current_key.as_bytes(), version_bytes.as_ref())?;
        }

        // 3. Agregar a lista de versiones
        let versions_key = keys::node_versions_key(versioned.id);
        let mut versions: Vec<u64> = match self.default_ks.get(versions_key.as_bytes())? {
            Some(v) => deserialize(&v)?,
            None => Vec::new(),
        };

        if !versions.contains(&versioned.version) {
            versions.push(versioned.version);
            versions.sort_unstable();
            versions.reverse(); // Más reciente primero

            let versions_value = serialize(&versions)?;
            self.default_ks.insert(versions_key.as_bytes(), &versions_value)?;
        }

        // 4. Indexar por timestamp
        let ts_key = keys::ts_key(versioned.timestamp);
        let mut node_ids: Vec<NodeId> = match self.default_ks.get(ts_key.as_bytes())? {
            Some(v) => deserialize(&v)?,
            None => Vec::new(),
        };

        if !node_ids.contains(&versioned.id) {
            node_ids.push(versioned.id);
            let ts_value = serialize(&node_ids)?;
            self.default_ks.insert(ts_key.as_bytes(), &ts_value)?;
        }

        // 5. Avanzar la cota persistida del reloj lógico (nunca retrocede)
        self.bump_clock(META_NEXT_TIMESTAMP, versioned.timestamp.saturating_add(1))?;

        log::debug!("Inserted node version: {} v{}", versioned.id, versioned.version);

        Ok(())
    }

    /// Aplica ATÓMICAMENTE (un solo `WriteBatch` sobre el keyspace default) el
    /// write-set de versión de un nodo commiteado:
    ///   - la versión anterior invalidada (si es update)
    ///   - la versión nueva + puntero current + lista de versiones + índice ts
    ///   - el registro current legacy `node:{id}`
    ///
    /// Antes esto eran 5+ escrituras independientes: un crash a la mitad dejaba
    /// al nodo sin versión current o con la cadena rota. Con el batch, o se
    /// aplica todo o no se aplica nada (el WAL redo cubre el caso "nada").
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
        let mut batch = kv::WriteBatch::default();

        // 1. Versión anterior invalidada (update)
        if let Some(prev) = invalidated_prev {
            let key = keys::node_version_key(id, prev.version);
            batch.insert(key.as_bytes(), serialize(prev)?);
        }

        // 2. Versión nueva
        let version_key = keys::node_version_key(id, new_version.version);
        batch.insert(version_key.as_bytes(), serialize(new_version)?);

        // 3. Puntero current
        let current_key = keys::node_current_key(id);
        batch.insert(current_key.as_bytes(), new_version.version.to_le_bytes().as_ref());

        // 4. Lista de versiones (RMW bajo commit lock)
        let versions_key = keys::node_versions_key(id);
        let mut versions: Vec<u64> = match self.default_ks.get(versions_key.as_bytes())? {
            Some(v) => deserialize(&v)?,
            None => Vec::new(),
        };
        if !versions.contains(&new_version.version) {
            versions.push(new_version.version);
            versions.sort_unstable();
            versions.reverse();
        }
        batch.insert(versions_key.as_bytes(), serialize(&versions)?);

        // 5. Índice por timestamp (RMW bajo commit lock)
        let ts_key = keys::ts_key(new_version.timestamp);
        let mut node_ids: Vec<NodeId> = match self.default_ks.get(ts_key.as_bytes())? {
            Some(v) => deserialize(&v)?,
            None => Vec::new(),
        };
        if !node_ids.contains(&id) {
            node_ids.push(id);
        }
        batch.insert(ts_key.as_bytes(), serialize(&node_ids)?);

        // 6. Registro current legacy (mismo keyspace → misma atomicidad)
        let node_key = keys::node_key(node.id);
        batch.insert(node_key.as_bytes(), serialize(node)?);

        self.default_ks.apply_batch(batch)?;

        // Cota del reloj: fuera del batch, con CAS-max (los escritores directos
        // concurrentes también la avanzan; un put plano podría retrocederla).
        // Si crasheamos antes de esto, el open la deriva del máximo del WAL.
        self.bump_clock(META_NEXT_TIMESTAMP, new_version.timestamp.saturating_add(1))?;

        log::debug!("Committed node {} v{} atomically", id, new_version.version);
        Ok(())
    }

    /// Obtiene la versión actual de un nodo
    pub async fn get_current_version(&self, id: NodeId) -> Result<u64> {
        let current_key = keys::node_current_key(id);

        let value = self.default_ks.get(current_key.as_bytes())?
            .ok_or_else(|| NopalError::NodeNotFound(id.to_string()))?;

        let version = u64::from_le_bytes(
            value.as_slice().try_into()
                .map_err(|_| NopalError::Custom("Invalid version format".into()))?
        );

        Ok(version)
    }

    /// Obtiene una versión específica de un nodo
    pub async fn get_node_version(&self, id: NodeId, version: u64) -> Result<VersionedNode> {
        let version_key = keys::node_version_key(id, version);

        let value = self.default_ks.get(version_key.as_bytes())?
            .ok_or_else(|| NopalError::NodeNotFound(
                format!("{}:v{}", id, version)
            ))?;

        let versioned: VersionedNode = deserialize(&value)?;

        Ok(versioned)
    }

    /// Obtiene nodo en un timestamp específico (MVCC as_of)
    pub async fn get_node_at_timestamp(&self, id: NodeId, timestamp: u64) -> Result<VersionedNode> {
        // Obtener lista de versiones
        let versions_key = keys::node_versions_key(id);

        let versions: Vec<u64> = match self.default_ks.get(versions_key.as_bytes())? {
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
        let versions_key = keys::node_versions_key(id);

        let versions: Vec<u64> = match self.default_ks.get(versions_key.as_bytes())? {
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
        let version_key = keys::node_version_key(id, current_version);
        let version_value = serialize(&versioned)?;

        self.default_ks.insert(version_key.as_bytes(), &version_value)?;

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

        // 1. Encontrar todos los nodos con versiones
        
        let mut node_ids_with_versions: Vec<NodeId> = Vec::new();

        for item in self.default_ks.scan_prefix(keys::NODE_PREFIX.as_bytes()) {
            let (key, _) = item?;
            let key_str = String::from_utf8_lossy(&key);

            // Solo procesar keys de versiones (e.g., "node:uuid:versions")
            if key_str.ends_with(":versions") {
                let node_id_str = key_str
                    .strip_prefix(keys::NODE_PREFIX)
                    .and_then(|s| s.strip_suffix(":versions"));

                if let Some(id_str) = node_id_str
                    && let Ok(node_id) = uuid::Uuid::parse_str(id_str) {
                        node_ids_with_versions.push(node_id);
                }
            }
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
            let version_key = keys::node_version_key(node_id, version);
            if let Some(value) = self.default_ks.get(version_key.as_bytes())? {
                batch.remove(version_key.as_bytes());
                deleted += 1;
                bytes_freed += value.len();
            }
        }

        // Fase de lectura 2: lista de versiones filtrada. Si queda vacía se
        // borra la clave; si no, se reescribe (misma semántica que el borrado
        // por versión que reemplazó este batch).
        let versions_key = keys::node_versions_key(node_id);
        if let Some(value) = self.default_ks.get(versions_key.as_bytes())? {
            let mut version_list: Vec<u64> = deserialize(&value)?;

            version_list.retain(|v| !versions.contains(v));

            if version_list.is_empty() {
                batch.remove(versions_key.as_bytes());
            } else {
                batch.insert(versions_key.as_bytes(), serialize(&version_list)?);
            }
        }

        // Fase de escritura: todo el write-set en una sola aplicación atómica.
        self.default_ks.apply_batch(batch)?;

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
    pub async fn get_all_nodes(&self) -> Result<Vec<Node>> {
        let mut nodes = Vec::new();

        for item in self.default_ks.scan_prefix(keys::NODE_PREFIX.as_bytes()) {
            let (key, value) = item?;

            // Predicado estructural (no blacklist por substring): solo claves
            // base `node:{uuid}` exactas. No se rompe si aparece un sufijo
            // nuevo en el namespace.
            if !keys::is_base_node_key(&key) {
                continue;
            }

            let node: Node = deserialize(&value)?;

            nodes.push(node);
        }

        log::debug!("Retrieved {} nodes for export", nodes.len());

        Ok(nodes)
    }

    /// Scan nodes in key order using a cursor and bounded batch size.
    ///
    /// This enables pull-based execution without materializing all nodes in memory.
    /// Returns `(nodes, next_cursor)` where `next_cursor` is the last scanned key.
    pub async fn scan_nodes_batch(
        &self,
        label: Option<&str>,
        start_after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Node>, Option<String>)> {
        if limit == 0 {
            return Ok((Vec::new(), start_after.map(|s| s.to_string())));
        }

        let start = start_after
            .map(|s| s.as_bytes().to_vec())
            .unwrap_or_else(|| keys::NODE_PREFIX.as_bytes().to_vec());

        let mut nodes = Vec::with_capacity(limit);
        let mut last_seen_key: Option<String> = None;

        for item in self.default_ks.range_from(&start) {
            let (key, value) = item?;

            if !key.starts_with(keys::NODE_PREFIX.as_bytes()) {
                if last_seen_key.is_some() {
                    break;
                }
                continue;
            }

            let key_str = String::from_utf8_lossy(&key).to_string();

            if let Some(cursor) = start_after
                && key_str.as_str() <= cursor {
                    continue;
            }

            // Solo claves base `node:{uuid}` (predicado estructural, ver
            // is_base_node_key). Nota de semántica del cursor: el loop corre
            // hasta juntar `limit` MATCHES o agotar el namespace de nodos;
            // `next_cursor == None` significa scan completo, nunca corte.
            if !keys::is_base_node_key(&key) {
                continue;
            }

            let node: Node = deserialize(&value)?;
            last_seen_key = Some(key_str);

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
            last_seen_key
        } else {
            None
        };

        Ok((nodes, next_cursor))
    }

    /// Obtiene todos los nodos versionados del storage (para MVCC export)
    pub async fn get_all_versioned_nodes(&self) -> Result<Vec<crate::mvcc::VersionedNode>> {
        let mut versioned_nodes = Vec::new();

        for item in self.default_ks.scan_prefix(keys::NODE_PREFIX.as_bytes()) {
            let (key, value) = item?;

            // Solo claves de versión `node:{uuid}:v{n}` exactas. La whitelist
            // anterior (`contains(":v") && !contains(":versions")`) aceptaría
            // cualquier sufijo futuro que empiece con `v` y reventaría el
            // export completo con SerializationError.
            if keys::is_version_node_key(&key) {
                let versioned: crate::mvcc::VersionedNode = deserialize(&value)?;

                versioned_nodes.push(versioned);
            }
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
            let key = keys::node_key(node.id);
            let value = serialize(node)?;
            batch.insert(key.as_bytes(), value);
            ids.push(node.id);
        }

        // Una sola operación de disco para todos los nodos
        self.default_ks.apply_batch(batch)?;

        log::debug!("Batch inserted {} nodes", ids.len());
        Ok(ids)
    }

    /// Inserta múltiples aristas en una sola operación atómica.
    pub async fn insert_edges_batch(&self, edges: &[Edge]) -> Result<Vec<EdgeId>> {
        if edges.is_empty() {
            return Ok(Vec::new());
        }

        let mut batch = kv::WriteBatch::default();
        let mut ids = Vec::with_capacity(edges.len());

        //Revisar el keyspace de edges
        for edge in edges {
            let key = edge.id.to_string();
            let value = serialize(edge)?;
            batch.insert(key.as_bytes(), value);
            ids.push(edge.id);
        }

        self.edges_ks.apply_batch(batch)?;

        log::debug!("Batch inserted {} edges to edges tree", ids.len());
        Ok(ids)
    }

    /// Guarda múltiples índices de adyacencia en batch
    pub async fn save_adjacency_batch(
        &self,
        out_indices: &[(NodeId, Vec<EdgeId>)],
        in_indices: &[(NodeId, Vec<EdgeId>)],
    ) -> Result<()> {
        let mut batch = kv::WriteBatch::default();

        for (node_id, edge_ids) in out_indices {
            let key = keys::adjacency_out_key(*node_id);
            let value = serialize(edge_ids)?;
            batch.insert(key.as_bytes(), value);
        }

        for (node_id, edge_ids) in in_indices {
            let key = keys::adjacency_in_key(*node_id);
            let value = serialize(edge_ids)?;
            batch.insert(key.as_bytes(), value);
        }

        self.default_ks.apply_batch(batch)?;

        log::debug!(
            "Batch saved {} out indices and {} in indices",
            out_indices.len(),
            in_indices.len()
        );
        Ok(())
    }

    /// Flush all pending writes to disk
    ///
    /// Forces the underlying storage engine to persist all buffered data.
    pub async fn flush(&self) -> Result<()> {
        self.engine.flush()
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
    async fn test_save_and_load_adjacency() {
        let storage = Storage::in_memory().await.unwrap();

        let node_id = uuid::Uuid::new_v4();
        let edge_ids = vec![
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
        ];

        // Guardar índices
        storage.save_adjacency_out(node_id, &edge_ids).await.unwrap();
        storage.save_adjacency_in(node_id, &edge_ids).await.unwrap();

        // Cargar índices
        let loaded_out = storage.load_adjacency_out(node_id).await.unwrap();
        let loaded_in = storage.load_adjacency_in(node_id).await.unwrap();

        assert_eq!(loaded_out, edge_ids);
        assert_eq!(loaded_in, edge_ids);
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

    // Note: The 4 contention tests (test_try_node_embedding_exists_sync_reports_busy_*,
    // test_load_*_sync_reports_busy_under_contention) were removed as part of the
    // P0 RwLock removal. Sled is thread-safe internally and no longer needs an
    // external RwLock, so contention-based "busy" errors no longer occur.
}
