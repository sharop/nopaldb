// benches/graph_ops.rs
//
// Línea base de rendimiento para el trabajo de concurrencia (roadmap M2):
//   (a) throughput de commit de transacciones pequeñas (fsync-bound hoy)
//   (b) lecturas get_node con 1/4/8 tasks concurrentes
//   (c) lecturas concurrentes con un escritor activo
//   (d) ingesta con BulkLoader
//
// Correr: cargo bench -p nopaldb
// Registrar los números ANTES de aterrizar el applier (I8), el commit atómico
// (I9) y el group commit del WAL (I10) para poder demostrar la mejora.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use nopaldb::{Edge, Graph, Node, PropertyValue};
use std::sync::Arc;

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

/// Grafo persistente pre-poblado con `n` nodos; retorna también sus ids.
async fn seeded_graph(dir: &std::path::Path, n: usize) -> (Arc<Graph>, Vec<nopaldb::NodeId>) {
    let graph = Arc::new(Graph::open(dir).await.expect("open"));
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

// (d) Ingesta con BulkLoader: 1000 nodos por iteración en una base nueva.
fn bench_bulk_load(c: &mut Criterion) {
    let rt = rt();

    let mut group = c.benchmark_group("bulk_load");
    group.sample_size(10);
    group.bench_function("1k_nodes", |b| {
        b.to_async(&rt).iter(|| async {
            let dir = tempfile::tempdir().unwrap();
            let graph = Graph::open(dir.path()).await.expect("open");
            let mut loader = graph.bulk_loader(256);
            for i in 0..1000usize {
                loader.add_node(person(i)).await.expect("bulk add");
            }
            let stats = loader.finish().await.expect("finish");
            assert_eq!(stats.nodes_inserted, 1000);
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_commit_small_tx,
    bench_read_concurrency,
    bench_reads_with_active_writer,
    bench_bulk_load
);
criterion_main!(benches);
