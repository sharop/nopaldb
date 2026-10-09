// #184: todo punto del índice HNSW tiene que ser alcanzable por la búsqueda.
//
// La poda heurística de vecinos de HNSW podía dejar un punto con enlaces
// salientes pero sin entrantes: estaba en el índice y ninguna búsqueda lo
// devolvía, ni con `ef` igual al tamaño del índice. Medido antes del
// arreglo: ~0.5% de los puntos de un build de 3000 y 1.35% de una muestra a
// 100k. Una pasada de verificación reinserta los que no se encuentran a sí
// mismos.
//
// #201: el arreglo original activaba además `keep_pruned`, que a escala hace
// lo contrario: deja sin enlaces entrantes a un tercio de los puntos (100k ×
// 384), la verificación los reinserta en cascada y cada búsqueda pagaba
// todas esas copias muertas. `a_large_clustered_build_needs_few_repairs`
// falla si vuelve.
//
// Garantías que se afirman (y las que no):
// - tras `build_batch`, TODOS los puntos son alcanzables;
// - tras `insert`, el punto recién insertado es alcanzable en ese momento.
//   Una inserción posterior aún puede podar el último enlace que llegaba a
//   un punto anterior; medido: 0 de 10 000 inserciones (384 dims) y 1 de
//   10 000 (16 dims). Eso no se afirma aquí porque no es una garantía.

use nopaldb::embeddings::HnswIndex;
use nopaldb::types::NodeId;

const DIM: usize = 32;

fn vector(seed: u64) -> Vec<f32> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..DIM)
        .map(|_| {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            ((x >> 40) as f32 / (1u64 << 24) as f32) - 0.5
        })
        .collect()
}

/// Puntos que no aparecen entre los 10 primeros buscando su propio vector
/// con `ef` igual al tamaño del índice (la búsqueda más ancha posible).
fn unreachable(index: &HnswIndex, points: &[(NodeId, Vec<f32>)]) -> Vec<NodeId> {
    let ef = index.len().max(10);
    points
        .iter()
        .filter(|(id, v)| !index.search_knn_with_ef(v, 10, ef).unwrap().iter().any(|(h, _)| h == id))
        .map(|(id, _)| *id)
        .collect()
}

#[test]
fn every_point_of_a_build_is_reachable() {
    for seed in 0..3u64 {
        let points: Vec<(NodeId, Vec<f32>)> =
            (0..3000).map(|i| (NodeId::new_v4(), vector(seed * 1_000_000 + i))).collect();
        let index = HnswIndex::build_batch(points.clone(), "m", DIM).unwrap();
        assert_eq!(unreachable(&index, &points), Vec::<NodeId>::new(), "semilla {seed}");
    }
}

#[test]
fn a_freshly_inserted_point_is_reachable() {
    let points: Vec<(NodeId, Vec<f32>)> = (0..2000).map(|i| (NodeId::new_v4(), vector(i))).collect();
    let mut index = HnswIndex::build_batch(points, "m", DIM).unwrap();
    for i in 0..500u64 {
        let point = (NodeId::new_v4(), vector(5_000_000 + i));
        index.insert(point.0, point.1.clone()).unwrap();
        assert_eq!(unreachable(&index, std::slice::from_ref(&point)), Vec::<NodeId>::new(), "inserción {i}");
    }
}

/// Datos agrupados como embeddings reales: grupos con un centroide y un
/// subespacio propio de dimensión baja.
fn clustered(n: usize, seed: u64) -> Vec<(NodeId, Vec<f32>)> {
    const CLUSTERS: u64 = 50;
    const SUBSPACE: u64 = 6;
    let unit = |s: u64| {
        let v = vector(s);
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.into_iter().map(|x| x / norm).collect::<Vec<f32>>()
    };
    (0..n as u64)
        .map(|i| {
            let c = i % CLUSTERS;
            let z = vector(seed ^ (i << 16));
            let mut v = unit(seed ^ (c << 40));
            for b in 0..SUBSPACE {
                let dir = unit(seed ^ (c << 40) ^ ((b + 1) << 32));
                v.iter_mut().zip(dir).for_each(|(x, d)| *x += 0.8 * z[b as usize] * d);
            }
            (NodeId::new_v4(), v)
        })
        .collect()
}

/// #201: a escala, casi todos los puntos tienen que quedar en el grafo.
/// Con `keep_pruned` (el arreglo original de #184), un tercio quedaba sin
/// enlaces entrantes a 100k; a esta escala, más del 5%.
#[test]
fn a_large_clustered_build_leaves_few_orphans() {
    let n = 20_000;
    let points = clustered(n, 7);
    let index = HnswIndex::build_batch(points.clone(), "m", DIM).unwrap();
    assert!(
        index.orphans() * 100 <= n * 2,
        "{} de {n} puntos sin enlaces entrantes (#201)",
        index.orphans()
    );
    // Con `ef` = 64 (el de la verificación), cada punto se encuentra: por el
    // grafo o, si es huérfano, por la comparación directa.
    for (i, (id, v)) in points.iter().enumerate().step_by(10) {
        let hits = index.search_knn_with_ef(v, 10, 64).unwrap();
        assert!(hits.iter().any(|(h, _)| h == id), "punto {i} no aparece");
        assert_eq!(hits.len(), 10);
    }
}
