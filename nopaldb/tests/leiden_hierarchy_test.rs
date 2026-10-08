// #190 b: jerarquía de comunidades de Leiden.
//
// Grafo con jerarquía sembrada: bloques de 50 nodos, cada uno con 5
// subbloques de 10. Dentro de un subbloque la probabilidad de arista es 0.8;
// entre subbloques del mismo bloque, 0.15; además ~0.5 aristas por nodo
// hacia cualquier nodo. Con γ = 0.1 conviene el bloque entero (0.15 > γ);
// con γ = 0.3 (nivel 1, factor 3) solo los subbloques (0.15 < γ < 0.8).
// Por azar, unos pocos nodos tienen más aristas fuera de su grupo que dentro
// y Leiden los pone donde CPM manda; por eso se exige recuperar exactos casi
// todos los grupos, no todos. Una comunidad del nivel 1 con más de 10 nodos
// se parte en el nivel 2 (γ = 0.9), así que la profundidad es 2 o 3.

use std::collections::{HashMap, HashSet};

use nopaldb::algorithms::community::{LeidenCommunity, LeidenConfig, LeidenHierarchy, LeidenHierarchyOptions};
use nopaldb::types::{Edge, Node, NodeId};
use nopaldb::Graph;

const BLOCK: usize = 50;
const SUB: usize = 10;

fn rng(seed: u64) -> impl FnMut(usize) -> usize {
    let mut x = seed;
    move |m| {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as usize % m
    }
}

async fn planted_hierarchy(n: usize) -> (Graph, Vec<NodeId>) {
    let g = Graph::in_memory().await.unwrap();
    let mut ids = Vec::with_capacity(n);
    let mut loader = g.bulk_loader(50_000);
    for _ in 0..n {
        let node = Node::new("Entity");
        ids.push(node.id);
        loader.add_node(node).await.unwrap();
    }
    let mut r = rng(0x9E37_79B9_7F4A_7C15);
    for i in 0..n {
        for j in (i + 1)..((i / BLOCK + 1) * BLOCK).min(n) {
            let p = if i / SUB == j / SUB { 80 } else { 15 };
            if r(100) < p {
                loader.add_edge(Edge::new(ids[i], ids[j], "RELATED")).await.unwrap();
            }
        }
        if r(2) == 0 {
            let j = r(n);
            if j != i {
                loader.add_edge(Edge::new(ids[i], ids[j], "RELATED")).await.unwrap();
            }
        }
    }
    loader.finish().await.unwrap();
    (g, ids)
}

fn options() -> LeidenHierarchyOptions {
    LeidenHierarchyOptions { resolution_factor: 3.0, ..Default::default() }
}

/// Al menos `min_fraction` de los grupos de `group` nodos son exactamente
/// una comunidad del nivel.
fn assert_recovers(level: &HashMap<NodeId, usize>, ids: &[NodeId], group: usize, min_fraction: f64) {
    let mut members: HashMap<usize, HashSet<usize>> = HashMap::new();
    for (i, id) in ids.iter().enumerate() {
        members.entry(level[id]).or_default().insert(i);
    }
    let groups = ids.len() / group;
    let exact = (0..groups)
        .filter(|g| members[&level[&ids[g * group]]] == (g * group..(g + 1) * group).collect())
        .count();
    let fraction = exact as f64 / groups as f64;
    assert!(fraction >= min_fraction, "grupos de {group} recuperados exactos: {exact}/{groups}");
}

fn assert_nested(h: &LeidenHierarchy) {
    for level in 1..h.depth() {
        for (id, &c) in &h.levels[level] {
            assert_eq!(h.parent(level, c), Some(h.levels[level - 1][id]), "nivel {level}: {id} fuera de su padre");
        }
    }
    assert_eq!(h.parent(0, 0), None);
}

#[tokio::test]
async fn recovers_planted_blocks_and_sub_blocks_10k() {
    let (g, ids) = planted_hierarchy(10_000).await;
    let leiden = LeidenCommunity::new(LeidenConfig::default());
    let h = leiden.detect_hierarchy(&g, &options()).await.unwrap();
    assert!((2..=3).contains(&h.depth()), "profundidad {}", h.depth());
    assert_eq!(h.levels[0], leiden.detect(&g).await.unwrap(), "el nivel 0 es detect()");
    assert_nested(&h);
    assert_recovers(&h.levels[0], &ids, BLOCK, 0.95);
    assert_recovers(&h.levels[1], &ids, SUB, 0.9);
}

#[tokio::test]
async fn hierarchy_is_deterministic() {
    let (g, _) = planted_hierarchy(2_000).await;
    let leiden = LeidenCommunity::new(LeidenConfig::default());
    let a = leiden.detect_hierarchy(&g, &options()).await.unwrap();
    let b = leiden.detect_hierarchy(&g, &options()).await.unwrap();
    assert_eq!(a.levels, b.levels);
}

#[tokio::test]
async fn small_communities_give_a_single_level() {
    let (g, _) = planted_hierarchy(2_000).await;
    let leiden = LeidenCommunity::new(LeidenConfig::default());
    let opts = LeidenHierarchyOptions { max_cluster_size: BLOCK, ..options() };
    let h = leiden.detect_hierarchy(&g, &opts).await.unwrap();
    assert_eq!(h.depth(), 1, "ningún bloque supera max_cluster_size");
    let opts = LeidenHierarchyOptions { max_levels: 1, ..options() };
    assert_eq!(leiden.detect_hierarchy(&g, &opts).await.unwrap().depth(), 1);
}

#[tokio::test]
async fn empty_graph_gives_an_empty_hierarchy() {
    let g = Graph::in_memory().await.unwrap();
    let h = LeidenCommunity::with_defaults().detect_hierarchy(&g, &options()).await.unwrap();
    assert_eq!(h.depth(), 0);
}

#[tokio::test]
async fn invalid_options_are_rejected() {
    let g = Graph::in_memory().await.unwrap();
    let leiden = LeidenCommunity::with_defaults();
    for (opts, needle) in [
        (LeidenHierarchyOptions { max_cluster_size: 0, ..Default::default() }, "max_cluster_size"),
        (LeidenHierarchyOptions { resolution_factor: 1.0, ..Default::default() }, "resolution_factor"),
        (LeidenHierarchyOptions { resolution_factor: f64::NAN, ..Default::default() }, "resolution_factor"),
        (LeidenHierarchyOptions { max_levels: 0, ..Default::default() }, "max_levels"),
    ] {
        let err = leiden.detect_hierarchy(&g, &opts).await.unwrap_err().to_string();
        assert!(err.contains(needle), "{err}");
    }
}

