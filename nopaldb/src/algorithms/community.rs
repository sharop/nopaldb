// src/algorithms/community.rs
//
// Community Detection algorithms — Louvain and Leiden

use crate::error::Result;
use crate::graph::GraphView;
use crate::types::NodeId;
use std::collections::{BTreeMap, HashMap, HashSet};

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
            .map_err(|e| crate::error::NopalError::custom(format!("community detect join error: {e}")))?
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
            adjacency.entry(edge.source).or_default().insert(edge.target, 1.0);
            adjacency.entry(edge.target).or_default().insert(edge.source, 1.0);
        }
        // total_weight = número de aristas no-dirigidas (cada par cuenta una vez).
        // Sumamos todos los valores de adyacencia y dividimos por 2.
        let total_weight: f64 = adjacency.values()
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
                    Some(c) if c == target_community  => k_i_in_t += w,
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
                if comm == target_community  { sigma_t += deg; }
                if comm == current_community { sigma_s += deg; }
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
// 3. Agregación (#194): cada parte refinada se vuelve un nodo del nivel
//    siguiente, lo que permite unir comunidades enteras. Hasta 0.6.10 la
//    implementación era plana, sin agregación, y estaba rota (ver el núcleo
//    más abajo). Costo ≈ O(m) por nivel, con pocos niveles; 100k nodos en
//    menos de 1 s (docs/ALGORITHMS.md).
//
// Garantía (ausente en Louvain)
// ─────────────────────────────
// Como en el paper, tras cada iteración las comunidades son γ-conexas: se
// construyen solo con fusiones bien conectadas, así que no quedan "islas"
// desconectadas dentro de una comunidad.
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
    /// Parámetro de resolución CPM. Default: 0.1. Con `weight_property`
    /// se compara contra la densidad en unidades de peso: si los pesos no
    /// son del orden de 1, escalar `gamma` en proporción.
    pub gamma: f64,
    /// Número máximo de iteraciones de Leiden completo (mover → refinar →
    /// agregar, todos los niveles); cada iteración parte del resultado de la
    /// anterior y se para antes si no cambia. Default: 10.
    pub max_iterations: usize,
    /// Ganancia CPM mínima para aceptar un movimiento de nodo. Default: 1e-9.
    pub min_gain: f64,
    /// Solo nodos con estas etiquetas (#190). `None` = todos. Las aristas
    /// con un extremo fuera del conjunto se descartan.
    pub labels: Option<Vec<String>>,
    /// Solo aristas de estos tipos (#190). `None` = todos.
    pub edge_types: Option<Vec<String>>,
    /// Propiedad numérica de la arista que da su peso (#190). `None` = cada
    /// par conectado pesa 1, como hasta 0.6.10. Con propiedad, el peso de un
    /// par es la SUMA de sus aristas (las que no la tienen pesan 1): varias
    /// co-ocurrencias suman. Si el grafo guarda cada relación en las dos
    /// direcciones, ponga el peso en una sola o filtre con `edge_types`.
    pub weight_property: Option<String>,
}

impl Default for LeidenConfig {
    fn default() -> Self {
        LeidenConfig {
            gamma: 0.1,
            max_iterations: 10,
            min_gain: 1e-9,
            labels: None,
            edge_types: None,
            weight_property: None,
        }
    }
}

impl LeidenConfig {
    /// Clave canónica de la configuración, para la caché por topología:
    /// misma clave ⇔ misma partición. Etiquetas y tipos se ordenan, así que
    /// el orden en que se escribieron no importa.
    pub fn cache_key(&self) -> String {
        let sorted = |v: &Option<Vec<String>>| {
            v.as_ref().map(|v| {
                let mut v = v.clone();
                v.sort();
                v.dedup();
                v.join(",")
            })
        };
        format!(
            "gamma={:?};iter={};min_gain={:?};labels={:?};edge_types={:?};weight={:?}",
            self.gamma,
            self.max_iterations,
            self.min_gain,
            sorted(&self.labels),
            sorted(&self.edge_types),
            self.weight_property
        )
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
        LeidenCommunity { config: LeidenConfig::default() }
    }

    /// Crea instancia configurando solo gamma; resto de parámetros por defecto.
    pub fn with_gamma(gamma: f64) -> Self {
        LeidenCommunity { config: LeidenConfig { gamma, ..LeidenConfig::default() } }
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
            .map_err(|e| crate::error::NopalError::custom(
                format!("leiden detect join error: {e}")
            ))?
    }

    /// Retorna el número de comunidades detectadas.
    pub fn count_communities(communities: &HashMap<NodeId, usize>) -> usize {
        communities.values().copied().collect::<HashSet<_>>().len()
    }

    /// Jerarquía de comunidades (#190 b): el nivel 0 es la partición de
    /// [`Self::detect`] y cada nivel siguiente parte las comunidades
    /// grandes del anterior. Ver [`LeidenHierarchyOptions`].
    pub async fn detect_hierarchy<G: GraphView>(
        &self,
        graph: &G,
        options: &LeidenHierarchyOptions,
    ) -> Result<LeidenHierarchy> {
        options.validate()?;
        let nodes = graph.get_all_nodes().await?;
        if nodes.is_empty() {
            return Ok(LeidenHierarchy::default());
        }
        let edges = graph.get_all_edges().await?;
        let (config, options) = (self.config.clone(), options.clone());
        tokio::task::spawn_blocking(move || {
            let Some(graph) = DenseGraph::from_scope(nodes, edges, &config)? else {
                return Ok(LeidenHierarchy::default());
            };
            let levels = leiden_hierarchy(&graph, &config, &options);
            Ok(LeidenHierarchy::from_levels(&graph.node_ids, levels))
        })
        .await
        .map_err(|e| crate::error::NopalError::custom(format!("leiden hierarchy join error: {e}")))?
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Núcleo (#194): Leiden según Traag, Waltman y van Eck (2019), con CPM y
    // nodos con tamaño. Hasta 0.6.10 el núcleo era plano, reutilizaba ids de
    // comunidad entre iteraciones (el tamaño de una comunidad podía bajar de
    // cero: pánico en debug, silencioso en release) y dejaba que el
    // refinamiento REEMPLAZARA la partición. Ahora:
    //
    //   1. Mover (`move_nodes_fast`): cada nodo se va a la comunidad vecina
    //      (o a una vacía) que más mejora la calidad CPM.
    //   2. Refinar (`refine`): dentro de cada comunidad, se arma desde
    //      singletons una partición más fina de partes bien conectadas.
    //   3. Agregar (`aggregate`): cada parte refinada se vuelve un nodo con
    //      TAMAÑO = sus miembros; la partición inicial del nivel siguiente es
    //      la del paso 1. Se repite hasta que mover no agrupe nada.
    //
    // Se reporta la partición del paso 1 (no la refinada), proyectada a los
    // nodos originales. Estructuras densas: nodos 0..n en orden de `NodeId`,
    // vecinos ordenados, comunidades = índices; no se inventan ids, así que
    // no hay choques. Determinista: colas y empates en orden de índice.
    // ─────────────────────────────────────────────────────────────────────────

    fn detect_cpu(
        nodes: Vec<crate::types::Node>,
        edges: Vec<crate::types::Edge>,
        config: LeidenConfig,
    ) -> Result<HashMap<NodeId, usize>> {
        let Some(graph) = DenseGraph::from_scope(nodes, edges, &config)? else {
            return Ok(HashMap::new());
        };
        let partition = leiden_partition(&graph, &config);
        Ok(graph.node_ids.iter().copied().zip(partition).collect())
    }
}

/// Opciones de [`LeidenCommunity::detect_hierarchy`] (#190 b).
///
/// El nivel 0 es la partición de Leiden con el `gamma` de la configuración.
/// Para el nivel L+1, cada comunidad del nivel L con más de
/// `max_cluster_size` nodos se vuelve a partir con Leiden sobre su propio
/// subgrafo, con `gamma · resolution_factor^(L+1)`; las demás pasan igual.
/// Se para cuando ninguna comunidad se parte o al llegar a `max_levels`.
/// Así cada comunidad del nivel L+1 está contenida en una sola del nivel L,
/// por construcción, y cada nivel optimiza CPM a su resolución (CPM no tiene
/// límite de resolución: partir el subgrafo equivale a lo que pediría el
/// grafo entero a esa γ). Es el esquema del GraphRAG de referencia (Leiden
/// jerárquico con tamaño máximo de comunidad).
///
/// Descartado: usar como niveles las particiones refinadas que el núcleo
/// agrega internamente. Son gratis, pero son fragmentos del refinamiento,
/// no subcomunidades: en un grafo de 200 bloques densos de 50 nodos (sin
/// estructura interna) darían 1 917 y 247 "comunidades" debajo de las 202
/// reales, y nunca un nivel por encima del resultado.
#[derive(Debug, Clone)]
pub struct LeidenHierarchyOptions {
    /// Comunidades con más nodos que esto se parten en el nivel siguiente.
    /// Default: 10 (el del GraphRAG de referencia).
    pub max_cluster_size: usize,
    /// Factor por el que se multiplica `gamma` en cada nivel (> 1).
    /// Default: 2.0.
    pub resolution_factor: f64,
    /// Número máximo de niveles, contando el 0 (≥ 1). Default: 8.
    pub max_levels: usize,
}

impl Default for LeidenHierarchyOptions {
    fn default() -> Self {
        LeidenHierarchyOptions { max_cluster_size: 10, resolution_factor: 2.0, max_levels: 8 }
    }
}

impl LeidenHierarchyOptions {
    fn validate(&self) -> Result<()> {
        let invalid = |msg: &str| Err(crate::error::NopalError::custom(format!("leiden hierarchy: {msg}")));
        if self.max_cluster_size == 0 {
            return invalid("max_cluster_size must be >= 1");
        }
        if !(self.resolution_factor.is_finite() && self.resolution_factor > 1.0) {
            return invalid("resolution_factor must be a number > 1");
        }
        if self.max_levels == 0 {
            return invalid("max_levels must be >= 1");
        }
        Ok(())
    }
}

/// Jerarquía de comunidades de Leiden (#190 b), de la más gruesa a la más
/// fina. `levels[0]` es la partición de [`LeidenCommunity::detect`] (la
/// misma, nodo por nodo); toda comunidad del nivel L+1 está contenida en
/// una sola del nivel L. Ids contiguos desde 0 en cada nivel, en orden de
/// primera aparición por `NodeId`.
#[derive(Debug, Clone, Default)]
pub struct LeidenHierarchy {
    pub levels: Vec<HashMap<NodeId, usize>>,
    /// `parents[L][c]`: la comunidad del nivel L que contiene a la `c` del
    /// nivel L+1.
    parents: Vec<Vec<usize>>,
}

impl LeidenHierarchy {
    fn from_levels(node_ids: &[NodeId], levels: Vec<Vec<usize>>) -> LeidenHierarchy {
        let parents = levels
            .windows(2)
            .map(|pair| {
                let k = pair[1].iter().copied().max().map_or(0, |m| m + 1);
                let mut parent = vec![0usize; k];
                for (child, coarse) in pair[1].iter().zip(&pair[0]) {
                    parent[*child] = *coarse;
                }
                parent
            })
            .collect();
        let levels = levels
            .into_iter()
            .map(|level| node_ids.iter().copied().zip(level).collect())
            .collect();
        LeidenHierarchy { levels, parents }
    }

    /// Número de niveles (0 si no había nodos).
    pub fn depth(&self) -> usize {
        self.levels.len()
    }

    /// La comunidad del nivel `level - 1` que contiene a la comunidad
    /// `community` del nivel `level`; `None` en el nivel 0 o si no existe.
    pub fn parent(&self, level: usize, community: usize) -> Option<usize> {
        self.parents.get(level.checked_sub(1)?)?.get(community).copied()
    }
}

/// Grafo no dirigido con nodos densos `0..n` (en orden de `NodeId`) y tamaño
/// por nodo. `adj[i]` = vecinos `(j, peso)` ordenados por `j`, sin
/// autolazos (el peso interno de un nodo no cambia al moverlo).
struct DenseGraph {
    node_ids: Vec<NodeId>,
    adj: Vec<Vec<(usize, f64)>>,
    size: Vec<f64>,
}

impl DenseGraph {
    /// Alcance y pesos de `config` (#190 a) sobre los nodos y aristas dados.
    /// `None` si no queda ningún nodo.
    fn from_scope(
        nodes: Vec<crate::types::Node>,
        edges: Vec<crate::types::Edge>,
        config: &LeidenConfig,
    ) -> Result<Option<DenseGraph>> {
        let mut node_ids: Vec<NodeId> = nodes
            .into_iter()
            .filter(|n| config.labels.as_ref().is_none_or(|l| l.contains(&n.label)))
            .map(|n| n.id)
            .collect();
        if node_ids.is_empty() {
            return Ok(None);
        }
        node_ids.sort_unstable();
        node_ids.dedup();
        let index: HashMap<NodeId, usize> = node_ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        // Pares no dirigidos: sin `weight_property`, peso 1 por par conectado
        // (una relación guardada en las dos direcciones no pesa 2); con ella,
        // suma de los pesos de las aristas del par. Orden de suma fijo.
        let mut sorted: Vec<&crate::types::Edge> = edges
            .iter()
            .filter(|e| config.edge_types.as_ref().is_none_or(|t| t.contains(&e.edge_type)))
            .collect();
        sorted.sort_by_key(|e| e.id);
        let mut pairs: BTreeMap<(usize, usize), f64> = BTreeMap::new();
        for edge in sorted {
            let (Some(&a), Some(&b)) = (index.get(&edge.source), index.get(&edge.target)) else { continue };
            if a == b {
                continue;
            }
            let key = (a.min(b), a.max(b));
            match &config.weight_property {
                None => {
                    pairs.insert(key, 1.0);
                }
                Some(prop) => {
                    let weight = match edge.properties.get(prop) {
                        None => 1.0,
                        Some(v) => v.as_number().ok_or_else(|| {
                            crate::error::NopalError::custom(format!(
                                "leiden: edge {} has a non-numeric `{prop}` ({v:?})",
                                edge.id
                            ))
                        })?,
                    };
                    if !weight.is_finite() || weight < 0.0 {
                        return Err(crate::error::NopalError::custom(format!(
                            "leiden: edge {} has weight {weight} in `{prop}`; weights must be finite and >= 0",
                            edge.id
                        )));
                    }
                    *pairs.entry(key).or_insert(0.0) += weight;
                }
            }
        }
        let n = node_ids.len();
        let mut adj: Vec<Vec<(usize, f64)>> = vec![Vec::new(); n];
        for ((a, b), w) in pairs {
            if w > 0.0 {
                adj[a].push((b, w));
                adj[b].push((a, w));
            }
        }
        for list in &mut adj {
            list.sort_unstable_by_key(|(j, _)| *j);
        }
        Ok(Some(DenseGraph { node_ids, adj, size: vec![1.0; n] }))
    }

    fn len(&self) -> usize {
        self.adj.len()
    }

    /// El subgrafo inducido por `members` (índices crecientes): el nodo `i`
    /// del subgrafo es `members[i]`.
    fn induced(&self, members: &[usize]) -> DenseGraph {
        let index: HashMap<usize, usize> = members.iter().enumerate().map(|(i, &m)| (m, i)).collect();
        let adj = members
            .iter()
            .map(|&m| self.adj[m].iter().filter_map(|&(j, w)| index.get(&j).map(|&i| (i, w))).collect())
            .collect();
        let size = members.iter().map(|&m| self.size[m]).collect();
        DenseGraph { node_ids: Vec::new(), adj, size }
    }

    /// El grafo agregado de `parts` (comunidad por nodo, ids `0..k`
    /// contiguos): un nodo por comunidad, tamaño = suma de tamaños, peso
    /// entre comunidades = suma de pesos; los pesos internos se descartan.
    fn aggregate(&self, parts: &[usize], k: usize) -> DenseGraph {
        let mut size = vec![0.0; k];
        for (i, &c) in parts.iter().enumerate() {
            size[c] += self.size[i];
        }
        let mut pairs: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); k];
        for (i, list) in self.adj.iter().enumerate() {
            for &(j, w) in list {
                let (ci, cj) = (parts[i], parts[j]);
                if ci != cj {
                    *pairs[ci].entry(cj).or_insert(0.0) += w;
                }
            }
        }
        let adj = pairs.into_iter().map(|m| m.into_iter().collect()).collect();
        DenseGraph { node_ids: Vec::new(), adj, size }
    }
}

/// Renumera `parts` a ids contiguos en orden de primera aparición; devuelve
/// la partición renumerada y cuántas comunidades hay.
fn renumber(parts: &[usize]) -> (Vec<usize>, usize) {
    let mut ids: HashMap<usize, usize> = HashMap::new();
    let out: Vec<usize> = parts
        .iter()
        .map(|c| {
            let next = ids.len();
            *ids.entry(*c).or_insert(next)
        })
        .collect();
    let k = ids.len();
    (out, k)
}

/// Los niveles de [`LeidenHierarchyOptions`], del más grueso al más fino.
fn leiden_hierarchy(graph: &DenseGraph, config: &LeidenConfig, options: &LeidenHierarchyOptions) -> Vec<Vec<usize>> {
    let mut levels = vec![leiden_partition(graph, config)];
    let mut gamma = config.gamma;
    while levels.len() < options.max_levels {
        gamma *= options.resolution_factor;
        let sub_config = LeidenConfig { gamma, ..config.clone() };
        let prev = levels.last().expect("level 0");
        let mut members: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (i, &c) in prev.iter().enumerate() {
            members.entry(c).or_default().push(i);
        }
        let mut next = vec![0usize; graph.len()];
        let (mut next_id, mut split) = (0usize, false);
        for nodes in members.values() {
            if nodes.len() <= options.max_cluster_size {
                for &i in nodes {
                    next[i] = next_id;
                }
                next_id += 1;
                continue;
            }
            let (sub, k) = renumber(&leiden_partition(&graph.induced(nodes), &sub_config));
            split |= k > 1;
            for (&i, &c) in nodes.iter().zip(&sub) {
                next[i] = next_id + c;
            }
            next_id += k;
        }
        if !split {
            break;
        }
        levels.push(renumber(&next).0);
    }
    levels
}

/// Tope de niveles de agregación del bucle externo. Cada nivel reduce el
/// número de nodos o termina, así que en la práctica para mucho antes.
const LEIDEN_MAX_LEVELS: usize = 64;

/// La partición de Leiden de `graph`, una comunidad por nodo (ids contiguos
/// en orden de primera aparición).
///
/// Leiden se itera (Traag et al. 2019, sección "Iterating"): cada corrida
/// empieza desde la partición de la anterior y no puede empeorarla, así que
/// se repite hasta que no cambie, como mucho `max_iterations` veces. Una sola
/// corrida desde singletons puede quedar en un óptimo local (en un grafo de
/// 200 bloques densos dejaba 2 bloques partidos).
fn leiden_partition(graph: &DenseGraph, config: &LeidenConfig) -> Vec<usize> {
    let mut partition: Vec<usize> = (0..graph.len()).collect();
    for _ in 0..config.max_iterations.max(1) {
        let next = leiden_pass(graph, &partition, config);
        if next == partition {
            break;
        }
        partition = next;
    }
    partition
}

/// Una corrida de Leiden (mover → refinar → agregar, hasta que mover no
/// agrupe nada) desde la partición `initial` de los nodos originales.
fn leiden_pass(graph: &DenseGraph, initial: &[usize], config: &LeidenConfig) -> Vec<usize> {
    let n = graph.len();
    // Nodo del grafo actual que representa a cada nodo original.
    let mut owner: Vec<usize> = (0..n).collect();
    let mut level_graph: Option<DenseGraph> = None;
    let mut parts: Vec<usize> = renumber(initial).0;
    for _ in 0..LEIDEN_MAX_LEVELS {
        let g = level_graph.as_ref().unwrap_or(graph);
        let moved = move_nodes_fast(g, &parts, config.gamma, config.min_gain);
        let (moved, k) = renumber(&moved);
        // Proyectar la partición del paso 1 a los nodos originales.
        let result: Vec<usize> = owner.iter().map(|&o| moved[o]).collect();
        if k == g.len() {
            // Mover no agrupó nada: cada nodo del nivel es su propia comunidad.
            return renumber(&result).0;
        }
        let refined = refine(g, &moved, config.gamma);
        let (refined, k_ref) = renumber(&refined);
        // Agregar sobre la partición refinada; si el refinamiento no unió
        // nada, sobre la del paso 1 (si no, el nivel no se reduciría).
        let (basis, k_basis) = if k_ref < g.len() { (refined, k_ref) } else { (moved.clone(), k) };
        let next_graph = g.aggregate(&basis, k_basis);
        // Partición inicial del nivel agregado: la comunidad del paso 1 de
        // cada nodo agregado (todos sus miembros comparten comunidad: el
        // refinamiento parte dentro de cada comunidad).
        let mut next_parts = vec![0usize; k_basis];
        for (i, &b) in basis.iter().enumerate() {
            next_parts[b] = moved[i];
        }
        owner = owner.iter().map(|&o| basis[o]).collect();
        parts = next_parts;
        level_graph = Some(next_graph);
        // Cada nivel tiene menos nodos que el anterior (`k_basis < g.len()`),
        // así que el bucle termina.
    }
    // Tope de niveles alcanzado: la última partición del paso 1.
    let g = level_graph.as_ref().unwrap_or(graph);
    let moved = renumber(&move_nodes_fast(g, &parts, config.gamma, config.min_gain)).0;
    renumber(&owner.iter().map(|&o| moved[o]).collect::<Vec<_>>()).0
}

/// Paso 1 del paper (MoveNodesFast), determinista: cola de nodos en orden de
/// índice; cada nodo se va a la comunidad (vecina o vacía) con la mayor
/// ganancia CPM si supera `min_gain`, y sus vecinos de otras comunidades
/// vuelven a la cola. Ganancia de mover `v` (tamaño `s`) de S a T:
/// `w(v,T) − w(v,S∖v) − γ·s·(n_T − (n_S − s))`.
fn move_nodes_fast(g: &DenseGraph, initial: &[usize], gamma: f64, min_gain: f64) -> Vec<usize> {
    let n = g.len();
    let mut comm: Vec<usize> = initial.to_vec();
    // Las comunidades son índices 0..n (como mucho n comunidades no vacías).
    let mut comm_size = vec![0.0f64; n];
    for (i, &c) in comm.iter().enumerate() {
        comm_size[c] += g.size[i];
    }
    let mut empty: std::collections::BTreeSet<usize> = (0..n).filter(|&c| comm_size[c] == 0.0).collect();
    let mut queue: std::collections::VecDeque<usize> = (0..n).collect();
    let mut queued = vec![true; n];
    let mut weight_to = vec![0.0f64; n];
    let mut touched: Vec<usize> = Vec::new();
    while let Some(v) = queue.pop_front() {
        queued[v] = false;
        let current = comm[v];
        for &(j, w) in &g.adj[v] {
            let c = comm[j];
            if weight_to[c] == 0.0 && !touched.contains(&c) {
                touched.push(c);
            }
            weight_to[c] += w;
        }
        touched.sort_unstable();
        let s_v = g.size[v];
        let n_s = comm_size[current];
        let w_self = weight_to[current];
        let (mut best, mut best_gain) = (current, min_gain);
        for &t in &touched {
            if t == current {
                continue;
            }
            let gain = weight_to[t] - w_self - gamma * s_v * (comm_size[t] - (n_s - s_v));
            if gain > best_gain {
                best_gain = gain;
                best = t;
            }
        }
        // Una comunidad vacía: no se gana peso, se deja de pagar la penalización.
        if n_s > s_v
            && let Some(&e) = empty.iter().next()
        {
            let gain = -w_self + gamma * s_v * (n_s - s_v);
            if gain > best_gain {
                best = e;
            }
        }
        for &c in &touched {
            weight_to[c] = 0.0;
        }
        touched.clear();
        if best == current {
            continue;
        }
        comm_size[current] -= s_v;
        if comm_size[current] <= 0.0 {
            comm_size[current] = 0.0;
            empty.insert(current);
        }
        empty.remove(&best);
        comm_size[best] += s_v;
        comm[v] = best;
        for &(j, _) in &g.adj[v] {
            if comm[j] != best && !queued[j] {
                queued[j] = true;
                queue.push_back(j);
            }
        }
    }
    comm
}

/// Paso 2 del paper (RefinePartition / MergeNodesSubset), determinista: en
/// cada comunidad S de `parts`, parte de singletons; cada nodo bien
/// conectado a S que siga solo se une a la parte bien conectada de S con la
/// mayor ganancia `w(v,C) − γ·s_v·‖C‖` si es positiva. Bien conectado:
/// `w(X, S∖X) ≥ γ·‖X‖·(‖S‖ − ‖X‖)`. Las partes nunca cruzan comunidades.
fn refine(g: &DenseGraph, parts: &[usize], gamma: f64) -> Vec<usize> {
    let n = g.len();
    let mut refined: Vec<usize> = (0..n).collect();
    let mut members: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, &c) in parts.iter().enumerate() {
        members.entry(c).or_default().push(i);
    }
    let mut ref_size: Vec<f64> = g.size.clone();
    // Peso de cada parte refinada hacia el resto de su comunidad.
    let mut ref_ext = vec![0.0f64; n];
    let mut alone = vec![true; n];
    let mut weight_to = vec![0.0f64; n];
    let mut touched: Vec<usize> = Vec::new();
    for nodes in members.values() {
        let s_total: f64 = nodes.iter().map(|&i| g.size[i]).sum();
        // w(v, S∖v) de cada nodo.
        for &v in nodes {
            ref_ext[v] = g.adj[v].iter().filter(|(j, _)| parts[*j] == parts[v]).map(|(_, w)| w).sum();
        }
        for &v in nodes {
            if !alone[v] {
                continue;
            }
            let s_v = g.size[v];
            if ref_ext[v] < gamma * s_v * (s_total - s_v) {
                continue; // v no está bien conectado a S
            }
            for &(j, w) in &g.adj[v] {
                if parts[j] != parts[v] {
                    continue;
                }
                let r = refined[j];
                if r == refined[v] {
                    continue;
                }
                if weight_to[r] == 0.0 && !touched.contains(&r) {
                    touched.push(r);
                }
                weight_to[r] += w;
            }
            touched.sort_unstable();
            let (mut best, mut best_gain) = (refined[v], 0.0f64);
            for &r in &touched {
                let well_connected = ref_ext[r] >= gamma * ref_size[r] * (s_total - ref_size[r]);
                if !well_connected {
                    continue;
                }
                let gain = weight_to[r] - gamma * s_v * ref_size[r];
                if gain > best_gain {
                    best_gain = gain;
                    best = r;
                }
            }
            if best != refined[v] {
                let w_v_best = weight_to[best];
                let old = refined[v];
                refined[v] = best;
                ref_size[best] += s_v;
                ref_size[old] = 0.0;
                // w(C ∪ v, S∖(C ∪ v)) = w(C, S∖C) + w(v, S∖v) − 2·w(v, C).
                ref_ext[best] = ref_ext[best] + ref_ext[v] - 2.0 * w_v_best;
                alone[v] = false;
                alone[best] = false;
            }
            for &r in &touched {
                weight_to[r] = 0.0;
            }
            touched.clear();
        }
    }
    refined
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::Graph;
    use crate::types::{Node, Edge};
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
        for (s, t) in [(a,b),(b,a),(b,c),(c,b),(c,a),(a,c)] {
            tx.add_edge(make_edge(s, t)).unwrap();
        }
        // Triángulo 2: d-e-f (bidireccional)
        for (s, t) in [(d,e),(e,d),(e,f),(f,e),(f,d),(d,f)] {
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
        assert_eq!(n, 2, "Dos triángulos + puente → 2 comunidades, obtuvo {}", n);
        assert_eq!(communities[&a], communities[&b], "a y b deben estar juntos");
        assert_eq!(communities[&b], communities[&c], "b y c deben estar juntos");
        assert_eq!(communities[&d], communities[&e], "d y e deben estar juntos");
        assert_eq!(communities[&e], communities[&f], "e y f deben estar juntos");
        assert_ne!(communities[&a], communities[&d], "Triángulos deben estar separados");
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
        for (s, t) in [(a,b),(b,c),(c,a)] {
            tx.add_edge(make_edge(s, t)).unwrap();
        }
        // Triángulo 2: d-e-f
        for (s, t) in [(d,e),(e,f),(f,d)] {
            tx.add_edge(make_edge(s, t)).unwrap();
        }
        // Puente c-d
        tx.add_edge(make_edge(c, d)).unwrap();
        tx.commit().await.unwrap();

        let louvain = LouvainCommunity::with_defaults();
        let communities = louvain.detect(&graph).await.unwrap();

        let num_communities = LouvainCommunity::count_communities(&communities);
        assert!(num_communities >= 1 && num_communities <= 6,
                "Expected 1-6 communities, got {}", num_communities);

        for &id in &[a, b, c, d, e, f] {
            assert!(communities.contains_key(&id), "Node missing from communities");
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Tests — LeidenCommunity
    // ─────────────────────────────────────────────────────────────────────────

    /// Helper: añade un nodo simple a una transacción y retorna su NodeId.
    async fn add_test_node(
        tx: &mut crate::transaction::Transaction,
        label: &str,
    ) -> NodeId {
        tx.add_node(Node {
            id: uuid::Uuid::new_v4(),
            label: label.to_string(),
            properties: HashMap::new(),
            kind: Default::default(),
        }).await.unwrap()
    }

    /// Helper: añade una arista no-dirigida (añade dos aristas simétricas) a la transacción.
    fn add_test_edge(
        tx: &mut crate::transaction::Transaction,
        src: NodeId,
        tgt: NodeId,
    ) {
        tx.add_edge(Edge {
            id: uuid::Uuid::new_v4(),
            source: src,
            target: tgt,
            edge_type: "E".to_string(),
            properties: HashMap::new(),
        }).unwrap();
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
        assert!(n_comm >= 1 && n_comm <= 4,
            "Esperaba 1-4 comunidades con gamma=0.1, obtuvo {}", n_comm);
        // Todos los nodos deben estar asignados
        for &node in &[a, b, c, d, e, f] {
            assert!(communities.contains_key(&node), "Nodo sin comunidad asignada");
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
        assert_eq!(n_comm, 1, "K4 con gamma=0.0 debe producir 1 comunidad, obtuvo {}", n_comm);
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
        assert_eq!(communities.len(), 4, "Todos los nodos deben tener asignación");
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
        let pairs: &[(usize, usize)] = &[(0,1),(1,2),(2,0),(3,4),(4,5),(5,3),(2,3)];
        for &(i, j) in pairs {
            add_test_edge(&mut tx, ids[i], ids[j]);
        }
        tx.commit().await.unwrap();

        let leiden = LeidenCommunity::with_gamma(0.1);
        let r1 = leiden.detect(&graph).await.unwrap();
        let r2 = leiden.detect(&graph).await.unwrap();

        for &id in &ids {
            assert_eq!(r1[&id], r2[&id],
                "Leiden no es determinista: resultado distinto para el mismo nodo en dos runs");
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
        assert_eq!(communities.len(), 5, "Todos los nodos deben tener asignación");
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
            "Acciaiuoli", "Albizzi", "Barbadori", "Bischeri", "Castellani",
            "Ginori", "Guadagni", "Lamberteschi", "Medici", "Pazzi",
            "Peruzzi", "Ridolfi", "Salviati", "Strozzi", "Tornabuoni",
        ];
        let mut ids: BTreeMap<&str, NodeId> = BTreeMap::new();
        for (i, name) in names.iter().enumerate() {
            let node = Node {
                id: uuid::Uuid::from_u128((i as u128 + 1) << 96),
                label: "Family".to_string(),
                properties: std::collections::HashMap::from([
                    ("name".to_string(), crate::types::PropertyValue::String(name.to_string())),
                ]),
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
            louv_groups.entry(louv_comm[&ids[name]]).or_default().push(name);
            leid_groups.entry(leid_comm[&ids[name]]).or_default().push(name);
        }
        for (c, members) in &louv_groups { eprintln!("  Louvain {}: {:?}", c, members); }
        for (c, members) in &leid_groups { eprintln!("  Leiden  {}: {:?}", c, members); }

        // Los 15 nodos deben estar asignados
        assert_eq!(louv_comm.len(), 15, "Louvain: todos los nodos deben tener comunidad");
        assert_eq!(leid_comm.len(), 15, "Leiden: todos los nodos deben tener comunidad");

        // Louvain: fórmula corregida → 5 comunidades
        assert_eq!(n_louv, 5, "Louvain Florentine: esperaba 5 comunidades, obtuvo {}", n_louv);

        // Leiden con gamma=0.1: 4 comunidades (#194). El núcleo anterior daba 5
        // con calidad CPM 11.1; esta partición tiene 11.7: Barbadori va con
        // Medici, y Albizzi/Ginori con Guadagni/Lamberteschi. Leiden maximiza
        // CPM, así que se afirma la calidad, no solo el número.
        assert_eq!(n_leid, 4, "Leiden Florentine: esperaba 4 comunidades, obtuvo {}", n_leid);
        let cpm: f64 = leid_groups
            .values()
            .map(|members| {
                let inside = edges_undirected
                    .iter()
                    .filter(|(a, b)| members.contains(a) && members.contains(b))
                    .count() as f64;
                let n = members.len() as f64;
                inside - 0.1 * n * (n - 1.0) / 2.0
            })
            .sum();
        assert!(cpm > 11.1 + 1e-9, "calidad CPM {cpm} no supera la del núcleo anterior (11.1)");

        // Louvain: bloque Medici (Acciaiuoli, Medici, Ridolfi, Tornabuoni)
        let medici_comm = louv_comm[&ids["Medici"]];
        for &ally in &["Acciaiuoli", "Ridolfi", "Tornabuoni"] {
            assert_eq!(louv_comm[&ids[ally]], medici_comm,
                "Louvain: {} debe estar con Medici", ally);
        }
        // Louvain: bloque Strozzi-sur (Bischeri, Castellani, Peruzzi, Strozzi)
        // Nota: Barbadori es broker equidistante (1 arista a Medici, 1 a Castellani).
        // Con estos UUIDs deterministas queda con Strozzi. En datos reales (UUIDs aleatorios)
        // Louvain también lo coloca con Strozzi — resultado consistente.
        let strozzi_comm = louv_comm[&ids["Strozzi"]];
        for &ally in &["Bischeri", "Castellani", "Peruzzi"] {
            assert_eq!(louv_comm[&ids[ally]], strozzi_comm,
                "Louvain: {} debe estar con Strozzi", ally);
        }
        assert_eq!(louv_comm[&ids["Barbadori"]], strozzi_comm,
            "Louvain: Barbadori debe estar con Strozzi (broker equidistante — resultado estable con Louvain)");

        // Leiden: bloque Medici sin Barbadori
        // Con UUIDs deterministas Leiden coloca a Barbadori con Strozzi (igual que Louvain).
        // En datos reales (UUIDs aleatorios), Leiden lo coloca con Medici porque CPM encuentra
        // la partición de mayor calidad — no hay garantía de qué lado cae el broker.
        // No afirmamos la comunidad de Barbadori en Leiden por ser sensible al orden de iteración.
        let medici_leid = leid_comm[&ids["Medici"]];
        for &ally in &["Acciaiuoli", "Ridolfi", "Tornabuoni"] {
            assert_eq!(leid_comm[&ids[ally]], medici_leid,
                "Leiden: {} debe estar con Medici", ally);
        }
        // Leiden: Bischeri, Castellani, Peruzzi, Strozzi siempre juntos (cluster denso)
        let strozzi_leid = leid_comm[&ids["Strozzi"]];
        for &ally in &["Bischeri", "Castellani", "Peruzzi"] {
            assert_eq!(leid_comm[&ids[ally]], strozzi_leid,
                "Leiden: {} debe estar con Strozzi", ally);
        }
        // Pazzi y Salviati juntos (par aislado)
        assert_eq!(leid_comm[&ids["Pazzi"]], leid_comm[&ids["Salviati"]],
            "Leiden: Pazzi y Salviati deben estar juntos");
    }
}