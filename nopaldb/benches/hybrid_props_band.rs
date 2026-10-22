//! `search_hybrid` con un filtro por propiedad, por selectividad (#253, #254).
//!
//! La tabla de `docs/HYBRID_SEARCH.md` § Filter: 100k chunks, 64 dims,
//! texto + vector, `k = 10`, filtro `label = Chunk` + una propiedad que llevan
//! el 10, 5, 2, 1 y 0.5 % de los chunks. Imprime la mediana y el p95 de 15
//! consultas por fila, y el camino que tomó la rama vectorial. Solo usa API
//! pública, así que el mismo archivo corre sobre versiones anteriores para
//! comparar.
//!
//! ```text
//! make bench BENCH=hybrid_props_band          # NOPALDB_BENCH_N=100000
//! ```

use std::time::{Duration, Instant};

use nopaldb::index::IndexType;
use nopaldb::types::{Node, NodeId, PropertyValue};
use nopaldb::{CandidateSet, Graph, HybridFilter, HybridQuery};

const DIM: usize = 64;
const K: usize = 10;
const MODEL: &str = "m";
const QUERIES: usize = 15;
const VOCAB: &[&str] = &[
    "nopal", "maguey", "biznaga", "riego", "huerto", "semilla", "sombra", "cosecha", "tierra", "lluvia",
    "cerca", "camino", "pueblo", "mercado", "cesta", "aceite", "sal", "fuego", "olla", "mole",
];
/// `(valor, por mil)`: qué parte de los chunks lleva cada valor de `band`.
const BANDS: &[(&str, usize)] = &[("p10", 100), ("p5", 50), ("p2", 20), ("p1", 10), ("p05", 5)];

struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn next_f32(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32 + 0.5) / (1u64 << 24) as f32
    }
    fn vector(&mut self) -> Vec<f32> {
        let mut v: Vec<f32> = (0..DIM).map(|_| self.next_f32() - 0.5).collect();
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
        v.iter_mut().for_each(|x| *x /= n);
        v
    }
}

/// El valor de `band` del chunk `i`: cada banda ocupa su tramo de cada mil.
fn band_of(i: usize) -> &'static str {
    let mut slot = i % 1000;
    for (name, per_mille) in BANDS {
        if slot < *per_mille {
            return name;
        }
        slot -= per_mille;
    }
    "none"
}

fn ms(d: Duration) -> String {
    let ms = d.as_secs_f64() * 1e3;
    if ms >= 10.0 { format!("{ms:.0}") } else if ms >= 1.0 { format!("{ms:.1}") } else { format!("{ms:.2}") }
}

fn query(vector: Vec<f32>, band: Option<&str>, label: bool) -> HybridQuery {
    let filter = (label || band.is_some()).then(|| HybridFilter {
        label: label.then(|| "Chunk".to_string()),
        props: band.map(|b| vec![("band".to_string(), PropertyValue::String(b.into()))]).unwrap_or_default(),
    });
    HybridQuery {
        text: Some("nopal riego".into()),
        text_index: None,
        vector: Some((vector, MODEL.into())),
        k: K,
        ef_search: None,
        rrf_k: 60.0,
        overfetch: 4,
        filter,
        candidates: CandidateSet::All,
    }
}

fn main() {
    let n: usize = std::env::var("NOPALDB_BENCH_N").ok().and_then(|v| v.parse().ok()).unwrap_or(100_000);
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    rt.block_on(async {
        let dir = tempfile::tempdir().expect("tempdir");
        let graph = Graph::open(dir.path()).await.expect("open");
        let mut rng = Rng(0x9E37_79B9);
        let mut ids: Vec<NodeId> = Vec::with_capacity(n);
        {
            let mut loader = graph.bulk_loader(10_000);
            for i in 0..n {
                let words: Vec<&str> = (0..8).map(|_| VOCAB[(rng.next_u64() % VOCAB.len() as u64) as usize]).collect();
                let node = Node::new("Chunk")
                    .with_property("band", PropertyValue::String(band_of(i).into()))
                    .with_property("text", PropertyValue::String(words.join(" ")));
                ids.push(node.id);
                loader.add_node(node).await.expect("chunk");
            }
            loader.finish().await.expect("finish");
        }
        let items: Vec<(NodeId, Vec<f32>)> = ids.iter().map(|id| (*id, rng.vector())).collect();
        for chunk in items.chunks(10_000) {
            graph.add_node_embeddings(MODEL, chunk.to_vec()).await.expect("embeddings");
        }
        graph.create_index("Chunk", "text", IndexType::FullText).await.expect("fulltext");
        graph.checkpoint().await.expect("checkpoint");
        // Calentar: la primera búsqueda construye el HNSW.
        graph.search_hybrid(query(rng.vector(), None, false)).await.expect("warm");

        println!("N = {n} chunks, {DIM} dims, k = {K}, texto + vector, mediana y p95 de {QUERIES} consultas\n");
        println!("| filtro | permitidos | p50 | p95 | rama vectorial |");
        println!("|---|---:|---:|---:|---|");
        let mut rows: Vec<(String, Option<&str>, bool, usize)> =
            vec![("sin filtro".into(), None, false, n), ("label".into(), None, true, n)];
        for (name, per_mille) in BANDS {
            rows.push((format!("label + band ({:.1} %)", *per_mille as f64 / 10.0), Some(*name), true, n * per_mille / 1000));
        }
        for (name, band, label, members) in rows {
            let mut times = Vec::with_capacity(QUERIES);
            let mut path = String::new();
            for _ in 0..QUERIES {
                let v = rng.vector();
                let t = Instant::now();
                let hits = graph.search_hybrid(query(v.clone(), band, label)).await.expect("hybrid");
                times.push(t.elapsed());
                assert!(hits.len() <= K);
                if path.is_empty() {
                    let ex = graph.search_hybrid_explain(query(v, band, label)).await.expect("explain");
                    path = format!("{:?}", ex.vector_path);
                }
            }
            times.sort();
            println!("| {name} | {members} | {} | {} | {path} |", ms(times[QUERIES / 2]), ms(times[QUERIES * 95 / 100]));
        }
    });
}
