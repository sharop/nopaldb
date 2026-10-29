// src/storage/graph_at.rs
//
// El grafo entero tal como era en un timestamp lógico (#298): lo que NQL
// `find … at timestamp <ts>` materializa para correr la consulta normal.
//
// Nodos: todo id con cadena (`l|`), con marca (`b|`/`f|`) o con registro
// vigente, resuelto con `node_version_at`, que ya aplica la tabla de
// precedencia de `Stamp`. Aristas: lo mismo sobre `versioned_edges` y sus
// marcas (`a|`/`e|`).
//
// Nada se omite en silencio (NQL_SEMANTICS §5e): si el estado de una sola
// entidad en `ts` no se conoce (bajo su suelo de historia, sin historia, o
// bajo el horizonte del GC), la respuesta entera sería inventada, así que es
// `HistoryUnavailable`. Una arista vigente con un extremo que no lo está es
// una historia incoherente, y es error, no una fila menos.
//
// Coste: O(historia entera) lecturas y memoria para todo lo vigente en `ts`.
// Se descartó un índice de aristas versionadas por nodo para esta primera
// versión: duplicaría la adyacencia en el historial antes de saber si el
// coste de materializar le importa a alguien.

use std::collections::{HashMap, HashSet};

use crate::error::{NopalError, Result};
use crate::mvcc::VersionedEdge;
use crate::types::{Edge, EdgeId, Node, NodeId};

use super::keys::v2;
use super::{
    deserialize, malformed_key, NodeVersionAt, Stamp, Storage, ENTITIES_TREE, HISTORY_TREE,
};

/// Nodos y aristas válidos en un timestamp.
#[derive(Debug, Default)]
pub(crate) struct GraphAt {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl Storage {
    /// El grafo válido en `timestamp`; ver la cabecera del módulo.
    pub(crate) async fn graph_at(&self, timestamp: u64) -> Result<GraphAt> {
        self.check_gc_horizon(timestamp)?;
        let nodes = self.nodes_at(timestamp).await?;
        let edges = self.edges_at(timestamp)?;

        // Una arista vigente apunta a nodos vigentes: los borrados cierran en
        // cascada, en el mismo lote, desde 0.10.
        let alive: HashSet<NodeId> = nodes.iter().map(|n| n.id).collect();
        if let Some(e) = edges.iter().find(|e| !alive.contains(&e.source) || !alive.contains(&e.target)) {
            return Err(NopalError::QueryExecutionError(format!(
                "inconsistent history at timestamp {timestamp}: edge {} ({} -> {}) is valid but one \
                 of its endpoints is not",
                e.id, e.source, e.target
            )));
        }

        // Un GC concurrente pudo recoger parte de lo leído.
        self.check_gc_horizon(timestamp)?;
        Ok(GraphAt { nodes, edges })
    }

    fn check_gc_horizon(&self, timestamp: u64) -> Result<()> {
        let horizon = self.gc_horizon()?;
        if timestamp < horizon {
            return Err(NopalError::HistoryUnavailable(format!(
                "timestamp {timestamp} is below the GC horizon {horizon}: the garbage collector \
                 already removed history before it"
            )));
        }
        Ok(())
    }

    async fn nodes_at(&self, timestamp: u64) -> Result<Vec<Node>> {
        let mut ids: HashSet<NodeId> = HashSet::new();
        for item in self.history_ks.scan_prefix(&[v2::HISTORY_VERSIONS_TAG]) {
            let (key, _) = item?;
            ids.insert(v2::parse_history_versions_key(&key).ok_or_else(|| malformed_key(HISTORY_TREE, &key))?);
        }
        for tag in [v2::HISTORY_NODE_BIRTH_TAG, v2::HISTORY_NODE_FLOOR_TAG] {
            for item in self.history_ks.scan_prefix(&[tag]) {
                let (key, _) = item?;
                ids.insert(v2::parse_history_stamp_key(&key).ok_or_else(|| malformed_key(HISTORY_TREE, &key))?);
            }
        }
        for item in self.entities_ks.iter() {
            let (key, _) = item?;
            ids.insert(v2::parse_node_key_v2(&key).ok_or_else(|| malformed_key(ENTITIES_TREE, &key))?);
        }

        let mut nodes = Vec::new();
        for id in ids {
            // Un escritor que materializa la v1 de un nodo con marca borra la
            // marca y escribe la cadena en un lote: leer entre las dos
            // lecturas de `node_version_at` puede dar "sin historia". Un
            // reintento ve el lote entero.
            let mut at = self.node_version_at(id, timestamp).await?;
            if matches!(at, NodeVersionAt::NoHistory | NodeVersionAt::Unknown(_)) {
                at = self.node_version_at(id, timestamp).await?;
            }
            match at {
                NodeVersionAt::Found(v) => nodes.push(v.node_data),
                NodeVersionAt::NotValidAt | NodeVersionAt::NotBorn => {}
                NodeVersionAt::NoHistory => {
                    return Err(NopalError::HistoryUnavailable(format!(
                        "node {id} at timestamp {timestamp}: it has no history (written with a \
                         low-level Storage primitive), so the graph then is unknown"
                    )))
                }
                NodeVersionAt::Unknown(msg) => return Err(NopalError::HistoryUnavailable(msg)),
            }
        }
        Ok(nodes)
    }

    fn edges_at(&self, timestamp: u64) -> Result<Vec<Edge>> {
        // Cadenas: la versión válida en `timestamp` (a lo sumo una).
        let mut chains: HashMap<EdgeId, Option<VersionedEdge>> = HashMap::new();
        for item in self.versioned_edges_ks.iter() {
            let (_, raw) = item?;
            let ve: VersionedEdge = deserialize(&raw)?;
            let slot = chains.entry(ve.id).or_insert(None);
            if ve.is_valid_at(timestamp) && slot.as_ref().is_none_or(|cur| ve.version > cur.version) {
                *slot = Some(ve);
            }
        }

        let mut edges = Vec::new();
        for (&id, valid) in &chains {
            if let Some(floor) = self.edge_history_floor(id)?
                && timestamp < floor
            {
                return Err(edge_below_floor(id, timestamp, floor));
            }
            if let Some(ve) = valid {
                edges.push(ve.edge_data.clone());
            }
        }

        // Sin cadena: la marca decide, como en los nodos.
        let mut chainless: HashSet<EdgeId> = HashSet::new();
        for tag in [v2::HISTORY_EDGE_BIRTH_TAG, v2::HISTORY_EDGE_FLOOR_TAG] {
            for item in self.history_ks.scan_prefix(&[tag]) {
                let (key, _) = item?;
                let id = v2::parse_history_stamp_key(&key).ok_or_else(|| malformed_key(HISTORY_TREE, &key))?;
                if !chains.contains_key(&id) {
                    chainless.insert(id);
                }
            }
        }
        for item in self.edges_ks.iter() {
            let (key, _) = item?;
            let id = std::str::from_utf8(&key)
                .ok()
                .and_then(|s| EdgeId::parse_str(s).ok())
                .ok_or_else(|| malformed_key("edges", &key))?;
            if !chains.contains_key(&id) {
                chainless.insert(id);
            }
        }
        for id in chainless {
            let record = self.edges_ks.get(id.to_string().as_bytes())?;
            match self.edge_stamp(id)? {
                None => {
                    return Err(NopalError::HistoryUnavailable(format!(
                        "edge {id} at timestamp {timestamp}: it has no history (written with a \
                         low-level Storage primitive), so the graph then is unknown"
                    )))
                }
                Some(Stamp::Birth(t)) if timestamp < t => {}
                Some(Stamp::Floor(f)) if timestamp < f => return Err(edge_below_floor(id, timestamp, f)),
                Some(_) => match record {
                    Some(raw) => edges.push(deserialize(&raw)?),
                    None => {
                        return Err(NopalError::HistoryUnavailable(format!(
                            "edge {id} at timestamp {timestamp}: its record was removed without a \
                             version (low-level Storage primitive), so its state then is unknown"
                        )))
                    }
                },
            }
        }
        Ok(edges)
    }
}

fn edge_below_floor(id: EdgeId, timestamp: u64, floor: u64) -> NopalError {
    NopalError::HistoryUnavailable(format!(
        "edge {id} at timestamp {timestamp}: its history starts at {floor} \
         (written without a version before 0.10.1, adopted when the database was opened)"
    ))
}

