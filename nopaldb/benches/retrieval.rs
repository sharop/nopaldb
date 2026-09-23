//! La ruta de recuperación de un GraphRAG, medida (0.6.8).
//!
//! Antes de este bench no había un solo número de lo que cuesta, tras una
//! búsqueda, hidratar los hits y expandir su vecindad: lo que domina el
//! tiempo de respuesta de un GraphRAG (con 100k chunks, la búsqueda híbrida
//! tardaba 0.2 ms y traer un hit por id vía NQL 327 ms). Grupos:
//!
//! - `search`: híbrida k=10 sin filtro y con filtro de etiqueta; KNN k=10
//! - `hydrate`: `get_nodes` de 10 ids
//! - `neighborhood`: 1 y 2 saltos desde 10 semillas
//! - `nql`: `where c.id = "…"`, `where c.id in [10 ids]`, patrón de 1 salto
//!
//! Fixture: `NOPALDB_BENCH_SCALE` chunks (default 100_000) con vectores de
//! 64 dims y texto, un 20 % de entidades, 3 `MENTIONS` por chunk y 2
//! `RELATED` por entidad, índice full-text sobre `Chunk.text` y hash sobre
//! `Entity.name`. Determinista (xorshift con semilla fija).
//!
//! Correr: `make bench BENCH=retrieval` (`CARGO_PROFILE_RELEASE_PANIC=unwind
//! cargo bench -p nopaldb --features core --bench retrieval`).

use std::hint::black_box;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use nopaldb::index::IndexType;
use nopaldb::types::{Edge, Node, NodeId, PropertyValue};
use nopaldb::{ExpandOptions, Graph, HybridFilter, HybridQuery, StorageOptions};

const DIM: usize = 64;
const K: usize = 10;
const MODEL: &str = "m";

fn scale() -> usize {
    std::env::var("NOPALDB_BENCH_SCALE").ok().and_then(|v| v.parse().ok()).unwrap_or(100_000)
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
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn vector(&mut self) -> Vec<f32> {
        let mut v: Vec<f32> = (0..DIM).map(|_| self.next_f32() - 0.5).collect();
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
        v.iter_mut().for_each(|x| *x /= n);
        v
    }
}

const VOCAB: &[&str] = &[
    "nopal", "maguey", "biznaga", "riego", "huerto", "semilla", "sombra", "cosecha", "tierra", "lluvia",
    "cerca", "camino", "pueblo", "mercado", "cesta", "aceite", "sal", "fuego", "olla", "mole",
];

struct Fixture {
    graph: Graph,
    seeds: Vec<NodeId>,
    query: Vec<f32>,
    _dir: tempfile::TempDir,
}

async fn build(n_chunks: usize) -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let graph = Graph::open_with_options(dir.path(), bench_options()).await.expect("open");
    let n_ent = (n_chunks / 5).max(10);
    let mut rng = Rng::new(0x9E37_79B9);

    let mut ents = Vec::with_capacity(n_ent);
    let mut chunks = Vec::with_capacity(n_chunks);
    let mut vectors = Vec::with_capacity(n_chunks);
    {
        let mut loader = graph.bulk_loader(10_000);
        for i in 0..n_ent {
            let node = Node::new("Entity")
                .with_property("name", PropertyValue::String(format!("entity-{i}")))
                .with_property("kind", PropertyValue::String(["persona", "lugar", "org"][i % 3].into()));
            ents.push(node.id);
            loader.add_node(node).await.expect("entity");
        }
        for i in 0..n_chunks {
            let words: Vec<&str> = (0..8).map(|_| VOCAB[(rng.next_u64() % VOCAB.len() as u64) as usize]).collect();
            let node = Node::new("Chunk")
                .with_property("doc", PropertyValue::String(format!("doc-{}", i / 20)))
                .with_property("text", PropertyValue::String(format!("{} entity-{}", words.join(" "), i % n_ent)));
            chunks.push(node.id);
            vectors.push(rng.vector());
            loader.add_node(node).await.expect("chunk");
        }
        for (i, c) in chunks.iter().enumerate() {
            for j in 0..3 {
                loader.add_edge(Edge::new(*c, ents[(i * 7 + j * 13) % n_ent], "MENTIONS")).await.expect("edge");
            }
        }
        for i in 0..n_ent {
            for j in 1..=2 {
                loader
                    .add_edge(Edge::new(ents[i], ents[(i * 13 + j) % n_ent], "RELATED").with_property("w", PropertyValue::Float(0.5)))
                    .await
                    .expect("edge");
            }
        }
        loader.finish().await.expect("finish");
    }
    for (c, v) in chunks.iter().zip(&vectors) {
        graph.add_node_embedding(*c, v.clone(), MODEL).await.expect("embedding");
    }
    graph.create_index("Chunk", "text", IndexType::FullText).await.expect("fulltext");
    graph.create_index("Entity", "name", IndexType::Hash).await.expect("hash");
    graph.checkpoint().await.expect("checkpoint");
    // Calentar el índice HNSW (la primera búsqueda lo construye o carga).
    let query = rng.vector();
    let _ = graph.knn_nodes_bench_warm(&query).await;
    let seeds: Vec<NodeId> = chunks.iter().step_by(n_chunks / 10).take(10).copied().collect();
    Fixture { graph, seeds, query, _dir: dir }
}

/// Pequeño puente: calentar el índice sin exponer nada nuevo en la API.
trait Warm {
    async fn knn_nodes_bench_warm(&self, q: &[f32]);
}
impl Warm for Graph {
    async fn knn_nodes_bench_warm(&self, q: &[f32]) {
        let idx = self.get_or_build_embedding_index(MODEL).await.expect("index");
        let guard = idx.read().unwrap_or_else(|e| e.into_inner());
        let _ = guard.search_knn(q, K);
    }
}

fn hybrid(fx: &Fixture, label: Option<&str>) -> HybridQuery {
    HybridQuery {
        text: Some("nopal riego".into()),
        text_index: None,
        vector: Some((fx.query.clone(), MODEL.into())),
        k: K,
        ef_search: None,
        rrf_k: 60.0,
        overfetch: 4,
        filter: label.map(|l| HybridFilter { label: Some(l.into()), props: Vec::new() }),
    }
}

fn bench_retrieval(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let n = scale();
    let fx = rt.block_on(build(n));
    let seed = fx.seeds[0];
    let id_list = fx.seeds.iter().map(|id| format!("\"{id}\"")).collect::<Vec<_>>().join(", ");

    let mut g = c.benchmark_group("search");
    g.sample_size(20).measurement_time(Duration::from_secs(10));
    g.bench_with_input(BenchmarkId::new("hybrid_k10_nolabel", n), &n, |b, _| {
        b.to_async(&rt).iter(|| async { black_box(fx.graph.search_hybrid(hybrid(&fx, None)).await.expect("hybrid")) })
    });
    g.bench_with_input(BenchmarkId::new("hybrid_k10_label", n), &n, |b, _| {
        b.to_async(&rt).iter(|| async { black_box(fx.graph.search_hybrid(hybrid(&fx, Some("Chunk"))).await.expect("hybrid")) })
    });
    g.bench_with_input(BenchmarkId::new("knn_k10", n), &n, |b, _| {
        b.to_async(&rt).iter(|| async {
            let idx = fx.graph.get_or_build_embedding_index(MODEL).await.expect("index");
            let guard = idx.read().unwrap_or_else(|e| e.into_inner());
            black_box(guard.search_knn(&fx.query, K).expect("knn"))
        })
    });
    g.finish();

    let mut g = c.benchmark_group("hydrate");
    g.sample_size(20).measurement_time(Duration::from_secs(10));
    g.bench_with_input(BenchmarkId::new("get_nodes_10", n), &n, |b, _| {
        b.to_async(&rt).iter(|| async { black_box(fx.graph.get_nodes(&fx.seeds).await.expect("get_nodes")) })
    });
    g.finish();

    let mut g = c.benchmark_group("neighborhood");
    g.sample_size(20).measurement_time(Duration::from_secs(10));
    for depth in [1usize, 2] {
        g.bench_with_input(BenchmarkId::new(format!("depth{depth}_10seeds"), n), &n, |b, _| {
            let opts = ExpandOptions { direction: nopaldb::Direction::Both, max_nodes: 5_000, ..Default::default() };
            b.to_async(&rt).iter(|| async { black_box(fx.graph.neighborhood(&fx.seeds, depth, &opts).await.expect("neighborhood")) })
        });
    }
    g.finish();

    let mut g = c.benchmark_group("nql");
    g.sample_size(20).measurement_time(Duration::from_secs(10));
    let q_id = format!("find c.text from (c:Chunk) where c.id = \"{seed}\"");
    let q_in = format!("find c.text from (c:Chunk) where c.id in [{id_list}]");
    let q_hop = format!("find e.name from (c:Chunk)-[:MENTIONS]->(e:Entity) where c.id = \"{seed}\"");
    for (name, q) in [("id_lookup", &q_id), ("id_in_10", &q_in), ("one_hop_pattern", &q_hop)] {
        g.bench_with_input(BenchmarkId::new(name, n), &n, |b, _| {
            b.to_async(&rt).iter(|| async { black_box(fx.graph.execute_nql(q).await.expect("nql")) })
        });
    }
    g.finish();
}

criterion_group!(benches, bench_retrieval);
criterion_main!(benches);
