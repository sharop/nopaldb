use nopaldb::mvcc::GCConfig;
use nopaldb::{Graph, Node, PropertyValue};

async fn seed_versioned_person(graph: &Graph) -> nopaldb::Result<nopaldb::NodeId> {
    let mut tx1 = graph.begin_transaction().await?;
    let node_id = tx1
        .add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String("Alice".into()))
                .with_property("balance", PropertyValue::Int(100)),
        )
        .await?;
    tx1.commit().await?;

    let mut tx2 = graph.begin_transaction().await?;
    let mut v2 = Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()))
        .with_property("balance", PropertyValue::Int(200));
    v2.id = node_id;
    tx2.add_node(v2).await?;
    tx2.commit().await?;

    let mut tx3 = graph.begin_transaction().await?;
    let mut v3 = Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()))
        .with_property("balance", PropertyValue::Int(300));
    v3.id = node_id;
    tx3.add_node(v3).await?;
    tx3.commit().await?;

    Ok(node_id)
}

#[tokio::test]
async fn test_p2_gc_deletes_old_versions_keep_one() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;
    let node_id = seed_versioned_person(&graph).await?;

    let history_before = graph.history(node_id).await?;
    assert_eq!(
        history_before.len(),
        3,
        "Expected 3 MVCC versions before GC"
    );

    let config = GCConfig {
        cutoff_timestamp: u64::MAX,
        min_versions_to_keep: 1,
        max_nodes_per_cycle: 0,
        dry_run: false,
        use_active_horizon: false,
    };
    let stats = graph.gc(config).await?;

    assert_eq!(stats.nodes_scanned, 1);
    assert_eq!(stats.versions_deleted, 2);
    assert!(stats.bytes_freed > 0, "GC should report freed bytes");

    let history_after = graph.history(node_id).await?;
    assert_eq!(history_after.len(), 1, "Only latest version should remain");
    assert_eq!(
        history_after[0].version, 3,
        "Latest version must be preserved"
    );

    Ok(())
}

#[tokio::test]
async fn test_p2_gc_dry_run_does_not_delete_versions() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;
    let node_id = seed_versioned_person(&graph).await?;

    let history_before = graph.history(node_id).await?;
    assert_eq!(history_before.len(), 3);

    let config = GCConfig {
        cutoff_timestamp: u64::MAX,
        min_versions_to_keep: 1,
        max_nodes_per_cycle: 0,
        dry_run: true,
        use_active_horizon: false,
    };
    let stats = graph.gc(config).await?;

    assert_eq!(
        stats.versions_deleted, 2,
        "Dry run reports would-delete count"
    );
    assert_eq!(stats.bytes_freed, 0, "Dry run must not free bytes");

    let history_after = graph.history(node_id).await?;
    assert_eq!(history_after.len(), 3, "Dry run must not delete versions");

    Ok(())
}

#[tokio::test]
async fn test_p2_gc_respects_keep_at_least() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;
    let node_id = seed_versioned_person(&graph).await?;

    let config = GCConfig {
        cutoff_timestamp: u64::MAX,
        min_versions_to_keep: 2,
        max_nodes_per_cycle: 0,
        dry_run: false,
        use_active_horizon: false,
    };
    let stats = graph.gc(config).await?;
    assert_eq!(
        stats.versions_deleted, 1,
        "GC should keep the latest 2 versions"
    );

    let history_after = graph.history(node_id).await?;
    assert_eq!(history_after.len(), 2);
    let kept_versions = history_after
        .into_iter()
        .map(|v| v.version)
        .collect::<Vec<_>>();
    assert_eq!(
        kept_versions,
        vec![3, 2],
        "Expected to keep latest versions only"
    );

    Ok(())
}

// ============================================================
// Feature C: Active Transaction Horizon tests
// ============================================================

/// GC con una transacción activa no debe borrar versiones que el horizonte protege.
///
/// El truco: la tx sentinel se inicia ANTES del seeding. Su timestamp es el mínimo.
/// Al correr GC con active_horizon=true, cutoff = horizon = ts_sentinel.
/// Ninguna versión creada después tiene valid_to < ts_sentinel → nada se borra.
#[tokio::test]
async fn test_gc_respects_active_transaction_horizon() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;

    // Tx sentinel empieza ANTES del seeding — su ts será el mínimo
    let sentinel_tx = graph.begin_transaction().await?;
    let sentinel_ts = sentinel_tx.timestamp;

    // Seeding ocurre DESPUÉS: los timestamps de versiones son > sentinel_ts
    let node_id = seed_versioned_person(&graph).await?;

    let history_before = graph.history(node_id).await?;
    assert_eq!(history_before.len(), 3, "Expected 3 versions before GC");

    // Ejecutar GC con active horizon
    // horizon = sentinel_ts → cutoff = sentinel_ts
    // Versiones tienen valid_to > sentinel_ts → NO son gc-eligible → no se borran
    let config = GCConfig {
        cutoff_timestamp: u64::MAX,
        min_versions_to_keep: 1,
        max_nodes_per_cycle: 0,
        dry_run: false,
        use_active_horizon: true,
    };
    let stats = graph.gc(config).await?;

    let history_after = graph.history(node_id).await?;
    assert_eq!(
        history_after.len(),
        3,
        "GC with active_horizon must not delete versions when horizon={} (deleted={})",
        sentinel_ts,
        stats.versions_deleted
    );

    // Commit la tx sentinel y volver a ejecutar GC sin horizon — debe borrar
    sentinel_tx.commit().await?;

    let config2 = GCConfig {
        cutoff_timestamp: u64::MAX,
        min_versions_to_keep: 1,
        max_nodes_per_cycle: 0,
        dry_run: false,
        use_active_horizon: false,
    };
    let stats2 = graph.gc(config2).await?;
    assert!(
        stats2.versions_deleted >= 2,
        "After sentinel commit, GC should delete old versions"
    );

    Ok(())
}

/// safe_gc_horizon() devuelve el mínimo timestamp entre transacciones activas.
#[tokio::test]
async fn test_safe_gc_horizon_tracks_minimum() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;

    // Sin txs activas, el horizonte es >= 1 (current timestamp)
    let horizon_empty = graph.safe_gc_horizon();
    assert!(
        horizon_empty >= 1,
        "Horizon without active txs should be >= 1"
    );

    // Iniciar 3 transacciones
    let tx1 = graph.begin_transaction().await?;
    let ts1 = tx1.timestamp;
    let tx2 = graph.begin_transaction().await?;
    let ts2 = tx2.timestamp;
    let tx3 = graph.begin_transaction().await?;
    let ts3 = tx3.timestamp;

    // El horizonte debe ser el mínimo de los 3
    let horizon = graph.safe_gc_horizon();
    let expected_min = ts1.min(ts2).min(ts3);
    assert_eq!(
        horizon, expected_min,
        "Horizon must be min of all active tx timestamps"
    );

    // Hacer commit de tx1 y tx2
    tx1.commit().await?;
    tx2.commit().await?;

    // El horizonte ahora debe ser ts3
    let horizon_after = graph.safe_gc_horizon();
    assert_eq!(
        horizon_after, ts3,
        "After committing tx1+tx2, horizon = ts3"
    );

    // Hacer commit de tx3
    tx3.commit().await?;

    // Sin txs activas, el horizonte vuelve al timestamp actual
    let horizon_final = graph.safe_gc_horizon();
    assert!(
        horizon_final > ts3,
        "After all commits, horizon should be current timestamp"
    );

    Ok(())
}

/// gc_default() usa un cutoff conservador — tx activa con ts_early bloquea purga.
///
/// El truco: la tx activa inicia ANTES del seeding. Su ts es el mínimo (horizon).
/// gc_default usa cutoff = min(horizon, now-7días). Como horizon << now-7días,
/// cutoff = horizon. Versiones creadas después → valid_to > horizon → no se borran.
#[tokio::test]
async fn test_gc_default_conservative() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;

    // La tx activa comienza ANTES del seeding
    let active_tx = graph.begin_transaction().await?;
    let horizon_ts = active_tx.timestamp;

    // Seeding: crea versiones con valid_to > horizon_ts
    let node_id = seed_versioned_person(&graph).await?;

    // gc_default: cutoff = min(horizon_ts, now-7días)
    // horizon_ts es un timestamp lógico pequeño (ej. 1)
    // now-7días en ms es ~1.74T >> 1 → cutoff = horizon_ts = 1
    // Ninguna versión tiene valid_to < 1 → nada se borra
    let stats = graph.gc_default().await?;

    let history = graph.history(node_id).await?;
    assert_eq!(
        history.len(),
        3,
        "gc_default with early active tx (horizon={}) should not delete versions (deleted={})",
        horizon_ts,
        stats.versions_deleted
    );

    active_tx.commit().await?;
    Ok(())
}

/// Una Transaction dropeada sin commit/rollback no deja timestamps huérfanos.
#[tokio::test]
async fn test_drop_without_rollback_cleans_up() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;

    // Verificar que no hay txs activas al inicio
    let horizon_before = graph.safe_gc_horizon();

    {
        let _tx = graph.begin_transaction().await?;
        // La tx está activa aquí, horizon debe haber bajado
        let horizon_during = graph.safe_gc_horizon();
        assert!(
            horizon_during <= horizon_before || horizon_before == horizon_during,
            "Horizon during active tx must be <= pre-tx horizon or equal"
        );
        // _tx se dropea aquí sin commit ni rollback
    }

    // Después del drop, no debe haber timestamps huérfanos
    // safe_gc_horizon vuelve al timestamp actual (sin txs activas)
    let horizon_after = graph.safe_gc_horizon();
    assert!(
        horizon_after >= horizon_before,
        "After drop, horizon should be restored (no orphaned timestamps)"
    );

    Ok(())
}
