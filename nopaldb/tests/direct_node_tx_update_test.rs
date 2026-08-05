// tests/direct_node_tx_update_test.rs
//
// Regresión del issue #60: un nodo creado con `graph.add_node()` directo
// (sin transacción) no tiene cadena MVCC; el primer `tx.commit()` que lo
// actualizaba clasificaba `is_update=true` mirando el registro base
// (`entities`) y reventaba con NodeNotFound al buscar la versión current.
//
// Semántica PINNEADA (espejo de `replay_node_upsert`): sin cadena, el commit
// crea la PRIMERA versión (v1) con los datos commiteados y el timestamp del
// commit. El estado pre-transaccional del nodo directo NO se materializa
// retroactivamente en la historia — la cadena nace en el primer commit que
// toca el nodo (saneo lazy), igual que en el redo del WAL.

use nopaldb::{Graph, Node, PropertyValue};

/// Caso 1 del issue: add_node directo → update transaccional → commit Ok,
/// con cadena MVCC coherente y semántica de primera versión pinneada.
#[tokio::test]
async fn test_direct_node_then_tx_update_commits() {
    let graph = Graph::in_memory().await.unwrap();

    // Nodo directo: existe en `entities` pero SIN cadena MVCC
    let node = Node::new("Device")
        .with_property("name", PropertyValue::String("sensor-a".into()))
        .with_property("state", PropertyValue::String("new".into()));
    let node_id = graph.add_node(node).await.unwrap();

    // Antes del fix esto fallaba con NodeNotFound en apply_commit_set
    {
        let mut tx = graph.begin_transaction().await.unwrap();
        let mut n = graph.get_node(node_id).await.unwrap();
        n.properties
            .insert("state".into(), PropertyValue::String("active".into()));
        tx.add_node(n).await.unwrap();
        tx.commit().await.unwrap();
    }

    // La cadena nace en el primer commit: v1 con los datos COMMITEADOS
    // (semántica del replay pinneada — el estado directo previo no se
    // materializa retroactivamente).
    let current = graph.get_current_version(node_id).await.unwrap();
    assert_eq!(current, 1, "first commit over a direct node creates v1");

    let history = graph.history(node_id).await.unwrap();
    assert_eq!(history.len(), 1, "chain is born at the first commit");
    assert_eq!(history[0].version, 1);
    assert_eq!(
        history[0].node_data.properties.get("state"),
        Some(&PropertyValue::String("active".into())),
        "v1 carries the committed (updated) properties"
    );
    assert_eq!(history[0].prev_version, None);
    assert_eq!(history[0].valid_to, None, "v1 is the current version");

    // Segundo update transaccional: ahora sí clasifica como update normal
    {
        let mut tx = graph.begin_transaction().await.unwrap();
        let mut n = graph.get_node(node_id).await.unwrap();
        n.properties
            .insert("state".into(), PropertyValue::String("retired".into()));
        tx.add_node(n).await.unwrap();
        tx.commit().await.unwrap();
    }

    let history = graph.history(node_id).await.unwrap();
    assert_eq!(history.len(), 2, "second commit chains v2 onto v1");
    // history viene newest-first
    assert_eq!(history[0].version, 2);
    assert_eq!(history[0].prev_version, Some(1));
    assert_eq!(
        history[0].node_data.properties.get("state"),
        Some(&PropertyValue::String("retired".into()))
    );
    assert_eq!(history[1].version, 1);
    assert!(
        history[1].valid_to.is_some(),
        "v1 must be invalidated by the v2 commit"
    );
    assert_eq!(
        history[1].node_data.properties.get("state"),
        Some(&PropertyValue::String("active".into())),
        "v1 keeps the first committed properties"
    );

    // El registro base también refleja el último estado
    let n = graph.get_node(node_id).await.unwrap();
    assert_eq!(
        n.properties.get("state"),
        Some(&PropertyValue::String("retired".into()))
    );
}

/// Corolario del issue: time-travel sobre un nodo directo actualizado.
/// `get_node_at_timestamp` fallaba porque no existía lista de versiones.
#[tokio::test]
async fn test_direct_node_time_travel_after_fix() {
    let graph = Graph::in_memory().await.unwrap();

    let node = Node::new("Device").with_property("v", PropertyValue::Int(0));
    let node_id = graph.add_node(node).await.unwrap();

    {
        let mut tx = graph.begin_transaction().await.unwrap();
        let mut n = graph.get_node(node_id).await.unwrap();
        n.properties.insert("v".into(), PropertyValue::Int(1));
        tx.add_node(n).await.unwrap();
        tx.commit().await.unwrap();
    }

    // ts del commit que creó la cadena (clock lógico: usamos el de la
    // versión, no el reloj de pared)
    let history = graph.history(node_id).await.unwrap();
    let t1 = history[0].timestamp;

    {
        let mut tx = graph.begin_transaction().await.unwrap();
        let mut n = graph.get_node(node_id).await.unwrap();
        n.properties.insert("v".into(), PropertyValue::Int(2));
        tx.add_node(n).await.unwrap();
        tx.commit().await.unwrap();
    }

    let history = graph.history(node_id).await.unwrap();
    assert_eq!(history.len(), 2);
    let t2 = history[0].timestamp;
    assert!(t2 > t1);

    let storage = graph.storage();

    // En t1 (posterior al saneo) el nodo directo YA es visible: v=1
    let at_t1 = storage.get_node_at_timestamp(node_id, t1).await.unwrap();
    assert_eq!(at_t1.version, 1);
    assert_eq!(
        at_t1.node_data.properties.get("v"),
        Some(&PropertyValue::Int(1))
    );

    // En t2 y después, la versión actual: v=2
    let at_t2 = storage.get_node_at_timestamp(node_id, t2).await.unwrap();
    assert_eq!(at_t2.version, 2);
    assert_eq!(
        at_t2.node_data.properties.get("v"),
        Some(&PropertyValue::Int(2))
    );
    let at_future = storage
        .get_node_at_timestamp(node_id, t2 + 1000)
        .await
        .unwrap();
    assert_eq!(at_future.version, 2);
}

/// Carga mixta: ~100 commits alternando nodos nacidos directos y nodos
/// nacidos transaccionales; todos deben commitear sin error.
#[tokio::test]
async fn test_mixed_direct_and_tx_nodes_commit_loop() {
    let graph = Graph::in_memory().await.unwrap();

    let mut ids = Vec::new();
    for i in 0..20i64 {
        if i % 2 == 0 {
            // Nacimiento directo (sin cadena MVCC)
            let id = graph
                .add_node(Node::new("Mixed").with_property("i", i))
                .await
                .unwrap();
            ids.push(id);
        } else {
            // Nacimiento transaccional (con cadena desde v1)
            let mut tx = graph.begin_transaction().await.unwrap();
            let id = tx
                .add_node(Node::new("Mixed").with_property("i", i))
                .await
                .unwrap();
            tx.commit().await.unwrap();
            ids.push(id);
        }
    }

    // 5 rondas de updates sobre los 20 nodos = 100 commits
    for round in 0..5i64 {
        for id in &ids {
            let mut tx = graph.begin_transaction().await.unwrap();
            let mut n = graph.get_node(*id).await.unwrap();
            n.properties.insert("round".into(), PropertyValue::Int(round));
            tx.add_node(n).await.unwrap();
            tx.commit()
                .await
                .unwrap_or_else(|e| panic!("commit failed for node {id} round {round}: {e:?}"));
        }
    }

    // Coherencia de cadenas: los directos tienen 5 versiones (la cadena nace
    // en el primer update), los transaccionales 6 (create + 5 updates).
    for (idx, id) in ids.iter().enumerate() {
        let history = graph.history(*id).await.unwrap();
        let expected = if idx % 2 == 0 { 5 } else { 6 };
        assert_eq!(
            history.len(),
            expected,
            "node {idx} (direct={}) has wrong chain length",
            idx % 2 == 0
        );
        let current = graph.get_current_version(*id).await.unwrap();
        assert_eq!(current as usize, expected);
        assert_eq!(
            graph
                .get_node(*id)
                .await
                .unwrap()
                .properties
                .get("round"),
            Some(&PropertyValue::Int(4))
        );
    }
}

/// El escenario EXACTO del bench gc_removals que destapó el bug:
/// N nodos creados con add_node directo, luego V rondas de updates
/// transaccionales por nodo (add_node con el mismo id dentro de la tx).
#[tokio::test]
async fn test_gc_bench_scenario_direct_nodes_tx_versions() {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path().join("db")).await.unwrap();

    let nodes = 50usize;
    let versions = 3usize;

    let mut ids = Vec::with_capacity(nodes);
    for i in 0..nodes {
        let id = graph
            .add_node(Node::new("N").with_property("i", i as i64))
            .await
            .unwrap();
        ids.push(id);
    }

    for v in 0..versions {
        for id in &ids {
            let mut tx = graph.begin_transaction().await.unwrap();
            let mut node = graph.get_node(*id).await.unwrap();
            node.properties
                .insert("v".into(), PropertyValue::Int(v as i64));
            // add_node con el mismo id = versión nueva al commit
            tx.add_node(node).await.unwrap();
            tx.commit()
                .await
                .unwrap_or_else(|e| panic!("bench scenario commit failed (v={v}): {e:?}"));
        }
    }

    // Cadenas coherentes: la cadena nace en el primer update → `versions`
    // versiones por nodo, current = la última
    for id in &ids {
        let history = graph.history(*id).await.unwrap();
        assert_eq!(history.len(), versions);
        assert_eq!(
            graph.get_current_version(*id).await.unwrap() as usize,
            versions
        );
        assert_eq!(
            history[0].node_data.properties.get("v"),
            Some(&PropertyValue::Int((versions - 1) as i64))
        );
    }

    // Y el GC del bench corre sin error sobre este estado
    let cfg = nopaldb::mvcc::GCConfig::older_than_ms(0);
    let stats = graph.gc(cfg).await.unwrap();
    // Debe haber recolectado versiones invalidadas (versions-1 por nodo)
    assert!(
        stats.versions_deleted > 0,
        "GC should collect invalidated versions, stats: {stats:?}"
    );
}
