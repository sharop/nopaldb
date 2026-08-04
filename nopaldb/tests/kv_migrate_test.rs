// Migración entre motores: round-trip byte-verificado y time-travel intacto.
// Requiere ambos backends compilados (required-features en Cargo.toml).

use nopaldb::{
    Graph, MigrationReport, Node, PropertyValue, Result, Storage, StorageEngine, StorageOptions,
};

fn opts(engine: StorageEngine) -> StorageOptions {
    StorageOptions {
        engine,
        ..StorageOptions::default()
    }
}

/// Base chica pero representativa: nodos con propiedades indexadas, aristas,
/// historia MVCC (updates + delete) y checkpoint (WAL aplicado al cerrar).
async fn build_fixture(dir: &std::path::Path) -> Result<(uuid::Uuid, uuid::Uuid)> {
    let graph = Graph::open(dir).await?;
    let (a, b) = {
        let mut tx = graph.begin_transaction().await?;
        let a = tx
            .add_node(Node::new("P").with_property("name", "Ana").with_property("v", 1i64))
            .await?;
        let b = tx.add_node(Node::new("P").with_property("name", "Beto")).await?;
        tx.add_edge(nopaldb::Edge::new(a, b, "CONOCE"))?;
        tx.commit().await?;
        (a, b)
    };

    // Historia MVCC: dos updates transaccionales sobre `a` (add_node con el
    // mismo id crea una versión nueva al commitear).
    for v in [2i64, 3i64] {
        let mut tx = graph.begin_transaction().await?;
        let mut node = graph.get_node(a).await?;
        node.properties
            .insert("v".into(), PropertyValue::Int(v));
        let _ = tx.add_node(node).await?;
        tx.commit().await?;
    }
    graph.checkpoint().await?;
    Ok((a, b))
}

#[tokio::test]
async fn round_trip_sled_redb_sled_is_byte_identical() -> Result<()> {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("src_sled");
    let mid = tmp.path().join("mid_redb");
    let back = tmp.path().join("back_sled");

    build_fixture(&src).await?;

    let r1: MigrationReport =
        Storage::copy_database(&src, opts(StorageEngine::Sled), &mid, opts(StorageEngine::Redb))
            .await?;
    assert!(r1.verified);
    assert!(r1.total_pairs() > 0);

    let r2 =
        Storage::copy_database(&mid, opts(StorageEngine::Redb), &back, opts(StorageEngine::Sled))
            .await?;
    assert!(r2.verified);

    // Byte-idéntico de punta a punta: mismos pares y bytes por keyspace en
    // ambos saltos (los checksums internos ya verificaron el contenido).
    assert_eq!(r1.keyspaces, r2.keyspaces, "sled→redb vs redb→sled difieren");
    Ok(())
}

#[tokio::test]
async fn time_travel_survives_migration() -> Result<()> {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("tt_src");
    let dst = tmp.path().join("tt_dst");

    let (a, _) = build_fixture(&src).await?;

    // Historia esperada, leída del ORIGEN antes de migrar.
    let expected: Vec<i64> = {
        let graph = Graph::open(&src).await?;
        let history = graph.history(a).await?;
        history
            .iter()
            .filter_map(|v: &nopaldb::mvcc::VersionedNode| match v.node_data.properties.get("v") {
                Some(PropertyValue::Int(n)) => Some(*n),
                _ => None,
            })
            .collect()
    };
    assert!(expected.len() >= 3, "el fixture debe tener historia: {expected:?}");

    Storage::copy_database(&src, opts(StorageEngine::Sled), &dst, opts(StorageEngine::Redb))
        .await?;

    // La MISMA historia, abriendo el destino con redb.
    let graph = Graph::open_with_options(&dst, opts(StorageEngine::Redb)).await?;
    let history = graph.history(a).await?;
    let got: Vec<i64> = history
        .iter()
        .filter_map(|v| match v.node_data.properties.get("v") {
            Some(PropertyValue::Int(n)) => Some(*n),
            _ => None,
        })
        .collect();
    assert_eq!(got, expected, "la historia MVCC cambió al migrar");

    // Y el índice tipado responde igual.
    let hits = graph
        .get_all_nodes_by_property("name", &PropertyValue::String("Ana".into()))
        .await?;
    assert_eq!(hits, vec![a]);
    Ok(())
}

#[tokio::test]
async fn refuses_non_empty_destination() -> Result<()> {
    let tmp = tempfile::tempdir().expect("tempdir");
    let src = tmp.path().join("ne_src");
    let dst = tmp.path().join("ne_dst");

    build_fixture(&src).await?;
    {
        // Destino con datos propios.
        let graph = Graph::open_with_options(&dst, opts(StorageEngine::Redb)).await?;
        graph.add_node(Node::new("X")).await?;
        graph.checkpoint().await?;
    }

    let err = Storage::copy_database(&src, opts(StorageEngine::Sled), &dst, opts(StorageEngine::Redb))
        .await
        .err()
        .expect("migrar a destino no vacío debe fallar");
    assert!(err.to_string().contains("no está vacío"), "{err}");
    Ok(())
}
