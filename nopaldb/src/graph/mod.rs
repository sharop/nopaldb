// src/graph/mod.rs

pub mod view;
pub use view::{GraphView, Subgraph};

use std::collections::{HashMap, BinaryHeap, VecDeque, HashSet};
use std::cmp::Ordering;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use tokio::sync::{Mutex, RwLock, watch};
use tokio::task::JoinHandle;
use tokio::time::{Duration, MissedTickBehavior};
use std::time::Instant;
use crate::storage::Storage;
use crate::transaction::{Transaction, TransactionId, Timestamp};
use crate::error::{NopalError, Result};
use crate::traversal::{TraversalConfig, TraversalResult};
use crate::types::{Node, Edge, NodeId, EdgeId, PropertyValue};
use crate::mvcc::VersionedNode;
use crate::schema::{SchemaManager, SchemaInfo};
use crate::index::{IndexManager, IndexType, IndexQuery};
use crate::planner::{QueryPlanner, GraphStats};

#[cfg(feature = "full-isolation")]
use crate::lock_manager::LockManager;


use crate::wal::{WalManager, WalRecord};
// NQL parse is used inline via crate::query::nql::parse in execute_statement/execute_nql


/// Dirección de traversal en el grafo
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Aristas salientes (outgoing): A -> B
    Outgoing,
    /// Aristas entrantes (incoming): A <- B
    Incoming,
    /// Ambas direcciones
    Both,
}


/// Graph es la API principal de NopalDB
#[derive(Clone)]
pub struct Graph {
    storage: Arc<Storage>,
    adjacency_out: Arc<RwLock<HashMap<NodeId, Vec<EdgeId>>>>,
    adjacency_in: Arc<RwLock<HashMap<NodeId, Vec<EdgeId>>>>,
    next_tx_id: Arc<AtomicU64>,
    next_timestamp: Arc<AtomicU64>,

    #[cfg(feature = "full-isolation")]
    last_modified: Arc<RwLock<HashMap<NodeId, u64>>>, // Mapa de última modificación por nodo

    #[cfg(feature = "full-isolation")]
    lock_manager: Arc<LockManager>,

    schema_manager: Arc<SchemaManager>,

    wal: Arc<WalManager>,

    index_manager: Arc<IndexManager>,

    auto_gc_task: Arc<Mutex<Option<JoinHandle<()>>>>,
    auto_gc_stop_tx: Arc<Mutex<Option<watch::Sender<bool>>>>,
    auto_gc_config: Arc<RwLock<Option<AutoGcConfig>>>,

    /// Mutex de serialización para la fase de commit de transacciones.
    /// Previene condiciones de carrera en índices de adyacencia y lost updates MVCC.
    commit_lock: Arc<tokio::sync::Mutex<()>>,

    /// Mapa de transacciones activas: tx_id → timestamp de inicio.
    /// Usado por el GC para calcular el horizonte seguro de purga.
    /// Usa std::sync::Mutex para ser usable desde contextos sync (rollback).
    active_tx_timestamps: Arc<std::sync::Mutex<std::collections::HashMap<TransactionId, Timestamp>>>,

    /// Version monotónica de topología (nodos/aristas) para invalidar cachés analíticas.
    topology_version: Arc<AtomicU64>,
    /// Caché exacta de community detection (Louvain), política tamaño 1.
    #[cfg(feature = "algorithms")]
    community_partition_cache_exact: Arc<RwLock<Option<CommunityPartitionCache>>>,

    /// Caché exacta de community detection (Leiden), independiente de Louvain.
    /// Se invalida con el mismo mecanismo de topology_version pero se almacena
    /// separado porque ambos algoritmos producen asignaciones distintas.
    #[cfg(feature = "algorithms")]
    leiden_partition_cache: Arc<RwLock<Option<CommunityPartitionCache>>>,

    /// Caché en memoria de índices HNSW por modelo (evita reconstruir desde Sled en cada query).
    #[cfg(feature = "embeddings-index")]
    embedding_indices: Arc<RwLock<HashMap<String, Arc<crate::embeddings::HnswIndex>>>>,
}

/// Estado de un nodo en el algoritmo de shortest path
#[derive(Copy, Clone, Eq, PartialEq)]
struct PathState {
    node_id: NodeId,
    cost: usize,
}

impl PartialOrd for PathState {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PathState {
    fn cmp(&self, other: &Self) -> Ordering {
        other.cost.cmp(&self.cost) // Invertido para min-heap
    }
}

/// Snapshot del grafo en un timestamp específico (inmutable)
#[derive(Clone)]
pub struct GraphSnapshot {
    graph: Graph,
    timestamp: u64,
}

/// Configuración de GC automático (scheduler en background).
#[derive(Debug, Clone)]
pub struct AutoGcConfig {
    /// Intervalo entre ciclos de GC en segundos.
    pub interval_secs: u64,
    /// Configuración aplicada en cada ciclo.
    pub gc_config: crate::mvcc::GCConfig,
}

/// Estado del scheduler de GC automático.
#[derive(Debug, Clone)]
pub struct AutoGcStatus {
    pub running: bool,
    pub config: Option<AutoGcConfig>,
}

#[cfg(feature = "algorithms")]
#[derive(Debug, Clone)]
struct CommunityPartitionCache {
    topology_version: u64,
    assignments: HashMap<NodeId, usize>,
}

impl GraphSnapshot {
    /// Obtiene un nodo del snapshot
    pub async fn get_node(&self, id: NodeId) -> Result<Node> {
        self.graph.get_node_at(id, self.timestamp).await
    }

    /// Obtiene múltiples nodos del snapshot
    pub async fn get_nodes(&self, ids: &[NodeId]) -> Result<Vec<Node>> {
        let mut nodes = Vec::new();

        for &id in ids {
            match self.get_node(id).await {
                Ok(node) => nodes.push(node),
                Err(_) => continue, // Skip nodos que no existen en este timestamp
            }
        }

        Ok(nodes)
    }

    /// Timestamp del snapshot
    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }
}

impl Graph {
    /// Expone el storage nativo para operaciones bulk
    pub fn storage(&self) -> Arc<Storage> {
        Arc::clone(&self.storage)
    }

    /// Crea un nuevo grafo con storage persistente (carga índices automáticamente)
    pub async fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Self::open_with_options(path, crate::storage::StorageOptions::default()).await
    }

    /// Crea un nuevo grafo con storage persistente y perfil de tuning.
    pub async fn open_with_profile(
        path: impl AsRef<std::path::Path>,
        profile: crate::storage::StorageProfile,
    ) -> Result<Self> {
        let options = crate::storage::StorageOptions {
            engine: crate::storage::StorageEngine::Sled,
            profile,
        };
        Self::open_with_options(path, options).await
    }

    /// Crea un nuevo grafo con storage persistente y opciones completas.
    pub async fn open_with_options(
        path: impl AsRef<std::path::Path>,
        options: crate::storage::StorageOptions,
    ) -> Result<Self> {
        let path_ref = path.as_ref();
        let storage = Storage::new_with_options(path_ref, options).await?;

        //Crear WAL
        let wal_path = path_ref.join("nopal.wal");
        let wal = WalManager::new(wal_path).await?;

        //Crear IndexManager
        let index_path = path_ref.join("indexes");
        let index_manager = IndexManager::new(Some(index_path.to_string_lossy().to_string()));
        
        // Cargar y reconstruir índices desde disco
        log::info!("Loading and rebuilding indices...");
        index_manager.load_indices(&storage).await?;

        //RECOVERY: Anlizar WAL y recuperar estado
        let recovery_info = wal.recover().await?;

        if !recovery_info.uncommitted_txs.is_empty() {
            log::warn!(
                "Found {} uncommitted transactions, will be rolled back",
                recovery_info.uncommitted_txs.len()
            );
        }


        // Intentar cargar índices existentes
        let (adjacency_out, adjacency_in) = storage.load_all_adjacency_indices().await?;

        // Si no hay índices guardados, reconstruirlos
        let (adjacency_out, adjacency_in) = if adjacency_out.is_empty() && adjacency_in.is_empty() {
            log::info!("No indices found, rebuilding from edges...");
            storage.rebuild_indices().await?
        } else {
            log::info!("Loaded {} outgoing and {} incoming adjacency entries",
                      adjacency_out.len(), adjacency_in.len());
            (adjacency_out, adjacency_in)
        };

        // Restaurar relojes lógicos persistidos. Sin esto, los timestamps se
        // reinician en 1 en cada open y los `valid_from/valid_to` nuevos
        // colisionan con versiones ya guardadas (time-travel corrupto).
        let next_timestamp_init = {
            let persisted = storage
                .get_meta_u64(crate::storage::META_NEXT_TIMESTAMP)
                .await?;
            let base = match persisted {
                Some(v) => v,
                // Migración: bases creadas antes de que los relojes se
                // persistieran — derivar del máximo timestamp ya escrito.
                None => storage.max_persisted_timestamp().await?.saturating_add(1),
            };
            base.max(recovery_info.max_timestamp.saturating_add(1)).max(1)
        };
        let next_tx_id_init = storage
            .get_meta_u64(crate::storage::META_NEXT_TX_ID)
            .await?
            .unwrap_or(1)
            .max(recovery_info.max_tx_id.saturating_add(1))
            .max(1);
        log::info!(
            "Logical clocks restored: next_timestamp={}, next_tx_id={}",
            next_timestamp_init,
            next_tx_id_init
        );

        let graph = Self {
            storage: Arc::new(storage),
            adjacency_out: Arc::new(RwLock::new(adjacency_out)),
            adjacency_in: Arc::new(RwLock::new(adjacency_in)),
            next_tx_id: Arc::new(AtomicU64::new(next_tx_id_init)),
            next_timestamp: Arc::new(AtomicU64::new(next_timestamp_init)),

            #[cfg(feature = "full-isolation")]
            last_modified: Arc::new(RwLock::new(HashMap::new())),

            #[cfg(feature = "full-isolation")]
            lock_manager: Arc::new(LockManager::new()),

            schema_manager: Arc::new(Default::default()),

            wal: Arc::new(wal),

            index_manager: Arc::new(index_manager),

            auto_gc_task: Arc::new(Mutex::new(None)),
            auto_gc_stop_tx: Arc::new(Mutex::new(None)),
            auto_gc_config: Arc::new(RwLock::new(None)),
            commit_lock: Arc::new(tokio::sync::Mutex::new(())),

            active_tx_timestamps: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            topology_version: Arc::new(AtomicU64::new(1)),
            #[cfg(feature = "algorithms")]
            community_partition_cache_exact: Arc::new(RwLock::new(None)),
            #[cfg(feature = "algorithms")]
            leiden_partition_cache: Arc::new(RwLock::new(None)),
            #[cfg(feature = "embeddings-index")]
            embedding_indices: Arc::new(RwLock::new(HashMap::new())),
        };

        if recovery_info.total_records>0 {
            log::info!("Replaying committed operations from WAL...");
            graph.replay_wal().await?;
        }

        // Rebuild TaxonomyIndex from Class nodes + subClassOf edges (if any).
        // Needed when a DB was populated via import_turtle in a previous session.
        #[cfg(feature = "reasoner")]
        {
            if let Err(e) = graph.rebuild_taxonomy_from_graph().await {
                log::warn!("Taxonomy rebuild skipped: {}", e);
            }
        }

        if cfg!(debug_assertions) {
            log::info!("🌵 NopalDB v{} - ¡Dale que es mole de olla!", env!("CARGO_PKG_VERSION"));
        }

        if std::env::var("NOPALDB_FIRST_RUN").is_ok() {
            println!(r#"

    Welcome to NopalDB! 🌵

         |\__/,|   (`\
       _.|o o  |_   ) )
    -(((---(((--------

    Your graph database with:
    ✓ ACID Transactions
    ✓ MVCC Time-Travel
    ✓ Deadlock Detection
    ✓ WAL Durability

    Made with 🦀 Rust & ❤️
    VIVA MÉXICO! 🇲🇽

            "#);
        }

        Ok(graph)
    }


    /// Persiste los índices en disco
    pub async fn flush_indices(&self) -> Result<()> {
        let adj_out = self.adjacency_out.read().await;
        let adj_in = self.adjacency_in.read().await;

        // Guardar todos los índices out
        for (node_id, edge_ids) in adj_out.iter() {
            self.storage.save_adjacency_out(*node_id, edge_ids).await?;
        }

        // Guardar todos los índices in
        for (node_id, edge_ids) in adj_in.iter() {
            self.storage.save_adjacency_in(*node_id, edge_ids).await?;
        }

        log::info!("Flushed {} nodes to disk", adj_out.len());
        Ok(())
    }

    // Metodo publico: agrega nodo con indexación automática
    pub async fn add_node(&self, node: Node) -> Result<NodeId> {
        self.add_node_internal(node, false).await
    }

    /// Metodo INTERNO: agrega nodo con control de indexación
    pub(crate) async fn add_node_internal(
        &self,
        node: Node,
        skip_indexing: bool,
    ) -> Result<NodeId> {
        let node_id = node.id;
        let existed = self.storage.node_exists(node_id).await?;

        // Guardar en storage
        self.storage.insert_node(&node).await?;

        // Inicializar en índices de adyacencia
        let mut adj_out = self.adjacency_out.write().await;
        let mut adj_in = self.adjacency_in.write().await;

        adj_out.insert(node_id, Vec::new());
        adj_in.insert(node_id, Vec::new());

        drop(adj_out);
        drop(adj_in);

        // Indexar propiedades SOLO si no se debe skip
        if !skip_indexing {
            self.index_node_properties(&node).await?;

            // actualizar índices secundarios
            for(property_key, property_value) in &node.properties {
                if let Some(index_name) = self.index_manager
                    .find_index(&node.label, property_key)
                    .await
                {
                    self.index_manager
                        .insert(&index_name, property_value.clone(), node_id)
                        .await?;
                }
            }
        }

        // Persistir índices vacíos
        self.storage.save_adjacency_out(node_id, &[]).await?;
        self.storage.save_adjacency_in(node_id, &[]).await?;

        if !existed {
            self.bump_topology_version();
        }

        Ok(node_id)
    }

    /// Indexa las propiedades de un nodo (uso interno)
    pub(crate) async fn index_node_properties(&self, node: &Node) -> Result<()> {
        for (key, value) in &node.properties {
            self.storage.save_property_index(key, value, node.id).await?;
        }
        Ok(())
    }

    /// Crea un grafo en memoria (útil para tests)
    pub async fn in_memory() -> Result<Self> {
        Self::in_memory_with_options(crate::storage::StorageOptions::default()).await
    }

    /// Crea un grafo en memoria con perfil de tuning.
    pub async fn in_memory_with_profile(profile: crate::storage::StorageProfile) -> Result<Self> {
        let options = crate::storage::StorageOptions {
            engine: crate::storage::StorageEngine::Sled,
            profile,
        };
        Self::in_memory_with_options(options).await
    }

    /// Crea un grafo en memoria con opciones completas.
    pub async fn in_memory_with_options(options: crate::storage::StorageOptions) -> Result<Self> {
        let storage = Storage::in_memory_with_options(options).await?;

        //WAL en direcotrio temporal
        let temp_dir = std::env::temp_dir();
        let wal_path = temp_dir.join(format!("nopal--{}.wal", uuid::Uuid::new_v4()));
        let wal = WalManager::new(wal_path).await?;

        let _index_manager = IndexManager::new(None);

        Ok(Self::from_storage(storage, wal))
    }

    /// Crea un grafo desde un storage existente
    fn from_storage(storage: Storage, wal: WalManager) -> Self {
        // Respetar relojes persistidos si el storage ya tiene datos
        // (para in-memory recién creado ambos parten de 1).
        let next_timestamp_init = storage
            .get_meta_u64_sync(crate::storage::META_NEXT_TIMESTAMP)
            .ok()
            .flatten()
            .unwrap_or(1)
            .max(1);
        let next_tx_id_init = storage
            .get_meta_u64_sync(crate::storage::META_NEXT_TX_ID)
            .ok()
            .flatten()
            .unwrap_or(1)
            .max(1);
        Self {
            storage: Arc::new(storage),
            adjacency_out: Arc::new(RwLock::new(HashMap::new())),
            adjacency_in: Arc::new(RwLock::new(HashMap::new())),
            next_tx_id: Arc::new(AtomicU64::new(next_tx_id_init)),
            next_timestamp: Arc::new(AtomicU64::new(next_timestamp_init)),

            #[cfg(feature = "full-isolation")]
            last_modified: Arc::new(RwLock::new(HashMap::new())),

            #[cfg(feature = "full-isolation")]
            lock_manager: Arc::new(LockManager::new()),

            schema_manager: Arc::new(Default::default()),

            index_manager: Arc::new(IndexManager::new(None)),

            wal: Arc::new(wal),

            auto_gc_task: Arc::new(Mutex::new(None)),
            auto_gc_stop_tx: Arc::new(Mutex::new(None)),
            auto_gc_config: Arc::new(RwLock::new(None)),
            commit_lock: Arc::new(tokio::sync::Mutex::new(())),

            active_tx_timestamps: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
            topology_version: Arc::new(AtomicU64::new(1)),
            #[cfg(feature = "algorithms")]
            community_partition_cache_exact: Arc::new(RwLock::new(None)),
            #[cfg(feature = "algorithms")]
            leiden_partition_cache: Arc::new(RwLock::new(None)),
            #[cfg(feature = "embeddings-index")]
            embedding_indices: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    //Getter WAL manager
    pub(crate) fn wal(&self) -> Arc<WalManager> {
        Arc::clone(&self.wal)
    }

    //Obtener el lock manager
    #[cfg(feature = "full-isolation")]
    pub(crate) fn lock_manager(&self) -> Arc<LockManager> {
        Arc::clone(&self.lock_manager)
    }

    /// Mutex de serialización para la fase de commit.
    pub(crate) fn commit_lock(&self) -> Arc<tokio::sync::Mutex<()>> {
        Arc::clone(&self.commit_lock)
    }


    pub async fn begin_transaction(&self) -> Result<Transaction> {
        let tx_id = self.next_tx_id.fetch_add(1, AtomicOrdering::SeqCst);
        let timestamp = self.next_logical_timestamp();

        log::info!("Starting transaction {} at t={}", tx_id, timestamp);

        self.register_tx_timestamp_sync(tx_id, timestamp);

        Ok(Transaction::new(tx_id, timestamp, Arc::new(self.clone())))
    }

    /// Registra el timestamp de inicio de una transacción activa.
    pub(crate) fn register_tx_timestamp_sync(&self, tx_id: TransactionId, ts: Timestamp) {
        let mut map = self.active_tx_timestamps
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        map.insert(tx_id, ts);
    }

    /// Elimina una transacción del mapa de activas (al commit, rollback o drop).
    pub(crate) fn deregister_tx_timestamp_sync(&self, tx_id: TransactionId) {
        let mut map = self.active_tx_timestamps
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        map.remove(&tx_id);
    }

    /// Retorna el horizonte seguro de GC: el menor timestamp de todas las
    /// transacciones activas, o `next_timestamp` si no hay ninguna activa.
    /// El GC no debe purgar versiones con `valid_to > safe_gc_horizon()`.
    pub fn safe_gc_horizon(&self) -> u64 {
        let map = self.active_tx_timestamps
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if map.is_empty() {
            // No hay transacciones activas: el horizonte es el timestamp actual
            self.next_timestamp.load(AtomicOrdering::SeqCst)
        } else {
            *map.values().min().unwrap_or(&0)
        }
    }

    /// Allocates a monotonically increasing logical timestamp for MVCC/transactions.
    pub(crate) fn next_logical_timestamp(&self) -> u64 {
        self.next_timestamp.fetch_add(1, AtomicOrdering::SeqCst)
    }

    /// Persiste las cotas actuales de los relojes lógicos (`next_timestamp`,
    /// `next_tx_id`) para que sobrevivan reinicios. Las keys meta solo crecen,
    /// así que es seguro llamarlo desde varios puntos concurrentes.
    pub(crate) async fn persist_clocks(&self) -> Result<()> {
        self.storage
            .put_meta_u64_max(
                crate::storage::META_NEXT_TIMESTAMP,
                self.next_timestamp.load(AtomicOrdering::SeqCst),
            )
            .await?;
        self.storage
            .put_meta_u64_max(
                crate::storage::META_NEXT_TX_ID,
                self.next_tx_id.load(AtomicOrdering::SeqCst),
            )
            .await
    }

    #[cfg(feature = "algorithms")]
    /// Topology version for analytics caches (structural node/edge mutations).
    pub(crate) fn topology_version(&self) -> u64 {
        self.topology_version.load(AtomicOrdering::SeqCst)
    }

    /// Bump topology version after structural mutations.
    pub(crate) fn bump_topology_version(&self) {
        self.topology_version.fetch_add(1, AtomicOrdering::SeqCst);
    }

    #[cfg(feature = "algorithms")]
    /// Return cached exact Louvain partition if available.
    pub(crate) async fn get_cached_community_partition_exact(
        &self,
    ) -> Option<(u64, HashMap<NodeId, usize>)> {
        self.community_partition_cache_exact
            .read()
            .await
            .as_ref()
            .map(|c| (c.topology_version, c.assignments.clone()))
    }

    #[cfg(feature = "algorithms")]
    /// Store exact Louvain partition cache (single entry).
    pub(crate) async fn set_cached_community_partition_exact(
        &self,
        topology_version: u64,
        assignments: HashMap<NodeId, usize>,
    ) {
        let mut cache = self.community_partition_cache_exact.write().await;
        *cache = Some(CommunityPartitionCache {
            topology_version,
            assignments,
        });
    }

    #[cfg(feature = "algorithms")]
    /// Return cached Leiden partition if the topology version matches.
    /// Cache miss retorna None; caller debe recomputar y llamar set_cached_leiden_partition.
    pub(crate) async fn get_cached_leiden_partition(
        &self,
    ) -> Option<(u64, HashMap<NodeId, usize>)> {
        self.leiden_partition_cache
            .read()
            .await
            .as_ref()
            .map(|c| (c.topology_version, c.assignments.clone()))
    }

    #[cfg(feature = "algorithms")]
    /// Store Leiden partition cache (single entry, invalidada cuando cambia topology_version).
    /// Independiente de la caché de Louvain — ambos algoritmos coexisten.
    pub(crate) async fn set_cached_leiden_partition(
        &self,
        topology_version: u64,
        assignments: HashMap<NodeId, usize>,
    ) {
        let mut cache = self.leiden_partition_cache.write().await;
        *cache = Some(CommunityPartitionCache {
            topology_version,
            assignments,
        });
    }

    /// Return a cloned snapshot of the first [`TaxonomyIndex`] found in the
    /// index manager, for synchronous use in query evaluation.
    ///
    /// Uses non-blocking `try_read()` internally; returns `None` if no taxonomy
    /// index exists or if the lock is momentarily contended.
    pub(crate) fn get_taxonomy_sync(&self) -> Option<crate::index::TaxonomyIndex> {
        self.index_manager.get_taxonomy_sync()
    }

    /// Instala un snapshot de taxonomía para uso interno del crate.
    ///
    /// Se usa para tests y para flujos internos que necesitan exponer una
    /// taxonomía consistente al executor sin abrir una API pública nueva.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) async fn install_taxonomy_snapshot(
        &self,
        taxonomy: crate::index::TaxonomyIndex,
    ) {
        self.index_manager.set_taxonomy(taxonomy).await;
    }


    /// Obtiene un nodo por ID
    pub async fn get_node(&self, id: NodeId) -> Result<Node> {
        self.storage.get_node(id).await
    }

    pub async fn get_node_by_property(&self, property: &str, value: &str) -> Result<Node> {
        // Asumimos búsqueda estricta de string
        let val = PropertyValue::String(value.to_string());
        let node_ids = self.storage.get_nodes_by_property(property, &val).await?;


        if let Some(id) = node_ids.first() {
            self.get_node(*id).await
        } else {
            Err(NopalError::NodeNotFound(format!("with property {}={}", property, value)))
        }
    }

    /// Obtiene TODOS los NodeIds con una propiedad específica
    pub async fn get_all_nodes_by_property(
        &self,
        property: &str,
        value: &PropertyValue,
    ) -> Result<Vec<NodeId>> {
        self.storage.get_nodes_by_property(property, value).await
    }


    // ═════════════════════════════════════════════════════════
    // PUBLIC API FOR QUERY EXECUTOR
    // ═════════════════════════════════════════════════════════

    /// Get all nodes (for query executor)
    pub async fn get_all_nodes(&self) -> Result<Vec<Node>> {
        self.storage.get_all_nodes().await
    }

    /// Scan nodes in bounded batches (internal use for streaming executor).
    pub(crate) async fn scan_nodes_batch(
        &self,
        label: Option<&str>,
        start_after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Node>, Option<String>)> {
        self.storage.scan_nodes_batch(label, start_after, limit).await
    }

    /// Re-insert a node (upsert) — used by UPDATE executor
    pub async fn storage_insert_node(&self, node: &Node) -> Result<()> {
        self.storage.insert_node(node).await
    }

    /// Re-insert an edge (upsert) — used by UPDATE executor
    pub async fn storage_insert_edge(&self, edge: &Edge) -> Result<()> {
        self.storage.insert_edge(edge).await
    }

    /// Remove a property value from the property index — used by UPDATE executor (P1)
    pub async fn storage_remove_property_index(&self, property: &str, value: &PropertyValue, node_id: NodeId) -> Result<()> {
        self.storage.remove_from_property_index(property, value, node_id).await
    }

    /// Add a property value to the property index — used by UPDATE executor (P1)
    pub async fn storage_add_property_index(&self, property: &str, value: &PropertyValue, node_id: NodeId) -> Result<()> {
        self.storage.save_property_index(property, value, node_id).await
    }

    /// Get all nodes with label filter (for query executor)
    pub async fn get_nodes_by_label(&self, label: &str) -> Result<Vec<Node>> {
        let all_nodes = self.storage.get_all_nodes().await?;
        Ok(all_nodes.into_iter()
            .filter(|n| n.label == label)
            .collect())
    }

    // ═════════════════════════════════════════════════════════
    // PUBLIC API FOR PATTERN MATCHING
    // ═════════════════════════════════════════════════════════

    /// Get all edges (for query executor)
    pub async fn get_all_edges(&self) -> Result<Vec<Edge>> {
        self.storage.get_all_edges().await
    }

    /// Get edges by type/label
    pub async fn get_edges_by_label(&self, edge_type: &str) -> Result<Vec<Edge>> {
        let all_edges = self.storage.get_all_edges().await?;
        Ok(all_edges.into_iter()
            .filter(|e| e.edge_type == edge_type)
            .collect())
    }

    /// Get outgoing edges from a node
    pub async fn get_outgoing_edges(&self, node_id: NodeId) -> Result<Vec<Edge>> {
        // Lock adjacency map
        let adjacency = self.adjacency_out.read().await;

        // Get edge IDs for this node
        let edge_ids = if let Some(edge_set) = adjacency.get(&node_id) {
            edge_set.clone()
        } else {
            return Ok(vec![]);
        };

        // Get the actual edges
        let mut edges = Vec::new();
        for edge_id in edge_ids {
            if let Ok(edge) = self.storage.get_edge(edge_id).await {
                edges.push(edge);
            }
        }

        Ok(edges)
    }

    /// Get incoming edges to a node
    pub async fn get_incoming_edges(&self, node_id: NodeId) -> Result<Vec<Edge>> {
        // Lock adjacency map
        let adjacency = self.adjacency_in.read().await;

        // Get edge IDs for this node
        let edge_ids = if let Some(edge_set) = adjacency.get(&node_id) {
            edge_set.clone()
        } else {
            return Ok(vec![]);
        };

        // Get the actual edges
        let mut edges = Vec::new();
        for edge_id in edge_ids {
            if let Ok(edge) = self.storage.get_edge(edge_id).await {
                edges.push(edge);
            }
        }

        Ok(edges)
    }



    // ═════════════════════════════════════════════════════════
    // NQL QUERY EXECUTION
    // ═════════════════════════════════════════════════════════

    /// Execute any NQL statement (FIND, ADD, DELETE, UPDATE, CREATE INDEX, etc.)
    ///
    /// This is the unified entry point that handles all NQL statement types.
    /// Returns `NqlResult` which can be a query result, write result, etc.
    ///
    /// # Examples
    /// ```no_run
    /// # use nopaldb::Graph;
    /// # async fn example() -> nopaldb::Result<()> {
    /// let graph = Graph::in_memory().await?;
    ///
    /// // Query
    /// let result = graph.execute_statement("find p.name from (p:Person)").await?;
    ///
    /// // Write
    /// let result = graph.execute_statement("add (alice:Person {name: 'Alice'})").await?;
    ///
    /// // Index
    /// let result = graph.execute_statement("create index on Person(name) type hash").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn execute_statement(&self, nql: &str) -> Result<crate::query::nql::NqlResult> {
        use crate::query::nql::{parse, Executor, NqlResult};
        use crate::query::nql::parser::ast::Statement;
        use crate::query::nql::executor::result::{WriteResult, ProfileResult};

        let stmt = parse(nql)?;
        let executor = Executor::new(self);

        match stmt {
            Statement::Query(q) => {
                let export_clause = q.export.clone();
                let result = executor.execute(q).await?;

                // If query has EXPORT clause, convert result to requested format
                if let Some(export) = export_clause {
                    crate::query::nql::executor::export::execute_export(&result, &export)
                } else {
                    Ok(NqlResult::Query(result))
                }
            }
            Statement::Add(add) => {
                let mut tx = self.begin_transaction().await?;
                let result = executor.execute_add(&add, &mut tx).await?;
                tx.commit().await?;
                Ok(NqlResult::Write(WriteResult::from_add(&result)))
            }
            Statement::Delete(del) => {
                let mut tx = self.begin_transaction().await?;
                let result = executor.execute_delete(&del, &mut tx).await?;
                tx.commit().await?;
                Ok(NqlResult::Write(WriteResult::from_delete(&result)))
            }
            Statement::Update(upd) => {
                let mut tx = self.begin_transaction().await?;
                let result = executor.execute_update(&upd, &mut tx).await?;
                tx.commit().await?;
                Ok(NqlResult::Write(WriteResult::from_update(&result)))
            }
            Statement::CreateIndex(idx) => {
                let name = executor.execute_create_index(idx).await?;
                Ok(NqlResult::Index(format!("Index created: {}", name)))
            }
            Statement::DropIndex(idx) => {
                let name = idx.index_name.clone();
                executor.execute_drop_index(idx).await?;
                Ok(NqlResult::Index(format!("Index dropped: {}", name)))
            }
            Statement::Explain(inner) => {
                let plan = executor.execute_explain(*inner).await?;
                Ok(NqlResult::Explain(plan))
            }
            Statement::Profile(inner) => match *inner {
                Statement::Query(q) => {
                    let plan = executor
                        .execute_explain(Statement::Query(q.clone()))
                        .await?;
                    let started = Instant::now();
                    let result = executor.execute(q).await?;
                    let execution_ms = started.elapsed().as_secs_f64() * 1000.0;
                    let path_metrics = executor.take_path_profile_value();
                    let path_query = path_metrics.is_some();

                    Ok(NqlResult::Profile(ProfileResult {
                        plan,
                        statement_type: "query".to_string(),
                        execution_ms,
                        rows_returned: result.len() as i64,
                        columns: result.columns.clone(),
                        path_query,
                        path_metrics,
                    }))
                }
                other => Err(NopalError::QueryExecutionError(format!(
                    "PROFILE only supports queries in F2, got {:?}",
                    other
                ))),
            },
            Statement::Sketch(_) => {
                Ok(NqlResult::Message("SKETCH: not yet available via execute_statement. Use SketchManager directly.".into()))
            }
            Statement::Commit(_) => {
                Ok(NqlResult::Message("COMMIT: not yet available via execute_statement. Use SketchManager directly.".into()))
            }
        }
    }

    /// Execute NQL query string (backward-compatible, FIND queries only)
    ///
    /// For full statement support (ADD, DELETE, UPDATE, CREATE INDEX, etc.),
    /// use `execute_statement()` instead.
    pub async fn execute_nql(&self, query_string: &str) -> Result<crate::query::nql::QueryResult> {
        use crate::query::nql::{parse, Executor};
        use crate::query::nql::parser::ast::Statement;
        use crate::types::PropertyValue;

        let stmt = parse(query_string)?;

        match stmt {
            Statement::Query(q) => {
                let export_clause = q.export.clone();
                let executor = Executor::new(self);
                let result = executor.execute(q).await?;

                // Backward-compatible behavior: execute_nql can return export summaries
                // as a QueryResult when EXPORT is present.
                if let Some(export) = export_clause {
                    let exported = crate::query::nql::executor::export::execute_export(&result, &export)?;

                    if let crate::query::nql::NqlResult::Export { format, data, rows_exported } = exported {
                        if let Some(PropertyValue::String(path)) = export.options.get("path") {
                            let mut qr = crate::query::nql::QueryResult::new(vec![
                                "format".to_string(),
                                "exported_to".to_string(),
                                "rows".to_string(),
                            ]);
                            let mut row = crate::query::nql::Row::new();
                            row.set("format", PropertyValue::String(format));
                            row.set("exported_to", PropertyValue::String(path.clone()));
                            row.set("rows", PropertyValue::Int(rows_exported as i64));
                            qr.add_row(row);
                            Ok(qr)
                        } else {
                            let mut qr = crate::query::nql::QueryResult::new(vec![
                                "format".to_string(),
                                "data".to_string(),
                            ]);
                            let mut row = crate::query::nql::Row::new();
                            row.set("format", PropertyValue::String(format));
                            row.set("data", PropertyValue::String(data));
                            qr.add_row(row);
                            Ok(qr)
                        }
                    } else {
                        Ok(result)
                    }
                } else {
                    Ok(result)
                }
            }
            Statement::Profile(_) => Err(NopalError::QueryExecutionError(
                "PROFILE is only available via execute_statement() in Path Queries F2".into()
            )),
            _ => {
                // For non-query statements, route through execute_statement
                // and extract the summary as a single-row result for compatibility
                let result = self.execute_statement(query_string).await?;
                let mut qr = crate::query::nql::QueryResult::new(vec!["result".to_string()]);
                let mut row = crate::query::nql::Row::new();
                row.set("result", crate::types::PropertyValue::String(result.summary()));
                qr.add_row(row);
                Ok(qr)
            }
        }
    }


    /// Elimina un nodo (y sus aristas)
    pub async fn delete_node(&self, id: NodeId) -> Result<()> {
        // ✅ Obtener nodo antes de borrar
        let node = self.get_node(id).await?;

        // ✅ Limpiar índices de propiedades
        for (key, value) in &node.properties {
            self.storage.remove_from_property_index(key, value, id).await?;
        }

        // ✅ Delete actual edges from storage (P0 fix: prevent orphaned edges)
        let outgoing = self.get_outgoing_edges(id).await?;
        let incoming = self.get_incoming_edges(id).await?;

        for edge in &outgoing {
            // Remove edge from storage
            self.storage.delete_edge(edge.id).await?;
            // Clean target's adjacency_in
            let mut adj_in = self.adjacency_in.write().await;
            if let Some(edges) = adj_in.get_mut(&edge.target) {
                edges.retain(|&e| e != edge.id);
            }
            drop(adj_in);
            self.storage.save_adjacency_in(edge.target,
                                           &self.adjacency_in.read().await.get(&edge.target).cloned().unwrap_or_default()
            ).await?;
        }

        for edge in &incoming {
            self.storage.delete_edge(edge.id).await?;
            // Clean source's adjacency_out
            let mut adj_out = self.adjacency_out.write().await;
            if let Some(edges) = adj_out.get_mut(&edge.source) {
                edges.retain(|&e| e != edge.id);
            }
            drop(adj_out);
            self.storage.save_adjacency_out(edge.source,
                                            &self.adjacency_out.read().await.get(&edge.source).cloned().unwrap_or_default()
            ).await?;
        }

        // Borrar nodo del storage
        self.storage.delete_node(id).await?;

        // Limpiar índices de adyacencia del nodo eliminado
        let mut adj_out = self.adjacency_out.write().await;
        let mut adj_in = self.adjacency_in.write().await;

        adj_out.remove(&id);
        adj_in.remove(&id);

        self.bump_topology_version();

        Ok(())
    }

    /// Agrega una arista al grafo
    pub async fn add_edge(&self, edge: Edge) -> Result<EdgeId> {
        let timestamp = self.next_logical_timestamp();
        self.add_edge_at(edge, timestamp).await
    }

    /// Variante interna: inserta arista con timestamp MVCC explícito (usado en commit de tx).
    pub(crate) async fn add_edge_at(&self, edge: Edge, timestamp: u64) -> Result<EdgeId> {
        let edge_id = edge.id;
        let source = edge.source;
        let target = edge.target;

        // Verificar que los nodos existen
        if !self.storage.node_exists(source).await? {
            return Err(NopalError::NodeNotFound(source.to_string()));
        }
        if !self.storage.node_exists(target).await? {
            return Err(NopalError::NodeNotFound(target.to_string()));
        }

        // Guardar en storage (árbol "edges" — sin cambios)
        self.storage.insert_edge(&edge).await?;

        // Guardar versión MVCC (árbol "versioned_edges")
        self.storage.insert_versioned_edge(&edge, timestamp).await?;

        // Actualizar índices
        let mut adj_out = self.adjacency_out.write().await;
        let mut adj_in = self.adjacency_in.write().await;

        adj_out.entry(source).or_insert_with(Vec::new).push(edge_id);
        adj_in.entry(target).or_insert_with(Vec::new).push(edge_id);

        let source_edges = adj_out.get(&source).cloned().unwrap_or_default();
        let target_edges = adj_in.get(&target).cloned().unwrap_or_default();

        drop(adj_out);
        drop(adj_in);

        self.storage.save_adjacency_out(source, &source_edges).await?;
        self.storage.save_adjacency_in(target, &target_edges).await?;

        self.bump_topology_version();

        Ok(edge_id)
    }

    /// Obtiene una arista por ID
    pub async fn get_edge(&self, id: EdgeId) -> Result<Edge> {
        self.storage.get_edge(id).await
    }

    /// Elimina una arista del grafo
    ///
    /// Elimina la arista del storage y actualiza los índices de adyacencia.
    ///
    /// # Arguments
    /// * `id` - ID de la arista a eliminar
    ///
    /// # Example
    /// ```no_run
    /// # use nopaldb::{Graph, Node, Edge};
    /// # async fn example() -> nopaldb::Result<()> {
    /// let graph = Graph::in_memory().await?;
    /// let a = graph.add_node(Node::new("Person")).await?;
    /// let b = graph.add_node(Node::new("Person")).await?;
    /// let edge = Edge::new(a, b, "KNOWS");
    /// let edge_id = graph.add_edge(edge).await?;
    ///
    /// // Eliminar la arista
    /// graph.delete_edge(edge_id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn delete_edge(&self, id: EdgeId) -> Result<()> {
        let timestamp = self.next_logical_timestamp();
        self.delete_edge_at(id, timestamp).await
    }

    /// Variante interna: elimina arista con timestamp MVCC explícito (usado en commit de tx).
    pub(crate) async fn delete_edge_at(&self, id: EdgeId, timestamp: u64) -> Result<()> {
        // 1. Obtener la arista para saber source/target
        let edge = self.get_edge(id).await?;
        let source = edge.source;
        let target = edge.target;

        // 2. Cerrar la versión MVCC (valid_to = timestamp)
        // Si no existe historial MVCC (aristas antiguas pre-versioning), ignorar silenciosamente
        if let Err(e) = self.storage.mark_edge_deleted(id, timestamp).await {
            log::debug!("mark_edge_deleted: no MVCC record for edge {} ({})", id, e);
        }

        // 3. Eliminar del storage principal (árbol "edges")
        self.storage.delete_edge(id).await?;

        // 4. Actualizar índices de adyacencia
        let mut adj_out = self.adjacency_out.write().await;
        let mut adj_in = self.adjacency_in.write().await;

        if let Some(edges) = adj_out.get_mut(&source) {
            edges.retain(|&e| e != id);
        }
        if let Some(edges) = adj_in.get_mut(&target) {
            edges.retain(|&e| e != id);
        }

        let source_edges = adj_out.get(&source).cloned().unwrap_or_default();
        let target_edges = adj_in.get(&target).cloned().unwrap_or_default();

        drop(adj_out);
        drop(adj_in);

        self.storage.save_adjacency_out(source, &source_edges).await?;
        self.storage.save_adjacency_in(target, &target_edges).await?;

        self.bump_topology_version();

        log::debug!("Deleted edge {} ({} -> {})", id, source, target);

        Ok(())
    }

    /// Add node with label and properties (within transaction)
    pub async fn add_node_with_label_and_props(
        &self,
        label: String,
        properties: HashMap<String, PropertyValue>,
        _tx: &mut Transaction,
    ) -> Result<Node> {
        // Create node
        let node = Node {
            id: NodeId::new_v4(),
            label,
            properties,
            kind: crate::types::NodeKind::Individual,
        };
        // Add to graph
        let _ = self.add_node(node.clone()).await?;
        Ok(node)
    }

    /// Delete node (within transaction)
    pub async fn delete_node_with_tx(
        &self,
        id: NodeId,
        _tx: &mut Transaction,
    ) -> Result<()> {
        // Use existing delete_node method
        self.delete_node(id).await
    }

    /// Update node (within transaction)
    pub async fn update_node_with_tx(
        &self,
        _node: Node,
        _tx: &mut Transaction,
    ) -> Result<()> {
        // TODO: Implement proper node update with transaction
        // For now, just return Ok
        log::warn!("update_node_with_tx not fully implemented");
        Ok(())
    }

    // ═════════════════════════════════════════════════════════
    // ✅ MÉTODOS DE EMBEDDINGS
    // ═════════════════════════════════════════════════════════

    /// Comprueba (sync, no-bloqueante) si existe un embedding para `node_id` y `model`.
    /// Útil para predicados WHERE en el executor NQL donde el contexto es síncrono.
    #[cfg(feature = "embeddings")]
    pub fn node_embedding_exists_sync(&self, node_id: NodeId, model: &str) -> bool {
        self.storage.node_embedding_exists_sync(node_id, model)
    }

    /// Comprueba (sync, estricta) si existe un embedding para `node_id` y `model`.
    ///
    /// Retorna error si el storage sigue ocupado tras los reintentos acotados.
    #[cfg(feature = "embeddings")]
    pub fn try_node_embedding_exists_sync(
        &self,
        node_id: NodeId,
        model: &str,
    ) -> std::result::Result<bool, NopalError> {
        self.storage.try_node_embedding_exists_sync(node_id, model)
    }

    /// Carga (sync, no-bloqueante) el embedding de `node_id` y `model`.
    #[cfg(feature = "embeddings")]
    pub fn get_node_embedding_sync(
        &self,
        node_id: NodeId,
        model: &str,
    ) -> std::result::Result<crate::embeddings::Embedding, NopalError> {
        self.storage.load_node_embedding_sync(node_id, model)
    }

    /// Comprueba (sync, estricta) si existe un embedding para `edge_id` y `model`.
    ///
    /// Retorna error si el storage sigue ocupado tras los reintentos acotados.
    #[cfg(feature = "embeddings")]
    pub fn try_edge_embedding_exists_sync(
        &self,
        edge_id: EdgeId,
        model: &str,
    ) -> std::result::Result<bool, NopalError> {
        self.storage.try_edge_embedding_exists_sync(edge_id, model)
    }

    /// Carga (sync, estricta) el embedding de `edge_id` y `model`.
    #[cfg(feature = "embeddings")]
    pub fn get_edge_embedding_sync(
        &self,
        edge_id: EdgeId,
        model: &str,
    ) -> std::result::Result<crate::embeddings::EdgeEmbedding, NopalError> {
        self.storage.load_edge_embedding_sync(edge_id, model)
    }

    /// Asigna un embedding a un nodo específico
    #[cfg(feature = "embeddings")]
    pub async fn add_node_embedding(&self, node_id: NodeId, vector: Vec<f32>, model: &str) -> std::result::Result<(), NopalError> {
        if !self.storage.node_exists(node_id).await? {
            return Err(NopalError::NodeNotFound(node_id.to_string()));
        }
        let embedding = crate::embeddings::Embedding::new(node_id, vector, model);
        self.storage.save_node_embedding(&embedding).await?;
        // Invalidar índice HNSW en caché: el nuevo embedding lo desactualiza
        #[cfg(feature = "embeddings-index")]
        self.embedding_indices.write().await.remove(model);
        Ok(())
    }

    /// Obtiene el embedding de un nodo
    #[cfg(feature = "embeddings")]
    pub async fn get_node_embedding(&self, node_id: NodeId, model: &str) -> std::result::Result<crate::embeddings::Embedding, NopalError> {
        self.storage.load_node_embedding(node_id, model).await
    }

    /// Asigna un embedding a una arista específica.
    /// Retorna `EdgeNotFound` si la arista no existe.
    #[cfg(feature = "embeddings")]
    pub async fn add_edge_embedding(&self, edge_id: EdgeId, vector: Vec<f32>, model: &str) -> std::result::Result<(), NopalError> {
        // Verificar existencia consultando storage directamente
        self.storage.get_edge(edge_id).await
            .map_err(|_| NopalError::EdgeNotFound(edge_id.to_string()))?;
        let embedding = crate::embeddings::EdgeEmbedding::new(edge_id, vector, model);
        self.storage.save_edge_embedding(&embedding).await?;
        Ok(())
    }

    /// Obtiene el embedding de una arista.
    /// Retorna `Custom` si no se encontró el embedding para ese modelo.
    #[cfg(feature = "embeddings")]
    pub async fn get_edge_embedding(&self, edge_id: EdgeId, model: &str) -> std::result::Result<crate::embeddings::EdgeEmbedding, NopalError> {
        self.storage.load_edge_embedding(edge_id, model).await
    }

    // ───────────────────────────────────────────────────────────
    // E-8: PathReferenceEmbedding
    // ───────────────────────────────────────────────────────────

    /// Persiste una referencia de path embedding para comparacion con `path_embedding_similarity`.
    #[cfg(feature = "embeddings")]
    pub async fn add_path_reference_embedding(
        &self,
        name: String,
        node_model: String,
        edge_model: String,
        vector: Vec<f32>,
    ) -> Result<()> {
        let emb = crate::embeddings::PathReferenceEmbedding::new(name, node_model, edge_model, vector);
        emb.validate()?;
        self.storage.save_path_reference_embedding(&emb).await
    }

    /// Carga (sync) una referencia de path embedding por (name, node_model, edge_model).
    #[cfg(feature = "embeddings")]
    pub fn get_path_reference_embedding_sync(
        &self,
        name: &str,
        node_model: &str,
        edge_model: &str,
    ) -> Result<crate::embeddings::PathReferenceEmbedding> {
        self.storage.load_path_reference_embedding_sync(name, node_model, edge_model)
    }

    /// Carga (sync) todas las PathReferenceEmbedding para el par (node_model, edge_model).
    #[cfg(feature = "embeddings")]
    pub fn get_all_path_references_for_models_sync(
        &self,
        node_model: &str,
        edge_model: &str,
    ) -> Result<Vec<crate::embeddings::PathReferenceEmbedding>> {
        self.storage.load_all_path_references_for_models_sync(node_model, edge_model)
    }

    /// Construye un `HnswIndex` HNSW en RAM para todos los nodos que tienen
    /// un embedding del modelo `model` persistido en Sled.
    ///
    /// Usa `build_batch` con parallel_insert para construcción eficiente.
    /// Retorna error si no hay embeddings para ese modelo.
    ///
    /// El índice devuelto puede usarse directamente para llamar `search_knn`.
    /// No modifica el estado del grafo — construir el índice es responsabilidad
    /// del llamador (por ejemplo, guardarlo en un `Arc<RwLock<HnswIndex>>`).
    #[cfg(feature = "embeddings-index")]
    pub async fn build_embedding_index(
        &self,
        model: &str,
    ) -> std::result::Result<crate::embeddings::HnswIndex, NopalError> {
        let embeddings = self.storage.load_all_node_embeddings_for_model(model).await?;
        if embeddings.is_empty() {
            return Err(NopalError::custom(format!(
                "build_embedding_index: no embeddings found for model '{}'",
                model
            )));
        }
        let dimension = embeddings[0].vector.len();
        let vectors: Vec<(crate::types::NodeId, Vec<f32>)> = embeddings
            .into_iter()
            .map(|emb| (emb.node_id, emb.vector))
            .collect();
        let model_owned = model.to_string();
        // build_batch usa parallel_insert internamente — wrap en spawn_blocking
        // para no bloquear el runtime de Tokio.
        tokio::task::spawn_blocking(move || {
            crate::embeddings::HnswIndex::build_batch(vectors, model_owned, dimension)
        })
        .await
        .map_err(|e| NopalError::custom(format!("build_embedding_index join error: {e}")))?
    }

    /// Devuelve el índice HNSW para `model` desde la caché en memoria,
    /// construyéndolo desde Sled si no existe todavía.
    ///
    /// Cada llamada subsecuente para el mismo `model` retorna el índice ya construido
    /// sin tocar el storage. La caché se invalida automáticamente cuando se guarda
    /// un nuevo embedding via `add_node_embedding()`.
    #[cfg(feature = "embeddings-index")]
    pub async fn get_or_build_embedding_index(
        &self,
        model: &str,
    ) -> std::result::Result<Arc<crate::embeddings::HnswIndex>, NopalError> {
        // Ruta rápida: leer con read-lock
        {
            let cache = self.embedding_indices.read().await;
            if let Some(idx) = cache.get(model) {
                return Ok(Arc::clone(idx));
            }
        }
        // Construir índice (costoso) fuera del lock
        let idx = self.build_embedding_index(model).await?;
        let arc = Arc::new(idx);
        // Escribir en caché con write-lock
        self.embedding_indices
            .write()
            .await
            .insert(model.to_string(), Arc::clone(&arc));
        Ok(arc)
    }

    /// Obtiene los vecinos de un nodo
    pub async fn neighbors(&self, node_id: NodeId, direction: Direction) -> Result<Vec<NodeId>> {
        let edge_ids = match direction {
            Direction::Outgoing => {
                let adj = self.adjacency_out.read().await;
                adj.get(&node_id).cloned().unwrap_or_default()
            }
            Direction::Incoming => {
                let adj = self.adjacency_in.read().await;
                adj.get(&node_id).cloned().unwrap_or_default()
            }
            Direction::Both => {
                let adj_out = self.adjacency_out.read().await;
                let adj_in = self.adjacency_in.read().await;

                let mut combined = adj_out.get(&node_id).cloned().unwrap_or_default();
                combined.extend(adj_in.get(&node_id).cloned().unwrap_or_default());
                combined
            }
        };

        // Obtener los nodos destino de cada arista
        let mut neighbors = Vec::new();
        for edge_id in edge_ids {
            let edge = self.get_edge(edge_id).await?;
            let neighbor = match direction {
                Direction::Outgoing => edge.target,
                Direction::Incoming => edge.source,
                Direction::Both => {
                    if edge.source == node_id {
                        edge.target
                    } else {
                        edge.source
                    }
                }
            };
            neighbors.push(neighbor);
        }

        Ok(neighbors)
    }

    /// Obtiene el grado de un nodo
    pub async fn degree(&self, node_id: NodeId, direction: Direction) -> Result<usize> {
        let count = match direction {
            Direction::Outgoing => {
                let adj = self.adjacency_out.read().await;
                adj.get(&node_id).map(|v| v.len()).unwrap_or(0)
            }
            Direction::Incoming => {
                let adj = self.adjacency_in.read().await;
                adj.get(&node_id).map(|v| v.len()).unwrap_or(0)
            }
            Direction::Both => {
                let adj_out = self.adjacency_out.read().await;
                let adj_in = self.adjacency_in.read().await;

                let out_degree = adj_out.get(&node_id).map(|v| v.len()).unwrap_or(0);
                let in_degree = adj_in.get(&node_id).map(|v| v.len()).unwrap_or(0);

                out_degree + in_degree
            }
        };

        Ok(count)
    }

    /// Obtiene todas las aristas de un nodo
    pub async fn edges_of(&self, node_id: NodeId, direction: Direction) -> Result<Vec<Edge>> {
        let edge_ids = match direction {
            Direction::Outgoing => {
                let adj = self.adjacency_out.read().await;
                adj.get(&node_id).cloned().unwrap_or_default()
            }
            Direction::Incoming => {
                let adj = self.adjacency_in.read().await;
                adj.get(&node_id).cloned().unwrap_or_default()
            }
            Direction::Both => {
                let adj_out = self.adjacency_out.read().await;
                let adj_in = self.adjacency_in.read().await;

                let mut combined = adj_out.get(&node_id).cloned().unwrap_or_default();
                combined.extend(adj_in.get(&node_id).cloned().unwrap_or_default());
                combined
            }
        };

        let mut edges = Vec::new();
        for edge_id in edge_ids {
            edges.push(self.get_edge(edge_id).await?);
        }

        Ok(edges)
    }

    /// Breadth-First Search desde un nodo inicial
    pub async fn bfs(
        &self,
        start: NodeId,
        config: TraversalConfig
    ) -> Result<TraversalResult> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        let mut result_nodes = Vec::new();
        let mut distances = Vec::new();

        queue.push_back((start, 0));
        visited.insert(start);

        while let Some((current_id, depth)) = queue.pop_front() {
            if let Some(max_depth) = config.max_depth
                && depth > max_depth {
                continue;
            }

            if let Some(max_nodes) = config.max_nodes
                && result_nodes.len() >= max_nodes {
                break;
            }

            let current_node = self.get_node(current_id).await?;

            if let Some(ref filter) = config.filter
                && !filter(&current_node) {
                continue;
            }

            result_nodes.push(current_id);
            distances.push(depth);

            let neighbors = self.neighbors(current_id, config.direction).await?;

            for neighbor_id in neighbors {
                if !visited.contains(&neighbor_id) {
                    visited.insert(neighbor_id);
                    queue.push_back((neighbor_id, depth + 1));
                }
            }
        }

        Ok(TraversalResult {
            nodes: result_nodes,
            distances: Some(distances),
            path: None,
        })
    }

    /// Depth-First Search desde un nodo inicial
    pub async fn dfs(
        &self,
        start: NodeId,
        config: TraversalConfig,
    ) -> Result<TraversalResult> {
        let mut visited = HashSet::new();
        let mut result_nodes = Vec::new();

        self.dfs_recursive(
            start,
            0,
            &config,
            &mut visited,
            &mut result_nodes,
        ).await?;

        Ok(TraversalResult {
            nodes: result_nodes,
            distances: None,
            path: None,
        })
    }

    /// Helper recursivo para DFS
    #[async_recursion::async_recursion]
    async fn dfs_recursive(
        &self,
        current_id: NodeId,
        depth: usize,
        config: &TraversalConfig,
        visited: &mut HashSet<NodeId>,
        result: &mut Vec<NodeId>,
    ) -> Result<()> {
        if let Some(max_depth) = config.max_depth
            && depth > max_depth {
            return Ok(());
        }

        if let Some(max_nodes) = config.max_nodes
            && result.len() >= max_nodes {
            return Ok(());
        }

        if !visited.insert(current_id) {
            return Ok(());
        }

        let current_node = self.get_node(current_id).await?;

        if let Some(ref filter) = config.filter
            && !filter(&current_node) {
            return Ok(());
        }

        result.push(current_id);

        let neighbors = self.neighbors(current_id, config.direction).await?;

        for neighbor_id in neighbors {
            if !visited.contains(&neighbor_id) {
                self.dfs_recursive(
                    neighbor_id,
                    depth + 1,
                    config,
                    visited,
                    result,
                ).await?;
            }
        }

        Ok(())
    }

    /// Encuentra el camino más corto entre dos nodos (Dijkstra)
    pub async fn shortest_path(
        &self,
        start: NodeId,
        target: NodeId,
        config: TraversalConfig,
    ) -> Result<Option<TraversalResult>> {
        let mut distances: HashMap<NodeId, usize> = HashMap::new();
        let mut previous: HashMap<NodeId, NodeId> = HashMap::new();
        let mut heap = BinaryHeap::new();

        distances.insert(start, 0);
        heap.push(PathState { node_id: start, cost: 0 });

        while let Some(PathState { node_id, cost }) = heap.pop() {
            if node_id == target {
                let mut path = Vec::new();
                let mut current = target;

                while current != start {
                    path.push(current);
                    match previous.get(&current) {
                        Some(&prev) => current = prev,
                        None => break, // Broken path chain, return what we have
                    }
                }
                path.push(start);
                path.reverse();

                return Ok(Some(TraversalResult {
                    nodes: path.clone(),
                    distances: None,
                    path: Some(path),
                }));
            }

            if let Some(&dist) = distances.get(&node_id)
                && cost > dist {
                continue;
            }

            let neighbors = self.neighbors(node_id, config.direction).await?;

            for neighbor_id in neighbors {
                let next_cost = cost + 1;

                let is_better = distances
                    .get(&neighbor_id)
                    .map(|&current| next_cost < current)
                    .unwrap_or(true);

                if is_better {
                    distances.insert(neighbor_id, next_cost);
                    previous.insert(neighbor_id, node_id);
                    heap.push(PathState {
                        node_id: neighbor_id,
                        cost: next_cost,
                    });
                }
            }
        }

        Ok(None)
    }

    /// Crea un traverse builder desde este grafo
    pub fn traverse(&self, start: NodeId) -> crate::query::TraverseBuilder {
        crate::query::TraverseBuilder::new(Arc::new(self.clone()), start)
    }

    // Método para registrar modificación
    #[cfg(feature = "full-isolation")]
    pub(crate) async fn mark_modified(&self, node_id: NodeId, timestamp: u64) -> Result<()> {
        let mut last_mod = self.last_modified.write().await;
        last_mod.insert(node_id, timestamp);
        Ok(())
    }

    // Método para obtener timestamp de última modificación
    #[cfg(feature = "full-isolation")]
    pub(crate) async fn get_last_modified(&self, node_id: NodeId) -> Option<u64> {
        let last_mod = self.last_modified.read().await;
        last_mod.get(&node_id).copied()
    }


    /// Replay operaciones desde WAL (recovery)
    async fn replay_wal(&self) -> Result<()> {
        let operations = self.wal.get_replay_operations().await?;

        let mut replayed = 0;

        for operation in operations {
            match operation {
                WalRecord::InsertNode { node, .. } => {
                    // Solo insertar si no existe (idempotencia)
                    if !self.storage.node_exists(node.id).await? {
                        self.add_node_internal(node, false).await?;
                        replayed += 1;
                    }
                }

                WalRecord::DeleteNode { node_id, .. } => {
                    // Solo borrar si existe
                    if self.storage.node_exists(node_id).await? {
                        self.delete_node(node_id).await?;
                        replayed += 1;
                    }
                }

                WalRecord::InsertEdge { edge, .. } => {
                    // Solo insertar si no existe
                    if !self.storage.edge_exists(edge.id).await? {
                        self.add_edge(edge).await?;
                        replayed += 1;
                    }
                }

                WalRecord::DeleteEdge { edge_id, .. } => {
                    // Solo borrar si existe
                    if self.storage.edge_exists(edge_id).await? {
                        self.delete_edge(edge_id).await?;
                        replayed += 1;
                    }
                }

                WalRecord::UpdateNode { node_id: _, new_node, .. } => {
                    // Update es insert (replace)
                    self.add_node_internal(new_node, false).await?;
                    replayed += 1;
                }

                _ => {}
            }
        }

        log::info!("Replayed {} operations from WAL", replayed);

        Ok(())
    }

    /// Busca nodos por propiedad (público para tests)
    pub async fn find_nodes_by_property(
        &self,
        property: &str,
        value: &PropertyValue,
    ) -> Result<Vec<NodeId>> {
        self.storage.get_nodes_by_property(property, value).await
    }

    pub async fn checkpoint(&self) -> Result<()> {
        log::info!("Creating checkpoint...");

        // 1. Flush todos los índices a disco
        self.flush_indices().await?;

        // 2. Obtener transacciones activas para el WAL checkpoint
        let active_txs: Vec<TransactionId> = {
            let map = self.active_tx_timestamps
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            map.keys().cloned().collect()
        };

        // 3. Escribir checkpoint al WAL
        self.wal.checkpoint(active_txs).await?;

        // 4. Truncar WAL antiguo
        self.wal.truncate_after_checkpoint().await?;

        // 5. Persistir relojes lógicos: tras truncar el WAL ya no se puede
        //    derivar el máximo timestamp desde el log en el próximo open.
        self.persist_clocks().await?;

        log::info!("Checkpoint completed");

        Ok(())
    }

    /// Ejecuta garbage collection de versiones MVCC antiguas.
    ///
    /// Elimina versiones que han sido invalidadas y son más viejas que el timestamp de corte.
    /// Siempre mantiene al menos `min_versions_to_keep` versiones por nodo.
    ///
    /// # Example
    /// ```ignore
    /// use nopaldb::mvcc::GCConfig;
    ///
    /// // Eliminar versiones más viejas de 7 días
    /// let config = GCConfig::older_than_days(7);
    /// let stats = graph.gc(config).await?;
    /// println!("Freed {} versions", stats.versions_deleted);
    ///
    /// // Dry run (solo reportar)
    /// let config = GCConfig::older_than_hours(24).dry_run();
    /// let stats = graph.gc(config).await?;
    /// println!("Would delete {} versions", stats.versions_deleted);
    /// ```
    pub async fn gc(&self, mut config: crate::mvcc::GCConfig) -> Result<crate::mvcc::GCStats> {
        // Si se pide usar el horizonte activo, limitamos el cutoff al mínimo
        // timestamp de todas las transacciones en vuelo para no borrar versiones
        // que alguna tx aún necesita.
        if config.use_active_horizon {
            let horizon = self.safe_gc_horizon();
            if config.cutoff_timestamp > horizon {
                config.cutoff_timestamp = horizon;
            }
        }

        log::info!(
            "Starting MVCC garbage collection (cutoff: {}, keep: {}, dry_run: {})",
            config.cutoff_timestamp,
            config.min_versions_to_keep,
            config.dry_run
        );

        self.storage.gc_old_versions(&config).await
    }

    /// Ejecuta garbage collection con configuración por defecto.
    /// Usa un cutoff conservador: el mínimo entre safe_gc_horizon() y now-7días.
    pub async fn gc_default(&self) -> Result<crate::mvcc::GCStats> {
        let horizon = self.safe_gc_horizon();
        let seven_days_ago = crate::mvcc::GCConfig::older_than_days(7).cutoff_timestamp;
        let safe_cutoff = horizon.min(seven_days_ago);
        let config = crate::mvcc::GCConfig {
            cutoff_timestamp: safe_cutoff,
            use_active_horizon: false, // ya aplicamos el horizonte manualmente
            ..Default::default()
        };
        self.gc(config).await
    }

    /// Inicia GC automático en background.
    ///
    /// Si ya hay un scheduler activo, se detiene y se reemplaza.
    pub async fn start_auto_gc(&self, config: AutoGcConfig) -> Result<()> {
        if config.interval_secs == 0 {
            return Err(NopalError::custom("Auto GC interval_secs must be > 0"));
        }

        let _ = self.stop_auto_gc().await?;

        let (stop_tx, mut stop_rx) = watch::channel(false);
        let graph = self.clone();
        let runtime_cfg = config.clone();

        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(runtime_cfg.interval_secs));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            ticker.tick().await; // consume immediate first tick

            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        match graph.gc(runtime_cfg.gc_config.clone()).await {
                            Ok(stats) => {
                                log::info!(
                                    "Auto GC cycle complete: scanned={}, deleted={}, bytes_freed={}, duration_ms={}",
                                    stats.nodes_scanned,
                                    stats.versions_deleted,
                                    stats.bytes_freed,
                                    stats.duration_ms
                                );
                            }
                            Err(err) => {
                                log::warn!("Auto GC cycle failed: {}", err);
                            }
                        }
                    }
                    changed = stop_rx.changed() => {
                        if changed.is_err() {
                            break;
                        }
                        if *stop_rx.borrow() {
                            break;
                        }
                    }
                }
            }
        });

        {
            let mut tx_slot = self.auto_gc_stop_tx.lock().await;
            *tx_slot = Some(stop_tx);
        }
        {
            let mut task_slot = self.auto_gc_task.lock().await;
            *task_slot = Some(handle);
        }
        {
            let mut cfg_slot = self.auto_gc_config.write().await;
            *cfg_slot = Some(config);
        }

        Ok(())
    }

    /// Detiene GC automático si está activo.
    ///
    /// Retorna `true` si había scheduler activo.
    pub async fn stop_auto_gc(&self) -> Result<bool> {
        let tx_opt = {
            let mut tx_slot = self.auto_gc_stop_tx.lock().await;
            tx_slot.take()
        };

        let mut was_running = false;

        if let Some(tx) = tx_opt {
            was_running = true;
            let _ = tx.send(true);
        }

        let handle_opt = {
            let mut task_slot = self.auto_gc_task.lock().await;
            task_slot.take()
        };

        if let Some(handle) = handle_opt {
            was_running = true;
            if let Err(err) = handle.await {
                log::warn!("Auto GC task join error: {}", err);
            }
        }

        if was_running {
            let mut cfg_slot = self.auto_gc_config.write().await;
            *cfg_slot = None;
        }

        Ok(was_running)
    }

    /// Estado actual del scheduler de GC automático.
    pub async fn auto_gc_status(&self) -> AutoGcStatus {
        let running = {
            let task_slot = self.auto_gc_task.lock().await;
            task_slot
                .as_ref()
                .map(|handle| !handle.is_finished())
                .unwrap_or(false)
        };
        let config = self.auto_gc_config.read().await.clone();
        AutoGcStatus { running, config }
    }

    // ═════════════════════════════════════════════════════════
    // METODOS MVCC - TIME TRAVEL
    // ═════════════════════════════════════════════════════════

    /// Obtiene un snapshot del grafo en un timestamp específico (Datomic-style)
    pub fn as_of(&self, timestamp: u64) -> GraphSnapshot {
        GraphSnapshot {
            graph: self.clone(),
            timestamp,
        }
    }

    /// Obtiene el historial completo de un nodo
    pub async fn history(&self, node_id: NodeId) -> Result<Vec<VersionedNode>> {
        self.storage.get_node_history(node_id).await
    }

    /// Return all `NodeKind::Class` nodes that were valid at `timestamp`.
    ///
    /// Uses the MVCC version chain: a node is considered valid at `timestamp`
    /// if its `valid_from <= timestamp < valid_to` (or `valid_to` is None).
    /// Falls back to the current node if no MVCC history exists for a node ID.
    #[cfg(feature = "reasoner")]
    pub async fn get_class_nodes_at(&self, timestamp: u64) -> Result<Vec<crate::types::Node>> {
        use crate::types::NodeKind;

        // Collect all node IDs from current storage.
        let all_current = self.storage.get_all_nodes().await?;

        let mut class_nodes = Vec::new();
        for current_node in &all_current {
            // Skip non-Class nodes quickly using the current state as a hint.
            // Nodes don't change kind after creation, so this is safe.
            if current_node.kind != NodeKind::Class {
                continue;
            }
            // Try to get the MVCC version valid at `timestamp`.
            match self.storage.get_node_at_timestamp(current_node.id, timestamp).await {
                Ok(versioned) => {
                    if versioned.is_valid_at(timestamp) {
                        class_nodes.push(versioned.node_data);
                    }
                }
                Err(_) => {
                    // No MVCC record — node was added in the same transaction and
                    // has only a current snapshot. Include if it was before timestamp.
                    class_nodes.push(current_node.clone());
                }
            }
        }

        Ok(class_nodes)
    }

    /// Return all edges of `edge_type` that were valid at `timestamp`.
    ///
    /// Since edges currently lack MVCC versioning in NopalDB, this method
    /// returns all edges of the given type from the current storage.
    /// Future: wire edge version chains when implemented.
    #[cfg(feature = "reasoner")]
    pub async fn get_edges_of_type_at(
        &self,
        edge_type: &str,
        timestamp: u64,
    ) -> Result<Vec<crate::types::Edge>> {
        self.storage
            .get_versioned_edges_of_type_at(edge_type, timestamp)
            .await
    }

    /// Retorna el historial MVCC completo de una arista, de más antigua a más reciente.
    pub async fn edge_history(&self, id: EdgeId) -> Result<Vec<crate::mvcc::VersionedEdge>> {
        self.storage.get_edge_history(id).await
    }

    // ═══════════════════════════════════════════════════════════════════════
    // OWL/TURTLE IMPORT API
    // ═══════════════════════════════════════════════════════════════════════

    /// Import a Turtle/OWL source string into the graph.
    ///
    /// Delegates to `crate::rdf_owl::importer::import_turtle`, using the
    /// `IndexManager` to obtain/update the shared `TaxonomyIndex`.
    ///
    /// Returns an `ImportReport` with counts of classes, edges, and instances added.
    #[cfg(feature = "owl-import")]
    pub async fn import_turtle(
        &self,
        turtle_source: &str,
    ) -> Result<crate::rdf_owl::importer::ImportReport> {
        let mut taxonomy = self.index_manager.get_or_create_taxonomy();
        let report = crate::rdf_owl::importer::import_turtle(self, &mut taxonomy, turtle_source).await?;
        self.index_manager.set_taxonomy(taxonomy).await;
        Ok(report)
    }

    /// Rebuild the TaxonomyIndex from Class nodes and `subClassOf` edges stored in the graph.
    ///
    /// Called automatically by `open_with_options` when `NodeKind::Class` nodes are detected,
    /// so that `instanceOf` / `subClassOf` NQL predicates work across process boundaries
    /// (e.g. when the MCP server opens a DB previously populated by `import_turtle`).
    ///
    /// Idempotent: safe to call multiple times; always rebuilds from current graph state.
    #[cfg(feature = "reasoner")]
    pub(crate) async fn rebuild_taxonomy_from_graph(&self) -> Result<()> {
        use crate::types::NodeKind;

        let nodes = self.storage.get_all_nodes().await?;
        let class_nodes: Vec<_> = nodes.into_iter().filter(|n| n.kind == NodeKind::Class).collect();
        if class_nodes.is_empty() {
            return Ok(());
        }

        let mut tax = crate::index::TaxonomyIndex::new();
        for node in &class_nodes {
            tax.register_class(node.id, &node.label);
        }

        // Edges stored by importer as source=child, target=parent.
        // add_subclass(parent, child) wires the hierarchy correctly.
        let edges = self.storage.get_all_edges().await?;
        for edge in &edges {
            if edge.edge_type == "subClassOf" {
                let _ = tax.add_subclass(edge.target, edge.source);
            }
        }

        self.index_manager.set_taxonomy(tax).await;
        Ok(())
    }

    /// Import a Turtle/OWL file from disk into the graph.
    ///
    /// Reads the file asynchronously and delegates to [`Self::import_turtle`].
    #[cfg(feature = "owl-import")]
    pub async fn import_owl_file(
        &self,
        path: impl AsRef<std::path::Path>,
    ) -> Result<crate::rdf_owl::importer::ImportReport> {
        let source = tokio::fs::read_to_string(path)
            .await
            .map_err(NopalError::IoError)?;
        self.import_turtle(&source).await
    }

    /// Export the ontological content of the graph to a Turtle (.ttl) string.
    ///
    /// Only exports OWL-origin content:
    /// - `NodeKind::Class` nodes → `rdf:type owl:Class`
    /// - Edges of type `"subClassOf"` → `rdfs:subClassOf`
    /// - `NodeKind::Individual` nodes with an `"iri"` property → instance triples + data properties
    ///
    /// Ordinary NopalDB data nodes (without an `"iri"` property) are not exported,
    /// allowing mixed graphs (OWL + data) to produce clean ontology output.
    #[cfg(feature = "owl-import")]
    pub async fn export_turtle(&self) -> Result<String> {
        crate::rdf_owl::exporter::export_turtle(self).await
    }

    /// Export the ontological content of the graph to a Turtle (.ttl) file.
    ///
    /// Delegates to [`Self::export_turtle`] and writes the result to `path`.
    #[cfg(feature = "owl-import")]
    pub async fn export_owl_file(
        &self,
        path: impl AsRef<std::path::Path>,
    ) -> Result<()> {
        let content = self.export_turtle().await?;
        tokio::fs::write(path, content)
            .await
            .map_err(NopalError::IoError)
    }

    /// Obtiene un nodo en un timestamp específico
    pub async fn get_node_at(&self, node_id: NodeId, timestamp: u64) -> Result<Node> {
        // Primero intentar obtener versión por timestamp
        match self.storage.get_node_at_timestamp(node_id, timestamp).await {
            Ok(versioned) => {
                log::debug!(
                    "Found node {} at t={} (version {})",
                    node_id, timestamp, versioned.version
                );
                Ok(versioned.node_data)
            }
            Err(_) => {
                // Fallback: intentar obtener nodo actual (sin MVCC)
                log::debug!(
                    "No MVCC version found for {} at t={}, trying current",
                    node_id, timestamp
                );
                self.get_node(node_id).await
            }
        }
    }

    /// Obtiene un nodo estrictamente desde MVCC en un timestamp específico.
    /// No hace fallback al estado actual, para preservar semántica de snapshot isolation.
    pub async fn get_node_at_strict(&self, node_id: NodeId, timestamp: u64) -> Result<Node> {
        let versioned = self.storage.get_node_at_timestamp(node_id, timestamp).await?;
        Ok(versioned.node_data)
    }

    // ═════════════════════════════════════════════════════════
    // MÉTODOS PÚBLICOS PARA MVCC (para Transaction)
    // ═════════════════════════════════════════════════════════

    /// Verifica si un nodo existe (público para Transaction)
    pub async fn node_exists(&self, id: NodeId) -> Result<bool> {
        self.storage.node_exists(id).await
    }

    /// Obtiene versión actual de un nodo (público para Transaction)
    pub async fn get_current_version(&self, id: NodeId) -> Result<u64> {
        self.storage.get_current_version(id).await
    }

    /// Obtiene versión específica de un nodo (público para Transaction)
    pub async fn get_node_version(&self, id: NodeId, version: u64) -> Result<VersionedNode> {
        self.storage.get_node_version(id, version).await
    }

    /// Invalida versión actual (público para Transaction)
    pub async fn invalidate_current_version(&self, id: NodeId, timestamp: u64) -> Result<()> {
        self.storage.invalidate_current_version(id, timestamp).await
    }

    /// Inserta versión de nodo (público para Transaction)
    pub async fn insert_node_version(&self, versioned: &VersionedNode) -> Result<()> {
        self.storage.insert_node_version(versioned).await
    }

    /// Get complete schema information
    ///
    /// # Example
    /// ```no_run
    /// # use nopaldb::Graph;
    /// # async fn example() -> nopaldb::Result<()> {
    /// let graph = Graph::in_memory().await?;
    /// let schema = graph.get_schema().await?;
    /// println!("Labels: {:?}", schema.node_labels);
    /// println!("Edge types: {:?}", schema.edge_types);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_schema(&self) -> Result<SchemaInfo> {
        self.schema_manager.get_info(self).await
    }

    /// Get all unique node labels
    ///
    /// # Example
    /// ```no_run
    /// # use nopaldb::Graph;
    /// # async fn example() -> nopaldb::Result<()> {
    /// let graph = Graph::in_memory().await?;
    /// let labels = graph.get_labels().await?;
    /// for label in labels {
    ///     println!("Label: {}", label);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_labels(&self) -> Result<Vec<String>> {
        let schema = self.get_schema().await?;
        Ok(schema.node_labels)
    }

    /// Get all unique edge types
    ///
    /// # Example
    /// ```no_run
    /// # use nopaldb::Graph;
    /// # async fn example() -> nopaldb::Result<()> {
    /// let graph = Graph::in_memory().await?;
    /// let types = graph.get_edge_types().await?;
    /// println!("Edge types: {:?}", types);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_edge_types(&self) -> Result<Vec<String>> {
        let schema = self.get_schema().await?;
        Ok(schema.edge_types)
    }

    /// Get all properties for a specific node label
    ///
    /// # Arguments
    /// * `label` - The node label to query
    ///
    /// # Example
    /// ```no_run
    /// # use nopaldb::Graph;
    /// # async fn example() -> nopaldb::Result<()> {
    /// let graph = Graph::in_memory().await?;
    /// let props = graph.get_label_properties("Person").await?;
    /// println!("Person properties: {:?}", props);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_label_properties(&self, label: &str) -> Result<Vec<String>> {
        let schema = self.get_schema().await?;
        Ok(schema
            .node_properties
            .get(label)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default())
    }

    /// Get node count for a specific label
    ///
    /// # Arguments
    /// * `label` - The node label to count
    ///
    /// # Example
    /// ```no_run
    /// # use nopaldb::Graph;
    /// # async fn example() -> nopaldb::Result<()> {
    /// let graph = Graph::in_memory().await?;
    /// let count = graph.get_label_count("Person").await?;
    /// println!("Total Person nodes: {}", count);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_label_count(&self, label: &str) -> Result<usize> {
        let schema = self.get_schema().await?;
        Ok(*schema.node_counts.get(label).unwrap_or(&0))
    }

    /// Get all properties for a specific edge type
    ///
    /// # Arguments
    /// * `edge_type` - The edge type to query
    ///
    /// # Example
    /// ```no_run
    /// # use nopaldb::Graph;
    /// # async fn example() -> nopaldb::Result<()> {
    /// let graph = Graph::in_memory().await?;
    /// let props = graph.get_edge_type_properties("KNOWS").await?;
    /// println!("KNOWS properties: {:?}", props);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_edge_type_properties(&self, edge_type: &str) -> Result<Vec<String>> {
        let schema = self.get_schema().await?;
        Ok(schema
            .edge_properties
            .get(edge_type)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default())
    }

    /// Get edge count for a specific type
    ///
    /// # Arguments
    /// * `edge_type` - The edge type to count
    ///
    /// # Example
    /// ```no_run
    /// # use nopaldb::Graph;
    /// # async fn example() -> nopaldb::Result<()> {
    /// let graph = Graph::in_memory().await?;
    /// let count = graph.get_edge_type_count("KNOWS").await?;
    /// println!("Total KNOWS edges: {}", count);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn get_edge_type_count(&self, edge_type: &str) -> Result<usize> {
        let schema = self.get_schema().await?;
        Ok(*schema.edge_counts.get(edge_type).unwrap_or(&0))
    }

    /// Force rebuild of schema cache
    ///
    /// Useful after bulk imports or major changes.
    ///
    /// # Example
    /// ```no_run
    /// # use nopaldb::Graph;
    /// # async fn example() -> nopaldb::Result<()> {
    /// let graph = Graph::in_memory().await?;
    /// graph.rebuild_schema().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn rebuild_schema(&self) -> Result<()> {
        self.schema_manager.rebuild(self).await
    }

    /// Mark schema as dirty (will be rebuilt on next access)
    pub fn invalidate_schema(&self) {
        self.schema_manager.mark_dirty();
    }


    #[doc(hidden)]
    pub fn konami(&self) {
        crate::easter_eggs::konami_code();
    }

    /// 🎬 Show NopalDB credits
    #[doc(hidden)]
    pub fn credits(&self) {
        crate::easter_eggs::show_credits();
    }

    /// 💡 Get a random fun fact about NopalDB
    #[doc(hidden)]
    pub fn fun_fact(&self) {
        crate::easter_eggs::fun_facts();
    }

    /// 💪 Get motivational message
    #[doc(hidden)]
    pub fn motivate(&self) -> &'static str {
        crate::easter_eggs::motivational_message()
    }


    #[cfg(feature = "analytics")]
    /// Export all nodes to Apache Arrow RecordBatch (columnar format)
    ///
    /// This enables:
    /// - SIMD-optimized analytics
    /// - Zero-copy to Python/PyTorch
    /// - Parquet file export
    /// - DuckDB/Polars integration
    ///
    /// # Example
    /// ```no_run
    /// # use nopaldb::Graph;
    /// # async fn example() -> nopaldb::Result<()> {
    /// let graph = Graph::in_memory().await?;
    ///
    /// // Export to Arrow
    /// let batch = graph.to_arrow().await?;
    /// println!("Exported {} nodes", batch.num_rows());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn to_arrow(&self) -> Result<arrow::record_batch::RecordBatch> {
        let nodes = self.storage.get_all_nodes().await?;
        crate::arrow_export::nodes_to_arrow(&nodes)
    }

    #[cfg(feature = "analytics")]
    /// Export versioned nodes (history) to Arrow (MVCC + Arrow)
    ///
    /// # Example
    /// ```no_run
    /// # use nopaldb::Graph;
    /// # async fn example() -> nopaldb::Result<()> {
    /// let graph = Graph::in_memory().await?;
    ///
    /// // Export full history
    /// let batch = graph.history_to_arrow().await?;
    /// println!("Exported {} versions", batch.num_rows());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn history_to_arrow(&self) -> Result<arrow::record_batch::RecordBatch> {
        // Get all versioned nodes from storage
        let nodes = self.storage.get_all_versioned_nodes().await?;

        if nodes.is_empty() {
            return Err(NopalError::Custom(
                "No versioned nodes found in database".into()
            ));
        }

        crate::arrow_export::versioned_nodes_to_arrow(&nodes)
    }

    #[cfg(feature = "analytics")]
    /// Export graph to Parquet file
    ///
    /// Parquet provides:
    /// - Efficient compression (SNAPPY)
    /// - Fast columnar queries
    /// - Industry standard format
    ///
    /// # Example
    /// ```no_run
    /// # use nopaldb::Graph;
    /// # async fn example() -> nopaldb::Result<()> {
    /// let graph = Graph::open("./data").await?;
    ///
    /// // Export to Parquet
    /// graph.export_parquet("snapshot.parquet").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn export_parquet(
        &self,
        path: impl AsRef<std::path::Path>,
    ) -> Result<()> {
        let batch = self.to_arrow().await?;
        crate::arrow_export::write_parquet(&batch, path)?;

        Ok(())
    }

    #[cfg(feature = "analytics")]
    /// Import graph from Parquet file
    pub async fn import_parquet(
        &self,
        path: impl AsRef<std::path::Path>,
    ) -> Result<()> {
        let batch = crate::arrow_export::read_parquet(&path)?;

        // Reconstruct nodes from Arrow columns: id, label, property_count
        let id_col = batch.column_by_name("id")
            .ok_or_else(|| NopalError::Custom("Parquet missing 'id' column".into()))?;
        let label_col = batch.column_by_name("label")
            .ok_or_else(|| NopalError::Custom("Parquet missing 'label' column".into()))?;

        let ids = id_col.as_any().downcast_ref::<arrow::array::StringArray>()
            .ok_or_else(|| NopalError::Custom("'id' column is not String type".into()))?;
        let labels = label_col.as_any().downcast_ref::<arrow::array::StringArray>()
            .ok_or_else(|| NopalError::Custom("'label' column is not String type".into()))?;

        let mut imported = 0usize;
        for i in 0..batch.num_rows() {
            if let (Some(id_str), Some(label)) = (ids.value(i).into(), labels.value(i).into()) {
                let id: NodeId = id_str.parse()
                    .map_err(|_| NopalError::Custom(format!("Invalid UUID in parquet row {}: {}", i, id_str)))?;
                let node = Node {
                    id,
                    label: label.to_string(),
                    properties: std::collections::HashMap::new(),
                    kind: crate::types::NodeKind::Individual,
                };
                self.storage.insert_node(&node).await?;
                imported += 1;
            }
        }

        log::info!("Imported {} nodes from parquet (properties not included in basic format — use export_parquet with label for full roundtrip)", imported);
        Ok(())
    }


    #[cfg(feature = "analytics")]
    /// Export nodes to Arrow with properties
    ///
    /// When label is provided, exports only nodes of that label with their properties.
    /// Otherwise, exports metadata only.
    pub async fn to_arrow_with_label(&self, label: Option<&str>) -> Result<arrow::record_batch::RecordBatch> {
        let nodes = self.storage.get_all_nodes().await?;

        if let Some(label_filter) = label {
            crate::arrow_export::nodes_to_arrow_with_properties(&nodes, Some(label_filter))
        } else {
            crate::arrow_export::nodes_to_arrow(&nodes)
        }
    }

    // ═══════════════════════════════════════════════════════════════════════
    // BULK LOAD API - High Performance Data Import
    // ═══════════════════════════════════════════════════════════════════════

    /// Crea un BulkLoader para importación masiva de datos.
    ///
    /// **USO RECOMENDADO** para cargar grandes volúmenes de datos (>10K registros).
    /// Es 100-1000x más rápido que insertar uno por uno.
    pub fn bulk_loader(&self, batch_size: usize) -> BulkLoader {
        BulkLoader::new(self.clone(), batch_size)
    }

    /// Inserta múltiples nodos en batch (sin indexación de propiedades).
    pub async fn add_nodes_batch(&self, nodes: Vec<Node>) -> Result<Vec<NodeId>> {
        if nodes.is_empty() {
            return Ok(Vec::new());
        }

        // 1. Batch insert en storage
        let ids = self.storage.insert_nodes_batch(&nodes).await?;

        // 2. Inicializar índices de adyacencia en memoria
        {
            let mut adj_out = self.adjacency_out.write().await;
            let mut adj_in = self.adjacency_in.write().await;

            for node in &nodes {
                adj_out.insert(node.id, Vec::new());
                adj_in.insert(node.id, Vec::new());
            }
        }

        // 3. Batch save de índices vacíos
        let empty_indices: Vec<_> = ids.iter().map(|id| (*id, Vec::new())).collect();
        self.storage.save_adjacency_batch(&empty_indices, &empty_indices).await?;

        self.bump_topology_version();

        Ok(ids)
    }

    /// Inserta múltiples aristas en batch.
    pub async fn add_edges_batch(&self, edges: Vec<Edge>) -> Result<Vec<EdgeId>> {
        if edges.is_empty() {
            return Ok(Vec::new());
        }

        // 1. Batch insert en storage
        let ids = self.storage.insert_edges_batch(&edges).await?;

        // 2. Actualizar índices de adyacencia en memoria
        {
            let mut adj_out = self.adjacency_out.write().await;
            let mut adj_in = self.adjacency_in.write().await;

            for edge in &edges {
                adj_out.entry(edge.source).or_default().push(edge.id);
                adj_in.entry(edge.target).or_default().push(edge.id);
            }
        }

        self.bump_topology_version();

        Ok(ids)
    }

    /// Create an index on a label's property
    pub async fn create_index(
        &self,
        label: &str,
        property: &str,
        index_type: IndexType,
    ) -> Result<String> {
        log::info!("Creating index on {}.{}", label, property);

        // Step 1: Create index metadata
        let index_name = self.index_manager.create_index(label, property, index_type.clone()).await?;
        log::debug!("Index metadata created: {}", index_name);

        // Taxonomy indexes require a two-phase population (nodes then edges).
        if index_type == IndexType::Taxonomy {
            log::info!("Populating taxonomy index {} (label={}, edge_type={})", index_name, label, property);

            // Phase A: register Class nodes.
            let nodes = self.get_nodes_by_label(label).await?;
            let mut node_count = 0;
            for node in &nodes {
                self.index_manager
                    .insert(&index_name, crate::types::PropertyValue::String(node.label.clone()), node.id)
                    .await?;
                node_count += 1;
            }

            // Phase B: wire subClassOf edges.
            let all_edges = self.storage.get_all_edges().await?;
            let mut edge_count = 0;
            for edge in &all_edges {
                if edge.edge_type == property {
                    self.index_manager
                        .add_relationship(&index_name, edge.source, edge.target)
                        .await?;
                    edge_count += 1;
                }
            }

            log::info!("✅ Taxonomy index {}: {} nodes, {} edges", index_name, node_count, edge_count);
            return Ok(index_name);
        }

        // Step 2: Populate index with existing nodes (hash / btree / fulltext).
        log::info!("Populating index with existing nodes...");

        let nodes = self.get_nodes_by_label(label).await?;
        log::debug!("Found {} nodes with label {}", nodes.len(), label);

        let mut indexed_count = 0;
        for node in nodes {
            if let Some(value) = node.properties.get(property) {
                self.index_manager
                    .insert(&index_name, value.clone(), node.id)
                    .await?;
                indexed_count += 1;
            }
        }

        log::info!("✅ Indexed {} nodes in {}", indexed_count, index_name);

        Ok(index_name)
    }
    /// Drop an index
    pub async fn drop_index(&self, index_name: &str) -> Result<()> {
        self.index_manager.drop_index(index_name).await
    }
    /// List all indexes
    pub async fn list_indexes(&self) -> Vec<crate::index::IndexMetadata> {
        self.index_manager.list_indexes().await
    }
    /// Find nodes by property using index (if available)
    pub async fn find_nodes_indexed(
        &self,
        label: &str,
        property: &str,
        value: PropertyValue,
    ) -> Result<Vec<Node>> {
        // Intentar usar índice
        if let Some(index_name) = self.index_manager.find_index(label, property).await {
            log::debug!("🚀 Using index: {}", index_name);
            let node_ids = self.index_manager
                .query(&index_name, &IndexQuery::Equals(value))
                .await?;

            // Cargar nodos desde storage
            let mut nodes = Vec::new();
            for node_id in node_ids {
                if let Ok(node)= self.get_node(node_id).await {
                    nodes.push(node);
                }
            }
            Ok(nodes)
        } else {
            // Fallback: full scan — filter by label and property value
            log::warn!("⚠️  No index for {}.{}, using full scan", label, property);
            let all_nodes = self.get_nodes_by_label(label).await?;
            let nodes = all_nodes.into_iter()
                .filter(|n| n.properties.get(property) == Some(&value))
                .collect();
            Ok(nodes)
        }
    }

    /// Close the database and flush all pending data
    ///
    /// This method ensures all data is persisted before closing.
    /// The Graph instance should not be used after calling close().
    ///
    /// # Example
    /// ```no_run
    /// # use nopaldb::Graph;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let graph = Graph::open("my.db").await?;
    /// // ... use graph ...
    /// graph.close().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn close(&self) -> Result<()> {
        log::info!("🔒 Closing NopalDB database...");

        // 1. Flush adjacency indices to disk
        self.flush_indices().await?;
        log::debug!("  ✓ Adjacency indices flushed");

        // 2. Flush Write-Ahead Log
        self.wal.flush().await?;
        log::debug!("  ✓ WAL flushed");

        // 3. Flush storage (sled database)
        self.storage.flush().await?;
        log::debug!("  ✓ Storage flushed");

        log::info!("✅ Database closed successfully");
        Ok(())
    }

    /// Get graph statistics for query planning
    ///
    /// Returns statistics used by the query planner to make optimization decisions.
    pub async fn get_stats(&self) -> Result<GraphStats> {
        let schema = self.get_schema().await?;

        let mut stats = GraphStats::new();
        stats.total_nodes = schema.total_nodes;
        stats.total_edges = schema.total_edges;
        stats.nodes_per_label = schema.node_counts.clone();
        stats.edges_per_type = schema.edge_counts.clone();

        // Calculate average degree
        if stats.total_nodes > 0 {
            stats.avg_degree = stats.total_edges as f64 / stats.total_nodes as f64;
        }

        // Estimate property cardinality
        // TODO: Store actual cardinality in schema
        for (label, count) in &schema.node_counts {
            if let Ok(props) = self.get_label_properties(label).await {
                for prop in props {
                    let key = format!("{}_{}", label, prop);
                    // Simple heuristic: assume 50% unique values
                    // In production, we'd track this properly
                    stats.property_cardinality.insert(key, count / 2);
                }
            }
        }

        Ok(stats)
    }

    /// Create a query planner instance
    ///
    /// The planner can be used to analyze and optimize queries.
    ///
    /// # Example
    /// ```no_run
    /// # use nopaldb::Graph;
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let graph = Graph::open("my.db").await?;
    /// let planner = graph.create_planner().await?;
    ///
    /// // Use planner to choose best plan
    /// let plan = planner.choose_best_plan("Person", Some("email"), true);
    /// println!("Plan: {:?}", plan);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_planner(&self) -> Result<QueryPlanner> {
        let stats = self.get_stats().await?;
        Ok(QueryPlanner::new(stats))
    }

}

/// BulkLoader - Cargador de alto rendimiento para importación masiva
pub struct BulkLoader {
    graph: Graph,
    pending_nodes: Vec<Node>,
    pending_edges: Vec<Edge>,
    batch_size: usize,
    nodes_inserted: usize,
    edges_inserted: usize,
    start_time: std::time::Instant,
}

/// Estadísticas de una operación de bulk load
#[derive(Debug, Clone)]
pub struct BulkLoadStats {
    pub nodes_inserted: usize,
    pub edges_inserted: usize,
    pub duration: std::time::Duration,
    pub nodes_per_second: f64,
}

impl BulkLoader {
    fn new(graph: Graph, batch_size: usize) -> Self {
        Self {
            graph,
            pending_nodes: Vec::with_capacity(batch_size),
            pending_edges: Vec::with_capacity(batch_size),
            batch_size,
            nodes_inserted: 0,
            edges_inserted: 0,
            start_time: std::time::Instant::now(),
        }
    }

    /// Agrega un nodo al buffer.
    pub async fn add_node(&mut self, node: Node) -> Result<()> {
        self.pending_nodes.push(node);
        if self.pending_nodes.len() >= self.batch_size {
            self.flush_nodes().await?;
        }
        Ok(())
    }

    /// Agrega una arista al buffer.
    pub async fn add_edge(&mut self, edge: Edge) -> Result<()> {
        self.pending_edges.push(edge);
        if self.pending_edges.len() >= self.batch_size {
            self.flush_edges().await?;
        }
        Ok(())
    }

    async fn flush_nodes(&mut self) -> Result<()> {
        if self.pending_nodes.is_empty() {
            return Ok(());
        }
        let nodes = std::mem::take(&mut self.pending_nodes);
        let count = nodes.len();
        self.graph.add_nodes_batch(nodes).await?;
        self.nodes_inserted += count;
        log::debug!("Flushed {} nodes (total: {})", count, self.nodes_inserted);
        Ok(())
    }

    async fn flush_edges(&mut self) -> Result<()> {
        if self.pending_edges.is_empty() {
            return Ok(());
        }
        let edges = std::mem::take(&mut self.pending_edges);
        let count = edges.len();
        self.graph.add_edges_batch(edges).await?;
        self.edges_inserted += count;
        log::debug!("Flushed {} edges (total: {})", count, self.edges_inserted);
        Ok(())
    }

    /// Finaliza la carga, insertando todos los pendientes.
    pub async fn finish(mut self) -> Result<BulkLoadStats> {
        self.flush_nodes().await?;
        self.flush_edges().await?;
        self.graph.flush_indices().await?;

        let duration = self.start_time.elapsed();
        let nodes_per_second = if duration.as_secs_f64() > 0.0 {
            self.nodes_inserted as f64 / duration.as_secs_f64()
        } else {
            0.0
        };

        Ok(BulkLoadStats {
            nodes_inserted: self.nodes_inserted,
            edges_inserted: self.edges_inserted,
            duration,
            nodes_per_second,
        })
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PropertyValue;

    #[tokio::test]
    async fn test_add_node() {
        let graph = Graph::in_memory().await.unwrap();

        let node = Node::new("Person")
            .with_property("name", PropertyValue::String("Alice".to_string()));

        let node_id = graph.add_node(node.clone()).await.unwrap();

        let retrieved = graph.get_node(node_id).await.unwrap();
        assert_eq!(retrieved.label, "Person");
    }

    #[tokio::test]
    async fn test_add_edge_and_neighbors() {
        let graph = Graph::in_memory().await.unwrap();

        let alice = Node::new("Person")
            .with_property("name", PropertyValue::String("Alice".to_string()));
        let bob = Node::new("Person")
            .with_property("name", PropertyValue::String("Bob".to_string()));

        let alice_id = graph.add_node(alice).await.unwrap();
        let bob_id = graph.add_node(bob).await.unwrap();

        let edge = Edge::new(alice_id, bob_id, "KNOWS");
        graph.add_edge(edge).await.unwrap();

        let neighbors = graph.neighbors(alice_id, Direction::Outgoing).await.unwrap();
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0], bob_id);

        let neighbors = graph.neighbors(bob_id, Direction::Incoming).await.unwrap();
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0], alice_id);
    }

    #[tokio::test]
    async fn test_degree() {
        let graph = Graph::in_memory().await.unwrap();

        let a = graph.add_node(Node::new("Node")).await.unwrap();
        let b = graph.add_node(Node::new("Node")).await.unwrap();
        let c = graph.add_node(Node::new("Node")).await.unwrap();

        graph.add_edge(Edge::new(a, b, "CONNECTS")).await.unwrap();
        graph.add_edge(Edge::new(a, c, "CONNECTS")).await.unwrap();

        let degree = graph.degree(a, Direction::Outgoing).await.unwrap();
        assert_eq!(degree, 2);

        let degree = graph.degree(b, Direction::Incoming).await.unwrap();
        assert_eq!(degree, 1);
    }
    #[tokio::test]
    async fn test_get_node_by_property() {
        let graph = Graph::in_memory().await.unwrap();

        // Add node with property
        let node = Node::new("Person")
            .with_property("name", PropertyValue::String("Alice".to_string()))
            .with_property("age", PropertyValue::Int(30));

        let node_id = graph.add_node(node).await.unwrap();

        // Get by property
        let retrieved = graph.get_node_by_property("name", "Alice").await.unwrap();
        assert_eq!(retrieved.id, node_id);

        // Non-existent property
        let result = graph.get_node_by_property("name", "Bob").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_edge() {
        let graph = Graph::in_memory().await.unwrap();

        // Crear nodos
        let alice = Node::new("Person")
            .with_property("name", PropertyValue::String("Alice".to_string()));
        let bob = Node::new("Person")
            .with_property("name", PropertyValue::String("Bob".to_string()));

        let alice_id = graph.add_node(alice).await.unwrap();
        let bob_id = graph.add_node(bob).await.unwrap();

        // Crear arista
        let edge = Edge::new(alice_id, bob_id, "KNOWS");
        let edge_id = edge.id;
        graph.add_edge(edge).await.unwrap();

        // Verificar que la arista existe
        assert!(graph.get_edge(edge_id).await.is_ok());
        assert_eq!(graph.degree(alice_id, Direction::Outgoing).await.unwrap(), 1);
        assert_eq!(graph.degree(bob_id, Direction::Incoming).await.unwrap(), 1);

        // Eliminar arista
        graph.delete_edge(edge_id).await.unwrap();

        // Verificar que la arista ya no existe
        assert!(graph.get_edge(edge_id).await.is_err());
        assert_eq!(graph.degree(alice_id, Direction::Outgoing).await.unwrap(), 0);
        assert_eq!(graph.degree(bob_id, Direction::Incoming).await.unwrap(), 0);

        // Los nodos deben seguir existiendo
        assert!(graph.get_node(alice_id).await.is_ok());
        assert!(graph.get_node(bob_id).await.is_ok());
    }

    #[tokio::test]
    async fn test_delete_edge_not_found() {
        let graph = Graph::in_memory().await.unwrap();

        // Intentar eliminar arista que no existe
        let fake_id = uuid::Uuid::new_v4();
        let result = graph.delete_edge(fake_id).await;

        assert!(result.is_err());
    }
}
