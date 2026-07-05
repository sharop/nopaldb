// tests/clock_persistence_test.rs
//
// Los relojes lógicos (`next_timestamp`, `next_tx_id`) deben sobrevivir
// reinicios. Si se reiniciaran en 1, los `valid_from`/`valid_to` de las
// versiones nuevas colisionarían con las ya persistidas y el time-travel
// (`history()`, `get_node_at()`) dejaría de ser fiable entre sesiones.

use nopaldb::storage::{Storage, META_NEXT_TIMESTAMP, META_NEXT_TX_ID};
use nopaldb::{Graph, Node, PropertyValue};
use uuid::Uuid;

fn person(id: Uuid, age: i64) -> Node {
    Node::with_id(id, "Person").with_property("age", PropertyValue::Int(age))
}

/// Commit de una versión del nodo `id` con `age` dado; retorna el grafo intacto.
async fn commit_age(graph: &Graph, id: Uuid, age: i64) -> nopaldb::Result<()> {
    let mut tx = graph.begin_transaction().await?;
    tx.add_node(person(id, age)).await?;
    tx.commit().await
}

#[tokio::test]
async fn restart_preserves_mvcc_history_order() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let node_id = Uuid::new_v4();

    // Sesión 1: dos versiones
    {
        let graph = Graph::open(dir.path()).await?;
        commit_age(&graph, node_id, 30).await?;
        commit_age(&graph, node_id, 31).await?;

        let history = graph.history(node_id).await?;
        assert_eq!(history.len(), 2, "expected 2 versions before restart");
    }

    // Sesión 2: tercera versión tras reabrir
    let graph = Graph::open(dir.path()).await?;
    commit_age(&graph, node_id, 32).await?;

    let mut history = graph.history(node_id).await?;
    assert_eq!(history.len(), 3, "expected 3 versions after restart");

    history.sort_by_key(|v| v.version);
    for pair in history.windows(2) {
        assert!(
            pair[1].timestamp > pair[0].timestamp,
            "timestamps must keep increasing across restarts: v{}@{} then v{}@{}",
            pair[0].version,
            pair[0].timestamp,
            pair[1].version,
            pair[1].timestamp
        );
        assert!(
            pair[1].valid_from > pair[0].valid_from,
            "valid_from must keep increasing across restarts"
        );
    }

    // Time-travel: en el timestamp de la segunda versión se ve age=31
    let middle_ts = history[1].timestamp;
    let node_then = graph.get_node_at(node_id, middle_ts).await?;
    assert_eq!(
        node_then.properties.get("age"),
        Some(&PropertyValue::Int(31)),
        "as-of read at the middle version must see the pre-restart value"
    );

    // La versión actual es la de después del restart
    let node_now = graph.get_node(node_id).await?;
    assert_eq!(node_now.properties.get("age"), Some(&PropertyValue::Int(32)));

    Ok(())
}

#[tokio::test]
async fn clocks_are_persisted_and_grow() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let node_id = Uuid::new_v4();

    let after_first = {
        let graph = Graph::open(dir.path()).await?;
        commit_age(&graph, node_id, 1).await?;
        let ts = graph.storage().get_meta_u64(META_NEXT_TIMESTAMP).await?;
        let tx = graph.storage().get_meta_u64(META_NEXT_TX_ID).await?;
        assert!(ts.unwrap_or(0) > 1, "timestamp clock must be persisted after commit");
        assert!(tx.unwrap_or(0) > 1, "tx id clock must be persisted after commit");
        (ts.unwrap(), tx.unwrap())
    };

    let graph = Graph::open(dir.path()).await?;
    commit_age(&graph, node_id, 2).await?;
    let ts2 = graph.storage().get_meta_u64(META_NEXT_TIMESTAMP).await?.unwrap();
    let tx2 = graph.storage().get_meta_u64(META_NEXT_TX_ID).await?.unwrap();
    assert!(ts2 > after_first.0, "timestamp clock must keep growing after restart");
    assert!(tx2 > after_first.1, "tx id clock must keep growing after restart");

    Ok(())
}

#[tokio::test]
async fn direct_edge_writes_advance_persisted_clock() -> nopaldb::Result<()> {
    use nopaldb::Edge;

    let dir = tempfile::tempdir().unwrap();

    let graph = Graph::open(dir.path()).await?;
    let a = graph.add_node(person(Uuid::new_v4(), 40)).await?;
    let b = graph.add_node(person(Uuid::new_v4(), 41)).await?;

    let before = graph
        .storage()
        .get_meta_u64(META_NEXT_TIMESTAMP)
        .await?
        .unwrap_or(0);

    // Escritura directa versionada, sin transacción explícita:
    // add_edge asigna timestamp MVCC y persiste la versión.
    graph.add_edge(Edge::new(a, b, "KNOWS")).await?;

    let after = graph
        .storage()
        .get_meta_u64(META_NEXT_TIMESTAMP)
        .await?
        .unwrap_or(0);
    assert!(
        after > before,
        "direct versioned writes must advance the persisted clock ({} -> {})",
        before,
        after
    );

    Ok(())
}

#[tokio::test]
async fn legacy_db_without_meta_keys_uses_fallback_scan() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let node_id = Uuid::new_v4();

    // Sesión 1: crear datos normalmente
    {
        let graph = Graph::open(dir.path()).await?;
        commit_age(&graph, node_id, 30).await?;
        commit_age(&graph, node_id, 31).await?;
    }

    // Simular una base creada antes de que los relojes se persistieran:
    // borrar las keys meta directamente en storage.
    {
        let storage = Storage::new(dir.path()).await?;
        storage.delete_meta(META_NEXT_TIMESTAMP).await?;
        storage.delete_meta(META_NEXT_TX_ID).await?;
    }

    // Sesión 2: el open debe derivar los relojes del máximo ya persistido
    let graph = Graph::open(dir.path()).await?;
    commit_age(&graph, node_id, 32).await?;

    let mut history = graph.history(node_id).await?;
    assert_eq!(history.len(), 3);
    history.sort_by_key(|v| v.version);
    for pair in history.windows(2) {
        assert!(
            pair[1].timestamp > pair[0].timestamp,
            "fallback scan must resume clocks above persisted versions"
        );
    }

    Ok(())
}
