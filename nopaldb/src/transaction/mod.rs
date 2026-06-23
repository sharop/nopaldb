// src/transaction/mod.rs

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::error::{NopalError, Result};
use crate::graph::Graph;
use crate::types::{Edge, EdgeId, Node, NodeId, PropertyValue};
use crate::wal::WalRecord;

use crate::mvcc::VersionedNode;

/// ID único de transacción
pub type TransactionId = u64;

/// Timestamp lógico monotónico
pub type Timestamp = u64;

/// Estado de una transacción
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionState {
    Active,    // En progreso
    Committed, // Completada exitosamente
    Aborted,   // Cancelada/revertida
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IsolationLevel;

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
        }
    }

    /// Obtiene un nodo (ve cambios pendientes de esta tx)
    pub async fn get_node(&self, id: NodeId) -> Result<Node> {
        if self.state != TransactionState::Active {
            return Err(NopalError::TransactionNotActive);
        }

        // ¿Fue borrado en esta tx?
        if self.deleted_nodes.contains(&id) {
            return Err(NopalError::NodeNotFound(id.to_string()));
        }

        // ¿Está en el write buffer?
        if let Some(node) = self.pending_nodes.get(&id) {
            return Ok(node.clone());
        }

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

        self.pending_nodes.insert(node_id, node);

        Ok(node_id)
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

        self.graph.get_nodes_by_label(label).await
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

        self.graph.get_all_nodes_by_property(property, value).await
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

        {
            self.scan_nodes_by_label_property_current(label, property, value)
                .await
        }
    }

    /// Obtiene todos los nodos respetando isolation level (útil para scans/paginación).
    pub async fn get_all_nodes(&self) -> Result<Vec<Node>> {
        if self.state != TransactionState::Active {
            return Err(NopalError::TransactionNotActive);
        }

        self.graph.get_all_nodes().await
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
        self.pending_nodes.remove(&id); // Si estaba pendiente, cancelarlo

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

        log::info!(
            "Committing transaction {} (isolation: {:?}",
            self.id,
            self.get_isolation_level_name()
        );

        // Timestamp lógico monotónico para mantener consistencia con snapshot_timestamp.
        let commit_timestamp = self.graph.next_logical_timestamp();

        // P1. Escribir en el WAL antes de modificar el storage.
        let wal = self.graph.wal();

        // Write BEGIN marker
        wal.append(WalRecord::Begin {
            tx_id: self.id,
            timestamp: self.timestamp,
        })
        .await?;

        // Write DELETE operations
        for node_id in &self.deleted_nodes {
            let node = self.graph.get_node(*node_id).await?;
            wal.append(WalRecord::DeleteNode {
                tx_id: self.id,
                node_id: *node_id,
                node,
            })
            .await?;
        }

        // Write INSERT/UPDATE nodes
        for node in self.pending_nodes.values() {
            wal.append(WalRecord::InsertNode {
                tx_id: self.id,
                node: node.clone(),
            })
            .await?;
        }

        // Write INSERT edges
        for edge in self.pending_edges.values() {
            wal.append(WalRecord::InsertEdge {
                tx_id: self.id,
                edge: edge.clone(),
            })
            .await?;
        }

        // Write COMMIT marker
        wal.append(WalRecord::Commit {
            tx_id: self.id,
            timestamp: commit_timestamp,
        })
        .await?;

        log::info!("Transaction {} written to WAL", self.id);

        //P2. Aplicar cambios al storage con MVCC

        // 1. Aplicar borrados
        for node_id in &self.deleted_nodes {
            self.graph.delete_node(*node_id).await?;
        }

        for edge_id in &self.deleted_edges {
            if let Err(e) = self.graph.delete_edge_at(*edge_id, commit_timestamp).await {
                log::warn!("Failed to delete edge {} during commit: {}", edge_id, e);
            }
        }

        // 2. Aplicar inserts/updates de nodos SIN indexar
        for (node_id, node) in &self.pending_nodes {
            let is_update = self.graph.node_exists(*node_id).await?;

            if is_update {
                // ═════════════════════════════════════════════════════
                // UPDATE: Crear nueva versión
                // ═════════════════════════════════════════════════════

                // 1. Obtener versión actual
                let current_version_num = self.graph.get_current_version(*node_id).await?;

                let current_version = self
                    .graph
                    .get_node_version(*node_id, current_version_num)
                    .await?;

                // 2. Invalidar versión actual
                self.graph
                    .invalidate_current_version(*node_id, commit_timestamp)
                    .await?;

                // 3. Crear nueva versión
                let new_version =
                    VersionedNode::new_version(&current_version, node.clone(), commit_timestamp);

                // 4. Guardar nueva versión
                self.graph.insert_node_version(&new_version).await?;

                log::debug!(
                    "Updated node {} (v{} -> v{})",
                    node_id,
                    current_version.version,
                    new_version.version
                );
            } else {
                // ═════════════════════════════════════════════════════
                // INSERT: Primera versión
                // ═════════════════════════════════════════════════════

                let first_version = VersionedNode::new(node.clone(), commit_timestamp);

                self.graph.insert_node_version(&first_version).await?;

                log::debug!("Inserted node {} (v1)", node_id);
            }

            // Mantener compatibilidad: actualizar storage tradicional
            self.graph.add_node_internal(node.clone(), true).await?;
        }

        // 3. Aplicar aristas (con timestamp MVCC del commit para consistencia)
        for edge in self.pending_edges.values() {
            self.graph
                .add_edge_at(edge.clone(), commit_timestamp)
                .await?;
        }

        // 4. Indexar propiedades (UNA SOLA VEZ)
        for node in self.pending_nodes.values() {
            self.graph.index_node_properties(node).await?;
        }

        // 5. Flush índices a disco
        self.graph.flush_indices().await?;

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

        log::info!(
            "Rolling back transaction {} (sync, WAL Abort NOT written — use rollback_async() when possible)",
            self.id
        );

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

        log::info!(
            "Rolling back transaction {} (async, writing WAL Abort)",
            self.id
        );

        // Escribir Abort al WAL ANTES de limpiar buffers
        // Esto garantiza durabilidad: si el proceso muere aquí, el WAL tiene el Abort.
        let wal = self.graph.wal();
        if let Err(e) = wal.append(WalRecord::Abort { tx_id: self.id }).await {
            // No fatal: el recovery ignora txs sin Commit de todas formas.
            // Loguear la advertencia pero continuar con el rollback en memoria.
            log::warn!(
                "Transaction {}: failed to write WAL Abort record: {} — rollback still applied in memory",
                self.id,
                e
            );
        }

        // Limpiar buffers en memoria
        self.pending_nodes.clear();
        self.pending_edges.clear();
        self.deleted_nodes.clear();
        self.deleted_edges.clear();

        self.state = TransactionState::Aborted;

        // Deregistrar del mapa de transacciones activas
        self.graph.deregister_tx_timestamp_sync(self.id);

        log::info!(
            "Transaction {} rolled back (async, WAL Abort written)",
            self.id
        );
        Ok(())
    }

    /// Helper para logging
    fn get_isolation_level_name(&self) -> &str {
        "ReadCommitted (minimal)"
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
                out_by_source
                    .entry(edge.source)
                    .or_default()
                    .push(edge.target);
            }
            if edge.edge_type == rel_type_2 {
                out_by_middle
                    .entry(edge.source)
                    .or_default()
                    .push(edge.target);
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
            log::warn!(
                "Transaction {} dropped without commit - auto-rollback",
                self.id
            );
            // Nota: No podemos llamar async aquí, solo limpiamos
            self.state = TransactionState::Aborted;
        }
        // Siempre deregistrar al hacer drop, sin importar el estado previo
        self.graph.deregister_tx_timestamp_sync(self.id);
    }
}

#[tokio::test]
async fn test_indexing_without_transaction() {
    use crate::types::PropertyValue; // ← Import SOLO en tests

    let graph = Graph::in_memory().await.unwrap();

    let alice = Node::new("Person").with_property("name", PropertyValue::String("Alice".into()));

    graph.add_node(alice.clone()).await.unwrap();

    // Debe estar indexada
    let found = graph.get_node_by_property("name", "Alice").await.unwrap();
    assert_eq!(found.id, alice.id);
}

#[tokio::test]
async fn test_indexing_with_transaction() {
    use crate::types::PropertyValue; // ← Import SOLO en tests

    let graph = Graph::in_memory().await.unwrap();

    let mut tx = graph.begin_transaction().await.unwrap();

    let alice = Node::new("Person").with_property("name", PropertyValue::String("Alice".into()));

    let alice_id = tx.add_node(alice).await.unwrap();

    // ❌ NO debe estar indexada todavía (tx no commiteada)
    let result = graph.get_node_by_property("name", "Alice").await;
    assert!(
        result.is_err(),
        "No debería encontrar a Alice antes de commit"
    );

    // Commit
    tx.commit().await.unwrap();

    // ✅ AHORA SÍ debe estar indexada
    let found = graph.get_node_by_property("name", "Alice").await.unwrap();
    assert_eq!(found.id, alice_id);
}

#[tokio::test]
async fn test_no_duplicate_indexing() {
    use crate::types::PropertyValue; // ← Import SOLO en tests

    let graph = Graph::in_memory().await.unwrap();

    let mut tx = graph.begin_transaction().await.unwrap();

    let alice = Node::new("Person").with_property("name", PropertyValue::String("Alice".into()));

    let alice_id = tx.add_node(alice).await.unwrap();
    tx.commit().await.unwrap();

    // Buscar por propiedad
    let nodes = graph
        .get_all_nodes_by_property("name", &PropertyValue::String("Alice".into()))
        .await
        .unwrap();

    // ✅ Debe haber EXACTAMENTE 1 resultado (no 2)
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0], alice_id);
}
