//! GC de versiones MVCC end-to-end — la carga de removals masivos que
//! discrimina motores (debilidad declarada de los B+tree CoW).
//!
//! Escala por env: NOPALDB_GC_NODES (default 2000) × NOPALDB_GC_VERSIONS
//! (default 10). Para la corrida del gate: 10000 × 20. Motor por
//! NOPALDB_BENCH_ENGINE=sled|redb (default: el default del build).

use criterion::{criterion_group, criterion_main, Criterion};
use nopaldb::{Graph, Node, PropertyValue, StorageOptions};

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

pub fn engine_from_env() -> StorageOptions {
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

fn bench_gc(c: &mut Criterion) {
    let nodes = env_usize("NOPALDB_GC_NODES", 2000);
    let versions = env_usize("NOPALDB_GC_VERSIONS", 10);
    let rt = tokio::runtime::Runtime::new().unwrap();

    let mut group = c.benchmark_group("gc_removals");
    group.sample_size(10);
    group.bench_function(format!("gc_{nodes}n_x_{versions}v"), |b| {
        b.iter_batched(
            || {
                // Setup fuera de la medición: base poblada con historia.
                let dir = tempfile::tempdir().unwrap();
                let opts = engine_from_env();
                rt.block_on(async {
                    let graph = Graph::open_with_options(dir.path().join("db"), opts)
                        .await
                        .unwrap();
                    let mut ids = Vec::with_capacity(nodes);
                    for i in 0..nodes {
                        let id = graph
                            .add_node(Node::new("N").with_property("i", i as i64))
                            .await
                            .unwrap();
                        ids.push(id);
                    }
                    for v in 0..versions {
                        for id in &ids {
                            let mut tx = graph.begin_transaction().await.unwrap();
                            let mut node = graph.get_node(*id).await.unwrap();
                            node.properties
                                .insert("v".into(), PropertyValue::Int(v as i64));
                            // add_node con el mismo id = versión nueva al commit
                            let _ = tx.add_node(node).await.unwrap();
                            tx.commit().await.unwrap();
                        }
                    }
                    (dir, graph)
                })
            },
            |(dir, graph)| {
                rt.block_on(async {
                    // Cutoff en el futuro cercano: TODO lo viejo es elegible,
                    // conservando el mínimo por nodo — el peor caso de removals.
                    let cfg = nopaldb::mvcc::GCConfig::older_than_ms(0);
                    let stats = graph.gc(cfg).await.unwrap();
                    criterion::black_box(stats);
                });
                drop(dir);
            },
            criterion::BatchSize::PerIteration,
        );
    });
    group.finish();
}

criterion_group!(benches, bench_gc);
criterion_main!(benches);
