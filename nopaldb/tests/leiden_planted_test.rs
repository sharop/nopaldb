// #194: Leiden sobre grafos con comunidades sembradas (bloques de 50 nodos).
//
// Hasta 0.6.10 el núcleo reutilizaba ids de comunidad entre iteraciones (el
// tamaño de una comunidad bajaba de cero: pánico en debug, resultado
// corrompido en release), el refinamiento reemplazaba la partición y no había
// agregación. Estos tests corren en debug en el CI.
//
// Dos criterios objetivos:
// - **Bloques densos** (probabilidad 0.3 dentro del bloque, muy por encima de
//   γ = 0.1): la partición CPM es, salvo nodos sueltos, la de los bloques.
//   Se exige que ninguna comunidad mezcle bloques, que cada bloque tenga una
//   comunidad con al menos 45 de sus 50 nodos y calidad CPM ≥ la sembrada.
//   No se exige recuperación exacta: un nodo con pocas aristas en su bloque
//   (p. ej. 4 en un bloque de 50) queda solo porque unirlo cuesta
//   4 − γ·49 < 0; con 10k nodos pasa en 2 bloques y la calidad de Leiden
//   supera a la sembrada. `main` daba 362 comunidades para 40 bloques.
// - **Bloques dispersos** (~4 aristas por nodo, densidad ≈ 0.155, cerca de
//   γ): las fluctuaciones forman subgrupos más densos y partir un bloque
//   puede MEJORAR la calidad CPM, así que no se exige un bloque por
//   comunidad. Se exige que la calidad CPM de Leiden sea al menos la de la
//   partición sembrada. `main` daba 1482 contra 2390 de la sembrada.

use std::collections::{HashMap, HashSet};

use nopaldb::algorithms::community::{LeidenCommunity, LeidenConfig};
use nopaldb::types::{Edge, Node, NodeId};
use nopaldb::Graph;

const BLOCK: usize = 50;
const GAMMA: f64 = 0.1;

fn rng(seed: u64) -> impl FnMut(usize) -> usize {
    let mut x = seed;
    move |m| {
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as usize % m
    }
}

/// Grafo de `n` nodos en bloques de 50 y sus pares no dirigidos (índices).
async fn planted(n: usize, dense: bool) -> (Graph, Vec<NodeId>, HashSet<(usize, usize)>) {
    let g = Graph::in_memory().await.unwrap();
    let mut ids = Vec::with_capacity(n);
    let mut pairs = HashSet::new();
    let mut loader = g.bulk_loader(50_000);
    for _ in 0..n {
        let node = Node::new("Entity");
        ids.push(node.id);
        loader.add_node(node).await.unwrap();
    }
    let mut r = rng(0x9E37_79B9_7F4A_7C15);
    for i in 0..n {
        let block = i / BLOCK;
        if dense {
            for j in (i + 1)..((block + 1) * BLOCK).min(n) {
                if r(100) < 30 {
                    pairs.insert((i, j));
                    loader.add_edge(Edge::new(ids[i], ids[j], "RELATED")).await.unwrap();
                }
            }
        } else {
            for _ in 0..4 {
                let j = block * BLOCK + r(BLOCK);
                if j < n && j != i {
                    pairs.insert((i.min(j), i.max(j)));
                    loader.add_edge(Edge::new(ids[i], ids[j], "RELATED")).await.unwrap();
                }
            }
        }
        if r(2) == 0 {
            let j = r(n);
            if j != i {
                pairs.insert((i.min(j), i.max(j)));
                loader.add_edge(Edge::new(ids[i], ids[j], "RELATED")).await.unwrap();
            }
        }
    }
    loader.finish().await.unwrap();
    (g, ids, pairs)
}

/// Calidad CPM: Σ_C [aristas internas − γ·n_C·(n_C − 1)/2].
fn cpm(part: &[usize], pairs: &HashSet<(usize, usize)>) -> f64 {
    let mut size: HashMap<usize, f64> = HashMap::new();
    for &c in part {
        *size.entry(c).or_default() += 1.0;
    }
    let inside = pairs.iter().filter(|(a, b)| part[*a] == part[*b]).count() as f64;
    inside - GAMMA * size.values().map(|n| n * (n - 1.0) / 2.0).sum::<f64>()
}

async fn leiden(g: &Graph, ids: &[NodeId]) -> Vec<usize> {
    let parts = LeidenCommunity::new(LeidenConfig::default()).detect(g).await.unwrap();
    assert_eq!(parts.len(), ids.len(), "todo nodo tiene comunidad");
    ids.iter().map(|id| parts[id]).collect()
}

async fn assert_dense_blocks_recovered(n: usize) {
    let (g, ids, pairs) = planted(n, true).await;
    let part = leiden(&g, &ids).await;
    let mut members_of: HashMap<(usize, usize), usize> = HashMap::new();
    let mut blocks_of_comm: HashMap<usize, HashSet<usize>> = HashMap::new();
    for (i, &c) in part.iter().enumerate() {
        *members_of.entry((i / BLOCK, c)).or_default() += 1;
        blocks_of_comm.entry(c).or_default().insert(i / BLOCK);
    }
    let mixed = blocks_of_comm.values().filter(|b| b.len() > 1).count();
    assert_eq!(mixed, 0, "comunidades que mezclan bloques");
    for block in 0..n / BLOCK {
        let largest = members_of.iter().filter(|((b, _), _)| *b == block).map(|(_, m)| *m).max().unwrap_or(0);
        assert!(largest >= 45, "el bloque {block} no tiene una comunidad con ≥ 45 de sus 50 nodos (la mayor: {largest})");
    }
    let planted: Vec<usize> = (0..n).map(|i| i / BLOCK).collect();
    let (q_leiden, q_planted) = (cpm(&part, &pairs), cpm(&planted, &pairs));
    assert!(q_leiden >= q_planted, "calidad CPM de Leiden {q_leiden:.1} < la sembrada {q_planted:.1}");
}

async fn assert_quality_at_least_planted(n: usize) {
    let (g, ids, pairs) = planted(n, false).await;
    let part = leiden(&g, &ids).await;
    let planted: Vec<usize> = (0..n).map(|i| i / BLOCK).collect();
    let (q_leiden, q_planted) = (cpm(&part, &pairs), cpm(&planted, &pairs));
    assert!(q_leiden >= q_planted, "calidad CPM de Leiden {q_leiden:.1} < la de los bloques sembrados {q_planted:.1}");
}

#[tokio::test]
async fn dense_blocks_are_recovered_2k() {
    assert_dense_blocks_recovered(2_000).await;
}

#[tokio::test]
async fn dense_blocks_are_recovered_10k() {
    assert_dense_blocks_recovered(10_000).await;
}

#[tokio::test]
async fn sparse_blocks_quality_is_at_least_the_planted_partition_2k() {
    assert_quality_at_least_planted(2_000).await;
}

#[tokio::test]
async fn sparse_blocks_quality_is_at_least_the_planted_partition_10k() {
    assert_quality_at_least_planted(10_000).await;
}
