//! HNSW: la primera medición del índice vectorial (#112).
//!
//! Hasta este bench no había un solo número de HNSW en el repo; las decisiones
//! sobre inserción incremental (#113) y persistencia (#114) se tomarían a
//! ciegas. Seis grupos, cada uno en dos tamaños:
//!
//! - `build_batch`: construir el índice desde cero
//! - `rebuild_after_insert`: el costo real de hoy, 1 embedding nuevo + rebuild
//! - `insert_incremental`: 1 `HnswIndex::insert` sobre un índice existente
//! - `search_knn`: k=10 con `ef_search` default y 2×
//! - `search_filtered`: selectividad 1 % / 10 % / 100 % (la lección de #71:
//!   el filtrado se encarece cuanto más selectivo)
//! - `open_first_search`: abrir una base persistida + primera búsqueda (la
//!   línea base para #114)
//!
//! Escala por env `NOPALDB_HNSW_N` (lista separada por comas; default
//! `10000,100000`); motor por `NOPALDB_BENCH_ENGINE=sled|redb` para
//! `open_first_search`. Vectores sintéticos de dimensión 384 con semilla fija:
//! los números son comparables entre corridas y entre máquinas del mismo tipo.
//!
//! Correr: `make bench BENCH=hnsw_ops`, que equivale a
//! `CARGO_PROFILE_RELEASE_PANIC=unwind cargo bench -p nopaldb --features core --bench hnsw_ops`.
//! El override es obligatorio: el perfil release del workspace lleva
//! `panic = "abort"` y el harness de bench exige `unwind`; sin él, desde un
//! target limpio, las dependencias chocan ("requires panic strategy abort").

use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use nopaldb::embeddings::{HnswIndex, DEFAULT_EF_SEARCH};
use nopaldb::types::{Node, NodeId, PropertyValue};
use nopaldb::{Graph, StorageOptions};

const DIM: usize = 384;
const K: usize = 10;
const MODEL: &str = "bench";

fn sizes() -> Vec<usize> {
    std::env::var("NOPALDB_HNSW_N")
        .ok()
        .map(|v| v.split(',').filter_map(|s| s.trim().parse().ok()).collect())
        .filter(|v: &Vec<usize>| !v.is_empty())
        .unwrap_or_else(|| vec![10_000, 100_000])
}

fn bench_options() -> StorageOptions {
    let mut opts = StorageOptions::default();
    if let Ok(name) = std::env::var("NOPALDB_BENCH_ENGINE") {
        opts.engine = match name.to_ascii_lowercase().as_str() {
            "sled" => nopaldb::StorageEngine::Sled,
            "redb" => nopaldb::StorageEngine::Redb,
            other => panic!("NOPALDB_BENCH_ENGINE desconocido: {other}"),
        };
    }
    opts
}

/// xorshift64*: determinista, sin dependencias, suficiente para vectores de
/// prueba (no hace falta calidad criptográfica, hace falta reproducibilidad).
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn next_f32(&mut self) -> f32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        let x = self.0.wrapping_mul(0x2545_F491_4F6C_DD1D);
        ((x >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
    }

    fn vector(&mut self) -> Vec<f32> {
        (0..DIM).map(|_| self.next_f32()).collect()
    }
}

/// `n` vectores con ids estables (uuid v5-like a partir del índice, para que
/// la selectividad del filtro sea reproducible).
fn dataset(n: usize) -> Vec<(NodeId, Vec<f32>)> {
    let mut rng = Rng::new(0x5EED_0000_0000_0001 + n as u64);
    (0..n)
        .map(|i| {
            let id = NodeId::from_u128(0xB000_0000_0000_0000_0000_0000_0000_0000u128 + i as u128);
            (id, rng.vector())
        })
        .collect()
}

fn queries(count: usize) -> Vec<Vec<f32>> {
    let mut rng = Rng::new(0xC0FFEE);
    (0..count).map(|_| rng.vector()).collect()
}

fn built(n: usize) -> (HnswIndex, Vec<(NodeId, Vec<f32>)>) {
    let data = dataset(n);
    let index = HnswIndex::build_batch(data.clone(), MODEL, DIM).expect("build_batch");
    (index, data)
}

fn bench_build_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw/build_batch");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(20));
    for n in sizes() {
        let data = dataset(n);
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| black_box(HnswIndex::build_batch(data.clone(), MODEL, DIM).expect("build")));
        });
    }
    group.finish();
}

/// Lo que cuesta hoy un embedding nuevo cuando ya hay un índice caliente:
/// `add_node_embedding` lo invalida y la siguiente búsqueda reconstruye todo.
fn bench_rebuild_after_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw/rebuild_after_insert");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(20));
    for n in sizes() {
        let mut data = dataset(n);
        let extra = (NodeId::from_u128(0xE000), Rng::new(7).vector());
        data.push(extra);
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                let index = HnswIndex::build_batch(data.clone(), MODEL, DIM).expect("build");
                black_box(index.search_knn(&data[0].1, K).expect("search"))
            });
        });
    }
    group.finish();
}

/// Lo que costará con #113: un `insert` sobre el índice existente.
fn bench_insert_incremental(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw/insert_incremental");
    group.sample_size(20);
    for n in sizes() {
        let (mut index, _) = built(n);
        let mut next: u128 = 0xE000_0000;
        let mut rng = Rng::new(11);
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            // Cada iteración inserta un id nuevo; el índice crece durante la
            // medición (unos cientos de puntos sobre N), lo que es despreciable.
            b.iter(|| {
                next += 1;
                let id = NodeId::from_u128(next);
                index.insert(id, rng.vector()).expect("insert");
                black_box(index.len())
            });
        });
    }
    group.finish();
}

fn bench_search_knn(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw/search_knn");
    for n in sizes() {
        let (index, _) = built(n);
        let qs = queries(64);
        for (label, ef) in [("ef_default", DEFAULT_EF_SEARCH), ("ef_2x", DEFAULT_EF_SEARCH * 2)] {
            group.bench_with_input(BenchmarkId::new(label, n), &n, |b, _| {
                let mut i = 0usize;
                b.iter(|| {
                    i = (i + 1) % qs.len();
                    black_box(index.search_knn_with_ef(&qs[i], K, ef).expect("search"))
                });
            });
        }
    }
    group.finish();
}

/// Selectividad del filtro: el predicado deja pasar el 1 %, el 10 % o el 100 %
/// de los ids. Con #71 la rama exacta sobre el conjunto permitido (≤1024
/// candidatos) es el camino principal en el extremo selectivo.
fn bench_search_filtered(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw/search_filtered");
    for n in sizes() {
        let (index, _) = built(n);
        let qs = queries(64);
        for (label, modulo) in [("1pct", 100u128), ("10pct", 10u128), ("100pct", 1u128)] {
            group.bench_with_input(BenchmarkId::new(label, n), &n, |b, _| {
                let mut i = 0usize;
                b.iter(|| {
                    i = (i + 1) % qs.len();
                    black_box(
                        index
                            .search_knn_filtered(&qs[i], K, DEFAULT_EF_SEARCH, |id| id.as_u128() % modulo == 0)
                            .expect("filtered"),
                    )
                });
            });
        }
    }
    group.finish();
}

/// Abrir una base con `n` embeddings persistidos y hacer la primera búsqueda:
/// hoy eso reconstruye el índice desde storage. Es la línea base de #114.
fn bench_open_first_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw/open_first_search");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(30));
    let rt = tokio::runtime::Runtime::new().expect("tokio");
    for n in sizes() {
        // Base persistida una vez por tamaño, fuera de la medición.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("db");
        let data = dataset(n);
        rt.block_on(async {
            let graph = Graph::open_with_options(&path, bench_options()).await.expect("open");
            let mut loader = graph.bulk_loader(1024);
            let mut nodes = Vec::with_capacity(n);
            for (id, _) in &data {
                let mut node = Node::new("Doc").with_property("i", PropertyValue::Int(0));
                node.id = *id;
                nodes.push(node);
            }
            for node in nodes {
                loader.add_node(node).await.expect("add_node");
            }
            loader.finish().await.expect("finish");
            for (id, v) in &data {
                graph.add_node_embedding(*id, v.clone(), MODEL).await.expect("embedding");
            }
            graph.close().await.expect("close");
        });
        let query = queries(1).remove(0);
        group.bench_with_input(BenchmarkId::from_parameter(n), &n, |b, _| {
            b.iter(|| {
                rt.block_on(async {
                    let graph = Graph::open_with_options(&path, bench_options()).await.expect("reopen");
                    let index = graph.get_or_build_embedding_index(MODEL).await.expect("index");
                    let hits = index.search_knn(&query, K).expect("search");
                    graph.close().await.expect("close");
                    black_box(hits)
                })
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_build_batch,
    bench_rebuild_after_insert,
    bench_insert_incremental,
    bench_search_knn,
    bench_search_filtered,
    bench_open_first_search
);
criterion_main!(benches);
