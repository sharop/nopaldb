// tests/shortest_path_edges_test.rs
//
// `Graph::shortest_path_between` (#416): el camino más corto con sus nodos
// Y sus aristas. Con varias aristas entre el mismo par, la arista dice por
// cuál se pasó; `edge_types`, `direction` y `max_depth` filtran como en
// `neighborhood`.
//
// Dominio ficticio: un zoo.
//   michi -COME-> raton, michi -JUEGA_CON-> raton   (dos aristas, mismo par)
//   raton -COME-> queso
//   michi -PERSIGUE-> pajaro -PERSIGUE-> mosca -PERSIGUE-> queso
//   firulais (sin aristas)

use std::collections::HashMap;

use nopaldb::types::{EdgeId, NodeId};
use nopaldb::{Direction, Edge, Graph, GraphPath, Node, PathOptions, PropertyValue, Result};

struct Zoo {
    g: Graph,
    id: HashMap<&'static str, NodeId>,
    edge: HashMap<(&'static str, &'static str, &'static str), EdgeId>,
}

async fn zoo() -> Result<Zoo> {
    let g = Graph::in_memory().await?;
    let mut id = HashMap::new();
    for nombre in ["michi", "raton", "queso", "pajaro", "mosca", "firulais"] {
        let n = g.add_node(Node::new("Animal").with_property("nombre", PropertyValue::String(nombre.into()))).await?;
        id.insert(nombre, n);
    }
    let mut edge = HashMap::new();
    for (from, kind, to) in [
        ("michi", "COME", "raton"),
        ("michi", "JUEGA_CON", "raton"),
        ("raton", "COME", "queso"),
        ("michi", "PERSIGUE", "pajaro"),
        ("pajaro", "PERSIGUE", "mosca"),
        ("mosca", "PERSIGUE", "queso"),
    ] {
        edge.insert((from, kind, to), g.add_edge(Edge::new(id[from], id[to], kind)).await?);
    }
    Ok(Zoo { g, id, edge })
}

impl Zoo {
    async fn path(&self, from: &str, to: &str, opts: PathOptions) -> Option<GraphPath> {
        self.g.shortest_path_between(self.id[from], self.id[to], &opts).await.unwrap()
    }

    fn names(&self, p: &GraphPath) -> Vec<String> {
        p.nodes
            .iter()
            .map(|n| match n.properties.get("nombre") {
                Some(PropertyValue::String(s)) => s.clone(),
                other => panic!("{other:?}"),
            })
            .collect()
    }
}

fn types(types: &[&str]) -> PathOptions {
    PathOptions { edge_types: Some(types.iter().map(|t| t.to_string()).collect()), ..Default::default() }
}

#[tokio::test]
async fn the_path_has_its_nodes_and_edges() -> Result<()> {
    let z = zoo().await?;
    let p = z.path("michi", "queso", PathOptions::default()).await.expect("hay camino");
    assert_eq!(z.names(&p), ["michi", "raton", "queso"]);
    assert_eq!(p.len(), 2);
    assert_eq!(p.edges.len(), p.nodes.len() - 1);
    for (i, e) in p.edges.iter().enumerate() {
        assert_eq!((e.source, e.target), (p.nodes[i].id, p.nodes[i + 1].id), "edges[{i}] une nodes[{i}] y nodes[{}]", i + 1);
    }
    Ok(())
}

/// Lo que pide el issue: entre michi y raton hay dos aristas, y el camino
/// dice cuál tomó; el filtro de tipo elige.
#[tokio::test]
async fn with_two_edges_between_a_pair_the_edge_says_which() -> Result<()> {
    let z = zoo().await?;
    let juega = z.path("michi", "raton", types(&["JUEGA_CON"])).await.expect("hay camino");
    assert_eq!(juega.edges[0].id, z.edge[&("michi", "JUEGA_CON", "raton")]);
    assert_eq!(juega.edges[0].edge_type, "JUEGA_CON");
    let come = z.path("michi", "raton", types(&["COME"])).await.expect("hay camino");
    assert_eq!(come.edges[0].id, z.edge[&("michi", "COME", "raton")]);
    // Sin filtro: la primera en orden de adyacencia (la de inserción).
    let any = z.path("michi", "raton", PathOptions::default()).await.expect("hay camino");
    assert_eq!(any.edges[0].id, z.edge[&("michi", "COME", "raton")]);
    // También cuando raton está a mitad del camino.
    let via = z.path("michi", "queso", PathOptions::default()).await.expect("hay camino");
    assert_eq!(via.edges[0].id, z.edge[&("michi", "COME", "raton")]);
    let via = z.path("michi", "queso", types(&["JUEGA_CON", "COME"])).await.expect("hay camino");
    assert_eq!(via.edges[0].id, z.edge[&("michi", "COME", "raton")]);
    Ok(())
}

#[tokio::test]
async fn edge_types_change_the_route() -> Result<()> {
    let z = zoo().await?;
    let p = z.path("michi", "queso", types(&["PERSIGUE"])).await.expect("hay camino");
    assert_eq!(z.names(&p), ["michi", "pajaro", "mosca", "queso"]);
    assert!(p.edges.iter().all(|e| e.edge_type == "PERSIGUE"));
    assert!(z.path("michi", "queso", types(&["JUEGA_CON"])).await.is_none());
    Ok(())
}

#[tokio::test]
async fn direction_and_max_depth() -> Result<()> {
    let z = zoo().await?;
    // Saliente no hay camino de vuelta; entrante y ambas, sí.
    assert!(z.path("queso", "michi", PathOptions::default()).await.is_none());
    for direction in [Direction::Incoming, Direction::Both] {
        let p = z.path("queso", "michi", PathOptions { direction, ..Default::default() }).await.expect("hay camino");
        assert_eq!(z.names(&p), ["queso", "raton", "michi"], "{direction:?}");
        // La arista va como está guardada: raton -> queso.
        assert_eq!((p.edges[0].source, p.edges[0].target), (z.id["raton"], z.id["queso"]));
    }
    // max_depth: el camino por PERSIGUE tiene 3 saltos.
    let persigue = |max_depth| PathOptions { max_depth, ..types(&["PERSIGUE"]) };
    assert!(z.path("michi", "queso", persigue(Some(2))).await.is_none());
    assert_eq!(z.path("michi", "queso", persigue(Some(3))).await.map(|p| p.len()), Some(3));
    assert_eq!(z.path("michi", "raton", PathOptions { max_depth: Some(0), ..Default::default() }).await.map(|p| p.len()), None);
    Ok(())
}

#[tokio::test]
async fn same_node_no_path_and_missing_nodes() -> Result<()> {
    let z = zoo().await?;
    let same = z.path("michi", "michi", PathOptions::default()).await.expect("el nodo solo");
    assert_eq!((z.names(&same), same.len()), (vec!["michi".to_string()], 0));
    assert!(same.edges.is_empty());
    assert!(z.path("michi", "firulais", PathOptions { direction: Direction::Both, ..Default::default() }).await.is_none());
    let ghost = NodeId::new_v4();
    assert!(z.g.shortest_path_between(z.id["michi"], ghost, &PathOptions::default()).await?.is_none());
    assert!(z.g.shortest_path_between(ghost, ghost, &PathOptions::default()).await?.is_none());
    Ok(())
}
