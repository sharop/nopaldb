//! La ruta de recuperación de un GraphRAG, medida (0.6.8).
//!
//! Antes de este bench no había un solo número de lo que cuesta, tras una
//! búsqueda, hidratar los hits y expandir su vecindad: lo que domina el
//! tiempo de respuesta de un GraphRAG (con 100k chunks, la búsqueda híbrida
//! tardaba 0.2 ms y traer un hit por id vía NQL 327 ms). Grupos:
//!
//! - `search`: híbrida k=10 sin filtro y con filtro de etiqueta; solo texto
//!   k=10 (aísla la rama full-text); solo texto e híbrida con filtro de
//!   propiedades selectivo (`doc` = 20 chunks, `tenant` = 1 %, #213); KNN k=10
//! - `hydrate`: `get_nodes` de 10 ids
//! - `neighborhood`: 1 y 2 saltos desde 10 semillas; 2 saltos recortado a
//!   50 nodos por BFS y por Personalized PageRank (#176)
//! - `nql`: `where c.id = "…"`, `where c.id in [10 ids]`, patrón de 1 salto;
//!   buscar + expandir en una consulta con `similar_to` y con `hybrid` sobre
//!   el vector literal (0.6.10, #174: `hybrid` ya no recorre la etiqueta)
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
use nopaldb::{CandidateSet, ExpandOptions, Graph, HybridFilter, HybridQuery, StorageOptions};

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
    /// 65 chunks del tenant 7: el conjunto explícito de un caller (#214).
    tenant7_65: Vec<NodeId>,
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
                // 100 tenants ⇒ un filtro por tenant deja el 1 % de los chunks (#213).
                .with_property("tenant", PropertyValue::String(format!("tenant-{}", i % 100)))
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
    let tenant7_65: Vec<NodeId> = chunks.iter().skip(7).step_by(100).take(65).copied().collect();
    Fixture { graph, seeds, tenant7_65, query, _dir: dir }
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
        candidates: CandidateSet::All,
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
    g.bench_with_input(BenchmarkId::new("text_only_k10", n), &n, |b, _| {
        b.to_async(&rt).iter(|| async {
            let mut q = hybrid(&fx, None);
            q.vector = None;
            black_box(fx.graph.search_hybrid(q).await.expect("text"))
        })
    });
    // Filtro selectivo por propiedades (#213): `doc` deja 20 chunks (term set
    // directo), `tenant` deja el 1 % (el top-N global se agota corto y cae al
    // term set por trozos). Antes de #213 ambos devolvían menos de lo que hay.
    for (name, prop, value) in [("doc", "doc", "doc-7"), ("tenant", "tenant", "tenant-7")] {
        let filter = HybridFilter {
            label: Some("Chunk".into()),
            props: vec![(prop.into(), PropertyValue::String(value.into()))],
        };
        g.bench_with_input(BenchmarkId::new(format!("text_only_k10_props_{name}"), n), &n, |b, _| {
            b.to_async(&rt).iter(|| async {
                let mut q = hybrid(&fx, None);
                q.vector = None;
                q.filter = Some(filter.clone());
                black_box(fx.graph.search_hybrid(q).await.expect("text"))
            })
        });
        g.bench_with_input(BenchmarkId::new(format!("hybrid_k10_props_{name}"), n), &n, |b, _| {
            b.to_async(&rt).iter(|| async {
                let mut q = hybrid(&fx, None);
                q.filter = Some(filter.clone());
                black_box(fx.graph.search_hybrid(q).await.expect("hybrid"))
            })
        });
    }
    // Universo explícito del caller (#214): 65 ids (lo que deja tenant ∩
    // caso en un despliegue real), solos y reducidos por un filtro que
    // confirma props y etiqueta sobre ellos. Coste ∝ |conjunto|.
    g.bench_with_input(BenchmarkId::new("hybrid_k10_candidates_65", n), &n, |b, _| {
        b.to_async(&rt).iter(|| async {
            let mut q = hybrid(&fx, None);
            q.candidates = CandidateSet::ids(fx.tenant7_65.iter().copied());
            black_box(fx.graph.search_hybrid(q).await.expect("hybrid"))
        })
    });
    // Por rama, para saber dónde va el coste del conjunto.
    g.bench_with_input(BenchmarkId::new("text_only_k10_candidates_65", n), &n, |b, _| {
        b.to_async(&rt).iter(|| async {
            let mut q = hybrid(&fx, None);
            q.vector = None;
            q.candidates = CandidateSet::ids(fx.tenant7_65.iter().copied());
            black_box(fx.graph.search_hybrid(q).await.expect("text"))
        })
    });
    g.bench_with_input(BenchmarkId::new("vector_only_k10_candidates_65", n), &n, |b, _| {
        b.to_async(&rt).iter(|| async {
            let mut q = hybrid(&fx, None);
            q.text = None;
            q.candidates = CandidateSet::ids(fx.tenant7_65.iter().copied());
            black_box(fx.graph.search_hybrid(q).await.expect("vector"))
        })
    });
    g.bench_with_input(BenchmarkId::new("hybrid_k10_candidates_65_props_tenant", n), &n, |b, _| {
        b.to_async(&rt).iter(|| async {
            let mut q = hybrid(&fx, None);
            q.candidates = CandidateSet::ids(fx.tenant7_65.iter().copied());
            q.filter = Some(HybridFilter {
                label: Some("Chunk".into()),
                props: vec![("tenant".into(), PropertyValue::String("tenant-7".into()))],
            });
            black_box(fx.graph.search_hybrid(q).await.expect("hybrid"))
        })
    });
    g.finish();

    // Lo que cuesta construir el conjunto permitido de un filtro con
    // propiedades (#213): la etiqueta entera por `label_idx` y cada
    // propiedad por `prop_idx`.
    let mut g = c.benchmark_group("filter");
    g.sample_size(20).measurement_time(Duration::from_secs(5));
    g.bench_with_input(BenchmarkId::new("label_chunk_100pct", n), &n, |b, _| {
        b.to_async(&rt).iter(|| async { black_box(fx.graph.get_nodes_by_label("Chunk").await.expect("label").len()) })
    });
    for (name, value) in [("doc_20", "doc-7"), ("tenant_1pct", "tenant-7")] {
        let prop = name.split('_').next().unwrap();
        g.bench_with_input(BenchmarkId::new(format!("prop_{name}"), n), &n, |b, _| {
            b.to_async(&rt).iter(|| async {
                black_box(
                    fx.graph
                        .get_all_nodes_by_property(prop, &PropertyValue::String(value.into()))
                        .await
                        .expect("prop")
                        .len(),
                )
            })
        });
    }
    g.finish();

    let mut g = c.benchmark_group("search");
    g.sample_size(20).measurement_time(Duration::from_secs(10));
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
    // #176: el mismo vecindario de 2 saltos recortado a 50 nodos, por orden
    // de BFS y por Personalized PageRank (candidatos = 5 × 50).
    for (name, rank) in [("depth2_10seeds_top50_bfs", nopaldb::Rank::Bfs), ("depth2_10seeds_top50_ppr", nopaldb::Rank::ppr())] {
        g.bench_with_input(BenchmarkId::new(name, n), &n, |b, _| {
            let opts = ExpandOptions { direction: nopaldb::Direction::Both, max_nodes: 50, rank: rank.clone(), ..Default::default() };
            b.to_async(&rt).iter(|| async { black_box(fx.graph.neighborhood(&fx.seeds, 2, &opts).await.expect("neighborhood")) })
        });
    }
    g.finish();

    let mut g = c.benchmark_group("nql");
    g.sample_size(20).measurement_time(Duration::from_secs(10));
    let q_id = format!("find c.text from (c:Chunk) where c.id = \"{seed}\"");
    let q_in = format!("find c.text from (c:Chunk) where c.id in [{id_list}]");
    let q_hop = format!("find e.name from (c:Chunk)-[:MENTIONS]->(e:Entity) where c.id = \"{seed}\"");
    let literal = fx.query.iter().map(|x| format!("{x:?}")).collect::<Vec<_>>().join(", ");
    let q_similar_hop = format!(
        "find c.text, e.name from (c:Chunk)-[:MENTIONS]->(e:Entity) where similar_to(c, vector = [{literal}], model = \"{MODEL}\", k = {K})"
    );
    let q_hybrid_hop = format!(
        "find c.text, e.name from (c:Chunk)-[:MENTIONS]->(e:Entity) where hybrid(c, text = \"nopal riego\", vector = [{literal}], model = \"{MODEL}\", k = {K})"
    );
    for (name, q) in [
        ("id_lookup", &q_id),
        ("id_in_10", &q_in),
        ("one_hop_pattern", &q_hop),
        ("similar_to_literal_one_hop", &q_similar_hop),
        ("hybrid_literal_one_hop", &q_hybrid_hop),
    ] {
        g.bench_with_input(BenchmarkId::new(name, n), &n, |b, _| {
            b.to_async(&rt).iter(|| async { black_box(fx.graph.execute_nql(q).await.expect("nql")) })
        });
    }
    g.finish();
}

criterion_group!(benches, bench_retrieval);
criterion_main!(benches);
