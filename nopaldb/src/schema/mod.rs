// src/schema/mod.rs
//! Schema inspection and management for NopalDB
//!
//! El esquema derivado (etiquetas, tipos de arista, conteos y propiedades
//! por etiqueta/tipo) se mantiene **por operación** desde el único escritor
//! (los cuerpos `apply_*` del `Graph`, bajo el write gate) y se **persiste
//! en cada checkpoint** en el keyspace de metadatos, de donde `open` lo
//! carga. Solo se reconstruye recorriendo nodos y aristas (O(N+E)) cuando no
//! hay de dónde cargarlo: tras una recuperación de crash, en una base
//! anterior a 0.6.7, o cuando alguien llama a `rebuild_schema()`.
//!
//! Hasta 0.6.5 el esquema nacía sucio y nada lo volvía a marcar (valores
//! congelados); 0.6.6 lo marcaba en cada escritura y reconstruía en la
//! lectura siguiente (#164: 2.3 s y +430 MB por lectura con 1M nodos).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tokio::sync::RwLock;
use serde::{Serialize, Deserialize};

use crate::error::{NopalError, Result};
use crate::graph::Graph;
use crate::types::{Edge, Node};

/// Complete schema information for a graph.
///
/// Semántica del mantenimiento incremental: las propiedades por etiqueta o
/// tipo son un **superconjunto** (se añaden al escribir, nunca se retiran al
/// borrar o sobrescribir); una etiqueta o tipo desaparece, con su entrada de
/// propiedades, cuando su conteo llega a 0, igual que haría una
/// reconstrucción. `Graph::rebuild_schema` es la reparación exacta.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct SchemaInfo {
    /// All unique node labels in the graph
    pub node_labels: Vec<String>,

    /// All unique edge types in the graph
    pub edge_types: Vec<String>,

    /// Properties per node label
    pub node_properties: HashMap<String, HashSet<String>>,

    /// Properties per edge type
    pub edge_properties: HashMap<String, HashSet<String>>,

    /// Node count per label
    pub node_counts: HashMap<String, usize>,

    /// Edge count per type
    pub edge_counts: HashMap<String, usize>,

    /// Total nodes
    pub total_nodes: usize,

    /// Total edges
    pub total_edges: usize,
}


impl SchemaInfo {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a node label to the schema
    pub fn add_node_label(&mut self, label: String) {
        if !self.node_labels.contains(&label) {
            self.node_labels.push(label.clone());
            self.node_properties.insert(label.clone(), HashSet::new());
            self.node_counts.insert(label, 0);
        }
    }

    /// Add a property to a node label
    pub fn add_node_property(&mut self, label: &str, property: String) {
        self.node_properties
            .entry(label.to_string())
            .or_default()
            .insert(property);
    }

    /// Add an edge type to the schema
    pub fn add_edge_type(&mut self, edge_type: String) {
        if !self.edge_types.contains(&edge_type) {
            self.edge_types.push(edge_type.clone());
            self.edge_properties.insert(edge_type.clone(), HashSet::new());
            self.edge_counts.insert(edge_type, 0);
        }
    }

    /// Add a property to an edge type
    pub fn add_edge_property(&mut self, edge_type: &str, property: String) {
        self.edge_properties
            .entry(edge_type.to_string())
            .or_default()
            .insert(property);
    }

    /// Increment node count for a label
    pub fn increment_node_count(&mut self, label: &str) {
        *self.node_counts.entry(label.to_string()).or_insert(0) += 1;
        self.total_nodes += 1;
    }

    /// Increment edge count for a type
    pub fn increment_edge_count(&mut self, edge_type: &str) {
        *self.edge_counts.entry(edge_type.to_string()).or_insert(0) += 1;
        self.total_edges += 1;
    }

    /// Resta un nodo de `label`; al llegar a 0 la etiqueta y sus propiedades
    /// desaparecen (como en una reconstrucción). `false` si la etiqueta no
    /// existía o ya estaba en 0: el esquema no cuadra con lo que se borra y
    /// el llamador debe marcarlo sucio.
    pub fn decrement_node_count(&mut self, label: &str) -> bool {
        let Some(c) = self.node_counts.get_mut(label) else { return false };
        if *c == 0 {
            return false;
        }
        *c -= 1;
        self.total_nodes = self.total_nodes.saturating_sub(1);
        if *c == 0 {
            self.node_counts.remove(label);
            self.node_properties.remove(label);
            self.node_labels.retain(|l| l != label);
        }
        true
    }

    /// Resta una arista de `edge_type`; ver [`Self::decrement_node_count`].
    pub fn decrement_edge_count(&mut self, edge_type: &str) -> bool {
        let Some(c) = self.edge_counts.get_mut(edge_type) else { return false };
        if *c == 0 {
            return false;
        }
        *c -= 1;
        self.total_edges = self.total_edges.saturating_sub(1);
        if *c == 0 {
            self.edge_counts.remove(edge_type);
            self.edge_properties.remove(edge_type);
            self.edge_types.retain(|t| t != edge_type);
        }
        true
    }

    /// Delta de una alta o sobrescritura de nodo. Se calcula del par
    /// `(viejo, nuevo)` y no de un booleano "existía": el redo del WAL
    /// escribe el registro antes de que el apply lo mire, así que el booleano
    /// mentiría; el par no. Idempotente: `old == new` es delta cero.
    pub fn apply_node_upsert(&mut self, old: Option<&Node>, new: &Node) -> bool {
        let mut ok = true;
        match old {
            None => {
                self.add_node_label(new.label.clone());
                self.increment_node_count(&new.label);
            }
            Some(o) if o.label != new.label => {
                ok = self.decrement_node_count(&o.label);
                self.add_node_label(new.label.clone());
                self.increment_node_count(&new.label);
            }
            Some(_) => self.add_node_label(new.label.clone()),
        }
        for k in new.properties.keys() {
            self.add_node_property(&new.label, k.clone());
        }
        ok
    }

    /// Delta de una baja de nodo (sus aristas incidentes se descuentan
    /// aparte, por tipo, desde la purga de adyacencia).
    pub fn apply_node_delete(&mut self, node: &Node) -> bool {
        self.decrement_node_count(&node.label)
    }

    /// Delta de una alta o sobrescritura de arista; ver [`Self::apply_node_upsert`].
    pub fn apply_edge_upsert(&mut self, old: Option<&Edge>, new: &Edge) -> bool {
        let mut ok = true;
        match old {
            None => {
                self.add_edge_type(new.edge_type.clone());
                self.increment_edge_count(&new.edge_type);
            }
            Some(o) if o.edge_type != new.edge_type => {
                ok = self.decrement_edge_count(&o.edge_type);
                self.add_edge_type(new.edge_type.clone());
                self.increment_edge_count(&new.edge_type);
            }
            Some(_) => self.add_edge_type(new.edge_type.clone()),
        }
        for k in new.properties.keys() {
            self.add_edge_property(&new.edge_type, k.clone());
        }
        ok
    }

    /// Delta de una baja de arista.
    pub fn apply_edge_delete(&mut self, edge: &Edge) -> bool {
        self.decrement_edge_count(&edge.edge_type)
    }
}

/// Versión del blob persistido (`META_SCHEMA_SNAPSHOT`). Un formato
/// desconocido se descarta y el esquema se reconstruye: nunca es un error de
/// apertura.
pub(crate) const SCHEMA_SNAPSHOT_FORMAT: u32 = 1;

/// Lo que va al keyspace de metadatos en cada checkpoint: MessagePack, como
/// los valores del KV.
#[derive(Serialize, Deserialize)]
pub(crate) struct SchemaSnapshot {
    pub format: u32,
    pub info: SchemaInfo,
}

impl SchemaSnapshot {
    pub(crate) fn encode(info: &SchemaInfo) -> Result<Vec<u8>> {
        rmp_serde::to_vec(&SchemaSnapshot { format: SCHEMA_SNAPSHOT_FORMAT, info: info.clone() })
            .map_err(|e| NopalError::SerializationError(format!("schema snapshot: {e}")))
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<SchemaInfo> {
        let snap: SchemaSnapshot = rmp_serde::from_slice(bytes)
            .map_err(|e| NopalError::SerializationError(format!("schema snapshot: {e}")))?;
        if snap.format != SCHEMA_SNAPSHOT_FORMAT {
            return Err(NopalError::SerializationError(format!(
                "schema snapshot format {} (this build reads {})",
                snap.format, SCHEMA_SNAPSHOT_FORMAT
            )));
        }
        Ok(snap.info)
    }
}

/// Schema manager with caching.
///
/// Orden de locks: `rebuild` toma el write gate del grafo y luego el
/// `RwLock`; los `apply` corren bajo el gate (los cuerpos `apply_*`) y toman
/// el `RwLock`; los lectores solo el `RwLock`. Nunca al revés, y **ningún
/// código que sostenga el gate debe llamar a `get_schema`/`get_stats`/
/// `create_planner`**: `rebuild` esperaría el gate que ese código sostiene.
pub struct SchemaManager {
    info: Arc<RwLock<SchemaInfo>>,
    /// `true` = no hay esquema válido en memoria; la próxima lectura
    /// reconstruye y los incrementos se ignoran hasta entonces.
    dirty: Arc<AtomicBool>,
    /// Reconstrucciones completas hechas por este manager. Los tests afirman
    /// "cero" tras escribir y tras reabrir limpio.
    rebuilds: Arc<AtomicU64>,
}

impl SchemaManager {
    /// Sin esquema: la primera lectura reconstruye.
    pub fn new() -> Self {
        Self {
            info: Arc::new(RwLock::new(SchemaInfo::new())),
            dirty: Arc::new(AtomicBool::new(true)),
            rebuilds: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Con un esquema ya válido (cargado del snapshot persistido, o vacío
    /// para una base recién creada).
    pub fn from_persisted(info: SchemaInfo) -> Self {
        Self {
            info: Arc::new(RwLock::new(info)),
            dirty: Arc::new(AtomicBool::new(false)),
            rebuilds: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Check if schema needs rebuilding
    pub fn is_dirty(&self) -> bool {
        self.dirty.load(Ordering::SeqCst)
    }

    /// Mark schema as dirty (needs rebuild)
    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::SeqCst);
    }

    /// Reconstrucciones completas hechas hasta ahora.
    pub fn rebuild_count(&self) -> u64 {
        self.rebuilds.load(Ordering::SeqCst)
    }

    /// Aplica un delta al esquema. El llamador SOSTIENE el write gate. Si el
    /// esquema está sucio no hay nada que mantener (la próxima lectura lo
    /// reconstruye entero). Si el delta devuelve `false` el esquema no cuadra
    /// con la escritura y pasa a sucio.
    pub(crate) async fn apply(&self, f: impl FnOnce(&mut SchemaInfo) -> bool) {
        if self.is_dirty() {
            return;
        }
        let mut info = self.info.write().await;
        if !f(&mut info) {
            log::warn!("schema: an increment did not match the cached schema; it will be rebuilt on the next read");
            self.dirty.store(true, Ordering::SeqCst);
        }
    }

    async fn scan(graph: &Graph) -> Result<SchemaInfo> {
        log::info!("Rebuilding schema from graph...");
        let mut info = SchemaInfo::new();

        let nodes = graph.get_all_nodes().await?;
        log::debug!("Scanning {} nodes for schema", nodes.len());
        for node in nodes {
            let label = node.label.clone();
            info.add_node_label(label.clone());
            info.increment_node_count(&label);
            for key in node.properties.keys() {
                info.add_node_property(&label, key.clone());
            }
        }

        let edges = graph.get_all_edges().await?;
        log::debug!("Scanning {} edges for schema", edges.len());
        for edge in edges {
            let edge_type = edge.edge_type.clone();
            info.add_edge_type(edge_type.clone());
            info.increment_edge_count(&edge_type);
            for key in edge.properties.keys() {
                info.add_edge_property(&edge_type, key.clone());
            }
        }
        Ok(info)
    }

    /// Reconstruye el esquema recorriendo nodos y aristas (O(N+E)). Toma el
    /// write gate del grafo para no correr contra los applies y re-comprueba
    /// `dirty` bajo el gate: dos lectores concurrentes reconstruyen una vez.
    /// `force` reconstruye aunque esté limpio (`Graph::rebuild_schema`).
    pub async fn rebuild(&self, graph: &Graph, force: bool) -> Result<()> {
        let gate = graph.write_gate();
        let _gate = gate.lock().await;
        if !force && !self.is_dirty() {
            return Ok(());
        }
        self.rebuild_locked(graph).await
    }

    /// Cuerpo de [`Self::rebuild`]. El llamador SOSTIENE el write gate.
    pub(crate) async fn rebuild_locked(&self, graph: &Graph) -> Result<()> {
        let info = Self::scan(graph).await?;
        *self.info.write().await = info;
        self.dirty.store(false, Ordering::SeqCst);
        self.rebuilds.fetch_add(1, Ordering::SeqCst);
        log::info!("Schema rebuilt successfully");
        Ok(())
    }

    /// Lee el esquema (reconstruyéndolo antes si está sucio) sin clonarlo.
    pub async fn with_info<R>(&self, graph: &Graph, f: impl FnOnce(&SchemaInfo) -> R) -> Result<R> {
        if self.is_dirty() {
            self.rebuild(graph, false).await?;
        }
        Ok(f(&*self.info.read().await))
    }

    /// Get cached schema info (rebuilds if dirty)
    pub async fn get_info(&self, graph: &Graph) -> Result<SchemaInfo> {
        self.with_info(graph, |info| info.clone()).await
    }

    /// Get cached schema without rebuilding
    pub async fn get_info_cached(&self) -> SchemaInfo {
        self.info.read().await.clone()
    }

    /// El esquema tal cual está si es válido; `None` si está sucio. Para el
    /// checkpoint: nunca reconstruye (corre bajo el gate).
    pub(crate) async fn snapshot_if_clean(&self) -> Option<SchemaInfo> {
        if self.is_dirty() {
            return None;
        }
        Some(self.info.read().await.clone())
    }

    /// `(total_nodes, total_edges)` si el esquema es válido; `None` si está
    /// sucio (el llamador cuenta claves).
    pub(crate) async fn cached_totals(&self) -> Option<(usize, usize)> {
        if self.is_dirty() {
            return None;
        }
        let info = self.info.read().await;
        Some((info.total_nodes, info.total_edges))
    }
}

impl Default for SchemaManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PropertyValue;

    #[test]
    fn upsert_delete_and_label_move_keep_counts_and_drop_empty_labels() {
        let mut s = SchemaInfo::new();
        let a = Node::new("A").with_property("x", PropertyValue::Int(1));
        assert!(s.apply_node_upsert(None, &a));
        assert_eq!((s.total_nodes, s.node_counts["A"]), (1, 1));
        assert!(s.node_properties["A"].contains("x"));

        // Sobrescritura sin cambio de etiqueta: delta cero, propiedad nueva listada.
        let a2 = Node::new("A").with_property("y", PropertyValue::Int(2));
        assert!(s.apply_node_upsert(Some(&a), &a2));
        assert_eq!(s.total_nodes, 1);
        assert!(s.node_properties["A"].contains("x") && s.node_properties["A"].contains("y"), "superset");

        // Cambio de etiqueta: A desaparece, B aparece.
        let b = Node::new("B");
        assert!(s.apply_node_upsert(Some(&a2), &b));
        assert_eq!(s.node_labels, vec!["B"]);
        assert!(!s.node_counts.contains_key("A") && !s.node_properties.contains_key("A"));

        assert!(s.apply_node_delete(&b));
        assert_eq!((s.total_nodes, s.node_labels.len()), (0, 0));
        assert!(!s.apply_node_delete(&b), "deleting from an empty schema is an inconsistency");
    }

    #[test]
    fn edge_type_change_moves_the_count() {
        let mut s = SchemaInfo::new();
        let (x, y) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        let e = Edge::new(x, y, "KNOWS");
        assert!(s.apply_edge_upsert(None, &e));
        let mut e2 = Edge::new(x, y, "LIKES");
        e2.id = e.id;
        assert!(s.apply_edge_upsert(Some(&e), &e2));
        assert_eq!(s.edge_types, vec!["LIKES"]);
        assert_eq!(s.total_edges, 1);
        assert!(s.apply_edge_delete(&e2));
        assert!(s.edge_types.is_empty() && s.total_edges == 0);
    }

    #[test]
    fn snapshot_round_trips_and_rejects_unknown_format() {
        let mut s = SchemaInfo::new();
        s.apply_node_upsert(None, &Node::new("A").with_property("p", PropertyValue::Bool(true)));
        let bytes = SchemaSnapshot::encode(&s).unwrap();
        let back = SchemaSnapshot::decode(&bytes).unwrap();
        assert_eq!(back.node_counts, s.node_counts);
        assert_eq!(back.node_properties, s.node_properties);
        let bad = rmp_serde::to_vec(&SchemaSnapshot { format: 99, info: s }).unwrap();
        assert!(SchemaSnapshot::decode(&bad).is_err());
    }
}
