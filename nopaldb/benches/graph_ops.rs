// benches/graph_ops.rs
//
// Línea base de rendimiento para el trabajo de concurrencia (roadmap M2):
//   (a) throughput de commit de transacciones pequeñas (fsync-bound hoy)
//   (b) lecturas get_node con 1/4/8 tasks concurrentes
//   (c) lecturas concurrentes con un escritor activo (esperado: mide el
//       máximo de leer y escribir) y con un escritor de fondo (solo lectura)
//   (d) ingesta con BulkLoader, en base nueva y en base abierta
//
// Correr: cargo bench -p nopaldb
// Registrar los números ANTES de aterrizar el applier (I8), el commit atómico
// (I9) y el group commit del WAL (I10) para poder demostrar la mejora.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use nopaldb::{Edge, Graph, Node, PropertyValue};
use std::sync::Arc;
use std::time::Duration;

const READ_BATCH: usize = 64;

fn person(i: usize) -> Node {
    Node::new("Person")
        .with_property("name", PropertyValue::String(format!("p{}", i)))
        .with_property("age", PropertyValue::Int(i as i64))
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(8)
        .enable_all()
        .build()
        .expect("tokio runtime")
}

// Grafo persistente pre-poblado con `n` nodos; retorna también sus ids
// (`seeded_graph`, más abajo).

/// Motor por env (NOPALDB_BENCH_ENGINE=sled|redb); default = el del build.
/// Permite comparar engines con el MISMO bench sin tocar código.
fn bench_options() -> nopaldb::StorageOptions {
    let mut opts = nopaldb::StorageOptions::default();
    if let Ok(name) = std::env::var("NOPALDB_BENCH_ENGINE") {
        opts.engine = match name.to_ascii_lowercase().as_str() {
            "sled" => nopaldb::StorageEngine::Sled,
            "redb" => nopaldb::StorageEngine::Redb,
            other => panic!("NOPALDB_BENCH_ENGINE desconocido: {other}"),
        };
    }
    opts
}

async fn seeded_graph(dir: &std::path::Path, n: usize) -> (Arc<Graph>, Vec<nopaldb::NodeId>) {
    let graph = Arc::new(Graph::open_with_options(dir, bench_options()).await.expect("open"));
    let mut ids = Vec::with_capacity(n);
    let mut loader = graph.bulk_loader(256);
    for i in 0..n {
        let node = person(i);
        ids.push(node.id);
        loader.add_node(node).await.expect("bulk add");
    }
    loader.finish().await.expect("bulk finish");
    (graph, ids)
}

/// Lote de READ_BATCH lecturas repartidas en `tasks` tasks concurrentes.
async fn read_batch(graph: &Arc<Graph>, ids: &Arc<Vec<nopaldb::NodeId>>, tasks: usize) {
    let per_task = READ_BATCH / tasks;
    let mut handles = Vec::with_capacity(tasks);
    for t in 0..tasks {
        let g = Arc::clone(graph);
        let ids = Arc::clone(ids);
        handles.push(tokio::spawn(async move {
            for k in 0..per_task {
                let id = ids[(t * per_task + k) % ids.len()];
                let _ = g.get_node(id).await.expect("get_node");
            }
        }));
    }
    for h in handles {
        h.await.expect("read task");
    }
}

// (a) Throughput de commit: transacción de 1 nodo + 1 arista.
fn bench_commit_small_tx(c: &mut Criterion) {
    let rt = rt();
    let dir = tempfile::tempdir().unwrap();
    let (graph, ids) = rt.block_on(seeded_graph(dir.path(), 128));

    let mut group = c.benchmark_group("commit");
    group.sample_size(20); // cada iteración paga fsyncs de WAL
    let mut i = 0usize;
    group.bench_function("small_tx_1node_1edge", |b| {
        b.to_async(&rt).iter(|| {
            i += 1;
            let g = Arc::clone(&graph);
            let target = ids[i % ids.len()];
            async move {
                let mut tx = g.begin_transaction().await.expect("begin");
                let a = tx.add_node(person(1_000_000 + i)).await.expect("add");
                tx.add_edge(Edge::new(a, target, "KNOWS")).expect("edge");
                tx.commit().await.expect("commit");
            }
        });
    });
    group.finish();
}

// (a2) Throughput con 8 escritores commiteando concurrentemente:
//      mide el efecto del group commit del WAL (1 fsync por commit).
fn bench_commit_concurrent(c: &mut Criterion) {
    let rt = rt();
    let dir = tempfile::tempdir().unwrap();
    let (graph, ids) = rt.block_on(seeded_graph(dir.path(), 128));

    let mut group = c.benchmark_group("commit");
    group.sample_size(15);
    let mut round = 0usize;
    group.bench_function("8_concurrent_small_tx", |b| {
        b.to_async(&rt).iter(|| {
            round += 1;
            let g = Arc::clone(&graph);
            let base = round * 8;
            let target = ids[round % ids.len()];
            async move {
                let mut handles = Vec::with_capacity(8);
                for k in 0..8usize {
                    let g = Arc::clone(&g);
                    handles.push(tokio::spawn(async move {
                        let mut tx = g.begin_transaction().await.expect("begin");
                        let a = tx.add_node(person(2_000_000 + base + k)).await.expect("add");
                        tx.add_edge(Edge::new(a, target, "KNOWS")).expect("edge");
                        tx.commit().await.expect("commit");
                    }));
                }
                for h in handles {
                    h.await.expect("commit task");
                }
            }
        });
    });
    group.finish();
}

// (b) Lecturas: mismo lote de 64 get_node repartido en 1/4/8 tasks.
fn bench_read_concurrency(c: &mut Criterion) {
    let rt = rt();
    let dir = tempfile::tempdir().unwrap();
    let (graph, ids) = rt.block_on(seeded_graph(dir.path(), 1024));
    let ids = Arc::new(ids);

    let mut group = c.benchmark_group("reads_64");
    for tasks in [1usize, 4, 8] {
        group.bench_with_input(BenchmarkId::from_parameter(tasks), &tasks, |b, &tasks| {
            b.to_async(&rt).iter(|| {
                let g = Arc::clone(&graph);
                let ids = Arc::clone(&ids);
                async move { read_batch(&g, &ids, tasks).await }
            });
        });
    }
    group.finish();
}

// (c) Lecturas (8 tasks) compitiendo con un escritor directo activo.
fn bench_reads_with_active_writer(c: &mut Criterion) {
    let rt = rt();
    let dir = tempfile::tempdir().unwrap();
    let (graph, ids) = rt.block_on(seeded_graph(dir.path(), 1024));
    let ids = Arc::new(ids);

    let mut group = c.benchmark_group("reads_64_with_writer");
    group.sample_size(30);
    let mut i = 0usize;
    group.bench_function("8_tasks_plus_4_edge_writes", |b| {
        b.to_async(&rt).iter(|| {
            i += 1;
            let g = Arc::clone(&graph);
            let ids = Arc::clone(&ids);
            async move {
                let writer = {
                    let g = Arc::clone(&g);
                    let ids = Arc::clone(&ids);
                    tokio::spawn(async move {
                        for k in 0..4usize {
                            let s = ids[(i + k) % ids.len()];
                            let t = ids[(i + k + 1) % ids.len()];
                            let _ = g.add_edge(Edge::new(s, t, "TOUCHES")).await;
                        }
                    })
                };
                read_batch(&g, &ids, 8).await;
                writer.await.expect("writer task");
            }
        });
    });
    group.finish();
}

// (c'') Lecturas (8 tasks) con un escritor DE FONDO que no se espera: la
// degradación real del lector. (c) espera a sus 4 escrituras, así que su
// tiempo es el máximo de leer y escribir, y en un motor con commits caros
// mide al escritor. Aquí el escritor corre durante todo el grupo a un ritmo
// FIJO (10 aristas cada 1 ms ≈ 10k/s, la misma carga en los dos motores) y
// solo se cronometra el lote de lecturas. El escritor tiene ritmo fijo y un
// tope de aristas: sin ritmo, la carga dependería del motor; sin tope, en
// sled el grupo no terminaba (la base crecía sin parar, 1.7 GB de RAM a los
// 20 min) porque criterion sigue iterando mientras el tiempo por lote sube.
// 100k aristas cubren con holgura los ~8 s de calentamiento + medición.
fn bench_reads_with_background_writer(c: &mut Criterion) {
    let rt = rt();
    let dir = tempfile::tempdir().unwrap();
    let (graph, ids) = rt.block_on(seeded_graph(dir.path(), 1024));
    let ids = Arc::new(ids);

    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writer = {
        let g = Arc::clone(&graph);
        let ids = Arc::clone(&ids);
        let stop = Arc::clone(&stop);
        rt.spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_millis(1));
            let mut i = 0usize;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) && i < 100_000 {
                tick.tick().await;
                for _ in 0..10 {
                    let s = ids[i % ids.len()];
                    let t = ids[(i + 7) % ids.len()];
                    let _ = g.add_edge(Edge::new(s, t, "BG")).await;
                    i += 1;
                }
            }
            i
        })
    };
    std::thread::sleep(std::time::Duration::from_millis(100));

    let mut group = c.benchmark_group("reads_64_background_writer");
    group.sample_size(30);
    group.bench_function("8_tasks", |b| {
        b.to_async(&rt).iter(|| {
            let g = Arc::clone(&graph);
            let ids = Arc::clone(&ids);
            async move { read_batch(&g, &ids, 8).await }
        });
    });
    group.finish();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let written = rt.block_on(writer).expect("writer task");
    eprintln!("reads_64_background_writer: el escritor de fondo insertó {written} aristas");
}

// (f) Escritores directos concurrentes: `tasks` tareas, cada una `per_task`
// `add_edge` sin transacción. Mide el throughput del applier con la cola
// llena. Es el bench con el que se cerró #139 (group commit de write-sets
// en el applier): en redb 8 tareas rinden igual que 1 (~31 µs por op, de los
// que solo ~3 son fusionables: el resto es canal, WAL y ack); en sled el
// `append_batch` por drenaje ya amortiza el WAL entre escritores (5×).
fn bench_writes_direct_concurrent(c: &mut Criterion) {
    let rt = rt();
    let dir = tempfile::tempdir().unwrap();
    let (graph, ids) = rt.block_on(seeded_graph(dir.path(), 1024));
    let ids = Arc::new(ids);

    let mut group = c.benchmark_group("writes_direct_concurrent");
    group.sample_size(10);
    let mut round = 0usize;
    for tasks in [1usize, 8] {
        // 64 aristas por iteración en total, repartidas: mismo trabajo,
        // distinto paralelismo, así el tiempo por iteración es comparable.
        let per_task = 64 / tasks;
        group.bench_with_input(BenchmarkId::from_parameter(tasks), &tasks, |b, &tasks| {
            b.to_async(&rt).iter(|| {
                round += 1;
                let g = Arc::clone(&graph);
                let ids = Arc::clone(&ids);
                async move {
                    let mut handles = Vec::with_capacity(tasks);
                    for t in 0..tasks {
                        let g = Arc::clone(&g);
                        let ids = Arc::clone(&ids);
                        handles.push(tokio::spawn(async move {
                            for k in 0..per_task {
                                let s = ids[(round * 131 + t * 17 + k) % ids.len()];
                                let d = ids[(round * 131 + t * 17 + k + 1) % ids.len()];
                                g.add_edge(Edge::new(s, d, "CONC")).await.expect("add_edge");
                            }
                        }));
                    }
                    for h in handles {
                        h.await.expect("writer task");
                    }
                }
            });
        });
    }
    group.finish();
}

// (e) Supernodo: 20k aristas directas desde el mismo origen. Hasta 0.6.0 la
// deduplicación de la adyacencia en RAM era `Vec::contains` por arista
// insertada (O(grado)): cuadrático en este patrón (#143). Correr con
// `NOPALDB_BENCH_ENGINE=redb`: en sled sobre macOS cada arista directa
// paga ~4 ms por la interacción WAL/F_FULLFSYNC (DURABILITY.md) y la
// iteración de 20k aristas tarda más de un minuto.
fn bench_supernode_fanout(c: &mut Criterion) {
    let rt = rt();
    let dir = tempfile::tempdir().unwrap();
    let (graph, ids) = rt.block_on(seeded_graph(dir.path(), 1024));
    let ids = Arc::new(ids);

    let mut group = c.benchmark_group("supernode_fanout");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(20));
    let mut round = 0usize;
    group.bench_function("20k_edges_one_source", |b| {
        b.to_async(&rt).iter(|| {
            round += 1;
            let g = Arc::clone(&graph);
            let ids = Arc::clone(&ids);
            async move {
                let hub = ids[round % ids.len()];
                for k in 0..20_000usize {
                    let t = ids[(k + round) % ids.len()];
                    g.add_edge(Edge::new(hub, t, "FAN")).await.expect("add_edge");
                }
            }
        });
    });
    group.finish();
}

// (d) Ingesta con BulkLoader: 1000 nodos por iteración. `fresh_db` abre una
// base nueva en cada iteración (crear + abrir + cerrar van dentro de la
// medición: ~18 ms en sled, ~75 ms en redb, casi todo fsync); `open_db`
// carga sobre una base ya abierta y mide solo la ingesta. Tamaño de lote
// del loader por `NOPALDB_BENCH_BATCH` (default 256): en un motor que
// escribe páginas en cada commit, el lote decide cuántos commits paga.
fn bench_bulk_load(c: &mut Criterion) {
    let rt = rt();
    let batch: usize = std::env::var("NOPALDB_BENCH_BATCH")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(256);

    let mut group = c.benchmark_group("bulk_load");
    group.sample_size(10);
    group.bench_function("1k_nodes_fresh_db", |b| {
        b.to_async(&rt).iter(|| async {
            let dir = tempfile::tempdir().unwrap();
            let graph = Graph::open_with_options(dir.path(), bench_options()).await.expect("open");
            let mut loader = graph.bulk_loader(batch);
            for i in 0..1000usize {
                loader.add_node(person(i)).await.expect("bulk add");
            }
            let stats = loader.finish().await.expect("finish");
            assert_eq!(stats.nodes_inserted, 1000);
        });
    });

    let dir = tempfile::tempdir().unwrap();
    let graph = Arc::new(rt.block_on(async {
        Graph::open_with_options(dir.path(), bench_options()).await.expect("open")
    }));
    let mut next = 0usize;
    group.bench_function("1k_nodes_open_db", |b| {
        b.to_async(&rt).iter(|| {
            let g = Arc::clone(&graph);
            let base = next;
            next += 1000;
            async move {
                let mut loader = g.bulk_loader(batch);
                for i in base..base + 1000 {
                    loader.add_node(person(i)).await.expect("bulk add");
                }
                let stats = loader.finish().await.expect("finish");
                assert_eq!(stats.nodes_inserted, 1000);
            }
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_commit_small_tx,
    bench_commit_concurrent,
    bench_read_concurrency,
    bench_reads_with_active_writer,
    bench_reads_with_background_writer,
    bench_bulk_load,
    bench_supernode_fanout,
    bench_writes_direct_concurrent
);
criterion_main!(benches);
