// tests/concurrent_writers_test.rs
//
// Suite de estrés de escritores concurrentes. Define "hecho" para el trabajo
// de concurrencia: las operaciones lógicas de escritura abarcan varias llamadas
// a storage sin aislamiento conjunto, así que escritores concurrentes pueden
// perder actualizaciones (lost update) en estructuras derivadas.
//
// Estado por test:
// - Los paths TRANSACCIONALES están serializados por el commit lock del Graph
//   → esos tests deben pasar hoy y quedan como regresión.
// - Los paths DIRECTOS (sin transacción) pasan por el mismo embudo desde que
//   existe el single-writer apply (write-gate, src/graph/applier.rs), así que
//   TODA la suite corre como regresión activa. Si un test falla aquí, hay una
//   escritura que quedó fuera del embudo.

use nopaldb::{Direction, Edge, Graph, Node, PropertyValue};
use std::sync::Arc;
use uuid::Uuid;

const WRITERS: usize = 32;

fn person(name: &str) -> Node {
    Node::new("Person").with_property("name", PropertyValue::String(name.into()))
}

// ─────────────────────────────────────────────────────────────────────────────
// A. Escrituras directas de aristas al MISMO nodo. Antes: la adyacencia se
//    snapshoteaba bajo el lock pero se persistía después de soltarlo (un
//    snapshot viejo podía sobrescribir a uno nuevo). Ahora snapshot y
//    persistencia ocurren dentro del write-gate — regresión activa.
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn direct_concurrent_edges_to_same_node_keep_adjacency_exact() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let graph = Arc::new(Graph::open(dir.path()).await?);

    let hub = graph.add_node(person("hub")).await?;
    let mut targets = Vec::with_capacity(WRITERS);
    for i in 0..WRITERS {
        targets.push(graph.add_node(person(&format!("t{}", i))).await?);
    }

    let mut handles = Vec::with_capacity(WRITERS);
    for target in targets {
        let g = Arc::clone(&graph);
        handles.push(tokio::spawn(async move {
            g.add_edge(Edge::new(hub, target, "KNOWS")).await
        }));
    }
    for h in handles {
        h.await.expect("task panicked")?;
    }

    // Adyacencia en memoria y persistida deben tener exactamente WRITERS aristas
    let neighbors = graph.neighbors(hub, Direction::Outgoing).await?;
    assert_eq!(
        neighbors.len(),
        WRITERS,
        "adjacency lost {} edge(s) under concurrent direct writers",
        WRITERS - neighbors.len()
    );

    // Y tras reabrir (estado persistido), el conteo debe mantenerse
    drop(neighbors);
    let graph = reopen(graph, dir.path()).await?;
    let neighbors = graph.neighbors(hub, Direction::Outgoing).await?;
    assert_eq!(
        neighbors.len(),
        WRITERS,
        "persisted adjacency lost {} edge(s)",
        WRITERS - neighbors.len()
    );

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// B. Escrituras directas de nodos con el MISMO valor de propiedad indexada
//    (race: lista bajo `idx:prop:{prop}:{value}` se actualiza read-modify-write)
//    Serializado por el single-writer apply — regresión activa.
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn direct_concurrent_nodes_same_property_keep_index_exact() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let graph = Arc::new(Graph::open(dir.path()).await?);

    let mut handles = Vec::with_capacity(WRITERS);
    for i in 0..WRITERS {
        let g = Arc::clone(&graph);
        handles.push(tokio::spawn(async move {
            let node = Node::new("City")
                .with_property("country", PropertyValue::String("MX".into()))
                .with_property("n", PropertyValue::Int(i as i64));
            g.add_node(node).await
        }));
    }
    for h in handles {
        h.await.expect("task panicked")?;
    }

    let indexed = graph
        .storage()
        .get_nodes_by_property("country", &PropertyValue::String("MX".into()))
        .await?;
    assert_eq!(
        indexed.len(),
        WRITERS,
        "property index lost {} entrie(s) under concurrent direct writers",
        WRITERS - indexed.len()
    );

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// C. Commits concurrentes con write-sets DISJUNTOS (serializados por el
//    commit lock del Graph) — regresión: debe pasar hoy.
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_disjoint_tx_commits_keep_exact_counts() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let graph = Arc::new(Graph::open(dir.path()).await?);

    let mut handles = Vec::with_capacity(WRITERS);
    for i in 0..WRITERS {
        let g = Arc::clone(&graph);
        handles.push(tokio::spawn(async move {
            let mut tx = g.begin_transaction().await?;
            let a = tx.add_node(person(&format!("a{}", i))).await?;
            let b = tx.add_node(person(&format!("b{}", i))).await?;
            tx.add_edge(Edge::new(a, b, "KNOWS"))?;
            tx.commit().await
        }));
    }
    for h in handles {
        h.await.expect("task panicked")?;
    }

    let nodes = graph.get_all_nodes().await?;
    let edges = graph.get_all_edges().await?;
    assert_eq!(nodes.len(), WRITERS * 2, "node count mismatch after concurrent commits");
    assert_eq!(edges.len(), WRITERS, "edge count mismatch after concurrent commits");

    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// D. Commits concurrentes actualizando el MISMO nodo — la cadena MVCC debe
//    terminar con exactamente WRITERS+1 versiones y timestamps crecientes.
//    (Serializado por el commit lock) — regresión: debe pasar hoy.
// ─────────────────────────────────────────────────────────────────────────────
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_tx_updates_same_node_keep_version_chain_exact() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let graph = Arc::new(Graph::open(dir.path()).await?);

    let node_id = Uuid::new_v4();
    let mut tx = graph.begin_transaction().await?;
    tx.add_node(Node::with_id(node_id, "Counter").with_property("v", PropertyValue::Int(0)))
        .await?;
    tx.commit().await?;

    let mut handles = Vec::with_capacity(WRITERS);
    for i in 0..WRITERS {
        let g = Arc::clone(&graph);
        handles.push(tokio::spawn(async move {
            let mut tx = g.begin_transaction().await?;
            tx.add_node(
                Node::with_id(node_id, "Counter").with_property("v", PropertyValue::Int(i as i64)),
            )
            .await?;
            tx.commit().await
        }));
    }
    for h in handles {
        h.await.expect("task panicked")?;
    }

    let mut history = graph.history(node_id).await?;
    assert_eq!(
        history.len(),
        WRITERS + 1,
        "version chain lost {} version(s)",
        (WRITERS + 1).saturating_sub(history.len())
    );

    history.sort_by_key(|v| v.version);
    for pair in history.windows(2) {
        assert!(
            pair[1].timestamp >= pair[0].timestamp,
            "version timestamps must be non-decreasing in commit order"
        );
    }

    // Exactamente una versión current (valid_to == None)
    let current = history.iter().filter(|v| v.valid_to.is_none()).count();
    assert_eq!(current, 1, "exactly one version must be current");

    Ok(())
}

/// Cierra el grafo (drop del último Arc) y lo reabre desde disco.
async fn reopen(graph: Arc<Graph>, path: &std::path::Path) -> nopaldb::Result<Arc<Graph>> {
    drop(graph);
    Ok(Arc::new(Graph::open(path).await?))
}
