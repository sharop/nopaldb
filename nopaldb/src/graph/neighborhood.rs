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
//!
//! **Ranking (0.6.10, #176).** Con [`Rank::Bfs`] (default), cuando
//! `max_nodes` corta, lo que sobrevive depende del orden del BFS, no de la
//! relevancia. Con [`Rank::Ppr`] el BFS junta hasta `candidate_factor ×
//! max_nodes` candidatos, corre un Personalized PageRank sembrado en las
//! semillas sobre ESE subgrafo (en RAM, sin volver a leer storage) y se queda
//! con los mejores por score. Un nodo a dos saltos conectado con varias
//! semillas puede así superar a uno a un salto conectado con una sola.
//!
//! Por qué no se reutiliza `PageRank::personalized_cpu`
//! (`algorithms/pagerank.rs`): vive tras la feature `algorithms` y
//! `neighborhood` es API base; pierde la masa de los nodos sin salida (no la
//! devuelve a las semillas); y suma en el orden de las aristas, así que dos
//! grafos iguales con aristas insertadas en otro orden podían empatar
//! distinto. El de aquí indexa por `NodeId` ordenado, suma en ese orden y
//! devuelve la masa colgante a las semillas.

use std::collections::{HashMap, HashSet};

use super::{Direction, Graph};
use crate::error::Result;
use crate::types::{Edge, EdgeId, Node, NodeId};

/// Cómo decidir qué nodos sobreviven cuando el vecindario no cabe en
/// `max_nodes` (#176).
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Rank {
    /// Orden del BFS: semillas, luego por nivel. Lo que sobrevive a un corte
    /// depende del orden de adyacencia.
    #[default]
    Bfs,
    /// Personalized PageRank sembrado en las semillas, sobre el subgrafo de
    /// candidatos del BFS.
    Ppr {
        /// Probabilidad de volver a las semillas en cada paso (0.15 habitual).
        alpha: f64,
        /// Iteraciones de potencia (20 basta para ordenar un vecindario).
        iterations: usize,
        /// El BFS junta hasta `candidate_factor × max_nodes` candidatos.
        /// Si ese BFS también se corta, el conjunto de candidatos depende
        /// del orden de adyacencia; el ranking dentro de él, no.
        candidate_factor: usize,
        /// Peso de cada semilla en el teletransporte (p. ej. el score del hit
        /// de la búsqueda). `None` o semillas ausentes = peso 1.
        seed_weights: Option<HashMap<NodeId, f64>>,
    },
}

impl Rank {
    /// `Ppr` con los valores habituales: `alpha = 0.15`, 20 iteraciones,
    /// `candidate_factor = 5`, semillas con el mismo peso.
    pub fn ppr() -> Self {
        Rank::Ppr { alpha: 0.15, iterations: 20, candidate_factor: 5, seed_weights: None }
    }
}

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
    /// Qué sobrevive a un corte por `max_nodes` (#176). Default: BFS.
    pub rank: Rank,
}

impl Default for ExpandOptions {
    fn default() -> Self {
        Self {
            direction: Direction::Outgoing,
            edge_types: None,
            labels: None,
            max_nodes: 1000,
            max_edges_per_node: None,
            rank: Rank::Bfs,
        }
    }
}

/// Lo que devuelve [`Graph::neighborhood`].
#[derive(Debug, Default, Clone)]
pub struct Neighborhood {
    /// Semillas primero (profundidad 0); después, por nivel con
    /// [`Rank::Bfs`] o por score descendente con [`Rank::Ppr`].
    pub nodes: Vec<Node>,
    /// Aristas recorridas cuyos DOS extremos están en `nodes`, cada una una vez.
    pub edges: Vec<Edge>,
    /// Profundidad mínima a la que se alcanzó cada nodo.
    pub depth_of: HashMap<NodeId, usize>,
    /// Score PPR de cada nodo devuelto; vacío con [`Rank::Bfs`].
    pub score_of: HashMap<NodeId, f64>,
    /// `true` si se alcanzó `max_nodes` y quedó vecindad sin recorrer (o,
    /// con PPR, si el ranking dejó fuera candidatos).
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
        let Rank::Ppr { alpha, iterations, candidate_factor, seed_weights } = &opts.rank else {
            return self.expand_bfs(seeds, depth, opts).await;
        };
        // 1. Candidatos: el mismo BFS con un tope mayor.
        let mut candidate_opts = opts.clone();
        candidate_opts.rank = Rank::Bfs;
        candidate_opts.max_nodes = opts.max_nodes.saturating_mul((*candidate_factor).max(1));
        let mut nb = self.expand_bfs(seeds, depth, &candidate_opts).await?;
        // 2. PPR en RAM sobre los candidatos.
        let seed_ids: Vec<NodeId> = nb.nodes.iter().filter(|n| nb.depth_of.get(&n.id) == Some(&0)).map(|n| n.id).collect();
        let scores = personalized_pagerank(&nb, &seed_ids, opts.direction, *alpha, *iterations, seed_weights.as_ref());
        // 3. Semillas primero; el resto por score descendente (desempate por
        //    `NodeId`); recorte a `max_nodes`.
        let mut rest: Vec<Node> = Vec::new();
        let mut kept: Vec<Node> = Vec::new();
        for node in std::mem::take(&mut nb.nodes) {
            if nb.depth_of.get(&node.id) == Some(&0) {
                kept.push(node);
            } else {
                rest.push(node);
            }
        }
        rest.sort_by(|a, b| scores[&b.id].total_cmp(&scores[&a.id]).then_with(|| a.id.cmp(&b.id)));
        let room = opts.max_nodes.saturating_sub(kept.len());
        if rest.len() > room {
            nb.truncated = true;
            rest.truncate(room);
        }
        kept.extend(rest);
        let keep: HashSet<NodeId> = kept.iter().map(|n| n.id).collect();
        nb.edges.retain(|e| keep.contains(&e.source) && keep.contains(&e.target));
        nb.depth_of.retain(|id, _| keep.contains(id));
        nb.score_of = kept.iter().map(|n| (n.id, scores[&n.id])).collect();
        nb.nodes = kept;
        Ok(nb)
    }

    /// El BFS acotado de [`Graph::neighborhood`] (orden de nivel, corte por
    /// `max_nodes`).
    async fn expand_bfs(
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

/// Personalized PageRank sobre los nodos y aristas de `nb` (ver el doc del
/// módulo). Determinista: los nodos se indexan por `NodeId` ordenado, las
/// aristas se suman en ese orden y una arista repetida cuenta con su
/// multiplicidad. La caminata sigue la dirección de la expansión (`Both` =
/// no dirigida). La masa de un nodo sin salida vuelve a las semillas, igual
/// que el teletransporte, así que la suma se conserva.
fn personalized_pagerank(
    nb: &Neighborhood,
    seeds: &[NodeId],
    direction: Direction,
    alpha: f64,
    iterations: usize,
    seed_weights: Option<&HashMap<NodeId, f64>>,
) -> HashMap<NodeId, f64> {
    let mut ids: Vec<NodeId> = nb.nodes.iter().map(|n| n.id).collect();
    ids.sort();
    let index: HashMap<NodeId, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let n = ids.len();
    if n == 0 {
        return HashMap::new();
    }
    // Vecinos de salida de la caminata, ordenados (con multiplicidad).
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    for edge in &nb.edges {
        let (Some(&s), Some(&t)) = (index.get(&edge.source), index.get(&edge.target)) else { continue };
        match direction {
            Direction::Outgoing => out[s].push(t),
            Direction::Incoming => out[t].push(s),
            Direction::Both => {
                out[s].push(t);
                if s != t {
                    out[t].push(s);
                }
            }
        }
    }
    for list in &mut out {
        list.sort_unstable();
    }
    // Vector de teletransporte: las semillas, con su peso.
    let mut teleport = vec![0.0f64; n];
    for seed in seeds {
        if let Some(&i) = index.get(seed) {
            let w = seed_weights.and_then(|m| m.get(seed)).copied().unwrap_or(1.0).max(0.0);
            teleport[i] += w;
        }
    }
    let total: f64 = teleport.iter().sum();
    if total <= 0.0 {
        return ids.into_iter().map(|id| (id, 0.0)).collect();
    }
    teleport.iter_mut().for_each(|w| *w /= total);

    let alpha = alpha.clamp(0.0, 1.0);
    let mut rank = teleport.clone();
    for _ in 0..iterations {
        let mut next = vec![0.0f64; n];
        let mut dangling = 0.0;
        for (i, targets) in out.iter().enumerate() {
            if targets.is_empty() {
                dangling += rank[i];
                continue;
            }
            let share = rank[i] / targets.len() as f64;
            for &t in targets {
                next[t] += share;
            }
        }
        for i in 0..n {
            next[i] = alpha * teleport[i] + (1.0 - alpha) * (next[i] + dangling * teleport[i]);
        }
        rank = next;
    }
    ids.into_iter().zip(rank).collect()
}
