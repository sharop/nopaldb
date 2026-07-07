// src/transaction/mod.rs

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

#[cfg(feature = "full-isolation")]
use tokio::sync::{RwLock};

use crate::error::{NopalError, Result};
use crate::types::{Node, Edge, NodeId, EdgeId, PropertyValue};
use crate::graph::Graph;
use crate::wal::WalRecord;


/// ID único de transacción
pub type TransactionId = u64;

/// Timestamp lógico monotónico
pub type Timestamp = u64;

/// Estado de una transacción
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionState {
    Active,      // En progreso
    Committed,   // Completada exitosamente
    Aborted,     // Cancelada/revertida
}


// ============================================================================
// ISOLATION LEVELS (con feature flag)
// ============================================================================

/// Niveles de aislamiento ACID (solo con feature "full-isolation")
#[cfg(feature = "full-isolation")]
#[derive(Debug, Clone, Copy, PartialEq, Eq,Default)]
pub enum IsolationLevel {
    /// Lee datos NO commiteados (permite dirty reads)
    /// - Más rápido
    /// - Menos consistente
    /// - Uso: Analytics de baja prioridad
    ReadUncommitted,

    /// Solo lee datos commiteados (default)
    /// - Balance entre velocidad y consistencia
    /// - Previene dirty reads
    /// - Permite non-repeatable reads
    /// - Uso: La mayoría de aplicaciones
    #[default]
    ReadCommitted,

    /// Snapshot isolation - ve datos del inicio de la tx
    /// - Previene dirty reads y non-repeatable reads
    /// - Permite phantom reads
    /// - Uso: Reportes, auditorías
    RepeatableRead,

    /// Máxima consistencia - como ejecución serial
    /// - Previene dirty, non-repeatable, y phantom reads
    /// - Detecta conflictos write-write
    /// - Uso: Transacciones financieras críticas
    Serializable,
}

#[cfg(feature = "full-isolation")]

// Si NO está la feature, usamos un tipo simple
#[cfg(not(feature = "full-isolation"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IsolationLevel;

#[cfg(feature = "full-isolation")]
#[derive(Debug, Clone, PartialEq, Eq)]
enum PredicateRead {
    AllNodes {
        node_ids: HashSet<NodeId>,
    },
    ByLabel {
        label: String,
        node_ids: HashSet<NodeId>,
    },
    ByProperty {
        property: String,
        value: PropertyValue,
        node_ids: HashSet<NodeId>,
    },
    ByLabelAndProperty {
        label: String,
        property: String,
        value: PropertyValue,
        node_ids: HashSet<NodeId>,
    },
    PatternSingleHop {
        source_label: String,
        rel_type: String,
        target_label: String,
        pairs: HashSet<(NodeId, NodeId)>,
    },
    PatternTwoHop {
        source_label: String,
        rel_type_1: String,
        middle_label: String,
        rel_type_2: String,
        target_label: String,
        triples: HashSet<(NodeId, NodeId, NodeId)>,
    },
}



/// Operación realizada en una transacción (para rollback)
#[derive(Debug, Clone)]
pub enum Operation {
    InsertNode(Node),
    UpdateNode { id: NodeId, old: Node, new: Node },
    DeleteNode { id: NodeId, old: Node },
    InsertEdge(Edge),
    UpdateEdge { id: EdgeId, old: Edge, new: Edge },
    DeleteEdge { id: EdgeId, old: Edge },
}


/// Una transacción sobre el grafo
pub struct Transaction {
    pub id: TransactionId,
    pub timestamp: Timestamp,
    state: TransactionState,

    // Cambios pendientes (write buffer)
    pending_nodes: HashMap<NodeId, Node>,
    pending_edges: HashMap<EdgeId, Edge>,
    deleted_nodes: HashSet<NodeId>,
    deleted_edges: HashSet<EdgeId>,

    // Referencia al grafo
    graph: Arc<Graph>,

    // Locks adquiridos (para liberar en drop)
    #[allow(dead_code)]
    locks: Vec<NodeId>,

    // ============ CAMPOS CONDICIONALES (solo con full-isolation) ============

    /// Nivel de isolation de esta transacción
    #[cfg(feature = "full-isolation")]
    isolation_level: IsolationLevel,

    /// Timestamp del snapshot (para Repeatable Read)
    #[cfg(feature = "full-isolation")]
    #[allow(dead_code)]
    snapshot_timestamp: Timestamp,

    /// Nodos leídos (para Serializable - conflict detection)
    #[cfg(feature = "full-isolation")]
    #[allow(dead_code)]
    read_set: Arc<RwLock<HashSet<NodeId>>>,

    /// Nodos escritos (para Serializable - conflict detection)
    #[cfg(feature = "full-isolation")]
    write_set: Arc<RwLock<HashSet<NodeId>>>,

    #[cfg(feature = "full-isolation")]
    acquired_locks: HashSet<NodeId>,

    #[cfg(feature = "full-isolation")]
    predicate_reads: Arc<RwLock<Vec<PredicateRead>>>,
}



impl Transaction {
    /// Crea una nueva transacción
    pub(crate) fn new(id: TransactionId, timestamp: Timestamp, graph: Arc<Graph>) -> Self {
        Self {
            id,
            timestamp,
            state: TransactionState::Active,
            pending_nodes: HashMap::new(),
            pending_edges: HashMap::new(),
            deleted_nodes: HashSet::new(),
            deleted_edges: HashSet::new(),
            graph,
            locks: Vec::new(),

            #[cfg(feature = "full-isolation")]
            isolation_level: IsolationLevel::default(),

            #[cfg(feature = "full-isolation")]
            snapshot_timestamp: timestamp,

            #[cfg(feature = "full-isolation")]
            read_set: Arc::new(RwLock::new(HashSet::new())),

            #[cfg(feature = "full-isolation")]
            write_set: Arc::new(RwLock::new(HashSet::new())),

            #[cfg(feature = "full-isolation")]
            acquired_locks: HashSet::new(),

            #[cfg(feature = "full-isolation")]
            predicate_reads: Arc::new(RwLock::new(Vec::new())),

        }
    }
    /// Configura el nivel de isolation (solo con feature "full-isolation")
    #[cfg(feature = "full-isolation")]
    pub fn with_isolation(mut self, level: IsolationLevel) -> Self {
        self.isolation_level = level;
        self
    }

    /// Obtiene un nodo (ve cambios pendientes de esta tx)
    pub async fn get_node(&self, id: NodeId) -> Result<Node> {
        if self.state != TransactionState::Active {
            return Err(NopalError::TransactionNotActive);
        }

        // Adquirir READ lock (solo con Serializable)
        #[cfg(feature = "full-isolation")]
        {
            if self.isolation_level == IsolationLevel::Serializable {
                self.graph.lock_manager().acquire_read_lock(id, self.id).await?;
            }
        }

        // Track read (solo con full-isolation)
        #[cfg(feature = "full-isolation")]
        {
            let mut read_set = self.read_set.write().await;
            read_set.insert(id);
            //self.read_set.write().await.insert(id);
        }

        // ¿Fue borrado en esta tx?
        if self.deleted_nodes.contains(&id) {
            return Err(NopalError::NodeNotFound(id.to_string()));
        }

        // ¿Está en el write buffer?
        if let Some(node) = self.pending_nodes.get(&id) {
            return Ok(node.clone());
        }

        // Leer del grafo según isolation level
        #[cfg(feature = "full-isolation")]
        {
            self.read_with_isolation(id).await
        }

        #[cfg(not(feature = "full-isolation"))]
        {
            // Leer del grafo (committed data) (Modo Minimal solo read committed)
            self.graph.get_node(id).await
        }
    }

    /// Agrega un nodo (buffered, no persistido aún)
    pub async fn add_node(&mut self, node: Node) -> Result<NodeId> {
        if self.state != TransactionState::Active {
            return Err(NopalError::TransactionNotActive);
        }

        let node_id = node.id;

        //Adquirir WRITE lock (solo con Serializable)
        #[cfg(feature = "full-isolation")]
        {
            if self.isolation_level == IsolationLevel::Serializable {
                self.graph.lock_manager().acquire_write_lock(node_id, self.id).await?;
                self.acquired_locks.insert(node_id);
            }
        }


        // Track write (solo con full-isolation)
        //TODO: optimizar con RwLock y verificar si ya está bien.
        #[cfg(feature = "full-isolation")]
        {
            let mut write_set = self.write_set.write().await;
            write_set.insert(node_id);
        }

        self.pending_nodes.insert(node_id, node);

        Ok(node_id)
    }


    /// Lee un nodo aplicando el nivel de isolation (solo full-isolation)
    #[cfg(feature = "full-isolation")]
    async fn read_with_isolation(&self, id: NodeId) -> Result<Node> {
        match self.isolation_level {
            IsolationLevel::ReadUncommitted => {
                // NOTA: En NopalDB actual, no hay "dirty data" visible
                // porque cada tx tiene su propio write buffer.
                // Para implementar dirty reads correctamente necesitaríamos:
                // 1. Staging area compartido, o
                // 2. Permitir leer write buffers de otras tx (inseguro)
                //
                // Por ahora, comportamiento = ReadCommitted
                log::debug!("ReadUncommitted: falling back to ReadCommitted behavior");
                self.graph.get_node(id).await
            }

            IsolationLevel::ReadCommitted => {
                // Solo lee datos commiteados (comportamiento actual)
                self.graph.get_node(id).await
            }

            IsolationLevel::RepeatableRead => {
                self.read_repeatable(id).await
            }

            IsolationLevel::Serializable => {
                self.read_serializable(id).await
            }
        }
    }


    #[cfg(feature = "full-isolation")]
    async fn read_repeatable(&self, id: NodeId) -> Result<Node> {
        // Snapshot isolation real: leer la versión visible al inicio de la tx.
        self.graph.get_node_at_strict(id, self.snapshot_timestamp).await
    }

    #[cfg(feature = "full-isolation")]
    async fn read_serializable(&self, id: NodeId) -> Result<Node> {
        // Serializable usa misma visibilidad por snapshot y valida conflictos en commit().
        self.read_repeatable(id).await
    }

    /// Agrega una arista
    pub fn add_edge(&mut self, edge: Edge) -> Result<EdgeId> {
        if self.state != TransactionState::Active {
            return Err(NopalError::TransactionNotActive);
        }

        let edge_id = edge.id;
        self.pending_edges.insert(edge_id, edge);

        Ok(edge_id)
    }

    /// Obtiene una arista
    pub async fn get_edge(&self, id: EdgeId) -> Result<Edge> {
        if self.state != TransactionState::Active {
            return Err(NopalError::TransactionNotActive);
        }

        if self.deleted_edges.contains(&id) {
            return Err(NopalError::EdgeNotFound(id.to_string()));
        }

        if let Some(edge) = self.pending_edges.get(&id) {
            return Ok(edge.clone());
        }

        self.graph.get_edge(id).await
    }

    /// Obtiene nodos por label respetando el isolation level de la transacción.
    pub async fn get_nodes_by_label(&self, label: &str) -> Result<Vec<Node>> {
        if self.state != TransactionState::Active {
            return Err(NopalError::TransactionNotActive);
        }

        #[cfg(feature = "full-isolation")]
        {
            match self.isolation_level {
                IsolationLevel::ReadUncommitted | IsolationLevel::ReadCommitted => {
                    self.graph.get_nodes_by_label(label).await
                }
                IsolationLevel::RepeatableRead | IsolationLevel::Serializable => {
                    let maybe_cached = self.get_cached_label_read(label).await;
                    let node_ids = if let Some(ids) = maybe_cached {
                        ids
                    } else {
                        let fresh = self.graph.get_nodes_by_label(label).await?;
                        let ids = fresh.into_iter().map(|n| n.id).collect::<HashSet<_>>();
                        self.record_predicate_read(PredicateRead::ByLabel {
                            label: label.to_string(),
                            node_ids: ids.clone(),
                        }).await;
                        ids
                    };

                    let mut result = Vec::new();
                    for node_id in node_ids {
                        if let Ok(node) = self.get_node(node_id).await
                            && node.label == label
                        {
                            result.push(node);
                        }
                    }
                    Ok(result)
                }
            }
        }

        #[cfg(not(feature = "full-isolation"))]
        {
            self.graph.get_nodes_by_label(label).await
        }
    }

    /// Obtiene NodeIds por propiedad respetando el isolation level de la transacción.
    pub async fn get_all_nodes_by_property(
        &self,
        property: &str,
        value: &PropertyValue,
    ) -> Result<Vec<NodeId>> {
        if self.state != TransactionState::Active {
            return Err(NopalError::TransactionNotActive);
        }

        #[cfg(feature = "full-isolation")]
        {
            match self.isolation_level {
                IsolationLevel::ReadUncommitted | IsolationLevel::ReadCommitted => {
                    self.graph.get_all_nodes_by_property(property, value).await
                }
                IsolationLevel::RepeatableRead | IsolationLevel::Serializable => {
                    let maybe_cached = self.get_cached_property_read(property, value).await;
                    let node_ids = if let Some(ids) = maybe_cached {
                        ids
                    } else {
                        let fresh = self.graph.get_all_nodes_by_property(property, value).await?;
                        let ids = fresh.into_iter().collect::<HashSet<_>>();
                        self.record_predicate_read(PredicateRead::ByProperty {
                            property: property.to_string(),
                            value: value.clone(),
                            node_ids: ids.clone(),
                        }).await;
                        ids
                    };

                    let mut visible = Vec::new();
                    for node_id in node_ids {
                        if let Ok(node) = self.get_node(node_id).await
                            && let Some(prop_val) = node.properties.get(property)
                            && prop_val == value
                        {
                            visible.push(node_id);
                        }
                    }
                    Ok(visible)
                }
            }
        }

        #[cfg(not(feature = "full-isolation"))]
        {
            self.graph.get_all_nodes_by_property(property, value).await
        }
    }

    /// Obtiene nodos por predicado compuesto: `label` + `property = value`.
    pub async fn get_nodes_by_label_and_property(
        &self,
        label: &str,
        property: &str,
        value: &PropertyValue,
    ) -> Result<Vec<Node>> {
        if self.state != TransactionState::Active {
            return Err(NopalError::TransactionNotActive);
        }

        #[cfg(feature = "full-isolation")]
        {
            match self.isolation_level {
                IsolationLevel::ReadUncommitted | IsolationLevel::ReadCommitted => {
                    self.scan_nodes_by_label_property_current(label, property, value).await
                }
                IsolationLevel::RepeatableRead | IsolationLevel::Serializable => {
                    let maybe_cached = self
                        .get_cached_label_property_read(label, property, value)
                        .await;
                    let node_ids = if let Some(ids) = maybe_cached {
                        ids
                    } else {
                        let fresh = self
                            .scan_nodes_by_label_property_current(label, property, value)
                            .await?;
                        let ids = fresh.iter().map(|n| n.id).collect::<HashSet<_>>();
                        self.record_predicate_read(PredicateRead::ByLabelAndProperty {
                            label: label.to_string(),
                            property: property.to_string(),
                            value: value.clone(),
                            node_ids: ids.clone(),
                        })
                        .await;
                        ids
                    };

                    let mut result = Vec::new();
                    for node_id in node_ids {
                        if let Ok(node) = self.get_node(node_id).await
                            && node.label == label
                            && let Some(prop_val) = node.properties.get(property)
                            && prop_val == value
                        {
                            result.push(node);
                        }
                    }
                    Ok(result)
                }
            }
        }

        #[cfg(not(feature = "full-isolation"))]
        {
            self.scan_nodes_by_label_property_current(label, property, value).await
        }
    }

    /// Obtiene todos los nodos respetando isolation level (útil para scans/paginación).
    pub async fn get_all_nodes(&self) -> Result<Vec<Node>> {
        if self.state != TransactionState::Active {
            return Err(NopalError::TransactionNotActive);
        }

        #[cfg(feature = "full-isolation")]
        {
            match self.isolation_level {
                IsolationLevel::ReadUncommitted | IsolationLevel::ReadCommitted => {
                    self.graph.get_all_nodes().await
                }
                IsolationLevel::RepeatableRead | IsolationLevel::Serializable => {
                    let maybe_cached = self.get_cached_all_nodes_read().await;
                    let node_ids = if let Some(ids) = maybe_cached {
                        ids
                    } else {
                        let fresh = self.graph.get_all_nodes().await?;
                        let ids = fresh.into_iter().map(|n| n.id).collect::<HashSet<_>>();
                        self.record_predicate_read(PredicateRead::AllNodes {
                            node_ids: ids.clone(),
                        })
                        .await;
                        ids
                    };

                    let mut result = Vec::new();
                    for node_id in node_ids {
                        if let Ok(node) = self.get_node(node_id).await {
                            result.push(node);
                        }
                    }
                    Ok(result)
                }
            }
        }

        #[cfg(not(feature = "full-isolation"))]
        {
            self.graph.get_all_nodes().await
        }
    }

    /// Obtiene pares (source, target) para un patrón simple:
    /// `(source:source_label)-[:rel_type]->(target:target_label)`.
    pub async fn get_pattern_pairs(
        &self,
        source_label: &str,
        rel_type: &str,
        target_label: &str,
    ) -> Result<Vec<(NodeId, NodeId)>> {
        if self.state != TransactionState::Active {
            return Err(NopalError::TransactionNotActive);
        }

        #[cfg(feature = "full-isolation")]
        {
            match self.isolation_level {
                IsolationLevel::ReadUncommitted | IsolationLevel::ReadCommitted => {
                    Ok(self
                        .scan_pattern_pairs_current(source_label, rel_type, target_label)
                        .await?
                        .into_iter()
                        .collect())
                }
                IsolationLevel::RepeatableRead | IsolationLevel::Serializable => {
                    let maybe_cached = self
                        .get_cached_pattern_read(source_label, rel_type, target_label)
                        .await;
                    let pairs = if let Some(pairs) = maybe_cached {
                        pairs
                    } else {
                        let fresh = self
                            .scan_pattern_pairs_current(source_label, rel_type, target_label)
                            .await?;
                        self.record_predicate_read(PredicateRead::PatternSingleHop {
                            source_label: source_label.to_string(),
                            rel_type: rel_type.to_string(),
                            target_label: target_label.to_string(),
                            pairs: fresh.clone(),
                        })
                        .await;
                        fresh
                    };

                    Ok(pairs.into_iter().collect())
                }
            }
        }

        #[cfg(not(feature = "full-isolation"))]
        {
            Ok(self
                .scan_pattern_pairs_current(source_label, rel_type, target_label)
                .await?
                .into_iter()
                .collect())
        }
    }

    /// Obtiene triples `(source, middle, target)` para patrón de dos saltos:
    /// `(source:source_label)-[:rel_type_1]->(middle:middle_label)-[:rel_type_2]->(target:target_label)`.
    pub async fn get_pattern_triples_two_hop(
        &self,
        source_label: &str,
        rel_type_1: &str,
        middle_label: &str,
        rel_type_2: &str,
        target_label: &str,
    ) -> Result<Vec<(NodeId, NodeId, NodeId)>> {
        if self.state != TransactionState::Active {
            return Err(NopalError::TransactionNotActive);
        }

        #[cfg(feature = "full-isolation")]
        {
            match self.isolation_level {
                IsolationLevel::ReadUncommitted | IsolationLevel::ReadCommitted => {
                    Ok(self
                        .scan_pattern_triples_two_hop_current(
                            source_label,
                            rel_type_1,
                            middle_label,
                            rel_type_2,
                            target_label,
                        )
                        .await?
                        .into_iter()
                        .collect())
                }
                IsolationLevel::RepeatableRead | IsolationLevel::Serializable => {
                    let maybe_cached = self
                        .get_cached_two_hop_pattern_read(
                            source_label,
                            rel_type_1,
                            middle_label,
                            rel_type_2,
                            target_label,
                        )
                        .await;
                    let triples = if let Some(triples) = maybe_cached {
                        triples
                    } else {
                        let fresh = self
                            .scan_pattern_triples_two_hop_current(
                                source_label,
                                rel_type_1,
                                middle_label,
                                rel_type_2,
                                target_label,
                            )
                            .await?;
                        self.record_predicate_read(PredicateRead::PatternTwoHop {
                            source_label: source_label.to_string(),
                            rel_type_1: rel_type_1.to_string(),
                            middle_label: middle_label.to_string(),
                            rel_type_2: rel_type_2.to_string(),
                            target_label: target_label.to_string(),
                            triples: fresh.clone(),
                        })
                        .await;
                        fresh
                    };

                    Ok(triples.into_iter().collect())
                }
            }
        }

        #[cfg(not(feature = "full-isolation"))]
        {
            Ok(self
                .scan_pattern_triples_two_hop_current(
                    source_label,
                    rel_type_1,
                    middle_label,
                    rel_type_2,
                    target_label,
                )
                .await?
                .into_iter()
                .collect())
        }
    }

    /// Elimina un nodo (marca para borrado)
    pub fn delete_node(&mut self, id: NodeId) -> Result<()> {
        if self.state != TransactionState::Active {
            return Err(NopalError::TransactionNotActive);
        }

        self.deleted_nodes.insert(id);
        self.pending_nodes.remove(&id);  // Si estaba pendiente, cancelarlo

        Ok(())
    }

    /// Elimina una arista
    pub fn delete_edge(&mut self, id: EdgeId) -> Result<()> {
        if self.state != TransactionState::Active {
            return Err(NopalError::TransactionNotActive);
        }

        self.deleted_edges.insert(id);
        self.pending_edges.remove(&id);

        Ok(())
    }

    /// Hace commit de la transacción (persiste todos los cambios)
    pub async fn commit(mut self) -> Result<()> {
        if self.state != TransactionState::Active {
            return Err(NopalError::TransactionNotActive);
        }

        let result = self.commit_inner().await;

        if result.is_err() {
            // Un commit fallido NO debe dejar residuos: sin esto, los locks de
            // una tx en conflicto quedaban tomados hasta el timeout (5s) y
            // bloqueaban a todos los escritores siguientes sobre esos nodos.
            #[cfg(feature = "full-isolation")]
            {
                self.graph.lock_manager().release_locks(self.id).await;
            }
            self.graph.deregister_tx_timestamp_sync(self.id);
            self.state = TransactionState::Aborted;
        }

        result
    }

    /// Cuerpo del commit. Los paths de error los limpia el wrapper `commit()`.
    async fn commit_inner(&mut self) -> Result<()> {

        log::info!("Committing transaction {} (isolation: {:?}",
            self.id,
            self.get_isolation_level_name());

        // Validar conflictos (solo Serializable)
        #[cfg(feature = "full-isolation")]
        {
            if self.isolation_level == IsolationLevel::Serializable {
                // Las eliminaciones no adquieren lock en delete_node(), así que se toman aquí.
                for node_id in &self.deleted_nodes {
                    self.graph
                        .lock_manager()
                        .acquire_write_lock(*node_id, self.id)
                        .await?;
                    self.acquired_locks.insert(*node_id);
                }
                self.validate_serializable().await?;
            }
        }

        // ── Write-set al applier ─────────────────────────────────────────────
        // El commit ya no toma un lock global: construye su write-set completo
        // y lo encola en la task del applier, que (a) asigna el timestamp de
        // commit en orden FIFO, (b) agrupa los registros WAL de TODOS los
        // commits en vuelo en UN solo fsync (group commit entre transacciones)
        // y (c) aplica cada write-set en orden de cola bajo el write-gate —
        // preservando orden-de-log == orden-de-apply, la invariante del redo.

        // Prefetch de nodos borrados: el registro WAL DeleteNode lleva el nodo.
        let mut deleted_nodes = Vec::with_capacity(self.deleted_nodes.len());
        for node_id in &self.deleted_nodes {
            deleted_nodes.push((*node_id, self.graph.get_node(*node_id).await?));
        }

        let set = crate::graph::applier::CommitSet {
            tx_id: self.id,
            begin_timestamp: self.timestamp,
            deleted_nodes,
            deleted_edges: self.deleted_edges.iter().cloned().collect(),
            pending_nodes: self.pending_nodes.values().cloned().collect(),
            pending_edges: self.pending_edges.values().cloned().collect(),
        };

        self.graph.submit_commit(set).await?;

        // 6. Liberar locks antes de marcar como committed
        #[cfg(feature = "full-isolation")]
        {
            self.graph.lock_manager().release_locks(self.id).await;
        }

        // 7. Marcar como committed
        self.state = TransactionState::Committed;

        // 8. Deregistrar del mapa de transacciones activas
        self.graph.deregister_tx_timestamp_sync(self.id);

        log::info!("Transaction {} committed successfully", self.id);

        Ok(())
    }


    /// Aborta la transacción de forma síncrona (descarta todos los cambios en memoria).
    ///
    /// ⚠️ Esta versión síncrona es intencional: es llamada desde el `Drop` impl,
    /// donde no es posible ejecutar código async. NO escribe `WalRecord::Abort` al WAL.
    /// Para persistencia completa, usar `rollback_async()` cuando se dispone de contexto async.
    pub fn rollback(mut self) -> Result<()> {
        if self.state != TransactionState::Active {
            return Err(NopalError::TransactionNotActive);
        }

        log::info!("Rolling back transaction {} (sync, WAL Abort NOT written — use rollback_async() when possible)", self.id);

        // Limpiar buffers en memoria
        self.pending_nodes.clear();
        self.pending_edges.clear();
        self.deleted_nodes.clear();
        self.deleted_edges.clear();

        self.state = TransactionState::Aborted;

        // Deregistrar del mapa de transacciones activas
        self.graph.deregister_tx_timestamp_sync(self.id);

        log::info!("Transaction {} rolled back (sync)", self.id);
        Ok(())
    }

    /// Aborta la transacción de forma asíncrona y escribe `WalRecord::Abort` al WAL.
    ///
    /// Preferir este método sobre `rollback()` en todo contexto async. La entrada en el WAL
    /// garantiza que, si el proceso muere después del rollback, el recovery en startup sabrá
    /// que las operaciones de esta transacción NO deben ser re-aplicadas (están sin commit).
    ///
    /// El recovery ya ignora transacciones sin `WalRecord::Commit`, pero este registro
    /// hace la intención explícita y acelera el análisis durante recovery en WALs grandes.
    pub async fn rollback_async(mut self) -> Result<()> {
        if self.state != TransactionState::Active {
            return Err(NopalError::TransactionNotActive);
        }

        log::info!("Rolling back transaction {} (async, writing WAL Abort)", self.id);

        // Escribir Abort al WAL ANTES de limpiar buffers
        // Esto garantiza durabilidad: si el proceso muere aquí, el WAL tiene el Abort.
        let wal = self.graph.wal();
        if let Err(e) = wal.append(WalRecord::Abort { tx_id: self.id }).await {
            // No fatal: el recovery ignora txs sin Commit de todas formas.
            // Loguear la advertencia pero continuar con el rollback en memoria.
            log::warn!("Transaction {}: failed to write WAL Abort record: {} — rollback still applied in memory", self.id, e);
        }

        // Limpiar buffers en memoria
        self.pending_nodes.clear();
        self.pending_edges.clear();
        self.deleted_nodes.clear();
        self.deleted_edges.clear();

        self.state = TransactionState::Aborted;

        // Liberar locks adquiridos (Serializable): un rollback sin release
        // dejaba los nodos bloqueados hasta el timeout para otros escritores.
        #[cfg(feature = "full-isolation")]
        {
            self.graph.lock_manager().release_locks(self.id).await;
        }

        // Persistir relojes: el tx id abortado quedó en el WAL, así que un
        // reopen no debe reutilizarlo. Best-effort, como el Abort de arriba.
        if let Err(e) = self.graph.persist_clocks().await {
            log::warn!("Transaction {}: failed to persist logical clocks on rollback: {}", self.id, e);
        }

        // Deregistrar del mapa de transacciones activas
        self.graph.deregister_tx_timestamp_sync(self.id);

        log::info!("Transaction {} rolled back (async, WAL Abort written)", self.id);
        Ok(())
    }



    /// Helper para logging
    fn get_isolation_level_name(&self) -> &str {
        #[cfg(feature = "full-isolation")]
        {
            match self.isolation_level {
                IsolationLevel::ReadUncommitted => "ReadUncommitted",
                IsolationLevel::ReadCommitted => "ReadCommitted",
                IsolationLevel::RepeatableRead => "RepeatableRead",
                IsolationLevel::Serializable => "Serializable",
            }
        }

        #[cfg(not(feature = "full-isolation"))]
        {
            "ReadCommitted (minimal)"
        }
    }

    #[cfg(feature = "full-isolation")]
    async fn validate_serializable(&self) -> Result<()> {
        let read_set = self.read_set.read().await;
        let write_set = self.write_set.read().await;
        let predicate_reads = self.predicate_reads.read().await;

        // Detectar conflictos write-write
        for node_id in write_set.iter() {
            if let Some(last_modified) = self.graph.get_last_modified(*node_id).await &&
                last_modified > self.snapshot_timestamp {
                return Err(NopalError::TransactionConflict(format!(
                    "Write-Write conflict on node {}: modified by tx at t={}",
                    node_id, last_modified
                )));
            }
        }

        // Detectar conflictos read-write
        for node_id in read_set.iter() {
            if let Some(last_modified) = self.graph.get_last_modified(*node_id).await &&
                last_modified > self.snapshot_timestamp {
                return Err(NopalError::TransactionConflict(format!(
                    "Read-Write conflict on node {}: read stale data",
                    node_id
                )));
            }
        }

        // Detectar conflictos en nodos marcados para delete.
        for node_id in &self.deleted_nodes {
            if let Some(last_modified) = self.graph.get_last_modified(*node_id).await
                && last_modified > self.snapshot_timestamp
            {
                return Err(NopalError::TransactionConflict(format!(
                    "Delete conflict on node {}: modified by tx at t={}",
                    node_id, last_modified
                )));
            }
        }

        // Detectar phantoms en lecturas por predicado (label/property).
        for predicate in predicate_reads.iter() {
            match predicate {
                PredicateRead::AllNodes { node_ids } => {
                    let current = self
                        .graph
                        .get_all_nodes()
                        .await?
                        .into_iter()
                        .map(|n| n.id)
                        .collect::<HashSet<_>>();

                    if &current != node_ids {
                        return Err(NopalError::TransactionConflict(
                            "Phantom conflict on global scan: node set changed during transaction"
                                .to_string(),
                        ));
                    }
                }
                PredicateRead::ByLabel { label, node_ids } => {
                    let current = self
                        .graph
                        .get_nodes_by_label(label)
                        .await?
                        .into_iter()
                        .map(|n| n.id)
                        .collect::<HashSet<_>>();

                    if &current != node_ids {
                        return Err(NopalError::TransactionConflict(format!(
                            "Phantom conflict on label '{}': result set changed during transaction",
                            label
                        )));
                    }
                }
                PredicateRead::ByProperty {
                    property,
                    value,
                    node_ids,
                } => {
                    let current = self
                        .graph
                        .get_all_nodes_by_property(property, value)
                        .await?
                        .into_iter()
                        .collect::<HashSet<_>>();

                    if &current != node_ids {
                        return Err(NopalError::TransactionConflict(format!(
                            "Phantom conflict on property '{}': result set changed during transaction",
                            property
                        )));
                    }
                }
                PredicateRead::ByLabelAndProperty {
                    label,
                    property,
                    value,
                    node_ids,
                } => {
                    let current = self
                        .scan_nodes_by_label_property_current(label, property, value)
                        .await?
                        .into_iter()
                        .map(|n| n.id)
                        .collect::<HashSet<_>>();
                    if &current != node_ids {
                        return Err(NopalError::TransactionConflict(format!(
                            "Phantom conflict on predicate {}.{} = {:?}: result set changed during transaction",
                            label, property, value
                        )));
                    }
                }
                PredicateRead::PatternSingleHop {
                    source_label,
                    rel_type,
                    target_label,
                    pairs,
                } => {
                    let current = self
                        .scan_pattern_pairs_current(source_label, rel_type, target_label)
                        .await?;
                    if &current != pairs {
                        return Err(NopalError::TransactionConflict(format!(
                            "Phantom conflict on pattern ({}-[:{}]->{}): result set changed during transaction",
                            source_label, rel_type, target_label
                        )));
                    }
                }
                PredicateRead::PatternTwoHop {
                    source_label,
                    rel_type_1,
                    middle_label,
                    rel_type_2,
                    target_label,
                    triples,
                } => {
                    let current = self
                        .scan_pattern_triples_two_hop_current(
                            source_label,
                            rel_type_1,
                            middle_label,
                            rel_type_2,
                            target_label,
                        )
                        .await?;
                    if &current != triples {
                        return Err(NopalError::TransactionConflict(format!(
                            "Phantom conflict on two-hop pattern ({}-[:{}]->{}-[:{}]->{}): result set changed during transaction",
                            source_label, rel_type_1, middle_label, rel_type_2, target_label
                        )));
                    }
                }
            }
        }

        log::debug!("Serializable validation passed (read_set: {}, write_set: {})",
                read_set.len(), write_set.len());

        Ok(())
    }

    #[cfg(feature = "full-isolation")]
    async fn record_predicate_read(&self, read: PredicateRead) {
        let mut reads = self.predicate_reads.write().await;
        if !reads.contains(&read) {
            reads.push(read);
        }
    }

    #[cfg(feature = "full-isolation")]
    async fn get_cached_label_read(&self, label: &str) -> Option<HashSet<NodeId>> {
        let reads = self.predicate_reads.read().await;
        reads.iter().find_map(|read| match read {
            PredicateRead::AllNodes { .. } => None,
            PredicateRead::ByLabel {
                label: cached_label,
                node_ids,
            } if cached_label == label => Some(node_ids.clone()),
            _ => None,
        })
    }

    #[cfg(feature = "full-isolation")]
    async fn get_cached_all_nodes_read(&self) -> Option<HashSet<NodeId>> {
        let reads = self.predicate_reads.read().await;
        reads.iter().find_map(|read| match read {
            PredicateRead::AllNodes { node_ids } => Some(node_ids.clone()),
            _ => None,
        })
    }

    #[cfg(feature = "full-isolation")]
    async fn get_cached_property_read(
        &self,
        property: &str,
        value: &PropertyValue,
    ) -> Option<HashSet<NodeId>> {
        let reads = self.predicate_reads.read().await;
        reads.iter().find_map(|read| match read {
            PredicateRead::AllNodes { .. } => None,
            PredicateRead::ByProperty {
                property: cached_property,
                value: cached_value,
                node_ids,
            } if cached_property == property && cached_value == value => Some(node_ids.clone()),
            _ => None,
        })
    }

    #[cfg(feature = "full-isolation")]
    async fn get_cached_label_property_read(
        &self,
        label: &str,
        property: &str,
        value: &PropertyValue,
    ) -> Option<HashSet<NodeId>> {
        let reads = self.predicate_reads.read().await;
        reads.iter().find_map(|read| match read {
            PredicateRead::ByLabelAndProperty {
                label: cached_label,
                property: cached_property,
                value: cached_value,
                node_ids,
            } if cached_label == label && cached_property == property && cached_value == value => {
                Some(node_ids.clone())
            }
            _ => None,
        })
    }

    #[cfg(feature = "full-isolation")]
    async fn get_cached_pattern_read(
        &self,
        source_label: &str,
        rel_type: &str,
        target_label: &str,
    ) -> Option<HashSet<(NodeId, NodeId)>> {
        let reads = self.predicate_reads.read().await;
        reads.iter().find_map(|read| match read {
            PredicateRead::PatternSingleHop {
                source_label: cached_source,
                rel_type: cached_rel,
                target_label: cached_target,
                pairs,
            } if cached_source == source_label
                && cached_rel == rel_type
                && cached_target == target_label =>
            {
                Some(pairs.clone())
            }
            _ => None,
        })
    }

    #[cfg(feature = "full-isolation")]
    async fn get_cached_two_hop_pattern_read(
        &self,
        source_label: &str,
        rel_type_1: &str,
        middle_label: &str,
        rel_type_2: &str,
        target_label: &str,
    ) -> Option<HashSet<(NodeId, NodeId, NodeId)>> {
        let reads = self.predicate_reads.read().await;
        reads.iter().find_map(|read| match read {
            PredicateRead::PatternTwoHop {
                source_label: cached_source,
                rel_type_1: cached_rel_1,
                middle_label: cached_middle,
                rel_type_2: cached_rel_2,
                target_label: cached_target,
                triples,
            } if cached_source == source_label
                && cached_rel_1 == rel_type_1
                && cached_middle == middle_label
                && cached_rel_2 == rel_type_2
                && cached_target == target_label =>
            {
                Some(triples.clone())
            }
            _ => None,
        })
    }

    async fn scan_pattern_pairs_current(
        &self,
        source_label: &str,
        rel_type: &str,
        target_label: &str,
    ) -> Result<HashSet<(NodeId, NodeId)>> {
        let edges = self.graph.get_all_edges().await?;
        let mut pairs = HashSet::new();

        for edge in edges {
            if edge.edge_type != rel_type {
                continue;
            }

            let source = match self.graph.get_node(edge.source).await {
                Ok(node) => node,
                Err(_) => continue,
            };
            if source.label != source_label {
                continue;
            }

            let target = match self.graph.get_node(edge.target).await {
                Ok(node) => node,
                Err(_) => continue,
            };
            if target.label != target_label {
                continue;
            }

            pairs.insert((source.id, target.id));
        }

        Ok(pairs)
    }

    async fn scan_nodes_by_label_property_current(
        &self,
        label: &str,
        property: &str,
        value: &PropertyValue,
    ) -> Result<Vec<Node>> {
        let nodes = self.graph.get_nodes_by_label(label).await?;
        Ok(nodes
            .into_iter()
            .filter(|n| n.properties.get(property) == Some(value))
            .collect())
    }

    async fn scan_pattern_triples_two_hop_current(
        &self,
        source_label: &str,
        rel_type_1: &str,
        middle_label: &str,
        rel_type_2: &str,
        target_label: &str,
    ) -> Result<HashSet<(NodeId, NodeId, NodeId)>> {
        let edges = self.graph.get_all_edges().await?;
        let mut out_by_source: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        let mut out_by_middle: HashMap<NodeId, Vec<NodeId>> = HashMap::new();

        for edge in edges {
            if edge.edge_type == rel_type_1 {
                out_by_source.entry(edge.source).or_default().push(edge.target);
            }
            if edge.edge_type == rel_type_2 {
                out_by_middle.entry(edge.source).or_default().push(edge.target);
            }
        }

        let mut triples = HashSet::new();

        for (source_id, middle_candidates) in out_by_source {
            let source = match self.graph.get_node(source_id).await {
                Ok(node) => node,
                Err(_) => continue,
            };
            if source.label != source_label {
                continue;
            }

            for middle_id in middle_candidates {
                let middle = match self.graph.get_node(middle_id).await {
                    Ok(node) => node,
                    Err(_) => continue,
                };
                if middle.label != middle_label {
                    continue;
                }

                let Some(target_candidates) = out_by_middle.get(&middle_id) else {
                    continue;
                };

                for target_id in target_candidates {
                    let target = match self.graph.get_node(*target_id).await {
                        Ok(node) => node,
                        Err(_) => continue,
                    };
                    if target.label == target_label {
                        triples.insert((source_id, middle_id, target.id));
                    }
                }
            }
        }

        Ok(triples)
    }
}

/// Auto-rollback si la transacción no se commitea
impl Drop for Transaction {
    fn drop(&mut self) {
        if self.state == TransactionState::Active {
            log::warn!("Transaction {} dropped without commit - auto-rollback", self.id);
            // Nota: No podemos llamar async aquí, solo limpiamos
            self.state = TransactionState::Aborted;

            #[cfg(feature = "full-isolation")]
            {
                log::warn!("Transaction {} dropped with locks held - may cause delays", self.id);
            }
        }
        // Siempre deregistrar al hacer drop, sin importar el estado previo
        self.graph.deregister_tx_timestamp_sync(self.id);
    }
}


#[tokio::test]
async fn test_indexing_without_transaction() {
    use crate::types::PropertyValue;  // ← Import SOLO en tests

    let graph = Graph::in_memory().await.unwrap();

    let alice = Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()));

    graph.add_node(alice.clone()).await.unwrap();

    // Debe estar indexada
    let found = graph.get_node_by_property("name", "Alice").await.unwrap();
    assert_eq!(found.id, alice.id);
}

#[tokio::test]
async fn test_indexing_with_transaction() {
    use crate::types::PropertyValue;  // ← Import SOLO en tests

    let graph = Graph::in_memory().await.unwrap();

    let mut tx = graph.begin_transaction().await.unwrap();

    let alice = Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()));

    let alice_id = tx.add_node(alice).await.unwrap();

    // ❌ NO debe estar indexada todavía (tx no commiteada)
    let result = graph.get_node_by_property("name", "Alice").await;
    assert!(result.is_err(), "No debería encontrar a Alice antes de commit");

    // Commit
    tx.commit().await.unwrap();

    // ✅ AHORA SÍ debe estar indexada
    let found = graph.get_node_by_property("name", "Alice").await.unwrap();
    assert_eq!(found.id, alice_id);
}

#[tokio::test]
async fn test_no_duplicate_indexing() {
    use crate::types::PropertyValue;  // ← Import SOLO en tests

    let graph = Graph::in_memory().await.unwrap();

    let mut tx = graph.begin_transaction().await.unwrap();

    let alice = Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()));

    let alice_id = tx.add_node(alice).await.unwrap();
    tx.commit().await.unwrap();

    // Buscar por propiedad
    let nodes = graph.get_all_nodes_by_property("name", &PropertyValue::String("Alice".into()))
        .await
        .unwrap();

    // ✅ Debe haber EXACTAMENTE 1 resultado (no 2)
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0], alice_id);
}
