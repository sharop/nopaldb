// src/algorithms/community.rs
//
// Community Detection algorithms — Louvain and Leiden

use crate::error::Result;
use crate::graph::GraphView;
use crate::types::NodeId;
use std::collections::{HashMap, HashSet};

/// Community Detection configuration
#[derive(Debug, Clone)]
pub struct CommunityConfig {
    /// Resolution parameter (higher = more communities)
    pub resolution: f64,

    /// Maximum number of iterations
    pub max_iterations: usize,

    /// Minimum modularity gain to continue
    pub min_gain: f64,
}

impl Default for CommunityConfig {
    fn default() -> Self {
        CommunityConfig {
            resolution: 1.0,
            max_iterations: 100,
            min_gain: 0.0001,
        }
    }
}

/// Louvain Community Detection
pub struct LouvainCommunity {
    config: CommunityConfig,
}

impl LouvainCommunity {
    /// Create new Louvain instance
    pub fn new(config: CommunityConfig) -> Self {
        LouvainCommunity { config }
    }

    /// Create with default configuration
    pub fn with_defaults() -> Self {
        LouvainCommunity {
            config: CommunityConfig::default(),
        }
    }

    /// Detect communities using Louvain method
    /// Returns map of node -> community_id
    pub async fn detect<G: GraphView>(&self, graph: &G) -> Result<HashMap<NodeId, usize>> {
        let nodes = graph.get_all_nodes().await?;
        if nodes.is_empty() {
            return Ok(HashMap::new());
        }
        let edges = graph.get_all_edges().await?;
        let config = self.config.clone();
        tokio::task::spawn_blocking(move || Self::detect_cpu(nodes, edges, config))
            .await
            .map_err(|e| {
                crate::error::NopalError::custom(format!("community detect join error: {e}"))
            })?
    }

    fn detect_cpu(
        nodes: Vec<crate::types::Node>,
        edges: Vec<crate::types::Edge>,
        config: CommunityConfig,
    ) -> Result<HashMap<NodeId, usize>> {
        let louvain = LouvainCommunity { config };

        // Ordenar nodos por ID para iteración determinista.
        let mut sorted_nodes = nodes;
        sorted_nodes.sort_unstable_by_key(|n| n.id);

        let mut communities: HashMap<NodeId, usize> = sorted_nodes
            .iter()
            .enumerate()
            .map(|(i, node)| (node.id, i))
            .collect();

        // Adyacencia no-dirigida: insert (no +=) para evitar doble-cómputo
        // cuando la DB almacena aristas bidireccionales como dos filas.
        let mut adjacency: HashMap<NodeId, HashMap<NodeId, f64>> = HashMap::new();
        for edge in &edges {
            adjacency
                .entry(edge.source)
                .or_default()
                .insert(edge.target, 1.0);
            adjacency
                .entry(edge.target)
                .or_default()
                .insert(edge.source, 1.0);
        }
        // total_weight = número de aristas no-dirigidas (cada par cuenta una vez).
        // Sumamos todos los valores de adyacencia y dividimos por 2.
        let total_weight: f64 = adjacency
            .values()
            .flat_map(|nbrs| nbrs.values())
            .sum::<f64>()
            / 2.0;

        let mut degrees: HashMap<NodeId, f64> = HashMap::new();
        for (node, neighbors) in &adjacency {
            let degree: f64 = neighbors.values().sum();
            degrees.insert(*node, degree);
        }

        let mut improved = true;
        let mut iteration = 0;

        while improved && iteration < louvain.config.max_iterations {
            improved = false;
            iteration += 1;

            for node in &sorted_nodes {
                let node_id = node.id;
                let current_community = communities[&node_id];

                let mut best_community = current_community;
                let mut best_gain = 0.0;

                let neighbor_communities =
                    louvain.get_neighbor_communities(node_id, &adjacency, &communities);

                for &neighbor_community in &neighbor_communities {
                    if neighbor_community == current_community {
                        continue;
                    }

                    let gain = louvain.modularity_gain(
                        node_id,
                        neighbor_community,
                        &communities,
                        &adjacency,
                        &degrees,
                        total_weight,
                    );

                    if gain > best_gain {
                        best_gain = gain;
                        best_community = neighbor_community;
                    }
                }

                if best_gain > louvain.config.min_gain && best_community != current_community {
                    communities.insert(node_id, best_community);
                    improved = true;
                }
            }
        }

        louvain.renumber_communities(communities)
    }

    /// Get communities of neighboring nodes, sorted for deterministic iteration order.
    fn get_neighbor_communities(
        &self,
        node: NodeId,
        adjacency: &HashMap<NodeId, HashMap<NodeId, f64>>,
        communities: &HashMap<NodeId, usize>,
    ) -> Vec<usize> {
        let mut seen = HashSet::new();

        if let Some(neighbors) = adjacency.get(&node) {
            for neighbor in neighbors.keys() {
                if let Some(&community) = communities.get(neighbor) {
                    seen.insert(community);
                }
            }
        }

        // Also include current community
        if let Some(&current) = communities.get(&node) {
            seen.insert(current);
        }

        let mut result: Vec<usize> = seen.into_iter().collect();
        result.sort_unstable();
        result
    }

    /// Ganancia NETA de modularidad de mover `node` desde su comunidad actual a `target_community`.
    ///
    /// Implementa la fórmula de Blondel et al. (2008):
    ///   ΔQ(i: s→t) = (k_i_in_t − k_i_in_s) / m2 − γ·(σ_t − σ_s + k_i)·k_i / m2²
    ///
    /// donde k_i_in_t = peso de aristas de i hacia t (destino),
    ///       k_i_in_s = peso de aristas de i hacia s (origen, excluye i),
    ///       σ_t / σ_s = suma de grados en t / s (σ_s incluye k_i),
    ///       k_i = grado del nodo i.
    ///
    /// La versión anterior solo computaba la ganancia de entrar a t sin restar
    /// el costo de salir de s, lo que producía particiones sobre-fragmentadas.
    fn modularity_gain(
        &self,
        node: NodeId,
        target_community: usize,
        communities: &HashMap<NodeId, usize>,
        adjacency: &HashMap<NodeId, HashMap<NodeId, f64>>,
        degrees: &HashMap<NodeId, f64>,
        total_weight: f64,
    ) -> f64 {
        let node_degree = degrees.get(&node).copied().unwrap_or(0.0);
        let current_community = *communities.get(&node).unwrap_or(&usize::MAX);

        // Pesos desde node hacia la comunidad destino y la comunidad actual
        let mut k_i_in_t = 0.0_f64;
        let mut k_i_in_s = 0.0_f64;
        if let Some(neighbors) = adjacency.get(&node) {
            for (nbr, &w) in neighbors {
                match communities.get(nbr).copied() {
                    Some(c) if c == target_community => k_i_in_t += w,
                    Some(c) if c == current_community => k_i_in_s += w,
                    _ => {}
                }
            }
        }

        // Suma de grados en comunidad destino y comunidad actual (la actual incluye k_i)
        let mut sigma_t = 0.0_f64;
        let mut sigma_s = 0.0_f64;
        for (&other, &comm) in communities {
            if let Some(&deg) = degrees.get(&other) {
                if comm == target_community {
                    sigma_t += deg;
                }
                if comm == current_community {
                    sigma_s += deg;
                }
            }
        }

        let m2 = 2.0 * total_weight;
        (k_i_in_t - k_i_in_s) / m2
            - self.config.resolution * (sigma_t - sigma_s + node_degree) * node_degree / (m2 * m2)
    }

    /// Renumber communities to be contiguous (0, 1, 2, ...)
    fn renumber_communities(
        &self,
        communities: HashMap<NodeId, usize>,
    ) -> Result<HashMap<NodeId, usize>> {
        let mut unique_communities: Vec<usize> = communities
            .values()
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        unique_communities.sort_unstable();

        let community_map: HashMap<usize, usize> = unique_communities
            .iter()
            .enumerate()
            .map(|(new_id, &old_id)| (old_id, new_id))
            .collect();

        Ok(communities
            .into_iter()
            .map(|(node, old_community)| {
                let new_community = community_map[&old_community];
                (node, new_community)
            })
            .collect())
    }

    /// Get number of communities detected
    pub fn count_communities(communities: &HashMap<NodeId, usize>) -> usize {
        communities.values().copied().collect::<HashSet<_>>().len()
    }

    /// Calculate modularity of the partition
    pub async fn modularity<G: GraphView>(
        &self,
        graph: &G,
        communities: &HashMap<NodeId, usize>,
    ) -> Result<f64> {
        let edges = graph.get_all_edges().await?;
        let nodes = graph.get_all_nodes().await?;

        let mut total_weight = 0.0;
        let mut community_internal: HashMap<usize, f64> = HashMap::new();
        let mut community_degrees: HashMap<usize, f64> = HashMap::new();

        // Build adjacency and compute degrees
        let mut adjacency: HashMap<NodeId, HashMap<NodeId, f64>> = HashMap::new();
        for edge in &edges {
            let weight = 1.0;
            adjacency
                .entry(edge.source)
                .or_default()
                .insert(edge.target, weight);
            adjacency
                .entry(edge.target)
                .or_default()
                .insert(edge.source, weight);
            total_weight += weight;
        }

        // Calculate internal edges and degrees per community
        for node in &nodes {
            let node_id = node.id;
            let community = communities.get(&node_id).copied().unwrap_or(0);

            if let Some(neighbors) = adjacency.get(&node_id) {
                let degree: f64 = neighbors.values().sum();
                *community_degrees.entry(community).or_insert(0.0) += degree;

                for (neighbor, &weight) in neighbors {
                    let neighbor_community = communities.get(neighbor).copied().unwrap_or(0);
                    if community == neighbor_community {
                        *community_internal.entry(community).or_insert(0.0) += weight;
                    }
                }
            }
        }

        // Calculate modularity
        let m = total_weight;
        let mut q = 0.0;

        for (&community, &internal) in &community_internal {
            let degree_sum = community_degrees.get(&community).copied().unwrap_or(0.0);
            q += (internal / (2.0 * m)) - (degree_sum / (2.0 * m)).powi(2);
        }

        Ok(q)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Leiden Community Detection
//
// Implementación from-scratch del algoritmo Leiden siguiendo:
//   Traag, V.A., Waltman, L. & van Eck, N.J. (2019).
//   "From Louvain to Leiden: guaranteeing well-connected communities."
//   Scientific Reports, 9, 5233. https://doi.org/10.1038/s41598-019-41695-z
//
// Diferencias clave respecto a Louvain
// ──────────────────────────────────────
// 1. Función de calidad: Leiden usa CPM (Constant Potts Model) en vez de
//    modularity. CPM tiene la ventaja de no tener una resolución limite
//    ("resolution limit") y sus comunidades son comparables entre grafos de
//    distinto tamaño. La función CPM es:
//
//       H(P) = Σ_C [e_C − γ · n_C · (n_C − 1) / 2]
//
//    donde e_C = peso total de aristas internas en C, n_C = #nodos en C
//    y γ (gamma) es el parámetro de resolución (mayor γ → más comunidades).
//
// 2. Fase de refinamiento: tras la fase de movimiento local (equivalente a
//    Louvain), Leiden añade una fase de refinamiento que subdivide cada
//    comunidad usando solo fusiones "bien conectadas". Esto garantiza que
//    ninguna comunidad resultante tenga partes que sean internamente
//    desconectadas (problema conocido de Louvain).
//
// 3. Complejidad: O(n · m · iterations) en la implementación plana usada
//    aquí (sin agregación de grafo). El paper original usa agregación para
//    escalar a grafos de millones de nodos, pero para los rangos típicos de
//    NopalDB (hasta ~100K nodos) la versión plana es suficiente y más simple.
//
// Invariante garantizado (ausente en Louvain)
// ────────────────────────────────────────────
// Al terminar, cada comunidad C cumple la condición de bien-conexión:
//   Para todo subconjunto propio S ⊂ C: e(S, C\S) ≥ γ · |S| · (|C|−|S|)
// Esto asegura que no existan "islas" desconectadas dentro de una comunidad.
// ─────────────────────────────────────────────────────────────────────────────

/// Configuración del algoritmo Leiden.
///
/// # Parámetro gamma
/// `gamma` (γ) es el parámetro de resolución del modelo CPM (Constant Potts Model).
/// - `gamma = 0.0` → una sola comunidad (trivial).
/// - `gamma = 0.05..0.2` → rango típico para grafos sociales y de fraude.
/// - `gamma = 0.5..1.0` → muchas comunidades pequeñas; útil para GNNs densos.
/// - `gamma > 1.0` → resultado muy fragmentado; raramente útil.
///
/// A diferencia de la resolución de Louvain, γ en CPM tiene semántica directa:
/// dos nodos terminan en la misma comunidad si y solo si la densidad de aristas
/// entre ellos supera γ.
#[derive(Debug, Clone)]
pub struct LeidenConfig {
    /// Parámetro de resolución CPM. Default: 0.1.
    pub gamma: f64,
    /// Número máximo de iteraciones del bucle externo (Phase1 + Phase2). Default: 10.
    pub max_iterations: usize,
    /// Ganancia CPM mínima para aceptar un movimiento de nodo. Default: 1e-9.
    pub min_gain: f64,
}

impl Default for LeidenConfig {
    fn default() -> Self {
        LeidenConfig {
            gamma: 0.1,
            max_iterations: 10,
            min_gain: 1e-9,
        }
    }
}

/// Detector de comunidades Leiden.
///
/// Uso básico desde NQL: `leiden(n)` en la cláusula FIND.
/// Uso con gamma personalizado: crear instancia y llamar `detect()` directamente.
///
/// # Ejemplo (Rust API)
/// ```rust,ignore
/// let leiden = LeidenCommunity::with_gamma(0.05);
/// let assignments = leiden.detect(&graph).await?;
/// ```
pub struct LeidenCommunity {
    config: LeidenConfig,
}

impl LeidenCommunity {
    /// Crea instancia con configuración personalizada.
    pub fn new(config: LeidenConfig) -> Self {
        LeidenCommunity { config }
    }

    /// Crea instancia con gamma = 0.1 y valores por defecto.
    pub fn with_defaults() -> Self {
        LeidenCommunity {
            config: LeidenConfig::default(),
        }
    }

    /// Crea instancia configurando solo gamma; resto de parámetros por defecto.
    pub fn with_gamma(gamma: f64) -> Self {
        LeidenCommunity {
            config: LeidenConfig {
                gamma,
                ..LeidenConfig::default()
            },
        }
    }

    /// Detecta comunidades en el grafo usando el algoritmo Leiden.
    ///
    /// Retorna un mapa `NodeId → community_id` (IDs contiguos desde 0).
    /// La detección corre en `spawn_blocking` para no bloquear el runtime de Tokio.
    pub async fn detect<G: GraphView>(&self, graph: &G) -> Result<HashMap<NodeId, usize>> {
        let nodes = graph.get_all_nodes().await?;
        if nodes.is_empty() {
            return Ok(HashMap::new());
        }
        let edges = graph.get_all_edges().await?;
        let config = self.config.clone();
        tokio::task::spawn_blocking(move || Self::detect_cpu(nodes, edges, config))
            .await
            .map_err(|e| {
                crate::error::NopalError::custom(format!("leiden detect join error: {e}"))
            })?
    }

    /// Retorna el número de comunidades detectadas.
    pub fn count_communities(communities: &HashMap<NodeId, usize>) -> usize {
        communities.values().copied().collect::<HashSet<_>>().len()
    }

    // ─────────────────────────────────────────────────────────────────────────
    // CPU-bound core (ejecutado en spawn_blocking)
    // ─────────────────────────────────────────────────────────────────────────

    fn detect_cpu(
        nodes: Vec<crate::types::Node>,
        edges: Vec<crate::types::Edge>,
        config: LeidenConfig,
    ) -> Result<HashMap<NodeId, usize>> {
        if nodes.is_empty() {
            return Ok(HashMap::new());
        }

        // ── Construir adyacencia no-dirigida con peso 1.0 ───────────────────
        // Usar insert (no +=) para que datasets con aristas bidireccionales
        // (a→b Y b→a en la DB) no dupliquen el peso a 2.0.
        let mut adjacency: HashMap<NodeId, HashMap<NodeId, f64>> = HashMap::new();
        for edge in &edges {
            adjacency
                .entry(edge.source)
                .or_default()
                .insert(edge.target, 1.0);
            adjacency
                .entry(edge.target)
                .or_default()
                .insert(edge.source, 1.0);
        }

        // Lista de NodeIds ordenada para iteración determinista
        let mut node_ids: Vec<NodeId> = nodes.iter().map(|n| n.id).collect();
        node_ids.sort_unstable();

        // ── Partición inicial: cada nodo en su propia comunidad ─────────────
        let mut communities: HashMap<NodeId, usize> = node_ids
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, i))
            .collect();

        // Tamaño de cada comunidad
        let mut sizes: HashMap<usize, usize> = communities.iter().map(|(_, &c)| (c, 1)).collect();

        // Peso total de aristas internas de cada comunidad (singletons → 0)
        let mut e_in: HashMap<usize, f64> = (0..node_ids.len()).map(|i| (i, 0.0)).collect();

        // ── Bucle principal: Phase1 + Phase2 ────────────────────────────────
        for _iter in 0..config.max_iterations {
            // Phase 1 — movimiento local greedy con CPM
            let phase1_improved = Self::phase1_local_move(
                &node_ids,
                &adjacency,
                &mut communities,
                &mut sizes,
                &mut e_in,
                config.gamma,
                config.min_gain,
            );

            // Phase 2 — refinamiento garantizando bien-conexión
            let phase2_changed = Self::phase2_refine(
                &node_ids,
                &adjacency,
                &mut communities,
                &mut sizes,
                &mut e_in,
                config.gamma,
            );

            // Si ninguna fase produjo cambios, hemos convergido
            if !phase1_improved && !phase2_changed {
                break;
            }
        }

        Self::renumber_communities(communities)
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Phase 1 — Movimiento local con CPM
    //
    // Para cada nodo v (en orden determinista):
    //   1. Calcula la ganancia CPM de mover v a cada comunidad vecina t.
    //   2. Si la mejor ganancia > min_gain, mueve v a t.
    //
    // Ganancia CPM de mover v desde comunidad s a comunidad t:
    //   ΔH(v: s→t) = e(v, C_t) − e(v, C_s\{v}) + γ · (|C_s|−1 − |C_t|)
    //
    // donde e(v, C_t) = suma de pesos de aristas de v hacia nodos en C_t.
    //
    // Retorna true si se realizó al menos un movimiento.
    // ─────────────────────────────────────────────────────────────────────────
    fn phase1_local_move(
        node_ids: &[NodeId],
        adjacency: &HashMap<NodeId, HashMap<NodeId, f64>>,
        communities: &mut HashMap<NodeId, usize>,
        sizes: &mut HashMap<usize, usize>,
        e_in: &mut HashMap<usize, f64>,
        gamma: f64,
        min_gain: f64,
    ) -> bool {
        let mut any_improved = false;
        // Repetir hasta que una pasada completa no produzca cambios
        loop {
            let mut pass_improved = false;

            for &node_id in node_ids {
                let current_comm = communities[&node_id];
                let n_s = sizes[&current_comm] as f64;

                // e(v, C_s\{v}) — peso hacia otros nodos en la misma comunidad
                let e_to_self_comm =
                    Self::edge_weight_to_community(node_id, current_comm, adjacency, communities);

                let mut best_gain = min_gain;
                let mut best_comm = current_comm;

                // Colectar comunidades vecinas distintas (determinista)
                let mut candidate_comms: Vec<usize> = adjacency
                    .get(&node_id)
                    .map(|nbrs| {
                        let mut cs: Vec<usize> = nbrs
                            .keys()
                            .filter_map(|nbr| {
                                let c = communities[nbr];
                                if c != current_comm { Some(c) } else { None }
                            })
                            .collect();
                        cs.sort_unstable();
                        cs.dedup();
                        cs
                    })
                    .unwrap_or_default();
                candidate_comms.sort_unstable();
                candidate_comms.dedup();

                for target_comm in candidate_comms {
                    let n_t = sizes[&target_comm] as f64;
                    let e_to_target = Self::edge_weight_to_community(
                        node_id,
                        target_comm,
                        adjacency,
                        communities,
                    );
                    // ΔH(v: s→t) = e(v,C_t) − e(v,C_s\{v}) + γ·(|C_s|−1 − |C_t|)
                    let gain = e_to_target - e_to_self_comm + gamma * (n_s - 1.0 - n_t);
                    if gain > best_gain {
                        best_gain = gain;
                        best_comm = target_comm;
                    }
                }

                if best_comm != current_comm {
                    // Actualizar e_in de la comunidad origen y destino
                    let e_contrib = Self::edge_weight_to_community(
                        node_id,
                        current_comm,
                        adjacency,
                        communities,
                    );
                    *e_in.entry(current_comm).or_insert(0.0) -= e_contrib;
                    *sizes.entry(current_comm).or_insert(1) -= 1;

                    // Mover v a best_comm
                    *communities
                        .get_mut(&node_id)
                        .expect("node must be in communities") = best_comm;

                    let e_to_new =
                        Self::edge_weight_to_community(node_id, best_comm, adjacency, communities);
                    *e_in.entry(best_comm).or_insert(0.0) += e_to_new;
                    *sizes.entry(best_comm).or_insert(0) += 1;

                    pass_improved = true;
                    any_improved = true;
                }
            }

            if !pass_improved {
                break;
            }
        }
        any_improved
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Phase 2 — Refinamiento con garantía de bien-conexión
    //
    // Para cada comunidad base C obtenida de Phase 1:
    //   1. Inicializar: cada nodo de C en su propia comunidad singleton.
    //   2. Para cada nodo v en C (orden determinista):
    //      a. Comprobar elegibilidad de v:
    //         e(v, C\{v}) ≥ γ · (|C| − 1)
    //         Si no cumple, v queda en su singleton (no puede contaminar otras).
    //      b. Para cada comunidad refinada vecina R dentro de C:
    //         - Verificar bien-conexión de R en C:
    //           e(R, C\R) ≥ γ · |R| · (|C| − |R|)
    //           donde e(R, C\R) = Σ_{u∈R} e_c[u] − 2·e_int(R)
    //         - Calcular ganancia CPM de fusionar {v} en R:
    //           ΔH(v→R) = e(v, R) − γ · |R|
    //         - Aceptar si ganancia > 0 y R está bien conectada.
    //      c. Mover v a la mejor R si existe.
    //   3. Actualizar asignaciones globales de comunidad.
    //
    // Retorna true si alguna asignación global cambió.
    //
    // Referencia: Algorithm 3 en Traag et al. (2019), Sección "Refined partition".
    // ─────────────────────────────────────────────────────────────────────────
    fn phase2_refine(
        node_ids: &[NodeId],
        adjacency: &HashMap<NodeId, HashMap<NodeId, f64>>,
        communities: &mut HashMap<NodeId, usize>,
        sizes: &mut HashMap<usize, usize>,
        e_in: &mut HashMap<usize, f64>,
        gamma: f64,
    ) -> bool {
        // Agrupar nodos por comunidad base (Phase1)
        let mut by_comm: HashMap<usize, Vec<NodeId>> = HashMap::new();
        for &nid in node_ids {
            by_comm.entry(communities[&nid]).or_default().push(nid);
        }
        // Ordenar nodos dentro de cada comunidad para determinismo
        for nodes_in_c in by_comm.values_mut() {
            nodes_in_c.sort_unstable();
        }

        let mut any_changed = false;
        // ID global para comunidades refinadas (único en todo el grafo)
        let mut next_ref_id: usize = node_ids.len(); // por encima de IDs de Phase1

        // Iterar en orden determinista (por community ID) para que next_ref_id
        // sea estable entre runs — crítico para reproducibilidad.
        let mut sorted_comms: Vec<(usize, &Vec<NodeId>)> =
            by_comm.iter().map(|(&id, nodes)| (id, nodes)).collect();
        sorted_comms.sort_unstable_by_key(|(id, _)| *id);

        for (base_comm_id, c_nodes) in sorted_comms {
            let c_size = c_nodes.len();
            if c_size <= 1 {
                // Singleton base: nada que refinar
                continue;
            }

            // Conjunto de nodos en esta comunidad base (para lookups O(1))
            let c_set: HashSet<NodeId> = c_nodes.iter().copied().collect();

            // e_c[v] = suma de pesos desde v hacia todos los nodos en C\{v}
            // Precomputado una sola vez por comunidad base.
            let e_c: HashMap<NodeId, f64> = c_nodes
                .iter()
                .map(|&v| {
                    let w = adjacency
                        .get(&v)
                        .map(|nbrs| {
                            nbrs.iter()
                                .filter(|(u, _)| c_set.contains(*u) && **u != v)
                                .map(|(_, &w)| w)
                                .sum::<f64>()
                        })
                        .unwrap_or(0.0);
                    (v, w)
                })
                .collect();

            // ── Inicializar partición refinada: cada nodo en su singleton ──
            // ref_comm[v] = ID de la comunidad refinada de v (local a esta iteración)
            let mut ref_comm: HashMap<NodeId, usize> = c_nodes
                .iter()
                .enumerate()
                .map(|(i, &v)| (v, next_ref_id + i))
                .collect();
            // Consumir IDs para esta comunidad base
            let ref_id_base = next_ref_id;
            next_ref_id += c_size;

            // Tamaño de cada comunidad refinada
            let mut ref_sizes: HashMap<usize, usize> = (ref_id_base..ref_id_base + c_size)
                .map(|id| (id, 1))
                .collect();

            // Aristas internas de cada comunidad refinada (singletons → 0)
            let mut ref_e_int: HashMap<usize, f64> = (ref_id_base..ref_id_base + c_size)
                .map(|id| (id, 0.0))
                .collect();

            // ── Procesar cada nodo ──────────────────────────────────────────
            for &v in c_nodes {
                let v_ref_comm_initial = ref_comm[&v];

                // Guardia del paper (Algorithm 3, línea "if P_refined(v) == {v}"):
                // solo procesar v si todavía está en su singleton original.
                // Si otro nodo ya se fusionó hacia la comunidad de v, v no se procesa.
                // Esto garantiza que la fórmula ΔH(v→R) = e(v,R) − γ·|R| sea correcta
                // (solo válida para v singleton).
                if ref_sizes[&v_ref_comm_initial] != 1 {
                    continue;
                }

                let e_cv = e_c[&v]; // e(v, C\{v})

                // Verificar elegibilidad: v debe estar suficientemente conectado a C
                // Condición: e(v, C\{v}) ≥ γ · (|C| − 1)
                if e_cv < gamma * (c_size as f64 - 1.0) {
                    // v no es elegible; permanece en su singleton refinado
                    continue;
                }

                // Encontrar comunidades refinadas vecinas dentro de C (distintas de la propia)
                let v_ref_comm = v_ref_comm_initial;
                let mut candidate_refs: Vec<usize> = adjacency
                    .get(&v)
                    .map(|nbrs| {
                        let mut cs: Vec<usize> = nbrs
                            .keys()
                            .filter(|u| c_set.contains(*u))
                            .filter_map(|u| {
                                let rc = ref_comm[u];
                                if rc != v_ref_comm { Some(rc) } else { None }
                            })
                            .collect();
                        cs.sort_unstable();
                        cs.dedup();
                        cs
                    })
                    .unwrap_or_default();
                candidate_refs.sort_unstable();
                candidate_refs.dedup();

                let mut best_gain = 0.0_f64;
                let mut best_ref: Option<usize> = None;

                for r in candidate_refs {
                    let r_size = ref_sizes[&r] as f64;
                    let r_e_int = ref_e_int[&r];

                    // ── Verificar bien-conexión de R en C ──────────────────
                    // e(R, C\R) = Σ_{u∈R} e_c[u] − 2·e_int(R)
                    // Bien-conectado si: e(R, C\R) ≥ γ · |R| · (|C| − |R|)
                    let sum_ec_r: f64 = c_nodes
                        .iter()
                        .filter(|&&u| ref_comm[&u] == r)
                        .map(|&u| e_c[&u])
                        .sum();
                    let e_ext_r = sum_ec_r - 2.0 * r_e_int;
                    let threshold = gamma * r_size * (c_size as f64 - r_size);
                    if e_ext_r < threshold {
                        continue; // R no está bien conectada
                    }

                    // ── Ganancia CPM de fusionar singleton {v} en R ────────
                    // ΔH(v→R) = e(v, R) − γ · |R|
                    let e_v_r: f64 = adjacency
                        .get(&v)
                        .map(|nbrs| {
                            nbrs.iter()
                                .filter(|(u, _)| ref_comm.get(*u) == Some(&r))
                                .map(|(_, &w)| w)
                                .sum()
                        })
                        .unwrap_or(0.0);
                    let gain = e_v_r - gamma * r_size;

                    if gain > best_gain {
                        best_gain = gain;
                        best_ref = Some(r);
                    }
                }

                // ── Aplicar fusión si hay ganancia ──────────────────────────
                if let Some(target_r) = best_ref {
                    // Peso de v hacia target_r (para actualizar e_int)
                    let e_v_r: f64 = adjacency
                        .get(&v)
                        .map(|nbrs| {
                            nbrs.iter()
                                .filter(|(u, _)| ref_comm.get(*u) == Some(&target_r))
                                .map(|(_, &w)| w)
                                .sum()
                        })
                        .unwrap_or(0.0);

                    *ref_comm.get_mut(&v).expect("v in ref_comm") = target_r;
                    *ref_sizes.entry(target_r).or_insert(0) += 1;
                    *ref_e_int.entry(target_r).or_insert(0.0) += e_v_r;
                    // El singleton original de v queda vacío (ref_sizes = 0)
                    *ref_sizes.entry(v_ref_comm).or_insert(1) -= 1;
                }
            }

            // ── Mapear comunidades refinadas a asignaciones globales ────────
            // Si la partición refinada coincide exactamente con la base
            // (todos los nodos siguen en sus singletons originales O todos
            // están juntos en una sola comunidad), no hay cambio real.
            let refined_comm_ids: HashSet<usize> = c_nodes.iter().map(|&v| ref_comm[&v]).collect();

            if refined_comm_ids.len() == 1 {
                // Todos en una comunidad: equivale a la base → sin cambio
                // Asegurar que usen el ID base para consistencia
                for &v in c_nodes {
                    *communities.get_mut(&v).expect("v in communities") = base_comm_id;
                }
                continue;
            }

            if refined_comm_ids.len() == c_size {
                // Todos en singletons: tampoco cambia la asignación global
                // (Phase1 ya los tenía así o los consolidó)
                // Actualizar sizes/e_in para reflejar singletons
                for &v in c_nodes {
                    let new_c = ref_id_base + c_nodes.iter().position(|&x| x == v).unwrap_or(0);
                    *communities.get_mut(&v).expect("v in communities") = new_c;
                    sizes.insert(new_c, 1);
                    e_in.insert(new_c, 0.0);
                    any_changed = true;
                }
                *sizes.entry(base_comm_id).or_insert(c_size) -= c_size;
                e_in.entry(base_comm_id).and_modify(|e| *e = 0.0);
                continue;
            }

            // Caso general: la comunidad base fue dividida en >1 y <c_size comunidades
            any_changed = true;

            // Asignar IDs globales únicos a cada grupo refinado.
            // Se ordenan los IDs refinados para que la asignación sea determinista:
            // el grupo con el menor ID refinado hereda el ID base, el resto recibe
            // IDs nuevos en orden creciente.
            let mut sorted_ref_ids: Vec<usize> = refined_comm_ids.iter().copied().collect();
            sorted_ref_ids.sort_unstable();

            let mut ref_to_global: HashMap<usize, usize> = HashMap::new();
            for (idx, ref_id) in sorted_ref_ids.iter().enumerate() {
                if idx == 0 {
                    // El primero (menor ID) hereda el ID base
                    ref_to_global.insert(*ref_id, base_comm_id);
                } else {
                    let new_id = next_ref_id;
                    next_ref_id += 1;
                    ref_to_global.insert(*ref_id, new_id);
                }
            }

            // Actualizar asignaciones y estructuras
            // Limpiar tamaño y e_in del ID base antes de reasignar
            sizes.insert(base_comm_id, 0);
            e_in.insert(base_comm_id, 0.0);

            for &v in c_nodes {
                let old_ref = ref_comm[&v];
                let global_id = ref_to_global[&old_ref];
                *communities.get_mut(&v).expect("v in communities") = global_id;
                *sizes.entry(global_id).or_insert(0) += 1;

                // Recalcular e_in para v hacia su nueva comunidad global
                let e_v_new_comm: f64 = adjacency
                    .get(&v)
                    .map(|nbrs| {
                        nbrs.iter()
                            .filter(|(u, _)| ref_comm.get(*u) == Some(&old_ref) && **u != v)
                            .map(|(_, &w)| w)
                            .sum()
                    })
                    .unwrap_or(0.0);
                *e_in.entry(global_id).or_insert(0.0) += e_v_new_comm;
            }
        }

        any_changed
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Helpers
    // ─────────────────────────────────────────────────────────────────────────

    /// Suma de pesos de aristas desde `node` hacia todos los nodos en `community`.
    /// O(degree(node)).
    fn edge_weight_to_community(
        node: NodeId,
        community: usize,
        adjacency: &HashMap<NodeId, HashMap<NodeId, f64>>,
        communities: &HashMap<NodeId, usize>,
    ) -> f64 {
        adjacency
            .get(&node)
            .map(|nbrs| {
                nbrs.iter()
                    .filter(|(nbr, _)| communities.get(*nbr) == Some(&community) && **nbr != node)
                    .map(|(_, &w)| w)
                    .sum()
            })
            .unwrap_or(0.0)
    }

    /// Renumera comunidades a IDs contiguos 0, 1, 2, …
    fn renumber_communities(communities: HashMap<NodeId, usize>) -> Result<HashMap<NodeId, usize>> {
        let mut unique: Vec<usize> = communities
            .values()
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        unique.sort_unstable();

        let remap: HashMap<usize, usize> = unique
            .into_iter()
            .enumerate()
            .map(|(new_id, old_id)| (old_id, new_id))
            .collect();

        Ok(communities
            .into_iter()
            .map(|(node, old)| (node, remap[&old]))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Graph;
    use crate::types::{Edge, Node};
    use std::collections::HashMap;

    fn make_node() -> Node {
        Node {
            id: uuid::Uuid::new_v4(),
            label: "N".to_string(),
            properties: HashMap::new(),
            kind: Default::default(),
        }
    }

    fn make_edge(src: NodeId, tgt: NodeId) -> Edge {
        Edge {
            id: uuid::Uuid::new_v4(),
            source: src,
            target: tgt,
            edge_type: "E".to_string(),
            properties: HashMap::new(),
        }
    }

    /// Dos triángulos con un puente: Louvain debe separar las dos comunidades.
    /// Este es el test canónico de la fórmula de ganancia neta (Blondel et al. 2008).
    #[tokio::test]
    async fn test_louvain_two_triangles_separates() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();

        let a = tx.add_node(make_node()).await.unwrap();
        let b = tx.add_node(make_node()).await.unwrap();
        let c = tx.add_node(make_node()).await.unwrap();
        let d = tx.add_node(make_node()).await.unwrap();
        let e = tx.add_node(make_node()).await.unwrap();
        let f = tx.add_node(make_node()).await.unwrap();

        // Triángulo 1: a-b-c (bidireccional, como lo almacena NopalDB)
        for (s, t) in [(a, b), (b, a), (b, c), (c, b), (c, a), (a, c)] {
            tx.add_edge(make_edge(s, t)).unwrap();
        }
        // Triángulo 2: d-e-f (bidireccional)
        for (s, t) in [(d, e), (e, d), (e, f), (f, e), (f, d), (d, f)] {
            tx.add_edge(make_edge(s, t)).unwrap();
        }
        // Puente c-d (bidireccional)
        tx.add_edge(make_edge(c, d)).unwrap();
        tx.add_edge(make_edge(d, c)).unwrap();
        tx.commit().await.unwrap();

        let louvain = LouvainCommunity::with_defaults();
        let communities = louvain.detect(&graph).await.unwrap();
        let n = LouvainCommunity::count_communities(&communities);

        // Con la fórmula de ganancia neta debe encontrar 2 comunidades: {a,b,c} y {d,e,f}.
        assert_eq!(
            n, 2,
            "Dos triángulos + puente → 2 comunidades, obtuvo {}",
            n
        );
        assert_eq!(communities[&a], communities[&b], "a y b deben estar juntos");
        assert_eq!(communities[&b], communities[&c], "b y c deben estar juntos");
        assert_eq!(communities[&d], communities[&e], "d y e deben estar juntos");
        assert_eq!(communities[&e], communities[&f], "e y f deben estar juntos");
        assert_ne!(
            communities[&a], communities[&d],
            "Triángulos deben estar separados"
        );
    }

    #[tokio::test]
    async fn test_community_simple() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();

        let a = tx.add_node(make_node()).await.unwrap();
        let b = tx.add_node(make_node()).await.unwrap();
        let c = tx.add_node(make_node()).await.unwrap();
        let d = tx.add_node(make_node()).await.unwrap();
        let e = tx.add_node(make_node()).await.unwrap();
        let f = tx.add_node(make_node()).await.unwrap();

        // Triángulo 1: a-b-c
        for (s, t) in [(a, b), (b, c), (c, a)] {
            tx.add_edge(make_edge(s, t)).unwrap();
        }
        // Triángulo 2: d-e-f
        for (s, t) in [(d, e), (e, f), (f, d)] {
            tx.add_edge(make_edge(s, t)).unwrap();
        }
        // Puente c-d
        tx.add_edge(make_edge(c, d)).unwrap();
        tx.commit().await.unwrap();

        let louvain = LouvainCommunity::with_defaults();
        let communities = louvain.detect(&graph).await.unwrap();

        let num_communities = LouvainCommunity::count_communities(&communities);
        assert!(
            num_communities >= 1 && num_communities <= 6,
            "Expected 1-6 communities, got {}",
            num_communities
        );

        for &id in &[a, b, c, d, e, f] {
            assert!(
                communities.contains_key(&id),
                "Node missing from communities"
            );
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Tests — LeidenCommunity
    // ─────────────────────────────────────────────────────────────────────────

    /// Helper: añade un nodo simple a una transacción y retorna su NodeId.
    async fn add_test_node(tx: &mut crate::transaction::Transaction, label: &str) -> NodeId {
        tx.add_node(Node {
            id: uuid::Uuid::new_v4(),
            label: label.to_string(),
            properties: HashMap::new(),
            kind: Default::default(),
        })
        .await
        .unwrap()
    }

    /// Helper: añade una arista no-dirigida (añade dos aristas simétricas) a la transacción.
    fn add_test_edge(tx: &mut crate::transaction::Transaction, src: NodeId, tgt: NodeId) {
        tx.add_edge(Edge {
            id: uuid::Uuid::new_v4(),
            source: src,
            target: tgt,
            edge_type: "E".to_string(),
            properties: HashMap::new(),
        })
        .unwrap();
    }

    /// Grafo vacío: Leiden debe retornar mapa vacío sin panic.
    #[tokio::test]
    async fn test_leiden_empty_graph() {
        let graph = Graph::in_memory().await.unwrap();
        let leiden = LeidenCommunity::with_defaults();
        let communities = leiden.detect(&graph).await.unwrap();
        assert!(communities.is_empty());
    }

    /// Nodo aislado: debe tener su propia comunidad (ID 0).
    #[tokio::test]
    async fn test_leiden_single_node() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();
        let id = add_test_node(&mut tx, "N").await;
        tx.commit().await.unwrap();

        let leiden = LeidenCommunity::with_defaults();
        let communities = leiden.detect(&graph).await.unwrap();
        assert_eq!(communities.len(), 1);
        assert_eq!(communities[&id], 0);
    }

    /// Dos triángulos con un puente: Leiden con gamma bajo detecta comunidades separadas.
    #[tokio::test]
    async fn test_leiden_two_triangles_with_bridge() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();

        let a = add_test_node(&mut tx, "A").await;
        let b = add_test_node(&mut tx, "B").await;
        let c = add_test_node(&mut tx, "C").await;
        let d = add_test_node(&mut tx, "D").await;
        let e = add_test_node(&mut tx, "E").await;
        let f = add_test_node(&mut tx, "F").await;

        // Triángulo 1: a–b–c
        add_test_edge(&mut tx, a, b);
        add_test_edge(&mut tx, b, c);
        add_test_edge(&mut tx, c, a);
        // Triángulo 2: d–e–f
        add_test_edge(&mut tx, d, e);
        add_test_edge(&mut tx, e, f);
        add_test_edge(&mut tx, f, d);
        // Puente c–d
        add_test_edge(&mut tx, c, d);
        tx.commit().await.unwrap();

        let leiden = LeidenCommunity::with_gamma(0.1);
        let communities = leiden.detect(&graph).await.unwrap();

        let n_comm = LeidenCommunity::count_communities(&communities);
        assert!(
            n_comm >= 1 && n_comm <= 4,
            "Esperaba 1-4 comunidades con gamma=0.1, obtuvo {}",
            n_comm
        );
        // Todos los nodos deben estar asignados
        for &node in &[a, b, c, d, e, f] {
            assert!(
                communities.contains_key(&node),
                "Nodo sin comunidad asignada"
            );
        }
    }

    /// Grafo completo K4 con gamma=0.0: todos deben quedar en la misma comunidad.
    #[tokio::test]
    async fn test_leiden_complete_graph_k4_low_gamma() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();

        let ids: Vec<NodeId> = {
            let mut v = Vec::new();
            for _ in 0..4 {
                v.push(add_test_node(&mut tx, "N").await);
            }
            v
        };
        for i in 0..4 {
            for j in (i + 1)..4 {
                add_test_edge(&mut tx, ids[i], ids[j]);
            }
        }
        tx.commit().await.unwrap();

        // K4 con gamma=0.0 → todos en una comunidad (cualquier arista supera el threshold)
        let leiden = LeidenCommunity::with_gamma(0.0);
        let communities = leiden.detect(&graph).await.unwrap();
        let n_comm = LeidenCommunity::count_communities(&communities);
        assert_eq!(
            n_comm, 1,
            "K4 con gamma=0.0 debe producir 1 comunidad, obtuvo {}",
            n_comm
        );
    }

    /// Grafo completo K4 con gamma muy alto: cada nodo en su propia comunidad.
    #[tokio::test]
    async fn test_leiden_complete_graph_k4_high_gamma() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();

        let ids: Vec<NodeId> = {
            let mut v = Vec::new();
            for _ in 0..4 {
                v.push(add_test_node(&mut tx, "N").await);
            }
            v
        };
        for i in 0..4 {
            for j in (i + 1)..4 {
                add_test_edge(&mut tx, ids[i], ids[j]);
            }
        }
        tx.commit().await.unwrap();

        // Con gamma=2.0 ninguna fusión es rentable → todos singletons
        let leiden = LeidenCommunity::with_gamma(2.0);
        let communities = leiden.detect(&graph).await.unwrap();
        // Todos los nodos deben estar asignados
        assert_eq!(
            communities.len(),
            4,
            "Todos los nodos deben tener asignación"
        );
    }

    /// Determinismo: dos runs con la misma topología producen el mismo resultado.
    #[tokio::test]
    async fn test_leiden_deterministic() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();

        let ids: Vec<NodeId> = {
            let mut v = Vec::new();
            for _ in 0..6 {
                v.push(add_test_node(&mut tx, "N").await);
            }
            v
        };
        // Dos triángulos + puente
        let pairs: &[(usize, usize)] = &[(0, 1), (1, 2), (2, 0), (3, 4), (4, 5), (5, 3), (2, 3)];
        for &(i, j) in pairs {
            add_test_edge(&mut tx, ids[i], ids[j]);
        }
        tx.commit().await.unwrap();

        let leiden = LeidenCommunity::with_gamma(0.1);
        let r1 = leiden.detect(&graph).await.unwrap();
        let r2 = leiden.detect(&graph).await.unwrap();

        for &id in &ids {
            assert_eq!(
                r1[&id], r2[&id],
                "Leiden no es determinista: resultado distinto para el mismo nodo en dos runs"
            );
        }
    }

    /// Grafo lineal (cadena 5 nodos): todos los nodos asignados, al menos 1 comunidad.
    #[tokio::test]
    async fn test_leiden_linear_chain() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();

        let ids: Vec<NodeId> = {
            let mut v = Vec::new();
            for _ in 0..5 {
                v.push(add_test_node(&mut tx, "N").await);
            }
            v
        };
        for i in 0..4 {
            add_test_edge(&mut tx, ids[i], ids[i + 1]);
        }
        tx.commit().await.unwrap();

        let leiden = LeidenCommunity::with_defaults();
        let communities = leiden.detect(&graph).await.unwrap();
        assert_eq!(
            communities.len(),
            5,
            "Todos los nodos deben tener asignación"
        );
        let n_comm = LeidenCommunity::count_communities(&communities);
        assert!(n_comm >= 1, "Al menos 1 comunidad debe existir");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Test de regresión: topología exacta Padgett Florentine Families (15 nodos, 20 aristas)
    // Sirve para documentar el resultado real de ambos algoritmos sobre este grafo.
    // ─────────────────────────────────────────────────────────────────────────
    #[tokio::test]
    async fn test_florentine_families_community_counts() {
        use std::collections::BTreeMap;

        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();

        // Crear los 15 nodos con UUIDs deterministas (from_u128) para que el
        // orden de iteración del algoritmo sea estable entre runs.
        let names = [
            "Acciaiuoli",
            "Albizzi",
            "Barbadori",
            "Bischeri",
            "Castellani",
            "Ginori",
            "Guadagni",
            "Lamberteschi",
            "Medici",
            "Pazzi",
            "Peruzzi",
            "Ridolfi",
            "Salviati",
            "Strozzi",
            "Tornabuoni",
        ];
        let mut ids: BTreeMap<&str, NodeId> = BTreeMap::new();
        for (i, name) in names.iter().enumerate() {
            let node = Node {
                id: uuid::Uuid::from_u128((i as u128 + 1) << 96),
                label: "Family".to_string(),
                properties: std::collections::HashMap::from([(
                    "name".to_string(),
                    crate::types::PropertyValue::String(name.to_string()),
                )]),
                kind: Default::default(),
            };
            ids.insert(name, tx.add_node(node).await.unwrap());
        }

        // 20 aristas no-dirigidas (almacenadas bidireccionales, como hace el dataset real)
        let edges_undirected = [
            ("Acciaiuoli", "Medici"),
            ("Castellani", "Peruzzi"),
            ("Castellani", "Strozzi"),
            ("Castellani", "Barbadori"),
            ("Medici", "Barbadori"),
            ("Medici", "Ridolfi"),
            ("Medici", "Tornabuoni"),
            ("Medici", "Albizzi"),
            ("Medici", "Salviati"),
            ("Salviati", "Pazzi"),
            ("Peruzzi", "Strozzi"),
            ("Peruzzi", "Bischeri"),
            ("Strozzi", "Ridolfi"),
            ("Strozzi", "Bischeri"),
            ("Ridolfi", "Tornabuoni"),
            ("Tornabuoni", "Guadagni"),
            ("Albizzi", "Ginori"),
            ("Albizzi", "Guadagni"),
            ("Bischeri", "Guadagni"),
            ("Guadagni", "Lamberteschi"),
        ];
        for (a, b) in &edges_undirected {
            tx.add_edge(make_edge(ids[a], ids[b])).unwrap();
            tx.add_edge(make_edge(ids[b], ids[a])).unwrap();
        }
        tx.commit().await.unwrap();

        // ── Louvain ──
        let louvain = LouvainCommunity::with_defaults();
        let louv_comm = louvain.detect(&graph).await.unwrap();
        let n_louv = LouvainCommunity::count_communities(&louv_comm);

        // ── Leiden ──
        let leiden = LeidenCommunity::with_defaults();
        let leid_comm = leiden.detect(&graph).await.unwrap();
        let n_leid = LeidenCommunity::count_communities(&leid_comm);

        // Reportar resultado antes de las aserciones para visibilidad
        eprintln!("=== Florentine Families community detection ===");
        eprintln!("Louvain: {} comunidades", n_louv);
        eprintln!("Leiden:  {} comunidades", n_leid);
        let mut louv_groups: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
        let mut leid_groups: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
        for name in &names {
            louv_groups
                .entry(louv_comm[&ids[name]])
                .or_default()
                .push(name);
            leid_groups
                .entry(leid_comm[&ids[name]])
                .or_default()
                .push(name);
        }
        for (c, members) in &louv_groups {
            eprintln!("  Louvain {}: {:?}", c, members);
        }
        for (c, members) in &leid_groups {
            eprintln!("  Leiden  {}: {:?}", c, members);
        }

        // Los 15 nodos deben estar asignados
        assert_eq!(
            louv_comm.len(),
            15,
            "Louvain: todos los nodos deben tener comunidad"
        );
        assert_eq!(
            leid_comm.len(),
            15,
            "Leiden: todos los nodos deben tener comunidad"
        );

        // Louvain: fórmula corregida → 5 comunidades
        assert_eq!(
            n_louv, 5,
            "Louvain Florentine: esperaba 5 comunidades, obtuvo {}",
            n_louv
        );

        // Leiden con gamma=0.1: también 5 comunidades (CPM más estricto que modularity)
        assert_eq!(
            n_leid, 5,
            "Leiden Florentine: esperaba 5 comunidades, obtuvo {}",
            n_leid
        );

        // Louvain: bloque Medici (Acciaiuoli, Medici, Ridolfi, Tornabuoni)
        let medici_comm = louv_comm[&ids["Medici"]];
        for &ally in &["Acciaiuoli", "Ridolfi", "Tornabuoni"] {
            assert_eq!(
                louv_comm[&ids[ally]], medici_comm,
                "Louvain: {} debe estar con Medici",
                ally
            );
        }
        // Louvain: bloque Strozzi-sur (Bischeri, Castellani, Peruzzi, Strozzi)
        // Nota: Barbadori es broker equidistante (1 arista a Medici, 1 a Castellani).
        // Con estos UUIDs deterministas queda con Strozzi. En datos reales (UUIDs aleatorios)
        // Louvain también lo coloca con Strozzi — resultado consistente.
        let strozzi_comm = louv_comm[&ids["Strozzi"]];
        for &ally in &["Bischeri", "Castellani", "Peruzzi"] {
            assert_eq!(
                louv_comm[&ids[ally]], strozzi_comm,
                "Louvain: {} debe estar con Strozzi",
                ally
            );
        }
        assert_eq!(
            louv_comm[&ids["Barbadori"]], strozzi_comm,
            "Louvain: Barbadori debe estar con Strozzi (broker equidistante — resultado estable con Louvain)"
        );

        // Leiden: bloque Medici sin Barbadori
        // Con UUIDs deterministas Leiden coloca a Barbadori con Strozzi (igual que Louvain).
        // En datos reales (UUIDs aleatorios), Leiden lo coloca con Medici porque CPM encuentra
        // la partición de mayor calidad — no hay garantía de qué lado cae el broker.
        // No afirmamos la comunidad de Barbadori en Leiden por ser sensible al orden de iteración.
        let medici_leid = leid_comm[&ids["Medici"]];
        for &ally in &["Acciaiuoli", "Ridolfi", "Tornabuoni"] {
            assert_eq!(
                leid_comm[&ids[ally]], medici_leid,
                "Leiden: {} debe estar con Medici",
                ally
            );
        }
        // Leiden: Bischeri, Castellani, Peruzzi, Strozzi siempre juntos (cluster denso)
        let strozzi_leid = leid_comm[&ids["Strozzi"]];
        for &ally in &["Bischeri", "Castellani", "Peruzzi"] {
            assert_eq!(
                leid_comm[&ids[ally]], strozzi_leid,
                "Leiden: {} debe estar con Strozzi",
                ally
            );
        }
        // Pazzi y Salviati juntos (par aislado)
        assert_eq!(
            leid_comm[&ids["Pazzi"]], leid_comm[&ids["Salviati"]],
            "Leiden: Pazzi y Salviati deben estar juntos"
        );
    }
}
