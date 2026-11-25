//! Camino más corto con sus nodos y aristas (#416, desde 0.11.4).
//!
//! [`Graph::shortest_path_between`] responde "¿cómo se conecta A con B?" con
//! el camino, no solo con su largo. Es un BFS por saltos (sin pesos) con el
//! mismo recorrido que [`Graph::neighborhood`]: lee los ids de arista de la
//! adyacencia en RAM, filtra por tipo ANTES de leer el nodo del otro
//! extremo y nunca recorre todas las aristas del grafo.
//!
//! Por qué no se reutilizan los que ya existían:
//! - [`Graph::shortest_path`] devuelve solo ids de nodo, no filtra por tipo
//!   e ignora `max_depth`; cambiar su firma rompería a quien lo usa.
//! - `algorithms::ShortestPath::find_path` empieza con `get_all_edges()`.
//!
//! Se guarda la arista por la que se llegó a cada nodo, no solo el nodo
//! anterior: con varias aristas entre el mismo par, los nodos solos no dicen
//! por cuál se pasó.

use std::collections::{HashMap, HashSet};

use super::{Direction, Graph};
use crate::error::Result;
use crate::types::{Edge, Node, NodeId};

/// Cómo buscar el camino. `Default`: aristas salientes de cualquier tipo,
/// sin límite de saltos.
#[derive(Debug, Clone)]
pub struct PathOptions {
    pub direction: Direction,
    /// Solo aristas de estos tipos; `None` = todas.
    pub edge_types: Option<Vec<String>>,
    /// Máximo de saltos; un camino más largo cuenta como inexistente.
    /// `None` = sin límite.
    pub max_depth: Option<usize>,
}

impl Default for PathOptions {
    fn default() -> Self {
        Self { direction: Direction::Outgoing, edge_types: None, max_depth: None }
    }
}

/// Lo que devuelve [`Graph::shortest_path_between`].
#[derive(Debug, Clone)]
pub struct GraphPath {
    /// Del origen al destino, ambos incluidos.
    pub nodes: Vec<Node>,
    /// `edges[i]` une `nodes[i]` con `nodes[i + 1]`. Cada arista va como está
    /// guardada: con `Incoming` o `Both` su `source` puede ser `nodes[i + 1]`.
    pub edges: Vec<Edge>,
}

impl GraphPath {
    /// Saltos del camino (`edges.len()`); 0 cuando origen y destino son el
    /// mismo nodo.
    pub fn len(&self) -> usize {
        self.edges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }
}

impl Graph {
    /// Camino más corto, en saltos, de `start` a `target`, con sus nodos y
    /// aristas. Ver el doc del módulo.
    ///
    /// `None` si no hay camino dentro de `max_depth`, o si `start` o `target`
    /// no existen. `start == target` da el nodo solo y ninguna arista. Entre
    /// varios caminos del mismo largo devuelve el primero que alcanza el BFS,
    /// que depende del orden de adyacencia (el de inserción de las aristas).
    pub async fn shortest_path_between(
        &self,
        start: NodeId,
        target: NodeId,
        opts: &PathOptions,
    ) -> Result<Option<GraphPath>> {
        let ends = self.get_nodes(&[start, target]).await?;
        if ends.iter().any(Option::is_none) {
            return Ok(None);
        }
        if start == target {
            return Ok(Some(GraphPath { nodes: ends.into_iter().flatten().take(1).collect(), edges: Vec::new() }));
        }

        // Para cada nodo alcanzado: de qué nodo y por qué arista se llegó.
        let mut came_from: HashMap<NodeId, (NodeId, Edge)> = HashMap::new();
        let mut seen: HashSet<NodeId> = HashSet::from([start]);
        let mut frontier = vec![start];
        let mut depth = 0;
        'levels: while !frontier.is_empty() && opts.max_depth.is_none_or(|max| depth < max) {
            depth += 1;
            let mut next = Vec::new();
            for src in std::mem::take(&mut frontier) {
                let edge_ids = self.adjacent_edge_ids(src, opts.direction, None).await;
                for edge in self.get_edges(&edge_ids).await?.into_iter().flatten() {
                    if let Some(types) = &opts.edge_types
                        && !types.contains(&edge.edge_type)
                    {
                        continue;
                    }
                    let other = if edge.source == src { edge.target } else { edge.source };
                    if !seen.insert(other) {
                        continue;
                    }
                    came_from.insert(other, (src, edge));
                    if other == target {
                        break 'levels;
                    }
                    next.push(other);
                }
            }
            frontier = next;
        }
        if !came_from.contains_key(&target) {
            return Ok(None);
        }

        let mut ids = vec![target];
        let mut edges = Vec::new();
        let mut current = target;
        while current != start {
            let (prev, edge) = came_from.remove(&current).expect("every reached node has a predecessor");
            edges.push(edge);
            ids.push(prev);
            current = prev;
        }
        ids.reverse();
        edges.reverse();
        // Un nodo del camino borrado entre la búsqueda y esta lectura deja
        // un camino que ya no existe.
        let nodes: Option<Vec<Node>> = self.get_nodes(&ids).await?.into_iter().collect();
        Ok(nodes.map(|nodes| GraphPath { nodes, edges }))
    }
}
