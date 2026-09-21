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

use nopaldb::{Direction, Edge, Graph, Node, PropertyValue, StorageEngine, StorageOptions};
use std::collections::HashSet;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;
use uuid::Uuid;

const ENV_DB_DIR: &str = "NOPAL_CRASH_DB_DIR";
/// Motor del harness: `sled` (default) o `redb`.
const ENV_ENGINE: &str = "NOPAL_CRASH_ENGINE";
/// Archivo (junto a la base) donde el hijo anota cada escritura DIRECTA dos
/// veces: la INTENCIÓN antes de pedirla (`n?:<uuid>`, `e?:<uuid>`,
/// `d?:<uuid>`) y el ACK después (`n:`, `e:`, `d:`). Un kill entre la op y
/// su línea de ack deja solo la intención: esa entidad queda en estado
/// desconocido y no se afirma nada sobre ella. Sin fsync a propósito: un
/// SIGKILL conserva lo que el proceso ya escribió al archivo, igual que
/// conserva el WAL, y eso es exactamente la garantía de
/// `DirectWriteDurability::ProcessCrash` que aquí se comprueba: todo lo
/// confirmado antes del kill está tras reabrir.
const ACKED_FILE: &str = "direct_acked.log";

/// Motor: `NOPAL_CRASH_ENGINE=sled|redb`; sin la variable, el default del
/// build (sled si está compilado; redb si es el único backend, que es como
/// lo corre el paso redb de CI).
fn engine() -> StorageEngine {
    match std::env::var(ENV_ENGINE).as_deref() {
        Ok("redb") => StorageEngine::Redb,
        Ok("sled") => StorageEngine::Sled,
        // `Auto`: el motor del directorio si ya hay base, si no el del build.
        _ => StorageOptions::default().engine,
    }
}

async fn open(dir: &std::path::Path) -> Graph {
    Graph::open_with_options(dir, StorageOptions { engine: engine(), ..Default::default() })
        .await
        .expect("open")
}

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

    let dir = std::path::PathBuf::from(dir);
    // Umbral de checkpoint diminuto: el WAL se trunca varias veces por
    // ronda, así que el kill cae con frecuencia justo después de un
    // truncado (#150). El padre reabre con el umbral por defecto: lo que
    // debe sobrevivir no depende de él.
    let graph = Graph::open_with_options(
        &dir,
        StorageOptions { engine: engine(), wal_checkpoint_bytes: 32 * 1024, ..Default::default() },
    )
    .await
    .expect("open child");
    let mut acked = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(ACKED_FILE))
        .expect("acked log");

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

        // Escrituras DIRECTAS (sin transacción): nodo, arista y, cada tres
        // rondas, el borrado del nodo directo anterior. Cada una se anota
        // tras su ack; el padre exige que todas estén tras el crash.
        let node = Node::new("Direct")
            .with_property("round", PropertyValue::Int(i))
            .with_property("team", PropertyValue::String("crash".into()));
        let d = node.id;
        writeln!(acked, "n?:{d}").expect("log");
        graph.add_node(node).await.expect("direct add_node");
        writeln!(acked, "n:{d}").expect("log");
        let edge = Edge::new(d, counter_id, "DIRECT");
        let e = edge.id;
        writeln!(acked, "e?:{e}").expect("log");
        graph.add_edge(edge).await.expect("direct add_edge");
        writeln!(acked, "e:{e}").expect("log");
        if i % 3 == 0 {
            writeln!(acked, "d?:{d}").expect("log");
            graph.delete_node(d).await.expect("direct delete_node");
            writeln!(acked, "d:{d}").expect("log");
        }
        // Checkpoint explícito de vez en cuando, además del automático: es
        // la llamada que Python expone y la que un usuario haría tras una
        // carga. Nada de lo confirmado antes puede perderse con él.
        if i % 17 == 0 {
            graph.checkpoint().await.expect("checkpoint");
        }
    }
}

/// Todo lo que el hijo confirmó (ack) de sus escrituras directas está tras
/// reabrir: nodos y aristas creados existen (salvo los borrados o en
/// borrado incierto), y los borrados confirmados no están. Las entidades
/// con intención sin ack (kill en medio) no se afirman.
async fn assert_direct_writes_recovered(graph: &Graph, dir: &std::path::Path) -> nopaldb::Result<()> {
    let Ok(log) = std::fs::read_to_string(dir.join(ACKED_FILE)) else { return Ok(()) };
    let mut created: Vec<Uuid> = Vec::new();
    let mut edges: Vec<(Uuid, usize)> = Vec::new(); // (arista, índice del nodo origen en `created`)
    let mut deleted: HashSet<Uuid> = HashSet::new();
    let mut maybe_deleted: HashSet<Uuid> = HashSet::new();
    let mut intents = 0usize;
    for line in log.lines() {
        // Una última línea rasgada por el kill se ignora.
        let Some((kind, id)) = line.split_once(':') else { continue };
        let Ok(id) = Uuid::parse_str(id) else { continue };
        match kind {
            "n?" => intents += 1,
            "n" => created.push(id),
            "e" => edges.push((id, created.len().saturating_sub(1))),
            "d?" => {
                maybe_deleted.insert(id);
            }
            "d" => {
                deleted.insert(id);
            }
            _ => {}
        }
    }
    let _ = intents;
    let nodes: HashSet<Uuid> = graph.get_all_nodes().await?.into_iter().map(|n| n.id).collect();
    let all_edges = graph.get_all_edges().await?;
    for id in &created {
        if deleted.contains(id) {
            if nodes.contains(id) {
                return Err(nopaldb::NopalError::custom(format!(
                    "nodo directo {id} borrado (ack) sigue tras el crash"
                )));
            }
        } else if !maybe_deleted.contains(id) && !nodes.contains(id) {
            return Err(nopaldb::NopalError::custom(format!(
                "nodo directo {id} confirmado se perdió en el crash"
            )));
        }
    }
    for (eid, src_idx) in &edges {
        let present = all_edges.iter().any(|e| e.id == *eid);
        let src = created.get(*src_idx).copied();
        let src_gone = src.is_some_and(|s| deleted.contains(&s) || maybe_deleted.contains(&s));
        if !present && !src_gone {
            return Err(nopaldb::NopalError::custom(format!(
                "arista directa {eid} confirmada se perdió en el crash"
            )));
        }
        if present && src.is_some_and(|s| deleted.contains(&s)) {
            return Err(nopaldb::NopalError::custom(format!(
                "arista {eid} de un nodo borrado (ack) sigue tras el crash"
            )));
        }
    }
    Ok(())
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

    // 6. El índice no conserva valores que el nodo YA NO tiene.
    //
    //    El hijo reescribe `v` del contador en cada commit, así que cada
    //    ronda deja valores viejos que hay que retractar. El retract ocurre
    //    ANTES de escribir la versión nueva —la única ventana en la que el
    //    valor viejo aún existe— y ese orden es lo que hace converger al
    //    redo: si el crash cae antes del retract, el replay lo recalcula
    //    contra el nodo viejo; si cae después, lo viejo ya salió y el replay
    //    solo reinserta lo nuevo.
    if node_ids.contains(&counter_id) {
        let counter = graph.get_node(counter_id).await?;
        if let Some(PropertyValue::Int(current)) = counter.properties.get("v") {
            for older in (0..*current).rev().take(5) {
                let hits = graph
                    .storage()
                    .get_nodes_by_property("v", &PropertyValue::Int(older))
                    .await?;
                assert!(
                    !hits.contains(&counter_id),
                    "el índice conserva v={older} para el contador, que ahora vale {current}"
                );
            }
        }
    }

    // 7. El grafo sigue siendo escribible (los relojes no colisionan)
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
    // Depuración: con NOPAL_CRASH_KEEP el directorio no se borra y se imprime.
    if std::env::var_os("NOPAL_CRASH_KEEP").is_some() {
        eprintln!("crash harness dir: {}", dir.path().display());
    }
    let exe = std::env::current_exe().expect("current_exe");

    for round in 0..rounds() {
        let mut child = Command::new(&exe)
            .args(["crash_child_writer", "--ignored", "--exact", "--nocapture"])
            .env(ENV_DB_DIR, dir.path())
            .env(ENV_ENGINE, std::env::var(ENV_ENGINE).unwrap_or_else(|_| "auto".to_string()))
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
        let graph = open(dir.path()).await;
        assert_invariants(&graph).await?;
        let direct = assert_direct_writes_recovered(&graph, dir.path()).await;
        drop(graph);
        if direct.is_err() && std::env::var_os("NOPAL_CRASH_KEEP").is_some() {
            let _ = dir.into_path();
            return direct;
        }
        direct?;
    }

    Ok(())
}
