// tests/mvcc_gc_age_and_reads_test.rs
//
// GC por antigüedad, horizonte del GC y lecturas de viaje en el tiempo sin
// fallback al estado actual.
//
// Hasta 0.9.4:
// - `GCConfig::older_than_*` comparaba milisegundos Unix con los `valid_to`
//   del reloj LÓGICO: `older_than_days(7)` borraba toda la historia cerrada,
//   y `gc_default` también;
// - `Graph::get_node_at` devolvía el estado ACTUAL cuando no había versión
//   en el timestamp (nodo creado después, sin historia o recolectado).
//
// Dominio ficticio: un vivero con lotes de plantas.

use nopaldb::error::NopalError;
use nopaldb::mvcc::GCConfig;
use nopaldb::{Graph, Node, NodeId, PropertyValue};
use std::time::Duration;

/// Crea un lote con `riegos` versiones (una por commit) y devuelve su id.
async fn lote_con_versiones(graph: &Graph, nombre: &str, riegos: i64) -> nopaldb::Result<NodeId> {
    let mut tx = graph.begin_transaction().await?;
    let id = tx
        .add_node(
            Node::new("Lote")
                .with_property("nombre", PropertyValue::String(nombre.into()))
                .with_property("riegos", PropertyValue::Int(1)),
        )
        .await?;
    tx.commit().await?;
    for r in 2..=riegos {
        regar(graph, id, r).await?;
    }
    Ok(id)
}

async fn regar(graph: &Graph, id: NodeId, riegos: i64) -> nopaldb::Result<()> {
    let mut tx = graph.begin_transaction().await?;
    let mut node = graph.get_node(id).await?;
    node.properties.insert("riegos".into(), PropertyValue::Int(riegos));
    tx.add_node(node).await?;
    tx.commit().await
}

fn riegos(node: &Node) -> Option<&PropertyValue> {
    node.properties.get("riegos")
}

// ─── GC por antigüedad ──────────────────────────────────────────────────────

/// Toda la historia tiene segundos de edad: un GC de "más de 7 días" no
/// puede borrar nada. En 0.9.4 borraba las dos versiones cerradas.
#[tokio::test]
async fn gc_older_than_days_keeps_recent_history() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;
    let id = lote_con_versiones(&graph, "lote-norte", 3).await?;

    let stats = graph.gc(GCConfig::older_than_days(7)).await?;
    assert_eq!(stats.versions_deleted, 0, "recent history deleted: {stats:?}");
    assert_eq!(graph.history(id).await?.len(), 3);

    let stats = graph.gc(GCConfig::older_than_hours(1)).await?;
    assert_eq!(stats.versions_deleted, 0, "recent history deleted: {stats:?}");
    assert_eq!(graph.history(id).await?.len(), 3);
    Ok(())
}

/// `gc_default` promete "más de 7 días": tampoco borra historia reciente.
#[tokio::test]
async fn gc_default_keeps_recent_history() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;
    let id = lote_con_versiones(&graph, "lote-sur", 3).await?;

    let stats = graph.gc_default().await?;
    assert_eq!(stats.versions_deleted, 0, "gc_default deleted recent history: {stats:?}");
    assert_eq!(graph.history(id).await?.len(), 3);
    Ok(())
}

/// La edad se mide de verdad: lo cerrado antes de una marca vieja se borra,
/// lo cerrado después se conserva.
#[tokio::test]
async fn gc_older_than_ms_splits_old_and_recent_versions() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;
    // v1, v2, v3: v1 y v2 cerradas.
    let id = lote_con_versiones(&graph, "lote-este", 3).await?;

    // Un GC por antigüedad (dry run) deja una marca de reloj "ahora".
    let probe = graph.gc(GCConfig::older_than_days(1).dry_run()).await?;
    assert_eq!(probe.versions_deleted, 0);
    tokio::time::sleep(Duration::from_millis(400)).await;

    // v4 cierra v3 DESPUÉS de la marca: v3 tiene menos de 400 ms cerrada.
    regar(&graph, id, 4).await?;

    let stats = graph.gc(GCConfig::older_than_ms(250)).await?;
    let kept: Vec<u64> = graph.history(id).await?.iter().map(|v| v.version).collect();
    assert_eq!(
        kept,
        vec![4, 3],
        "only v1 and v2 were closed more than 250 ms ago (stats: {stats:?})"
    );
    assert_eq!(stats.versions_deleted, 2);

    // Edad 0 = todo lo cerrado hasta ahora.
    graph.gc(GCConfig::older_than_ms(0)).await?;
    let kept: Vec<u64> = graph.history(id).await?.iter().map(|v| v.version).collect();
    assert_eq!(kept, vec![4]);
    Ok(())
}

// ─── Horizonte del GC ───────────────────────────────────────────────────────

#[tokio::test]
async fn gc_horizon_is_persisted_and_guards_old_reads() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let (id, horizon, ts_v2, ts_v3) = {
        let graph = Graph::open(dir.path()).await?;
        let id = lote_con_versiones(&graph, "lote-oeste", 3).await?;
        assert_eq!(graph.gc_horizon()?, 0, "no GC yet");

        let history = graph.history(id).await?; // más reciente primero
        let ts_v2 = history[1].valid_from;
        let ts_v3 = history[0].valid_from;

        // Un dry run no mueve el horizonte.
        graph.gc(GCConfig::before_timestamp(u64::MAX).dry_run()).await?;
        assert_eq!(graph.gc_horizon()?, 0, "dry run must not move the horizon");

        let stats = graph.gc(GCConfig::before_timestamp(u64::MAX)).await?;
        assert_eq!(stats.versions_deleted, 2);
        // Exacto: el mayor valid_to borrado (el de v2 = valid_from de v3),
        // no el corte pedido (u64::MAX).
        let horizon = graph.gc_horizon()?;
        assert_eq!(horizon, ts_v3);
        assert_eq!(graph.stats().await?.gc.horizon, horizon);
        graph.close().await?;
        (id, horizon, ts_v2, ts_v3)
    };

    let graph = Graph::open(dir.path()).await?;
    assert_eq!(graph.gc_horizon()?, horizon, "the GC horizon must survive a reopen");

    // Por debajo del horizonte la versión de entonces ya no está: se dice.
    let err = graph.get_node_at(id, ts_v2).await.unwrap_err();
    assert!(matches!(err, NopalError::HistoryUnavailable(_)), "got {err:?}");
    // Desde el horizonte, la historia está completa.
    let now = graph.get_node_at(id, ts_v3).await?;
    assert_eq!(riegos(&now), Some(&PropertyValue::Int(3)));
    Ok(())
}

// ─── get_node_at sin fallback ───────────────────────────────────────────────

/// Un nodo creado después del timestamp no existía entonces. En 0.9.4
/// `get_node_at` devolvía su estado actual.
#[tokio::test]
async fn get_node_at_before_creation_is_not_found() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;
    let viejo = lote_con_versiones(&graph, "lote-viejo", 1).await?;
    let t_viejo = graph.history(viejo).await?[0].valid_from;
    let nuevo = lote_con_versiones(&graph, "lote-nuevo", 1).await?;

    let err = graph.get_node_at(nuevo, t_viejo).await.unwrap_err();
    assert!(matches!(err, NopalError::NodeNotFound(_)), "got {err:?}");

    // El snapshot omite el que no existía, sin inventarlo.
    let nodes = graph.as_of(t_viejo).get_nodes(&[viejo, nuevo]).await?;
    let ids: Vec<NodeId> = nodes.iter().map(|n| n.id).collect();
    assert_eq!(ids, vec![viejo]);
    Ok(())
}

/// La versión vieja de un nodo con historia se lee tal cual era.
#[tokio::test]
async fn get_node_at_reads_the_version_of_that_moment() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;
    let id = lote_con_versiones(&graph, "lote-centro", 2).await?;
    let t1 = graph.history(id).await?[1].valid_from;

    let then = graph.get_node_at(id, t1).await?;
    assert_eq!(riegos(&then), Some(&PropertyValue::Int(1)));
    let now = graph.get_node_at(id, graph.last_commit_timestamp()).await?;
    assert_eq!(riegos(&now), Some(&PropertyValue::Int(2)));
    Ok(())
}

/// Un nodo sin historia: su estado en el pasado no se conoce. En 0.9.4 se
/// devolvía el actual. Hasta 0.10.0 era lo que dejaba `add_node` directo;
/// desde 0.10.1 este versiona, y el registro sin cadena se fabrica con la
/// primitiva de bajo nivel `Storage::insert_node`.
#[tokio::test]
async fn get_node_at_without_history_is_an_error() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;
    let ancla = lote_con_versiones(&graph, "lote-ancla", 1).await?;
    let t_ancla = graph.history(ancla).await?[0].valid_from;

    let suelto_nodo =
        Node::new("Lote").with_property("nombre", PropertyValue::String("lote-suelto".into()));
    let suelto = suelto_nodo.id;
    graph.storage().insert_node(&suelto_nodo).await?;
    assert!(graph.history(suelto).await?.is_empty(), "precondition: low-level writes are unversioned");

    let err = graph.get_node_at(suelto, t_ancla).await.unwrap_err();
    assert!(matches!(err, NopalError::HistoryUnavailable(_)), "got {err:?}");

    // El snapshot no lo omite en silencio: un resultado incompleto que
    // parece completo es peor que un error.
    let err = graph.as_of(t_ancla).get_nodes(&[ancla, suelto]).await.unwrap_err();
    assert!(matches!(err, NopalError::HistoryUnavailable(_)), "got {err:?}");

    // Un id que no existe ni existió sigue siendo NodeNotFound.
    let err = graph.get_node_at(uuid::Uuid::new_v4(), t_ancla).await.unwrap_err();
    assert!(matches!(err, NopalError::NodeNotFound(_)), "got {err:?}");
    Ok(())
}

// ─── last_commit_timestamp ──────────────────────────────────────────────────

#[tokio::test]
async fn last_commit_timestamp_tracks_applied_writes() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await?;
    let t0 = graph.last_commit_timestamp();

    let id = lote_con_versiones(&graph, "lote-reloj", 1).await?;
    let t1 = graph.last_commit_timestamp();
    assert!(t1 > t0);
    assert_eq!(t1, graph.history(id).await?[0].valid_from, "= commit ts of the last write");

    // Ni begin_transaction ni un rollback ni un commit vacío escriben.
    let tx = graph.begin_transaction().await?;
    tx.rollback_async().await?;
    graph.begin_transaction().await?.commit().await?;
    assert_eq!(graph.last_commit_timestamp(), t1);

    regar(&graph, id, 2).await?;
    let t2 = graph.last_commit_timestamp();
    assert!(t2 > t1);
    assert_eq!(t2, graph.history(id).await?[0].valid_from);

    // Una escritura directa también cuenta.
    graph.add_node(Node::new("Lote")).await?;
    let t3 = graph.last_commit_timestamp();
    assert!(t3 > t2);

    // Tras reabrir no retrocede.
    graph.close().await?;
    drop(graph);
    let graph = Graph::open(dir.path()).await?;
    assert!(graph.last_commit_timestamp() >= t3);
    let then = graph.get_node_at(id, t1).await?;
    assert_eq!(riegos(&then), Some(&PropertyValue::Int(1)));
    Ok(())
}
