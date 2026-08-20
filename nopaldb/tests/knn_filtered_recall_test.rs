// Recall de la búsqueda KNN filtrada por encima del umbral exacto.
//
// Sobre `EXACT_SEARCH_THRESHOLD` el diseño anterior pedía `k × 4` vecinos
// GLOBALES al grafo HNSW y filtraba después: con un predicado selectivo los
// nodos permitidos no estaban entre esos candidatos, así que la búsqueda
// devolvía menos de `k` (o nada) aunque hubiera vecinos permitidos de sobra.
// Seguía siendo fail-closed, pero perdía recall en silencio.
//
// Ahora hay dos caminos: filtro NATIVO durante el recorrido del grafo
// (`search_filter`) con escalada adaptativa de `ef_search`, y —cuando el
// llamador conoce la cardinalidad de lo permitido y es chica— scan exacto
// sobre ese conjunto, que es a la vez exacto y más barato.

#![cfg(feature = "embeddings-index")]

use std::collections::HashSet;

use nopaldb::embeddings::{HnswIndex, EXACT_SEARCH_THRESHOLD};
use nopaldb::types::NodeId;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

/// Distancia coseno de referencia, independiente del índice (acumulación f64,
/// como la `DistCosine` de hnsw_rs).
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

fn seeded_vectors(n: usize, dim: usize, rng: &mut StdRng) -> Vec<(NodeId, Vec<f32>)> {
    (0..n)
        .map(|_| {
            let v: Vec<f32> = (0..dim).map(|_| rng.gen_range(-1.0f32..1.0)).collect();
            (NodeId::new_v4(), v)
        })
        .collect()
}

/// Top-k por fuerza bruta restringido a `allowed` — la verdad de referencia.
fn brute_force_allowed(
    query: &[f32],
    vectors: &[(NodeId, Vec<f32>)],
    allowed: &HashSet<NodeId>,
    k: usize,
) -> Vec<NodeId> {
    let mut ranked: Vec<(NodeId, f32)> = vectors
        .iter()
        .filter(|(id, _)| allowed.contains(id))
        .map(|(id, v)| (*id, cosine_dist(query, v)))
        .collect();
    ranked.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    ranked.truncate(k);
    ranked.into_iter().map(|(id, _)| id).collect()
}

/// Reproduce el diseño ANTERIOR: `k × 4` vecinos globales, filtrados después.
/// Está aquí para que la regresión sea medible, no solo afirmada.
fn legacy_overfetch_then_filter(
    index: &HnswIndex,
    query: &[f32],
    k: usize,
    ef: usize,
    allowed: &HashSet<NodeId>,
) -> Vec<NodeId> {
    let mut hits = index.search_knn_with_ef(query, k * 4, ef).unwrap();
    hits.retain(|(id, _)| allowed.contains(id));
    hits.truncate(k);
    hits.into_iter().map(|(id, _)| id).collect()
}

/// Índice grande + query fija, reutilizado por varios tests.
fn large_index(n: usize, dim: usize, seed: u64) -> (HnswIndex, Vec<(NodeId, Vec<f32>)>, Vec<f32>) {
    let mut rng = StdRng::seed_from_u64(seed);
    let vectors = seeded_vectors(n, dim, &mut rng);
    let query: Vec<f32> = (0..dim).map(|_| rng.gen_range(-1.0f32..1.0)).collect();
    let index = HnswIndex::build_batch(vectors.clone(), "m", dim).unwrap();
    assert!(index.len() > EXACT_SEARCH_THRESHOLD, "el test necesita el camino HNSW");
    (index, vectors, query)
}

/// La garantía dura: nada fuera del conjunto permitido, en ninguna
/// selectividad. Es la propiedad que NO puede regresionar.
#[test]
fn filtered_search_never_leaks_disallowed() {
    let (index, vectors, query) = large_index(5_000, 16, 42);

    for allowed_size in [1usize, 5, 50, 500, 2_500, 5_000] {
        let allowed: HashSet<NodeId> = vectors.iter().take(allowed_size).map(|(id, _)| *id).collect();
        let hits = index
            .search_knn_filtered(&query, 10, 30, |id| allowed.contains(id))
            .unwrap();

        assert!(
            hits.iter().all(|(id, _)| allowed.contains(id)),
            "fuga del filtro con |allowed|={allowed_size}"
        );
        assert!(hits.len() <= 10, "nunca más de k resultados");
    }
}

/// La regresión concreta: nodos permitidos que el over-fetch global NO
/// alcanzaba porque están lejos de la query. El filtro nativo sí los ve.
#[test]
fn finds_allowed_neighbours_the_global_overfetch_missed() {
    let (index, vectors, query) = large_index(5_000, 16, 7);
    let k = 10;

    // Permitir SOLO los 40 vectores más lejanos a la query: por construcción
    // ninguno cae entre los k*4 = 40 vecinos globales más cercanos.
    let mut ranked: Vec<(NodeId, f32)> = vectors
        .iter()
        .map(|(id, v)| (*id, cosine_dist(&query, v)))
        .collect();
    ranked.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let allowed: HashSet<NodeId> = ranked.iter().rev().take(40).map(|(id, _)| *id).collect();

    let legacy = legacy_overfetch_then_filter(&index, &query, k, 30, &allowed);
    assert!(
        legacy.is_empty(),
        "el diseño anterior no debería encontrar ninguno de los más lejanos \
         (encontró {}) — si esto cambia, el test perdió su sentido",
        legacy.len()
    );

    let outcome = index
        .search_knn_filtered_explained(&query, k, 30, |id| allowed.contains(id))
        .unwrap();
    assert!(
        outcome.hits.iter().all(|(id, _)| allowed.contains(id)),
        "fail-closed se preserva"
    );
    assert_eq!(
        outcome.hits.len(),
        k,
        "el filtro nativo debe juntar los k permitidos que el over-fetch global perdía"
    );
    assert!(
        outcome.attempts >= 1 && outcome.ef_used.is_some(),
        "el camino HNSW reporta su traza"
    );
    assert!(!outcome.underfilled);
}

/// Barrido de selectividad contra fuerza bruta. Mide recall real, no
/// "devolvió algo": para cada tamaño de conjunto permitido compara los ids
/// devueltos con el top-k exacto restringido a ese conjunto.
#[test]
fn recall_across_selectivities_beats_the_old_overfetch() {
    let n = 5_000;
    let k = 10;

    // Varias semillas: el grafo HNSW depende de la asignación aleatoria de
    // niveles, así que una sola corrida no distingue diseño de suerte.
    for seed in [1234u64, 55, 90_210] {
            let (index, vectors, query) = large_index(n, 16, seed);

        // ~0.5%, 1%, 5%, 50%, 100% del índice.
        for allowed_size in [25usize, 50, 250, 2_500, 5_000] {
            let mut rng = StdRng::seed_from_u64(seed ^ allowed_size as u64);
            // Muestra dispersa (no un prefijo) para no favorecer ninguna región.
            let allowed: HashSet<NodeId> = vectors
                .iter()
                .filter(|_| rng.gen_bool(allowed_size as f64 / n as f64))
                .map(|(id, _)| *id)
                .collect();
            if allowed.is_empty() {
                continue;
            }

            let expected = brute_force_allowed(&query, &vectors, &allowed, k);
            let got: Vec<NodeId> = index
                .search_knn_filtered(&query, k, 30, |id| allowed.contains(id))
                .unwrap()
                .into_iter()
                .map(|(id, _)| id)
                .collect();

            assert!(
                got.iter().all(|id| allowed.contains(id)),
                "fuga del filtro con |allowed|={}",
                allowed.len()
            );

            let expected_set: HashSet<NodeId> = expected.iter().copied().collect();
            let overlap = got.iter().filter(|id| expected_set.contains(id)).count();
            let recall = overlap as f64 / expected.len() as f64;

            let legacy = legacy_overfetch_then_filter(&index, &query, k, 30, &allowed);
            let legacy_overlap = legacy.iter().filter(|id| expected_set.contains(id)).count();
            let legacy_recall = legacy_overlap as f64 / expected.len() as f64;

            eprintln!(
                "MEDIDA seed={seed:6} |allowed|={:5} ({:6.2}%)  recall_nativo={recall:.3}  recall_legacy={legacy_recall:.3}",
                allowed.len(),
                100.0 * allowed.len() as f64 / n as f64
            );
            assert!(
                recall >= legacy_recall,
                "|allowed|={}: el filtro nativo no debe ser peor que el over-fetch global \
                 (recall {recall:.2} vs {legacy_recall:.2})",
                allowed.len()
            );
            assert!(
                recall >= 0.8,
                "|allowed|={}: recall {recall:.2} demasiado bajo (esperados {:?}, obtenidos {:?})",
                allowed.len(),
                expected.len(),
                got.len()
            );
        }
    }
}

/// La escalada se corta en el tope en vez de recorrer el índice entero por
/// query: con menos permitidos que `k`, el resultado se marca `underfilled`
/// y no promete lo que no hay.
#[test]
fn underfill_is_reported_not_hidden() {
    let (index, vectors, query) = large_index(3_000, 8, 99);

    // Solo 3 permitidos, se piden 10.
    let allowed: HashSet<NodeId> = vectors.iter().take(3).map(|(id, _)| *id).collect();
    let outcome = index
        .search_knn_filtered_explained(&query, 10, 30, |id| allowed.contains(id))
        .unwrap();

    assert!(outcome.underfilled, "menos de k resultados debe reportarse");
    assert!(outcome.hits.len() <= 3);
    assert!(
        outcome.hits.iter().all(|(id, _)| allowed.contains(id)),
        "fail-closed se preserva en el underfill"
    );
}

/// Bajo el umbral nada cambia: el camino exacto sigue siendo exacto,
/// determinista y sin escalada.
#[test]
fn below_threshold_still_exact_and_deterministic() {
    let dim = 8;
    let mut rng = StdRng::seed_from_u64(5);
    let vectors = seeded_vectors(200, dim, &mut rng);
    let query: Vec<f32> = (0..dim).map(|_| rng.gen_range(-1.0f32..1.0)).collect();
    let allowed: HashSet<NodeId> = vectors.iter().skip(100).map(|(id, _)| *id).collect();

    let index = HnswIndex::build_batch(vectors.clone(), "m", dim).unwrap();
    let outcome = index
        .search_knn_filtered_explained(&query, 10, 30, |id| allowed.contains(id))
        .unwrap();

    assert_eq!(outcome.path, nopaldb::embeddings::FilteredSearchPath::Exact);
    assert_eq!(outcome.ef_used, None, "ef_search no aplica al camino exacto");
    assert_eq!(outcome.attempts, 1, "el camino exacto no escala");

    let expected = brute_force_allowed(&query, &vectors, &allowed, 10);
    let got: Vec<NodeId> = outcome.hits.iter().map(|(id, _)| *id).collect();
    assert_eq!(got, expected, "bajo el umbral el filtrado es exacto");

    // Repetido = idéntico.
    for _ in 0..10 {
        let again = index
            .search_knn_filtered(&query, 10, 30, |id| allowed.contains(id))
            .unwrap();
        assert_eq!(again, outcome.hits);
    }
}
