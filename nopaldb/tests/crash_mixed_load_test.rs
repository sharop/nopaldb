// tests/crash_mixed_load_test.rs
//
// Crash safety bajo CARGA MIXTA: el hijo intercala commits transaccionales,
// escrituras directas, deletes de nodos/aristas y updates de propiedades
// indexadas — el perfil de una app real — y muere con SIGKILL en momentos
// aleatorios. Al reabrir se verifican los invariantes estructurales y de
// índices. Complementa a crash_commit_test (perfil enfocado a commits).
//
// Rondas configurables con NOPAL_CRASH_ROUNDS (default 12; el nightly sube).

#![cfg(unix)]

use nopaldb::{Direction, Edge, Graph, Node, PropertyValue};
use std::collections::HashSet;
use std::process::{Command, Stdio};
use std::time::Duration;
use uuid::Uuid;

const ENV_DB_DIR: &str = "NOPAL_MIXED_CRASH_DB_DIR";

fn rounds() -> usize {
    std::env::var("NOPAL_CRASH_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(12)
}

/// Hijo: carga mixta infinita hasta que lo maten.
#[tokio::test]
#[ignore = "solo corre como proceso hijo del harness de crash"]
async fn mixed_load_child() {
    let Some(dir) = std::env::var_os(ENV_DB_DIR) else {
        return;
    };
    let graph = Graph::open(std::path::Path::new(&dir)).await.expect("child open");

    let hub = Uuid::from_u128(0xFACADE);
    if graph.get_node(hub).await.is_err() {
        graph
            .add_node(Node::with_id(hub, "Hub").with_property("kind", PropertyValue::String("hub".into())))
            .await
            .expect("seed hub");
    }

    let mut spokes: Vec<Uuid> = Vec::new();
    let mut i: i64 = 0;
    loop {
        i += 1;
        match i % 5 {
            // Commit transaccional: nodo indexado + arista al hub
            0 | 1 => {
                let mut tx = graph.begin_transaction().await.expect("begin");
                let n = tx
                    .add_node(
                        Node::new("Item")
                            .with_property("bucket", PropertyValue::String("mixed".into()))
                            .with_property("seq", PropertyValue::Int(i)),
                    )
                    .await
                    .expect("tx add");
                tx.add_edge(Edge::new(n, hub, "BELONGS")).expect("tx edge");
                tx.commit().await.expect("commit");
                spokes.push(n);
            }
            // Escritura directa (sin tx)
            2 => {
                let n = graph
                    .add_node(
                        Node::new("Direct")
                            .with_property("bucket", PropertyValue::String("mixed".into())),
                    )
                    .await
                    .expect("direct add");
                graph.add_edge(Edge::new(hub, n, "OWNS")).await.expect("direct edge");
                spokes.push(n);
            }
            // Update del hub vía tx (cadena de versiones + adyacencia intacta)
            3 => {
                let mut tx = graph.begin_transaction().await.expect("begin");
                tx.add_node(
                    Node::with_id(hub, "Hub")
                        .with_property("kind", PropertyValue::String("hub".into()))
                        .with_property("tick", PropertyValue::Int(i)),
                )
                .await
                .expect("hub update");
                tx.commit().await.expect("hub commit");
            }
            // Delete de un spoke viejo (nodo + sus aristas)
            _ => {
                if spokes.len() > 8
                    && let Some(victim) = spokes.first().cloned()
                {
                    let _ = graph.delete_node(victim).await;
                    spokes.remove(0);
                }
            }
        }
    }
}

/// Invariantes estructurales y de índices tras crash + reopen.
async fn assert_invariants(graph: &Graph) -> nopaldb::Result<()> {
    let nodes = graph.get_all_nodes().await?;
    let edges = graph.get_all_edges().await?;
    let node_ids: HashSet<_> = nodes.iter().map(|n| n.id).collect();

    // 1. Sin aristas huérfanas y adyacencia ↔ aristas en ambas direcciones
    for edge in &edges {
        assert!(
            node_ids.contains(&edge.source) && node_ids.contains(&edge.target),
            "orphaned edge {} after mixed-load crash",
            edge.id
        );
        let out = graph.neighbors(edge.source, Direction::Outgoing).await?;
        assert!(out.contains(&edge.target), "adjacency_out missing {}", edge.id);
        let inn = graph.neighbors(edge.target, Direction::Incoming).await?;
        assert!(inn.contains(&edge.source), "adjacency_in missing {}", edge.id);
    }

    // 2. Sin duplicados de adyacencia
    for node in &nodes {
        let out = graph.neighbors(node.id, Direction::Outgoing).await?;
        let uniq: HashSet<_> = out.iter().collect();
        assert_eq!(out.len(), uniq.len(), "duplicated adjacency for {}", node.id);
    }

    // 3. Índice de propiedades ↔ datos: todo id indexado existe y conserva el valor
    let indexed = graph
        .storage()
        .get_nodes_by_property("bucket", &PropertyValue::String("mixed".into()))
        .await?;
    for id in &indexed {
        assert!(node_ids.contains(id), "property index points to deleted node {}", id);
        let node = graph.get_node(*id).await?;
        assert_eq!(
            node.properties.get("bucket"),
            Some(&PropertyValue::String("mixed".into())),
            "indexed node {} lost its property value",
            id
        );
    }

    // 4. Cadena del hub: una sola versión current, timestamps monotónicos,
    //    y el reloj persistido nunca por debajo del máximo de la cadena
    let hub = Uuid::from_u128(0xFACADE);
    if node_ids.contains(&hub) {
        let mut history = graph.history(hub).await?;
        assert_eq!(
            history.iter().filter(|v| v.valid_to.is_none()).count(),
            1,
            "hub must have exactly one current version"
        );
        history.sort_by_key(|v| v.version);
        for pair in history.windows(2) {
            assert!(pair[1].timestamp >= pair[0].timestamp, "timestamps regressed");
        }
        let max_ts = history.iter().map(|v| v.timestamp).max().unwrap_or(0);
        let clock = graph
            .storage()
            .get_meta_u64(nopaldb::storage::META_NEXT_TIMESTAMP)
            .await?
            .unwrap_or(0);
        assert!(
            clock > max_ts,
            "persisted clock ({clock}) must stay above the newest version ({max_ts})"
        );
    }

    // 5. El grafo sigue siendo escribible tras el recovery
    let mut tx = graph.begin_transaction().await?;
    tx.add_node(Node::new("Probe")).await?;
    tx.commit().await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn mixed_load_crash_recovery_rounds() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe().expect("current_exe");

    for round in 0..rounds() {
        let mut child = Command::new(&exe)
            .args(["mixed_load_child", "--ignored", "--exact", "--nocapture"])
            .env(ENV_DB_DIR, dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn child");

        let ms = 50 + (round as u64 * 53) % 300;
        tokio::time::sleep(Duration::from_millis(ms)).await;

        child.kill().expect("SIGKILL child");
        let _ = child.wait();

        let graph = Graph::open(dir.path()).await?;
        assert_invariants(&graph).await?;
        drop(graph);
    }

    Ok(())
}
