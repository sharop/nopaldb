//! Exacto sobre el conjunto permitido vs HNSW filtrado (#254).
//!
//! La búsqueda híbrida con filtro elige entre dos caminos para la rama
//! vectorial (`Graph::vector_path`):
//!
//! - **exacto**: lee del índice los vectores de los ids permitidos
//!   (`HnswIndex::vectors_for`) y los puntúa todos (`rank_exact`);
//! - **filtrado**: recorre el grafo HNSW aplicando el filtro
//!   (`search_knn_filtered_explained`), aproximado, con escalada de `ef`.
//!
//! El umbral que los separa se midió en 0.5.5, cuando el exacto leía cada
//! vector del storage. Este bench mide los dos caminos tal como los llama la
//! búsqueda híbrida (k = 10 × overfetch 4 = 40 candidatos, `ef` = 30) sobre
//! un índice de `N` puntos, con conjuntos permitidos al azar de varios
//! tamaños y en 64, 384 y 1024 dimensiones. Imprime una tabla Markdown, la
//! de `docs/HYBRID_SEARCH.md` § Cost.
//!
//! Los datos son los "agrupados" de `retrieval_dims` (100 temas con
//! dimensión intrínseca baja). El filtro es un subconjunto al azar,
//! independiente de los temas: como un filtro por inquilino o por fecha.
//!
//! Variables de entorno: `NOPALDB_BENCH_N` (100000), `NOPALDB_BENCH_DIMS`
//! (64,384,1024), `NOPALDB_BENCH_SETS` (1000,2000,5000,10000,20000,50000),
//! `NOPALDB_BENCH_QUERIES` (40).
//!
//! ```text
//! cargo bench -p nopaldb --features embeddings-index --bench exact_threshold
//! ```

use std::collections::HashSet;
use std::time::{Duration, Instant};

use nopaldb::embeddings::{rank_exact, HnswIndex, DEFAULT_EF_SEARCH};
use nopaldb::types::NodeId;

const CANDIDATES: usize = 40;
const CLUSTERS: usize = 100;
const CLUSTER_DIMS: usize = 16;
const NOISE: f32 = 0.6;
const ISOTROPIC: f32 = 0.05;

fn env_list(name: &str, default: &[usize]) -> Vec<usize> {
    match std::env::var(name) {
        Ok(v) => v.split(',').map(|x| x.trim().parse().unwrap_or_else(|_| panic!("{name}: {x:?}"))).collect(),
        Err(_) => default.to_vec(),
    }
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn next_f32(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32 + 0.5) / (1u64 << 24) as f32
    }
    fn gauss(&mut self) -> f32 {
        let (u, v) = (self.next_f32(), self.next_f32());
        (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
    }
    fn gauss_vec(&mut self, dim: usize) -> Vec<f32> {
        (0..dim).map(|_| self.gauss()).collect()
    }
}

fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    v.iter_mut().for_each(|x| *x /= n);
    v
}

/// `n` puntos agrupados y `q` consultas de la misma distribución.
fn generate(dim: usize, n: usize, q: usize, seed: u64) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
    let mut rng = Rng::new(seed);
    let centroids: Vec<Vec<f32>> = (0..CLUSTERS).map(|_| normalize(rng.gauss_vec(dim))).collect();
    let bases: Vec<Vec<Vec<f32>>> = (0..CLUSTERS)
        .map(|_| (0..CLUSTER_DIMS).map(|_| normalize(rng.gauss_vec(dim))).collect())
        .collect();
    let along = NOISE / (CLUSTER_DIMS as f32).sqrt();
    let across = ISOTROPIC / (dim as f32).sqrt();
    let point = |rng: &mut Rng| {
        let c = (rng.next_u64() % CLUSTERS as u64) as usize;
        let mut v = centroids[c].clone();
        for base in &bases[c] {
            let z = along * rng.gauss();
            v.iter_mut().zip(base).for_each(|(x, b)| *x += z * b);
        }
        v.iter_mut().for_each(|x| *x += across * rng.gauss());
        normalize(v)
    };
    let data = (0..n).map(|_| point(&mut rng)).collect();
    let queries = (0..q).map(|_| point(&mut rng)).collect();
    (data, queries)
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    sorted[((sorted.len() - 1) as f64 * p).round() as usize]
}

fn ms(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1e3;
    if ms >= 10.0 {
        format!("{ms:.0}")
    } else if ms >= 1.0 {
        format!("{ms:.1}")
    } else {
        format!("{ms:.2}")
    }
}

/// El camino exacto, como `Graph::vector_path`.
fn exact(index: &HnswIndex, query: &[f32], set: &HashSet<NodeId>) -> Vec<(NodeId, f32)> {
    let vectors = index.vectors_for(set.iter().copied());
    rank_exact(query, vectors.iter().map(|(id, v)| (*id, v.as_slice())), CANDIDATES)
}

fn main() {
    let n = env_list("NOPALDB_BENCH_N", &[100_000])[0];
    let dims = env_list("NOPALDB_BENCH_DIMS", &[64, 384, 1024]);
    let sets = env_list("NOPALDB_BENCH_SETS", &[1_000, 2_000, 5_000, 10_000, 20_000, 50_000]);
    let q = env_list("NOPALDB_BENCH_QUERIES", &[40])[0];

    println!("N = {n} puntos, {q} consultas por celda, {CANDIDATES} candidatos, ef = {DEFAULT_EF_SEARCH}\n");
    println!("| dims | permitidos | % del índice | exacto p50 | exacto p95 | filtrado p50 | filtrado p95 | recall filtrado | ef final (mediana) |");
    println!("|---:|---:|---:|---:|---:|---:|---:|---:|---:|");

    for &dim in &dims {
        let (data, queries) = generate(dim, n, q, 0x5EED + dim as u64);
        let ids: Vec<NodeId> = (0..n).map(|_| NodeId::new_v4()).collect();
        let t = Instant::now();
        let index = HnswIndex::build_batch(ids.iter().copied().zip(data).collect(), "m", dim).expect("build");
        eprintln!("dims {dim}: build {:.1} s", t.elapsed().as_secs_f64());

        let mut rng = Rng::new(7 + dim as u64);
        for &size in &sets {
            // Subconjunto al azar de `size` ids (Fisher-Yates parcial).
            let mut pool = ids.clone();
            for i in 0..size.min(n) {
                let j = i + (rng.next_u64() % (n - i) as u64) as usize;
                pool.swap(i, j);
            }
            let set: HashSet<NodeId> = pool[..size.min(n)].iter().copied().collect();

            let mut t_exact = Vec::with_capacity(q);
            let mut t_filt = Vec::with_capacity(q);
            let mut efs = Vec::with_capacity(q);
            let (mut hit, mut total) = (0usize, 0usize);
            for query in &queries {
                let t = Instant::now();
                let truth = exact(&index, query, &set);
                t_exact.push(t.elapsed());

                let t = Instant::now();
                let out = index
                    .search_knn_filtered_explained(query, CANDIDATES, DEFAULT_EF_SEARCH, |id| set.contains(id))
                    .expect("filtered");
                t_filt.push(t.elapsed());
                efs.push(out.ef_used.unwrap_or(0));

                let want: HashSet<NodeId> = truth.iter().map(|(id, _)| *id).collect();
                hit += out.hits.iter().filter(|(id, _)| want.contains(id)).count();
                total += want.len();
            }
            t_exact.sort();
            t_filt.sort();
            efs.sort();
            println!(
                "| {dim} | {size} | {:.1} % | {} | {} | {} | {} | {:.3} | {} |",
                100.0 * size as f64 / n as f64,
                ms(percentile(&t_exact, 0.5)),
                ms(percentile(&t_exact, 0.95)),
                ms(percentile(&t_filt, 0.5)),
                ms(percentile(&t_filt, 0.95)),
                hit as f64 / total.max(1) as f64,
                efs[efs.len() / 2],
            );
        }
    }
}
