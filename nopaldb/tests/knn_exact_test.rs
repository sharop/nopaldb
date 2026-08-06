// Tests M1-6: búsqueda vectorial exacta bajo umbral.
//
// Con N ≤ EXACT_SEARCH_THRESHOLD el índice responde con un scan lineal
// exacto (determinista, tie-break por NodeId) en lugar del grafo HNSW,
// que es no-determinista entre builds en índices chicos. Por encima del
// umbral la lectura sigue yendo por HNSW con `ef_search` configurable.

#![cfg(feature = "embeddings-index")]

use nopaldb::embeddings::{HnswIndex, EXACT_SEARCH_THRESHOLD};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use uuid::Uuid;

/// Distancia coseno de referencia, implementada de forma independiente al
/// índice (acumulación f64, como DistCosine de hnsw_rs).
fn cosine_dist(a: &[f32], b: &[f32]) -> f32 {
    let (mut dot, mut na, mut nb) = (0f64, 0f64, 0f64);
    for (x, y) in a.iter().zip(b) {
        dot += (*x as f64) * (*y as f64);
        na += (*x as f64) * (*x as f64);
        nb += (*y as f64) * (*y as f64);
    }
    if na > 0.0 && nb > 0.0 {
        (1.0 - dot / (na * nb).sqrt()).max(0.0) as f32
    } else {
        0.0
    }
}

fn seeded_vectors(n: usize, dim: usize, rng: &mut StdRng) -> Vec<(Uuid, Vec<f32>)> {
    (0..n)
        .map(|_| {
            let v: Vec<f32> = (0..dim).map(|_| rng.gen_range(-1.0f32..1.0)).collect();
            (Uuid::new_v4(), v)
        })
        .collect()
}

/// (a) Bajo el umbral, `search_knn` == fuerza bruta de referencia:
/// mismo ranking (con tie-break por NodeId) y mismas distancias.
#[test]
fn exact_matches_brute_force_reference() {
    let dim = 16;
    let k = 10;
    for seed in [1u64, 7, 42, 1234, 987_654] {
        let mut rng = StdRng::seed_from_u64(seed);
        let vectors = seeded_vectors(100, dim, &mut rng);
        let index = HnswIndex::build_batch(vectors.clone(), "m", dim).unwrap();

        for q in 0..5 {
            let query: Vec<f32> = (0..dim).map(|_| rng.gen_range(-1.0f32..1.0)).collect();
            let got = index.search_knn(&query, k).unwrap();

            let mut expected: Vec<(Uuid, f32)> = vectors
                .iter()
                .map(|(id, v)| (*id, cosine_dist(&query, v)))
                .collect();
            expected.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
            expected.truncate(k);

            let got_ids: Vec<Uuid> = got.iter().map(|(id, _)| *id).collect();
            let expected_ids: Vec<Uuid> = expected.iter().map(|(id, _)| *id).collect();
            assert_eq!(got_ids, expected_ids, "ranking difiere (seed {seed}, query {q})");
            for ((_, gd), (_, ed)) in got.iter().zip(&expected) {
                assert!(
                    (gd - ed).abs() < 1e-5,
                    "distancia difiere: {gd} vs {ed} (seed {seed}, query {q})"
                );
            }
        }
    }
}

/// (b) Determinismo: la misma query repetida 50 veces devuelve EXACTAMENTE
/// el mismo ranking, y rebuilds independientes del mismo dataset también.
#[test]
fn exact_search_is_deterministic() {
    let dim = 8;
    let mut rng = StdRng::seed_from_u64(2024);
    let vectors = seeded_vectors(100, dim, &mut rng);
    let query: Vec<f32> = (0..dim).map(|_| rng.gen_range(-1.0f32..1.0)).collect();

    let index = HnswIndex::build_batch(vectors.clone(), "m", dim).unwrap();
    let first = index.search_knn(&query, 10).unwrap();
    assert_eq!(first.len(), 10);

    for i in 0..49 {
        let again = index.search_knn(&query, 10).unwrap();
        assert_eq!(again, first, "ranking cambió en la repetición {}", i + 2);
    }

    // Rebuild del mismo dataset → mismo resultado (independiente del grafo HNSW).
    for _ in 0..5 {
        let rebuilt = HnswIndex::build_batch(vectors.clone(), "m", dim).unwrap();
        assert_eq!(rebuilt.search_knn(&query, 10).unwrap(), first);
    }
}

/// (d) Cruce del umbral: un índice con N > EXACT_SEARCH_THRESHOLD sigue
/// funcionando vía HNSW (resultados ordenados, self-match con ef alto).
#[test]
fn above_threshold_still_works_via_hnsw() {
    let dim = 8;
    let n = EXACT_SEARCH_THRESHOLD + 76;
    let mut rng = StdRng::seed_from_u64(7);
    let vectors = seeded_vectors(n, dim, &mut rng);
    let (target_id, target_vec) = (vectors[500].0, vectors[500].1.clone());

    let index = HnswIndex::build_batch(vectors, "m", dim).unwrap();
    assert_eq!(index.len(), n);

    // ef alto para recall alto en la aserción de self-match.
    let results = index.search_knn_with_ef(&target_vec, 10, 400).unwrap();
    assert_eq!(results.len(), 10);
    assert!(
        results.windows(2).all(|w| w[0].1 <= w[1].1),
        "resultados deben venir ordenados por distancia ascendente"
    );
    assert!(
        results.iter().any(|(id, d)| *id == target_id && *d < 1e-4),
        "el propio vector debe aparecer con distancia ~0"
    );
}

/// El camino exacto también cubre la búsqueda filtrada (sin over-fetch:
/// filtra sobre todos los puntos, así el filtro nunca deja resultados fuera).
#[test]
fn exact_filtered_search_sees_all_points() {
    let dim = 8;
    let mut rng = StdRng::seed_from_u64(11);
    let vectors = seeded_vectors(200, dim, &mut rng);
    let query: Vec<f32> = (0..dim).map(|_| rng.gen_range(-1.0f32..1.0)).collect();

    // Permitir solo los 5 vectores MÁS LEJANOS según la referencia: un
    // over-fetch de k*4 no los alcanzaría, el scan exacto sí.
    let mut ranked: Vec<(Uuid, f32)> = vectors
        .iter()
        .map(|(id, v)| (*id, cosine_dist(&query, v)))
        .collect();
    ranked.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let farthest: Vec<Uuid> = ranked.iter().rev().take(5).map(|(id, _)| *id).collect();

    let index = HnswIndex::build_batch(vectors, "m", dim).unwrap();
    let results = index
        .search_knn_filtered(&query, 5, 30, |id| farthest.contains(id))
        .unwrap();

    assert_eq!(results.len(), 5, "el filtro exacto debe encontrar los 5 permitidos");
    let got: std::collections::HashSet<Uuid> = results.iter().map(|(id, _)| *id).collect();
    assert_eq!(got, farthest.iter().copied().collect());
}
