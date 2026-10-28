// src/storage/history_adoption.rs
//
// Adopción de historia (0.10.1): pasada única al abrir una base escrita por
// una versión que NO versionaba todas las escrituras (≤ 0.10.0: `add_node`
// directo, `BulkLoader`, la importación Turtle, `storage_insert_*`) para que,
// a partir de aquí, todo nodo y toda arista tengan historia: una cadena MVCC
// o una versión implícita (ver `Stamp` en `storage/mod.rs`).
//
// Con un solo timestamp lógico F, tomado del reloj la primera vez que hace
// falta y persistido para reanudar con el mismo:
//
//   1. nodos     — registro en `entities` sin cadena ⇒ solo su SUELO F, sin
//                  copia: su versión implícita es el registro desde F (una
//                  v1 copiada costaba lo mismo que el bulk: +62 % de disco).
//                  Con cadena desincronizada (la última versión abierta con
//                  otros datos, o cerrada por un borrado aunque el registro
//                  exista: un `add_node` directo la pisó) ⇒ la siguiente
//                  versión en F, cerrando la vigente;
//   2. cadenas   — puntero `c|` abierto de un nodo que ya no está en
//                  `entities` (borrado antes de que el borrado cerrara la
//                  versión) ⇒ se cierra en F;
//   3. aristas   — registro en `edges` sin versión ⇒ solo su suelo F (o la
//                  siguiente versión de su cadena si tiene versiones
//                  cerradas); con versión vigente distinta del registro ⇒
//                  v+1 en F;
//   4. versiones — current abierto de una arista que ya no está ⇒ se cierra
//                  en F.
//
// Cada nodo o arista tocado recibe su SUELO de historia = F
// (`keys::v2::history_*_floor_key`): antes de F su historia no se conoce, y
// `Graph::get_node_at` lo dice con `HistoryUnavailable` en vez de contestar
// "no existía". El suelo persiste cuando más tarde la entidad tenga cadena.
// Se descartó una ventana `[desde, F)` en lugar del suelo
// (conservaría el pasado real de las cadenas desincronizadas antes de su
// última versión): la decisión fue un suelo por entidad, más simple de
// explicar; las cadenas sanas, que son la inmensa mayoría de lo escrito por
// transacción, no reciben suelo y conservan su pasado entero.
//
// Reanudable e idempotente: cada lote es un `apply_multi` atómico (versión +
// suelo juntos) y adoptar algo ya adoptado es un no-op (tiene ya su suelo, o
// su cadena una versión en F con los mismos datos); la marca de terminado
// (`META_HISTORY_ADOPTED`) se escribe al final, tras un flush si se adoptó
// algo. Coste: O(nodos + aristas + cadenas) lecturas puntuales, y cada tramo
// recorrido con `range_from` publica antes la ventana de escritura de redb
// (un commit por tramo de `ADOPTION_CHUNK` claves). Memoria
// acotada: se recorre por tramos de `ADOPTION_CHUNK` claves con
// `range_from` (sin iterar un keyspace mientras se escribe en él).

use std::collections::HashSet;

use crate::error::Result;
use crate::mvcc::{VersionedEdge, VersionedNode};
use crate::types::{Edge, EdgeId, Node, NodeId};

use super::keys::{self, v2};
use super::kv::{self, KvKeyspace};
use super::{
    deserialize, malformed_key, serialize, EdgeVersionPlan, NodeVersionPlan, NodeWriteBatches,
    Storage, ENTITIES_TREE, HISTORY_TREE, META_NEXT_TIMESTAMP, VERSIONED_EDGES_CURRENT_TREE,
    VERSIONED_EDGES_TREE,
};

/// Marca de la adopción terminada (u64 = formato de la pasada). Ausente ⇒ el
/// open la corre. Una base creada por 0.10.1 la escribe en su primer open
/// (pasada vacía).
pub const META_HISTORY_ADOPTED: &str = "history_adopted";
/// El F de la adopción, persistido al tomarlo: una pasada interrumpida se
/// reanuda con el MISMO timestamp (un suelo por base, no uno por intento).
pub const META_HISTORY_ADOPTION_TS: &str = "history_adoption_ts";
/// Formato actual de la pasada.
pub(crate) const HISTORY_ADOPTION_FORMAT: u64 = 1;

/// Claves por tramo (y por `apply_multi`).
const ADOPTION_CHUNK: usize = 1024;

/// Lo que hizo una pasada de adopción.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AdoptionStats {
    /// El F usado, o `None` si no hubo nada que adoptar.
    pub floor_ts: Option<u64>,
    /// Nodos sin cadena: solo suelo.
    pub nodes_floored: usize,
    /// Nodos con cadena desincronizada: v+1 en F.
    pub nodes_versioned: usize,
    pub node_chains_closed: usize,
    /// Aristas sin versiones: solo suelo.
    pub edges_floored: usize,
    pub edges_versioned: usize,
    pub edge_versions_closed: usize,
}

/// Tramo de hasta `ADOPTION_CHUNK` pares `(clave, valor)` con `prefix`,
/// desde `cursor` exclusive. El iterador se suelta al volver.
fn next_chunk(
    ks: &dyn KvKeyspace,
    prefix: &[u8],
    cursor: &Option<Vec<u8>>,
) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
    let start = match cursor {
        // El sucesor inmediato de la última clave vista.
        Some(last) => {
            let mut s = last.clone();
            s.push(0);
            s
        }
        None => prefix.to_vec(),
    };
    let mut out = Vec::with_capacity(ADOPTION_CHUNK);
    for item in ks.range_from(&start) {
        let (key, value) = item?;
        if !key.starts_with(prefix) {
            break;
        }
        out.push((key, value));
        if out.len() >= ADOPTION_CHUNK {
            break;
        }
    }
    Ok(out)
}

fn same_node(a: &Node, b: &Node) -> bool {
    a.label == b.label && a.kind == b.kind && a.properties == b.properties
}

fn same_edge(a: &Edge, b: &Edge) -> bool {
    a.source == b.source
        && a.target == b.target
        && a.edge_type == b.edge_type
        && a.properties == b.properties
}

fn parse_edge_key(keyspace: &str, key: &[u8]) -> Result<EdgeId> {
    std::str::from_utf8(key)
        .ok()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .ok_or_else(|| malformed_key(keyspace, key))
}

impl Storage {
    /// `true` si la adopción ya terminó en esta base.
    pub(crate) fn history_adopted(&self) -> Result<bool> {
        Ok(self.get_meta_u64_sync(META_HISTORY_ADOPTED)?.is_some_and(|f| f >= HISTORY_ADOPTION_FORMAT))
    }

    /// El F de la adopción: el persistido (reanudación) o uno nuevo de
    /// `alloc`, persistido junto con la cota del reloj ANTES de escribir
    /// nada con él (un reopen tras un crash no puede volver a repartirlo).
    fn adoption_ts(&self, cached: &mut Option<u64>, alloc: &dyn Fn() -> u64) -> Result<u64> {
        if let Some(f) = *cached {
            return Ok(f);
        }
        let f = match self.get_meta_u64_sync(META_HISTORY_ADOPTION_TS)? {
            Some(f) => f,
            None => {
                let f = alloc();
                self.bump_clock(META_NEXT_TIMESTAMP, f.saturating_add(1))?;
                self.bump_clock(META_HISTORY_ADOPTION_TS, f)?;
                f
            }
        };
        *cached = Some(f);
        Ok(f)
    }

    /// La pasada de adopción completa (ver la cabecera del módulo). `alloc`
    /// reparte el timestamp F del reloj del grafo; solo se llama si hay algo
    /// que adoptar y la base no tenía ya un F persistido.
    pub(crate) fn adopt_unversioned_history(&self, alloc: &dyn Fn() -> u64) -> Result<AdoptionStats> {
        let mut stats = AdoptionStats::default();
        let mut f_cached: Option<u64> = None;

        self.adopt_nodes(alloc, &mut f_cached, &mut stats)?;
        self.close_orphan_node_chains(alloc, &mut f_cached, &mut stats)?;
        self.adopt_edges(alloc, &mut f_cached, &mut stats)?;
        self.close_orphan_edge_versions(alloc, &mut f_cached, &mut stats)?;

        // Lo adoptado, durable antes que la marca: un crash no puede dejar
        // la marca puesta con lotes perdidos. Sin nada adoptado (toda base
        // creada por 0.10.1) no hay qué hacer durable: el fsync costaría lo
        // mismo que el open de una base nueva. La marca en sí no se fuerza a
        // disco: perderla en un crash solo repite una pasada idempotente.
        if f_cached.is_some() {
            self.engine.flush()?;
        }
        self.bump_clock(META_HISTORY_ADOPTED, HISTORY_ADOPTION_FORMAT)?;
        stats.floor_ts = f_cached;
        Ok(stats)
    }

    /// Fase 1: registros de nodo sin cadena, o con cadena desincronizada.
    fn adopt_nodes(
        &self,
        alloc: &dyn Fn() -> u64,
        f_cached: &mut Option<u64>,
        stats: &mut AdoptionStats,
    ) -> Result<()> {
        let mut cursor = None;
        loop {
            let chunk = next_chunk(self.entities_ks.as_ref(), &[v2::ENTITY_TAG], &cursor)?;
            let Some((last, _)) = chunk.last() else { break };
            cursor = Some(last.clone());

            let mut batches = NodeWriteBatches::default();
            for (key, value) in &chunk {
                let id = v2::parse_node_key_v2(key).ok_or_else(|| malformed_key(ENTITIES_TREE, key))?;
                if !self.history_ks.contains_key(&v2::history_current_key(id))? {
                    // Sin cadena: solo el suelo, sin copiar el registro. Con
                    // marca ya puesta (intento anterior, o un bulk de
                    // 0.10.1) no hay nada que hacer.
                    if self.node_stamp(id)?.is_none() {
                        let f = self.adoption_ts(f_cached, alloc)?;
                        batches.history.insert(v2::history_node_floor_key(id), f.to_be_bytes().to_vec());
                        stats.nodes_floored += 1;
                    }
                    continue;
                }
                let node: Node = deserialize(value)?;
                if self.node_chain_matches(id, &node)? {
                    continue;
                }
                let f = self.adoption_ts(f_cached, alloc)?;
                match self.plan_node_version(&node, f)? {
                    // Ya adoptado en un intento anterior (o con versión
                    // posterior a F): nada que hacer.
                    NodeVersionPlan::Superseded => {}
                    NodeVersionPlan::Write(write) => {
                        self.stage_node_history(&write, &mut batches)?;
                        batches.history.insert(v2::history_node_floor_key(id), f.to_be_bytes().to_vec());
                        stats.nodes_versioned += 1;
                    }
                }
            }
            self.engine.apply_multi(batches.into_multi())?;
        }
        Ok(())
    }

    /// `true` si la cadena de `id` termina en una versión ABIERTA con los
    /// mismos datos que el registro: nada que adoptar.
    fn node_chain_matches(&self, id: NodeId, node: &Node) -> Result<bool> {
        let Some(raw) = self.history_ks.get(&v2::history_current_key(id))? else {
            return Ok(false);
        };
        let Ok(bytes) = <[u8; 8]>::try_from(raw.as_slice()) else {
            return Ok(false);
        };
        let version = u64::from_le_bytes(bytes);
        let Some(value) = self.history_ks.get(&v2::history_version_key(id, version))? else {
            return Ok(false);
        };
        let current: VersionedNode = deserialize(&value)?;
        Ok(current.valid_to.is_none() && same_node(&current.node_data, node))
    }

    /// Fase 2: cadenas abiertas de nodos que ya no existen.
    fn close_orphan_node_chains(
        &self,
        alloc: &dyn Fn() -> u64,
        f_cached: &mut Option<u64>,
        stats: &mut AdoptionStats,
    ) -> Result<()> {
        let mut cursor = None;
        loop {
            let chunk = next_chunk(self.history_ks.as_ref(), &[v2::HISTORY_CURRENT_TAG], &cursor)?;
            let Some((last, _)) = chunk.last() else { break };
            cursor = Some(last.clone());

            let mut history = kv::WriteBatch::default();
            for (key, raw) in &chunk {
                let id = v2::parse_history_current_key(key).ok_or_else(|| malformed_key(HISTORY_TREE, key))?;
                if self.entities_ks.contains_key(&v2::node_key_v2(id))? {
                    continue;
                }
                let Ok(bytes) = <[u8; 8]>::try_from(raw.as_slice()) else { continue };
                let version = u64::from_le_bytes(bytes);
                let Some(value) = self.history_ks.get(&v2::history_version_key(id, version))? else {
                    continue;
                };
                let mut current: VersionedNode = deserialize(&value)?;
                if current.valid_to.is_some() {
                    continue;
                }
                let f = self.adoption_ts(f_cached, alloc)?;
                current.invalidate(f.max(current.valid_from));
                history.insert(v2::history_version_key(id, version), serialize(&current)?);
                history.insert(v2::history_node_floor_key(id), f.to_be_bytes().to_vec());
                stats.node_chains_closed += 1;
            }
            self.engine.apply_multi(vec![(HISTORY_TREE.to_string(), history)])?;
        }
        Ok(())
    }

    /// Fase 3: registros de arista sin versión, o con la vigente distinta.
    fn adopt_edges(
        &self,
        alloc: &dyn Fn() -> u64,
        f_cached: &mut Option<u64>,
        stats: &mut AdoptionStats,
    ) -> Result<()> {
        let mut cursor = None;
        loop {
            let chunk = next_chunk(self.edges_ks.as_ref(), &[], &cursor)?;
            let Some((last, _)) = chunk.last() else { break };
            cursor = Some(last.clone());

            let mut versions = kv::WriteBatch::default();
            let mut currents = kv::WriteBatch::default();
            let mut history = kv::WriteBatch::default();
            // Una arista se adopta una vez por tramo (las claves son únicas,
            // pero el plan lee el disco: dos versiones en el mismo lote
            // leerían la misma cadena vieja).
            let mut seen: HashSet<EdgeId> = HashSet::with_capacity(chunk.len());
            for (key, value) in &chunk {
                let id = parse_edge_key(super::EDGES_TREE, key)?;
                if !seen.insert(id) {
                    continue;
                }
                let edge: Edge = deserialize(value)?;
                if let Some(raw) = self.versioned_edges_current_ks.get(key)? {
                    let current: VersionedEdge = deserialize(&raw)?;
                    if same_edge(&current.edge_data, &edge) {
                        continue;
                    }
                } else if !self.versioned_edges_ks.contains_key(keys::edge_version_key(id, 1).as_bytes())? {
                    // Sin versiones: solo el suelo (ver la fase 1).
                    if self.edge_stamp(id)?.is_none() {
                        let f = self.adoption_ts(f_cached, alloc)?;
                        history.insert(v2::history_edge_floor_key(id), f.to_be_bytes().to_vec());
                        stats.edges_floored += 1;
                    }
                    continue;
                }
                let f = self.adoption_ts(f_cached, alloc)?;
                match self.plan_edge_version(&edge, f)? {
                    EdgeVersionPlan::Superseded => {}
                    EdgeVersionPlan::Write(write) => {
                        Self::stage_edge_version(&write, &mut versions, &mut currents, &mut history)?;
                        history.insert(v2::history_edge_floor_key(id), f.to_be_bytes().to_vec());
                        stats.edges_versioned += 1;
                    }
                }
            }
            self.engine.apply_multi(vec![
                (VERSIONED_EDGES_TREE.to_string(), versions),
                (VERSIONED_EDGES_CURRENT_TREE.to_string(), currents),
                (HISTORY_TREE.to_string(), history),
            ])?;
        }
        Ok(())
    }

    /// Fase 4: versiones vigentes de aristas que ya no existen.
    fn close_orphan_edge_versions(
        &self,
        alloc: &dyn Fn() -> u64,
        f_cached: &mut Option<u64>,
        stats: &mut AdoptionStats,
    ) -> Result<()> {
        let mut cursor = None;
        loop {
            let chunk = next_chunk(self.versioned_edges_current_ks.as_ref(), &[], &cursor)?;
            let Some((last, _)) = chunk.last() else { break };
            cursor = Some(last.clone());

            let mut versions = kv::WriteBatch::default();
            let mut currents = kv::WriteBatch::default();
            let mut history = kv::WriteBatch::default();
            for (key, raw) in &chunk {
                if self.edges_ks.contains_key(key)? {
                    continue;
                }
                let id = parse_edge_key(VERSIONED_EDGES_CURRENT_TREE, key)?;
                let current: VersionedEdge = deserialize(raw)?;
                let f = self.adoption_ts(f_cached, alloc)?;
                let closed = current.with_valid_to(f);
                versions.insert(keys::edge_version_key(id, closed.version).as_bytes(), serialize(&closed)?);
                currents.remove(key.clone());
                history.insert(v2::history_edge_floor_key(id), f.to_be_bytes().to_vec());
                stats.edge_versions_closed += 1;
            }
            self.engine.apply_multi(vec![
                (VERSIONED_EDGES_TREE.to_string(), versions),
                (VERSIONED_EDGES_CURRENT_TREE.to_string(), currents),
                (HISTORY_TREE.to_string(), history),
            ])?;
        }
        Ok(())
    }
}
