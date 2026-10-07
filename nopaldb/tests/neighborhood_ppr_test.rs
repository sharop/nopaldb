// #176: `neighborhood` con `Rank::Ppr` decide qué sobrevive a `max_nodes`
// por relevancia (Personalized PageRank sembrado en las semillas) y no por el
// orden del BFS.

use std::collections::HashMap;

use nopaldb::types::{Edge, Node, NodeId, PropertyValue};
use nopaldb::{Direction, ExpandOptions, Graph, Rank};

async fn node(g: &Graph, name: &str) -> NodeId {
    g.add_node(Node::new("N").with_property("name", PropertyValue::String(name.into()))).await.unwrap()
}

fn names(nb: &nopaldb::Neighborhood) -> Vec<String> {
    nb.nodes
        .iter()
        .map(|n| n.properties.get("name").unwrap().as_str().unwrap().to_string())
        .collect()
}

/// Tres semillas S1..S3. A cuelga solo de S1 (profundidad 1). B está a dos
/// saltos de cada semilla, por M1..M3 (profundidad 2). Con sitio para las
/// semillas y 4 nodos más, el BFS se queda con el nivel 1 (A y las M) y deja
/// fuera a B; el PPR reconoce que B recibe de las tres semillas y deja fuera
/// a A, que solo recibe de una.
#[tokio::test]
async fn a_depth_two_node_reached_from_many_seeds_beats_a_depth_one_node_reached_from_one() {
    let g = Graph::in_memory().await.unwrap();
    let s: Vec<NodeId> = vec![node(&g, "S1").await, node(&g, "S2").await, node(&g, "S3").await];
    let a = node(&g, "A").await;
    let b = node(&g, "B").await;
    g.add_edge(Edge::new(s[0], a, "R")).await.unwrap();
    for (i, seed) in s.iter().enumerate() {
        let m = node(&g, &format!("M{}", i + 1)).await;
        g.add_edge(Edge::new(*seed, m, "R")).await.unwrap();
        g.add_edge(Edge::new(m, b, "R")).await.unwrap();
    }

    let bfs = ExpandOptions { direction: Direction::Both, max_nodes: 7, ..Default::default() };
    let nb = g.neighborhood(&s, 2, &bfs).await.unwrap();
    assert!(nb.truncated);
    assert!(nb.depth_of.contains_key(&a) && !nb.depth_of.contains_key(&b), "BFS: el nivel 1 llena el cupo");
    assert!(nb.score_of.is_empty(), "BFS no puntúa");

    let ppr = ExpandOptions { rank: Rank::ppr(), ..bfs };
    let nb = g.neighborhood(&s, 2, &ppr).await.unwrap();
    assert!(nb.truncated, "el ranking dejó fuera candidatos");
    assert_eq!(nb.nodes.len(), 7);
    assert!(nb.depth_of.contains_key(&b), "B sobrevive: {:?}", names(&nb));
    assert!(!nb.depth_of.contains_key(&a), "A queda fuera: {:?}", names(&nb));
    assert_eq!(nb.depth_of[&b], 2, "conserva su profundidad");
    assert_eq!(&names(&nb)[..3], ["S1", "S2", "S3"], "semillas primero");
    let tail: Vec<f64> = nb.nodes[3..].iter().map(|n| nb.score_of[&n.id]).collect();
    assert!(tail.windows(2).all(|w| w[0] >= w[1]), "por score descendente: {tail:?}");
    assert!(nb.edges.iter().all(|e| nb.depth_of.contains_key(&e.source) && nb.depth_of.contains_key(&e.target)));
}

/// El mismo grafo con las aristas insertadas en orden inverso da el mismo
/// resultado, nodo por nodo y score por score.
#[tokio::test]
async fn the_ranking_does_not_depend_on_edge_insertion_order() {
    async fn build(reverse: bool) -> (Graph, Vec<NodeId>) {
        let g = Graph::in_memory().await.unwrap();
        let mut ids = Vec::new();
        for i in 0..12 {
            ids.push(node(&g, &format!("n{i:02}")).await);
        }
        let mut edges: Vec<(usize, usize)> = (0..12).flat_map(|i| [(i, (i * 5 + 1) % 12), (i, (i * 7 + 3) % 12)]).collect();
        if reverse {
            edges.reverse();
        }
        for (x, y) in edges {
            g.add_edge(Edge::new(ids[x], ids[y], "R")).await.unwrap();
        }
        (g, ids)
    }
    let opts = ExpandOptions { direction: Direction::Both, max_nodes: 6, rank: Rank::ppr(), ..Default::default() };
    let (g1, ids1) = build(false).await;
    let (g2, ids2) = build(true).await;
    let r1 = g1.neighborhood(&[ids1[0], ids1[4]], 3, &opts).await.unwrap();
    let r2 = g2.neighborhood(&[ids2[0], ids2[4]], 3, &opts).await.unwrap();
    assert_eq!(names(&r1), names(&r2));
    let score = |nb: &nopaldb::Neighborhood, ids: &[NodeId], i: usize| nb.score_of.get(&ids[i]).copied();
    for i in 0..12 {
        let (x, y) = (score(&r1, &ids1, i), score(&r2, &ids2, i));
        assert_eq!(x.is_some(), y.is_some(), "n{i:02}");
        if let (Some(x), Some(y)) = (x, y) {
            assert!((x - y).abs() < 1e-12, "n{i:02}: {x} vs {y}");
        }
    }
}

/// Un empate de score se rompe por `NodeId`, así que el resultado es estable.
#[tokio::test]
async fn ties_break_by_node_id() {
    let g = Graph::in_memory().await.unwrap();
    let seed = node(&g, "S").await;
    let mut leaves = Vec::new();
    for i in 0..6 {
        let leaf = node(&g, &format!("L{i}")).await;
        g.add_edge(Edge::new(seed, leaf, "R")).await.unwrap();
        leaves.push(leaf);
    }
    let opts = ExpandOptions { max_nodes: 4, rank: Rank::ppr(), ..Default::default() };
    let nb = g.neighborhood(&[seed], 1, &opts).await.unwrap();
    let kept: Vec<NodeId> = nb.nodes[1..].iter().map(|n| n.id).collect();
    let mut sorted = leaves.clone();
    sorted.sort();
    assert_eq!(kept, sorted[..3].to_vec(), "mismo score: los de menor id");
    let again = g.neighborhood(&[seed], 1, &opts).await.unwrap();
    assert_eq!(names(&nb), names(&again));
}

/// `seed_weights` reparte el teletransporte: con más peso en una semilla,
/// su vecino sube.
#[tokio::test]
async fn seed_weights_shift_the_ranking() {
    let g = Graph::in_memory().await.unwrap();
    let (s1, s2) = (node(&g, "S1").await, node(&g, "S2").await);
    let (x, y) = (node(&g, "X").await, node(&g, "Y").await);
    g.add_edge(Edge::new(s1, x, "R")).await.unwrap();
    g.add_edge(Edge::new(s2, y, "R")).await.unwrap();
    let opts = |w1: f64, w2: f64| ExpandOptions {
        max_nodes: 3,
        rank: Rank::Ppr {
            alpha: 0.15,
            iterations: 20,
            candidate_factor: 5,
            seed_weights: Some(HashMap::from([(s1, w1), (s2, w2)])),
        },
        ..Default::default()
    };
    let nb = g.neighborhood(&[s1, s2], 1, &opts(5.0, 1.0)).await.unwrap();
    assert_eq!(names(&nb), ["S1", "S2", "X"]);
    let nb = g.neighborhood(&[s1, s2], 1, &opts(1.0, 5.0)).await.unwrap();
    assert_eq!(names(&nb), ["S1", "S2", "Y"]);
}

/// Sin recorte la masa se conserva: los scores suman 1 (la de los nodos sin
/// salida vuelve a las semillas).
#[tokio::test]
async fn without_a_cut_the_scores_sum_to_one() {
    let g = Graph::in_memory().await.unwrap();
    let s = node(&g, "S").await;
    let a = node(&g, "A").await;
    let b = node(&g, "B").await;
    g.add_edge(Edge::new(s, a, "R")).await.unwrap();
    g.add_edge(Edge::new(a, b, "R")).await.unwrap(); // B no tiene salida
    let opts = ExpandOptions { rank: Rank::ppr(), ..Default::default() };
    let nb = g.neighborhood(&[s], 2, &opts).await.unwrap();
    assert!(!nb.truncated);
    let total: f64 = nb.score_of.values().sum();
    assert!((total - 1.0).abs() < 1e-9, "{total}");
}
