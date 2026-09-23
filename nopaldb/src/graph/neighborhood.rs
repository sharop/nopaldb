//! Vecindario de un conjunto de nodos en una llamada (#GraphRAG, 0.6.8).
//!
//! Un GraphRAG hace lo mismo en cada pregunta: buscar (KNN / híbrida), tomar
//! los hits y expandir su vecindad a 1 o 2 saltos para armar el contexto.
//! Hasta 0.6.7 desde Python solo existía NQL para ese segundo paso, y NQL
//! resolvía `where n.id = "…"` con un scan completo y enumeraba caminos en
//! el k-hop: 590 ms por hit a 1 salto y 1.7 s a 2 saltos con 100k nodos.
//!
//! [`Graph::neighborhood`] es un BFS por nodo con visitados global (un nodo
//! alcanzable por m caminos entra una vez, a su profundidad mínima), lee los
//! ids de arista de la adyacencia en RAM, filtra por tipo ANTES de leer el
//! nodo destino, corta por `max_nodes` y por aristas por nodo (supernodos), y
//! devuelve nodos y aristas con sus propiedades. La adyacencia RAM guarda solo
//! ids de arista, así que cada arista se lee del storage una vez; la
//! adyacencia tipada (0.6.9) sustituirá [`Graph::adjacent_edge_ids`] y ese
//! coste desaparecerá sin tocar este algoritmo.

use std::collections::{HashMap, HashSet};

use super::{Direction, Graph};
use crate::error::Result;
use crate::types::{Edge, EdgeId, Node, NodeId};

/// Cómo expandir. `Default`: salientes, sin filtros, 1000 nodos como máximo.
#[derive(Debug, Clone)]
pub struct ExpandOptions {
    pub direction: Direction,
    /// Solo aristas de estos tipos; `None` = todas.
    pub edge_types: Option<Vec<String>>,
    /// Solo nodos con estas etiquetas; un nodo filtrado no entra ni expande.
    /// Las semillas no se filtran.
    pub labels: Option<Vec<String>>,
    /// Tope de nodos en el resultado, semillas incluidas. Al alcanzarlo la
    /// expansión para y `truncated` queda en `true`.
    pub max_nodes: usize,
    /// Tope de aristas consideradas por nodo (las primeras en orden de
    /// adyacencia). Es el freno real ante un supernodo: `max_nodes` corta
    /// nodos, no aristas leídas.
    pub max_edges_per_node: Option<usize>,
}

impl Default for ExpandOptions {
    fn default() -> Self {
        Self {
            direction: Direction::Outgoing,
            edge_types: None,
            labels: None,
            max_nodes: 1000,
            max_edges_per_node: None,
        }
    }
}

/// Lo que devuelve [`Graph::neighborhood`].
#[derive(Debug, Default, Clone)]
pub struct Neighborhood {
    /// Semillas primero (profundidad 0), luego por nivel.
    pub nodes: Vec<Node>,
    /// Aristas recorridas cuyos DOS extremos están en `nodes`, cada una una vez.
    pub edges: Vec<Edge>,
    /// Profundidad mínima a la que se alcanzó cada nodo.
    pub depth_of: HashMap<NodeId, usize>,
    /// `true` si se alcanzó `max_nodes` y quedó vecindad sin recorrer.
    pub truncated: bool,
}

impl Graph {
    /// Ids de arista incidentes a `id` desde la adyacencia RAM, sin tocar
    /// storage. `Both` = salientes ∪ entrantes con dedup por id (un self-loop
    /// está en las dos listas). `cap` corta supernodos: los primeros k ids.
    pub(crate) async fn adjacent_edge_ids(
        &self,
        id: NodeId,
        direction: Direction,
        cap: Option<usize>,
    ) -> Vec<EdgeId> {
        let mut ids = match direction {
            Direction::Outgoing => self.adjacency_out.read().await.get(&id).cloned().unwrap_or_default(),
            Direction::Incoming => self.adjacency_in.read().await.get(&id).cloned().unwrap_or_default(),
            Direction::Both => {
                let out = self.adjacency_out.read().await;
                let inn = self.adjacency_in.read().await;
                let mut v = out.get(&id).cloned().unwrap_or_default();
                let seen: HashSet<EdgeId> = v.iter().copied().collect();
                if let Some(entrantes) = inn.get(&id) {
                    v.extend(entrantes.iter().copied().filter(|e| !seen.contains(e)));
                }
                v
            }
        };
        if let Some(k) = cap {
            ids.truncate(k);
        }
        ids
    }

    /// Vecindario de `seeds` hasta `depth` saltos. Ver el doc del módulo.
    ///
    /// Semillas inexistentes se ignoran; repetidas cuentan una vez. `depth`
    /// 0 devuelve solo las semillas.
    pub async fn neighborhood(
        &self,
        seeds: &[NodeId],
        depth: usize,
        opts: &ExpandOptions,
    ) -> Result<Neighborhood> {
        let mut nb = Neighborhood::default();
        let mut seen_edges: HashSet<EdgeId> = HashSet::new();
        let mut frontier: Vec<NodeId> = Vec::new();

        for node in self.get_nodes(seeds).await?.into_iter().flatten() {
            if nb.depth_of.contains_key(&node.id) {
                continue;
            }
            if nb.nodes.len() >= opts.max_nodes {
                nb.truncated = true;
                break;
            }
            nb.depth_of.insert(node.id, 0);
            frontier.push(node.id);
            nb.nodes.push(node);
        }

        'levels: for level in 1..=depth {
            if frontier.is_empty() || nb.truncated {
                break;
            }
            let mut next: Vec<NodeId> = Vec::new();
            for src in std::mem::take(&mut frontier) {
                let edge_ids = self.adjacent_edge_ids(src, opts.direction, opts.max_edges_per_node).await;
                // 1. Aristas, filtradas por tipo ANTES de leer ningún nodo.
                let mut pending: Vec<(Edge, NodeId)> = Vec::new();
                for edge in self.get_edges(&edge_ids).await?.into_iter().flatten() {
                    if let Some(types) = &opts.edge_types
                        && !types.contains(&edge.edge_type)
                    {
                        continue;
                    }
                    let other = if edge.source == src { edge.target } else { edge.source };
                    pending.push((edge, other));
                }
                // 2. Nodos destino no vistos, en una pasada.
                let mut want: Vec<NodeId> = Vec::new();
                let mut dedup: HashSet<NodeId> = HashSet::new();
                for (_, other) in &pending {
                    if !nb.depth_of.contains_key(other) && dedup.insert(*other) {
                        want.push(*other);
                    }
                }
                for node in self.get_nodes(&want).await?.into_iter().flatten() {
                    if let Some(labels) = &opts.labels
                        && !labels.contains(&node.label)
                    {
                        continue;
                    }
                    if nb.nodes.len() >= opts.max_nodes {
                        nb.truncated = true;
                        break;
                    }
                    nb.depth_of.insert(node.id, level);
                    next.push(node.id);
                    nb.nodes.push(node);
                }
                // 3. Aristas con ambos extremos en el resultado, una vez.
                for (edge, other) in pending {
                    if nb.depth_of.contains_key(&other) && seen_edges.insert(edge.id) {
                        nb.edges.push(edge);
                    }
                }
                if nb.truncated {
                    break 'levels;
                }
            }
            frontier = next;
        }
        Ok(nb)
    }
}
