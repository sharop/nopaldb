//! `Graph::neighborhood` (0.6.8): el subgrafo de contexto de un GraphRAG en
//! una llamada. BFS por nodo con visitados global, filtros por tipo y
//! etiqueta, topes de nodos y de aristas por nodo, y `get_nodes`/`get_edges`
//! por lote.

use std::collections::HashSet;

use nopaldb::{Direction, Edge, ExpandOptions, Graph, Node, NodeId, PropertyValue};

struct Fixture {
    g: Graph,
    a: NodeId,
    b: NodeId,
    c: NodeId,
    d: NodeId,
    e: NodeId,
    f: NodeId,
    other: NodeId,
}

/// a→b→c→d, a→e (RELATED), f→b, self-loop c→c, b→other (label Other), a→d (diamante).
async fn fixture() -> Fixture {
    let g = Graph::in_memory().await.unwrap();
    let mk = |name: &str| Node::new("Node").with_property("name", PropertyValue::String(name.into()));
    let a = g.add_node(mk("a")).await.unwrap();
    let b = g.add_node(mk("b")).await.unwrap();
    let c = g.add_node(mk("c")).await.unwrap();
    let d = g.add_node(mk("d")).await.unwrap();
    let e = g.add_node(mk("e")).await.unwrap();
    let f = g.add_node(mk("f")).await.unwrap();
    let other = g.add_node(Node::new("Other")).await.unwrap();
    for (s, t, ty) in [(a, b, "KNOWS"), (b, c, "KNOWS"), (c, d, "KNOWS"), (a, e, "RELATED"), (f, b, "KNOWS"), (c, c, "KNOWS"), (b, other, "KNOWS"), (a, d, "KNOWS")] {
        g.add_edge(Edge::new(s, t, ty).with_property("w", PropertyValue::Int(1))).await.unwrap();
    }
    Fixture { g, a, b, c, d, e, f, other }
}

fn ids(nb: &nopaldb::Neighborhood) -> HashSet<NodeId> {
    nb.nodes.iter().map(|n| n.id).collect()
}

fn assert_edges_closed(nb: &nopaldb::Neighborhood) {
    let seen: HashSet<_> = nb.edges.iter().map(|e| e.id).collect();
    assert_eq!(seen.len(), nb.edges.len(), "each edge once");
    for e in &nb.edges {
        assert!(nb.depth_of.contains_key(&e.source) && nb.depth_of.contains_key(&e.target), "edge {} has an endpoint outside the result", e.id);
    }
    assert_eq!(nb.depth_of.len(), nb.nodes.len());
}

#[tokio::test]
async fn depth_zero_returns_only_existing_seeds_once() {
    let fx = fixture().await;
    let nb = fx.g.neighborhood(&[fx.a, uuid::Uuid::new_v4(), fx.a], 0, &ExpandOptions::default()).await.unwrap();
    assert_eq!(nb.nodes.len(), 1);
    assert_eq!(nb.depth_of[&fx.a], 0);
    assert!(nb.edges.is_empty() && !nb.truncated);
}

#[tokio::test]
async fn one_hop_outgoing_and_minimum_depth_on_a_diamond() {
    let fx = fixture().await;
    let nb = fx.g.neighborhood(&[fx.a], 1, &ExpandOptions::default()).await.unwrap();
    assert_eq!(ids(&nb), [fx.a, fx.b, fx.e, fx.d].into_iter().collect());
    assert_eq!(nb.edges.len(), 3);
    assert_edges_closed(&nb);

    let nb = fx.g.neighborhood(&[fx.a], 2, &ExpandOptions::default()).await.unwrap();
    assert_eq!(nb.depth_of[&fx.d], 1, "reachable by a→d and a→b→… : minimum depth wins");
    assert_eq!(nb.depth_of[&fx.c], 2);
    assert!(nb.depth_of.contains_key(&fx.other));
    assert_edges_closed(&nb);
    // Nodos con la propiedad: la hidratación viene incluida.
    assert!(nb.nodes.iter().all(|n| n.label == "Other" || n.properties.contains_key("name")));
}

#[tokio::test]
async fn filters_by_edge_type_and_label() {
    let fx = fixture().await;
    let opts = ExpandOptions { edge_types: Some(vec!["RELATED".into()]), ..Default::default() };
    let nb = fx.g.neighborhood(&[fx.a], 2, &opts).await.unwrap();
    assert_eq!(ids(&nb), [fx.a, fx.e].into_iter().collect());
    assert_eq!(nb.edges.len(), 1);

    let opts = ExpandOptions { labels: Some(vec!["Node".into()]), ..Default::default() };
    let nb = fx.g.neighborhood(&[fx.b], 2, &opts).await.unwrap();
    assert!(!nb.depth_of.contains_key(&fx.other), "filtered label is not returned");
    assert!(nb.edges.iter().all(|e| e.target != fx.other));
    assert_edges_closed(&nb);
}

#[tokio::test]
async fn max_nodes_truncates_and_max_edges_per_node_brakes_supernodes() {
    let fx = fixture().await;
    let opts = ExpandOptions { max_nodes: 2, ..Default::default() };
    let nb = fx.g.neighborhood(&[fx.a], 3, &opts).await.unwrap();
    assert!(nb.truncated);
    assert_eq!(nb.nodes.len(), 2);
    assert_edges_closed(&nb);

    let opts = ExpandOptions { max_edges_per_node: Some(1), ..Default::default() };
    let nb = fx.g.neighborhood(&[fx.a], 1, &opts).await.unwrap();
    assert_eq!(nb.nodes.len(), 2, "seed + one neighbour");
    assert!(!nb.truncated);
}

#[tokio::test]
async fn both_directions_dedup_self_loops_and_incoming_works() {
    let fx = fixture().await;
    let opts = ExpandOptions { direction: Direction::Both, ..Default::default() };
    let nb = fx.g.neighborhood(&[fx.c], 1, &opts).await.unwrap();
    assert_eq!(ids(&nb), [fx.c, fx.b, fx.d].into_iter().collect());
    let loops = nb.edges.iter().filter(|e| e.source == fx.c && e.target == fx.c).count();
    assert_eq!(loops, 1, "the self-loop is in both adjacency lists but counts once");
    assert_edges_closed(&nb);

    let opts = ExpandOptions { direction: Direction::Incoming, ..Default::default() };
    let nb = fx.g.neighborhood(&[fx.b], 1, &opts).await.unwrap();
    assert_eq!(ids(&nb), [fx.b, fx.a, fx.f].into_iter().collect());
}

#[tokio::test]
async fn batch_getters_keep_order_and_report_missing() {
    let fx = fixture().await;
    let missing = uuid::Uuid::new_v4();
    let nodes = fx.g.get_nodes(&[fx.a, missing, fx.b, fx.a]).await.unwrap();
    assert_eq!(nodes.iter().map(|n| n.as_ref().map(|n| n.id)).collect::<Vec<_>>(), vec![Some(fx.a), None, Some(fx.b), Some(fx.a)]);
    let eid = fx.g.edges_of(fx.a, Direction::Outgoing).await.unwrap()[0].id;
    let edges = fx.g.get_edges(&[missing, eid]).await.unwrap();
    assert!(edges[0].is_none() && edges[1].as_ref().unwrap().id == eid);
    assert!(fx.g.get_nodes(&[]).await.unwrap().is_empty());
    let _ = fx.e;
}
