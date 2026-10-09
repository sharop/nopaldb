//! Recuperación vectorial con dimensiones de modelos reales (#192, parte 1).
//!
//! `retrieval` mide la ruta de un GraphRAG con vectores de 64 dims y solo
//! latencia. Este bench mide lo que cambia al pasar a 384 (MiniLM) y 1024
//! dims: cuánto cuesta cargar y construir el índice HNSW, cuánta memoria
//! ocupa, cuánto tarda una búsqueda k=10 y qué tan bien recupera (recall@10
//! contra búsqueda exacta).
//!
//! No usa criterion: criterion mide tiempo, y aquí la mitad de las cifras
//! (recall, memoria) no lo son. Imprime una tabla en Markdown, la misma que
//! va en `docs/GRAPHRAG.md`.
//!
//! Dos tipos de datos, con semilla fija:
//!
//! - **agrupados**: 100 grupos; cada punto = centroide al azar en la esfera
//!   + ruido en un subespacio de 16 direcciones propio del grupo + un ruido
//!   isotrópico pequeño, normalizado. Se parece más a embeddings reales, que
//!   forman temas y tienen dimensión intrínseca baja. Se descartó el ruido
//!   isotrópico en todas las dimensiones: deja los puntos de un grupo casi
//!   equidistantes entre sí, un caso que se comporta como datos uniformes a
//!   escala chica.
//! - **uniformes**: gaussianas normalizadas, uniformes en la esfera. Es el
//!   peor caso de cualquier índice aproximado: el vecino más cercano casi no
//!   está más cerca que uno al azar. Sirve para comparar, no como el recall
//!   que tendrán embeddings reales.
//!
//! Las consultas salen de la misma distribución que los datos pero no son
//! puntos del índice.
//!
//! Columnas:
//!
//! - **carga**: `add_node_embeddings` (#175) en lotes de 10k, sin índice en
//!   caché (escribe en storage).
//! - **build**: primera `get_or_build_embedding_index`: lee los embeddings
//!   de storage, construye el HNSW y escribe su dump en disco.
//! - **huérfanos**: puntos a los que el grafo no llega tras el build; cada
//!   búsqueda los compara directamente (#184, #201).
//! - **memoria**: bytes de heap que retiene el índice, medidos con un
//!   allocator que cuenta (vivos después − vivos antes) sobre un
//!   `HnswIndex::build_batch` aparte con los mismos vectores. Medirlo sobre
//!   la `get_or_build_embedding_index` de arriba sumaría la caché del motor
//!   que se llena al leer los embeddings.
//! - **p50 / p95**: `search_knn` k=10 por consulta, con el `ef_search` de la
//!   fila (30 es el default).
//! - **exacto**: p50 de un scan lineal (`rank_exact`) sobre todos los
//!   vectores, para dar perspectiva.
//! - **recall@10**: fracción de los 10 vecinos exactos que devuelve el HNSW,
//!   promediada sobre las consultas.
//!
//! Correr: `make bench BENCH=retrieval_dims`. Variables:
//! `NOPALDB_BENCH_DIMS` (default `384,1024`), `NOPALDB_BENCH_SCALES`
//! (default `10000,100000`), `NOPALDB_BENCH_QUERIES` (default 200),
//! `NOPALDB_BENCH_ENGINE` (`redb` | `sled`).

use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use nopaldb::embeddings::{rank_exact, HnswIndex};
use nopaldb::types::{Node, NodeId};
use nopaldb::{Graph, StorageOptions};

const K: usize = 10;
const MODEL: &str = "m";
const CLUSTERS: usize = 100;
/// Dimensión intrínseca de cada grupo.
const CLUSTER_DIMS: usize = 16;
/// Norma esperada del ruido del subespacio respecto a la del centroide
/// (1.0). Con 0.6 el coseno de un punto con su centroide ronda 0.85: grupos
/// claros pero con estructura interna.
const NOISE: f32 = 0.6;
/// Norma esperada del ruido isotrópico: saca a los puntos del subespacio.
const ISOTROPIC: f32 = 0.05;
const EF_SEARCH: &[usize] = &[30, 100];

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            LIVE.fetch_add(new_size, Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn env_list(name: &str, default: &[usize]) -> Vec<usize> {
    match std::env::var(name) {
        Ok(v) => v.split(',').map(|x| x.trim().parse().unwrap_or_else(|_| panic!("{name}: {x:?} no es un número"))).collect(),
        Err(_) => default.to_vec(),
    }
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
    /// Normal estándar (Box-Muller).
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

#[derive(Clone, Copy)]
enum Data {
    Clustered,
    Uniform,
}

impl Data {
    fn name(self) -> &'static str {
        match self {
            Data::Clustered => "agrupados",
            Data::Uniform => "uniformes",
        }
    }
}

/// Genera `n` vectores de datos y `q` consultas de la misma distribución.
fn generate(data: Data, dim: usize, n: usize, q: usize, seed: u64) -> (Vec<Vec<f32>>, Vec<Vec<f32>>) {
    let mut rng = Rng::new(seed);
    match data {
        Data::Uniform => {
            let mut all: Vec<Vec<f32>> = (0..n + q).map(|_| normalize(rng.gauss_vec(dim))).collect();
            let queries = all.split_off(n);
            (all, queries)
        }
        Data::Clustered => {
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
    }
}

/// Los `K` vecinos exactos de cada consulta, en paralelo por consulta.
fn ground_truth(ids: &[NodeId], data: &[Vec<f32>], queries: &[Vec<f32>]) -> Vec<Vec<NodeId>> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let chunk = queries.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = queries
            .chunks(chunk)
            .map(|qs| {
                s.spawn(move || {
                    qs.iter()
                        .map(|q| {
                            let candidates = ids.iter().copied().zip(data.iter().map(|v| v.as_slice()));
                            rank_exact(q, candidates, K).into_iter().map(|(id, _)| id).collect()
                        })
                        .collect::<Vec<Vec<NodeId>>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().expect("ground truth")).collect()
    })
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

fn secs(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f64())
}

fn mib(bytes: usize) -> String {
    format!("{:.0}", bytes as f64 / (1024.0 * 1024.0))
}

async fn run(dim: usize, n: usize, q: usize, data: Data) -> Vec<String> {
    let seed = 0x9E37_79B9 ^ ((dim as u64) << 32) ^ n as u64 ^ matches!(data, Data::Uniform) as u64;
    let (vectors, queries) = generate(data, dim, n, q, seed);

    let dir = tempfile::tempdir().expect("tempdir");
    let graph = Graph::open_with_options(dir.path(), bench_options()).await.expect("open");
    let mut ids = Vec::with_capacity(n);
    {
        let mut loader = graph.bulk_loader(10_000);
        for _ in 0..n {
            let node = Node::new("Chunk");
            ids.push(node.id);
            loader.add_node(node).await.expect("node");
        }
        loader.finish().await.expect("finish");
    }

    let t = Instant::now();
    for (id_chunk, vec_chunk) in ids.chunks(10_000).zip(vectors.chunks(10_000)) {
        let items = id_chunk.iter().copied().zip(vec_chunk.iter().cloned()).collect();
        graph.add_node_embeddings(MODEL, items).await.expect("embeddings");
    }
    let load = t.elapsed();

    let t = Instant::now();
    let index = graph.get_or_build_embedding_index(MODEL).await.expect("index");
    let build = t.elapsed();

    let memory = {
        let before = LIVE.load(Ordering::Relaxed);
        let items = ids.iter().copied().zip(vectors.iter().cloned()).collect();
        let alone = HnswIndex::build_batch(items, MODEL, dim).expect("build_batch");
        let memory = LIVE.load(Ordering::Relaxed).saturating_sub(before);
        drop(alone);
        memory
    };

    let truth = ground_truth(&ids, &vectors, &queries);
    let mut exact: Vec<Duration> = queries
        .iter()
        .take(20)
        .map(|qv| {
            let t = Instant::now();
            let candidates = ids.iter().copied().zip(vectors.iter().map(|v| v.as_slice()));
            std::hint::black_box(rank_exact(qv, candidates, K));
            t.elapsed()
        })
        .collect();
    exact.sort();
    let exact_p50 = percentile(&exact, 0.5);

    let guard = index.read().unwrap_or_else(|e| e.into_inner());
    // Calentar cachés antes de medir.
    for qv in queries.iter().take(10) {
        let _ = guard.search_knn(qv, K);
    }
    let mut rows = Vec::new();
    for &ef in EF_SEARCH {
        let mut lat = Vec::with_capacity(q);
        let mut hits = 0usize;
        for (qv, expected) in queries.iter().zip(&truth) {
            let t = Instant::now();
            let got = guard.search_knn_with_ef(qv, K, ef).expect("knn");
            lat.push(t.elapsed());
            let expected: HashSet<&NodeId> = expected.iter().collect();
            hits += got.iter().filter(|(id, _)| expected.contains(id)).count();
        }
        lat.sort();
        let recall = hits as f64 / (q * K) as f64;
        rows.push(format!(
            "| {dim} | {n} | {} | {ef} | {} | {} | {} | {} | {} | {} | {} | {recall:.3} |",
            data.name(),
            secs(load),
            secs(build),
            guard.orphans(),
            mib(memory),
            ms(percentile(&lat, 0.5)),
            ms(percentile(&lat, 0.95)),
            ms(exact_p50),
        ));
    }
    drop(guard);
    drop(index);
    graph.close().await.expect("close");
    rows
}

fn main() {
    // `cargo bench` pasa `--bench`; con `harness = false` no hay nada que
    // filtrar, así que se ignoran los argumentos.
    let dims = env_list("NOPALDB_BENCH_DIMS", &[384, 1024]);
    let scales = env_list("NOPALDB_BENCH_SCALES", &[10_000, 100_000]);
    let q = env_list("NOPALDB_BENCH_QUERIES", &[200])[0];
    let rt = tokio::runtime::Runtime::new().expect("runtime");

    println!("| dims | nodos | datos | ef | carga (s) | build (s) | huérfanos | memoria (MiB) | p50 (ms) | p95 (ms) | exacto (ms) | recall@10 |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|");
    for &dim in &dims {
        for &n in &scales {
            for data in [Data::Clustered, Data::Uniform] {
                for row in rt.block_on(run(dim, n, q, data)) {
                    println!("{row}");
                }
            }
        }
    }
}
