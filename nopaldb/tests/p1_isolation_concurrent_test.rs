// tests/p1_isolation_concurrent_test.rs
//
// Integración del aislamiento con concurrencia real:
//   - conflicto write-write entre transacciones Serializable concurrentes
//   - detección de interferencia de ESCRITORES DIRECTOS (sin locks) vía
//     last_modified — el safety net de la validación
//   - el GC respeta los snapshots de transacciones abiertas (horizonte)
//   - GC concurrente con commits no corrompe la cadena de versiones
//     (regresión del race GC-vs-listas: ambos pasan por el write-gate)

#![cfg(feature = "full-isolation")]

use nopaldb::mvcc::GCConfig;
use nopaldb::{Edge, Graph, IsolationLevel, Node, NopalError, PropertyValue};
use std::sync::Arc;
use uuid::Uuid;

fn counter(id: Uuid, v: i64) -> Node {
    Node::with_id(id, "Counter").with_property("v", PropertyValue::Int(v))
}

async fn seed(graph: &Graph, id: Uuid, v: i64) -> nopaldb::Result<()> {
    let mut tx = graph.begin_transaction().await?;
    tx.add_node(counter(id, v)).await?;
    tx.commit().await
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. Dos Serializable leyendo y escribiendo el MISMO nodo: exactamente una
//    gana; la otra recibe TransactionConflict o Deadlock (nunca ambas ganan).
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_serializable_write_write_conflict() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let graph = Arc::new(Graph::open(dir.path()).await?);
    let id = Uuid::new_v4();
    seed(&graph, id, 0).await?;

    let mut wins = 0usize;
    let mut conflicts = 0usize;

    // Repetir varias rondas para ejercitar interleavings distintos
    for round in 0..8 {
        let mut handles = Vec::new();
        for k in 0..2 {
            let g = Arc::clone(&graph);
            handles.push(tokio::spawn(async move {
                let mut tx = g
                    .begin_transaction()
                    .await?
                    .with_isolation(IsolationLevel::Serializable);
                // Patrón de cliente: ante cualquier error (deadlock víctima,
                // conflicto), hacer rollback explícito para liberar locks.
                let body = async {
                    let _current = tx.get_node(id).await?; // read lock + read set
                    tx.add_node(counter(id, round * 10 + k)).await?; // write
                    Ok::<(), NopalError>(())
                }
                .await;
                match body {
                    Ok(()) => tx.commit().await,
                    Err(e) => {
                        tx.rollback_async().await.ok();
                        Err(e)
                    }
                }
            }));
        }
        for h in handles {
            match h.await.expect("task panicked") {
                Ok(()) => wins += 1,
                Err(NopalError::TransactionConflict(_))
                | Err(NopalError::Deadlock(_))
                | Err(NopalError::ConcurrencyError(_)) => conflicts += 1,
                Err(e) => panic!("unexpected error kind: {e}"),
            }
        }
    }

    assert!(wins >= 8, "at least one tx must win each round (wins={wins})");
    assert!(
        conflicts >= 1,
        "overlapping serializable writers must produce conflicts (got none in 8 rounds)"
    );
    // Nunca se pierde una ronda completa: cada ronda produce 2 resultados
    assert_eq!(wins + conflicts, 16);

    // La cadena quedó consistente: una sola versión current
    let history = graph.history(id).await?;
    assert_eq!(history.iter().filter(|v| v.valid_to.is_none()).count(), 1);

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Interferencia de un ESCRITOR DIRECTO (bypass del LockManager): la tx
//    Serializable que leyó el nodo debe recibir TransactionConflict al commit.
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serializable_detects_direct_writer_interference() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let graph = Arc::new(Graph::open(dir.path()).await?);
    let id = Uuid::new_v4();
    seed(&graph, id, 1).await?;

    let mut tx = graph
        .begin_transaction()
        .await?
        .with_isolation(IsolationLevel::Serializable);
    let read = tx.get_node(id).await?;
    assert_eq!(read.properties.get("v"), Some(&PropertyValue::Int(1)));

    // Escritura directa concurrente (no toma locks): actualiza el mismo nodo
    graph.add_node(counter(id, 99)).await?;

    // La tx escribió basándose en una lectura ya inválida
    tx.add_node(counter(id, 2)).await?;
    match tx.commit().await {
        Err(NopalError::TransactionConflict(_)) => {}
        Ok(()) => panic!("commit must fail: a direct writer invalidated the read set"),
        Err(e) => panic!("expected TransactionConflict, got: {e}"),
    }

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. El GC no puede borrar versiones que una tx abierta aún necesita
//    (use_active_horizon clampa el cutoff al snapshot más viejo en vuelo).
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gc_preserves_versions_readable_by_open_snapshots() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let graph = Arc::new(Graph::open(dir.path()).await?);
    let id = Uuid::new_v4();
    seed(&graph, id, 10).await?;

    // Snapshot abierto ANTES de las actualizaciones
    let snapshot_tx = graph
        .begin_transaction()
        .await?
        .with_isolation(IsolationLevel::RepeatableRead);
    let seen = snapshot_tx.get_node(id).await?;
    assert_eq!(seen.properties.get("v"), Some(&PropertyValue::Int(10)));

    // Generar versiones nuevas que invalidan la que el snapshot lee
    for v in [11, 12, 13] {
        seed(&graph, id, v).await?;
    }

    // GC agresivo pero respetando el horizonte activo
    let stats = graph
        .gc(GCConfig {
            cutoff_timestamp: u64::MAX,
            min_versions_to_keep: 1,
            max_nodes_per_cycle: 0,
            dry_run: false,
            use_active_horizon: true,
        })
        .await?;
    log::info!("GC stats: {:?}", stats);

    // El snapshot sigue leyendo su versión
    let still = snapshot_tx.get_node(id).await?;
    assert_eq!(
        still.properties.get("v"),
        Some(&PropertyValue::Int(10)),
        "GC deleted a version still readable by an open RepeatableRead snapshot"
    );
    snapshot_tx.rollback_async().await?;

    // Y el estado current es el último commit
    let now = graph.get_node(id).await?;
    assert_eq!(now.properties.get("v"), Some(&PropertyValue::Int(13)));

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. GC corriendo concurrente con commits: la cadena de versiones queda
//    exacta (regresión del race GC-vs-listas de versiones, ambos gated).
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn gc_concurrent_with_commits_keeps_chain_consistent() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let graph = Arc::new(Graph::open(dir.path()).await?);
    let id = Uuid::new_v4();
    seed(&graph, id, 0).await?;

    let gc_graph = Arc::clone(&graph);
    let gc_task = tokio::spawn(async move {
        for _ in 0..6 {
            let _ = gc_graph
                .gc(GCConfig {
                    cutoff_timestamp: u64::MAX,
                    min_versions_to_keep: 1,
                    max_nodes_per_cycle: 0,
                    dry_run: false,
                    use_active_horizon: true,
                })
                .await;
            tokio::task::yield_now().await;
        }
    });

    let hub = graph.add_node(Node::new("Hub")).await?;
    for i in 1..=20i64 {
        seed(&graph, id, i).await?;
        // también actividad de aristas para ejercitar más estructuras
        let n = graph.add_node(Node::new("Spoke")).await?;
        graph.add_edge(Edge::new(hub, n, "LINKS")).await?;
    }
    gc_task.await.expect("gc task panicked");

    // Cadena consistente: exactamente una versión current y lectura correcta
    let history = graph.history(id).await?;
    assert_eq!(
        history.iter().filter(|v| v.valid_to.is_none()).count(),
        1,
        "GC racing commits corrupted the version chain"
    );
    let now = graph.get_node(id).await?;
    assert_eq!(now.properties.get("v"), Some(&PropertyValue::Int(20)));

    Ok(())
}
