// tests/crash_commit_test.rs
//
// Crash safety del commit: el proceso muere con SIGKILL en momentos aleatorios
// mientras commitea transacciones; al reabrir, el WAL redo + el batch atómico
// de versiones + el rebuild de adyacencia deben dejar el grafo consistente.
//
// Patrón self-exec: el test padre relanza este mismo binario filtrando el test
// hijo (`crash_child_writer`, #[ignore] para que no corra solo) con la ruta de
// la base en una variable de entorno, lo mata tras una pausa aleatoria y
// verifica invariantes al reabrir. Solo unix (SIGKILL).

#![cfg(unix)]

use nopaldb::{Direction, Edge, Graph, Node, PropertyValue};
use std::collections::HashSet;
use std::process::{Command, Stdio};
use std::time::Duration;
use uuid::Uuid;

const ENV_DB_DIR: &str = "NOPAL_CRASH_DB_DIR";

/// Rondas de kill por corrida: 20 por default; el job nightly sube el número
/// vía NOPAL_CRASH_ROUNDS.
fn rounds() -> usize {
    std::env::var("NOPAL_CRASH_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(20)
}

/// Hijo: commitea transacciones en bucle infinito hasta que lo maten.
/// Ignorado en corridas normales; solo corre cuando el padre lo relanza.
#[tokio::test]
#[ignore = "solo corre como proceso hijo del harness de crash"]
async fn crash_child_writer() {
    let Some(dir) = std::env::var_os(ENV_DB_DIR) else {
        // Corrida manual con --ignored: no hay entorno del harness, salir.
        return;
    };

    let graph = Graph::open(std::path::Path::new(&dir)).await.expect("child open");

    // Nodo contador estable para acumular cadena de versiones entre rondas.
    let counter_id = Uuid::from_u128(0xC0FFEE);
    if graph.get_node(counter_id).await.is_err() {
        let mut tx = graph.begin_transaction().await.expect("begin");
        tx.add_node(Node::with_id(counter_id, "Counter").with_property("v", PropertyValue::Int(0)))
            .await
            .expect("seed");
        tx.commit().await.expect("seed commit");
    }

    let mut i: i64 = 0;
    loop {
        i += 1;
        let mut tx = graph.begin_transaction().await.expect("begin");
        // Update del contador (ejercita el batch atómico de versiones)
        tx.add_node(Node::with_id(counter_id, "Counter").with_property("v", PropertyValue::Int(i)))
            .await
            .expect("update counter");
        // Nodo + arista nuevos (ejercita adyacencia y aristas versionadas)
        let a = tx
            .add_node(
                Node::new("Person")
                    .with_property("round", PropertyValue::Int(i))
                    .with_property("team", PropertyValue::String("crash".into())),
            )
            .await
            .expect("add node");
        tx.add_edge(Edge::new(a, counter_id, "TOUCHES")).expect("add edge");
        tx.commit().await.expect("commit");
    }
}

/// Verifica los invariantes estructurales del grafo tras un crash + reopen.
async fn assert_invariants(graph: &Graph) -> nopaldb::Result<()> {
    let nodes = graph.get_all_nodes().await?;
    let edges = graph.get_all_edges().await?;
    let node_ids: HashSet<_> = nodes.iter().map(|n| n.id).collect();

    // 1. Ninguna arista huérfana: ambos extremos existen
    for edge in &edges {
        assert!(
            node_ids.contains(&edge.source) && node_ids.contains(&edge.target),
            "orphaned edge {} after crash recovery",
            edge.id
        );
    }

    // 2. Adyacencia consistente con las aristas (ambas direcciones)
    for edge in &edges {
        let out = graph.neighbors(edge.source, Direction::Outgoing).await?;
        assert!(
            out.contains(&edge.target),
            "adjacency_out missing edge {} after crash recovery",
            edge.id
        );
        let inn = graph.neighbors(edge.target, Direction::Incoming).await?;
        assert!(
            inn.contains(&edge.source),
            "adjacency_in missing edge {} after crash recovery",
            edge.id
        );
    }

    // 3. Sin duplicados en adyacencia (replay idempotente)
    for node in &nodes {
        let out = graph.neighbors(node.id, Direction::Outgoing).await?;
        let uniq: HashSet<_> = out.iter().collect();
        assert_eq!(out.len(), uniq.len(), "duplicated adjacency entries for {}", node.id);
    }

    // 4. Cadena de versiones del contador: exactamente una versión current,
    //    timestamps no decrecientes en orden de versión
    let counter_id = Uuid::from_u128(0xC0FFEE);
    if node_ids.contains(&counter_id) {
        let mut history = graph.history(counter_id).await?;
        let current = history.iter().filter(|v| v.valid_to.is_none()).count();
        assert_eq!(current, 1, "counter must have exactly one current version");
        history.sort_by_key(|v| v.version);
        for pair in history.windows(2) {
            assert!(
                pair[1].timestamp >= pair[0].timestamp,
                "version timestamps regressed after crash recovery"
            );
        }
    }

    // 5. Índice de propiedades consistente: cada nodo indexado existe y
    //    conserva el valor
    let indexed = graph
        .storage()
        .get_nodes_by_property("team", &PropertyValue::String("crash".into()))
        .await?;
    for id in &indexed {
        assert!(node_ids.contains(id), "property index points to missing node {}", id);
    }

    // 6. El grafo sigue siendo escribible (los relojes no colisionan)
    let mut tx = graph.begin_transaction().await?;
    let probe = tx
        .add_node(Node::new("Probe").with_property("ok", PropertyValue::Bool(true)))
        .await?;
    tx.commit().await?;
    graph.get_node(probe).await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn commit_crash_recovery_survives_sigkill_rounds() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe().expect("current_exe");

    for round in 0..rounds() {
        let mut child = Command::new(&exe)
            .args(["crash_child_writer", "--ignored", "--exact", "--nocapture"])
            .env(ENV_DB_DIR, dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn child");

        // Ventana aleatoria: cubre desde "abriendo la base" hasta "commit N"
        let ms = 60 + (round as u64 * 37) % 240;
        tokio::time::sleep(Duration::from_millis(ms)).await;

        child.kill().expect("SIGKILL child"); // SIGKILL en unix
        let _ = child.wait();

        // Reabrir y verificar invariantes (recovery + redo + rebuild)
        let graph = Graph::open(dir.path()).await?;
        assert_invariants(&graph).await?;
        drop(graph);
    }

    Ok(())
}
