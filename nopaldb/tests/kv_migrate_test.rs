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
    let graph = Graph::open_with_options(dir, opts(StorageEngine::Sled)).await?;
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
        let graph = Graph::open_with_options(&src, opts(StorageEngine::Sled)).await?;
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

/// #152: what lives outside the KV must travel too. A full-text index with a
/// non-default analyzer, a hash index and a persisted HNSW dump in a sled
/// source; after `copy_database` the redb destination lists the same indexes,
/// `describe_index` returns the same analyzer, a hybrid (text + vector) search
/// returns the same ids, and the HNSW is loaded from the copied dump, not
/// rebuilt. Before 0.6.3 `list_indexes()` came back empty on the destination.
#[cfg(all(feature = "fulltext", feature = "embeddings-index", feature = "hybrid"))]
#[tokio::test]
async fn user_indexes_analyzer_and_hnsw_travel_with_the_migration() -> Result<()> {
    use nopaldb::graph::hybrid::HybridQuery;
    use nopaldb::index::{FullTextAnalyzer, IndexOptions, IndexType};

    const MODEL: &str = "m";
    let src_dir = tempfile::tempdir()?;
    let dst_dir = tempfile::tempdir()?;
    let dst_path = dst_dir.path().join("redb");

    let docs = [
        "los nopales se cultivan en las laderas",
        "el maguey da aguamiel en la meseta",
        "la biznaga crece lenta en el desierto",
        "las laderas del cerro tienen nopales jóvenes",
    ];
    // Deterministic 8-d vectors; the dump is only written above the exact-search
    // threshold (1 024 live points), so pad the corpus with `Relleno` nodes.
    let vector = |i: usize| -> Vec<f32> {
        (0..8).map(|d| ((i * 31 + d * 17) % 101) as f32 / 101.0).collect()
    };
    const PAD: usize = 1_100;

    let (ft_name, hash_name, expected) = {
        let g = Graph::open_with_options(src_dir.path(), opts(StorageEngine::Sled)).await?;
        for (i, text) in docs.iter().enumerate() {
            let id = g
                .add_node(Node::new("Doc").with_property("texto", *text).with_property("n", i as i64))
                .await?;
            g.add_node_embedding(id, vector(i), MODEL).await?;
        }
        let mut loader = g.bulk_loader(256);
        let mut pad_ids = Vec::with_capacity(PAD);
        for i in 0..PAD {
            let node = Node::new("Relleno").with_property("i", i as i64);
            pad_ids.push(node.id);
            loader.add_node(node).await?;
        }
        loader.finish().await?;
        for (i, id) in pad_ids.iter().enumerate() {
            g.add_node_embedding(*id, vector(docs.len() + i), MODEL).await?;
        }
        let ft_name = g
            .create_index_with(
                "Doc",
                "texto",
                IndexType::FullText,
                IndexOptions { analyzer: Some(FullTextAnalyzer::for_language("spanish")) },
            )
            .await?;
        let hash_name = g.create_index("Doc", "n", IndexType::Hash).await?;
        // A search builds the HNSW and leaves its dump in `hnsw/`.
        let mut q = HybridQuery::new();
        q.text = Some("nopales laderas".into());
        q.vector = Some((vector(0), MODEL.into()));
        q.k = 5;
        let expected: Vec<_> = g.search_hybrid(q).await?.into_iter().map(|h| h.node_id).collect();
        assert!(!expected.is_empty());
        assert!(g.embedding_index_stats(MODEL).await.map(|s| s.persisted).unwrap_or(false), "dump written");
        g.close().await?;
        (ft_name, hash_name, expected)
    };
    assert!(src_dir.path().join("indexes").join("metadata.bin").is_file());
    assert!(src_dir.path().join("hnsw").is_dir());

    let report = Storage::copy_database(
        src_dir.path(),
        opts(StorageEngine::Sled),
        &dst_path,
        opts(StorageEngine::Redb),
    )
    .await?;
    assert!(report.verified);
    assert!(report.hnsw_copied, "{report:?}");
    let mut names: Vec<_> = report.indexes.iter().map(|ix| ix.name.clone()).collect();
    names.sort();
    let mut want = vec![ft_name.clone(), hash_name.clone()];
    want.sort();
    assert_eq!(names, want, "{report:?}");
    let ft = report.indexes.iter().find(|ix| ix.name == ft_name).unwrap();
    assert_eq!(ft.kind, "FullText");
    assert_eq!(ft.analyzer.as_deref(), Some("spanish+stemming+stopwords+ascii_folding"));
    assert!(report.sidecars.iter().any(|sc| sc.dir == "indexes" && sc.files >= 2), "{report:?}");
    assert!(report.sidecars.iter().any(|sc| sc.dir == "hnsw" && sc.files == 3), "{report:?}");

    let g = Graph::open_with_options(&dst_path, opts(StorageEngine::Redb)).await?;
    let mut got: Vec<_> = g.list_indexes().await.into_iter().map(|m| m.name).collect();
    got.sort();
    assert_eq!(got, want, "the destination lists the same user indexes");
    let info = g.describe_index(&ft_name).await.expect("full-text index present");
    assert_eq!(info.analyzer, Some(FullTextAnalyzer::for_language("spanish")));

    let mut q = HybridQuery::new();
    q.text = Some("nopales laderas".into());
    q.vector = Some((vector(0), MODEL.into()));
    q.k = 5;
    let got: Vec<_> = g.search_hybrid(q).await?.into_iter().map(|h| h.node_id).collect();
    assert_eq!(got, expected, "same hybrid results after migrating");
    let stats = g.embedding_index_stats(MODEL).await.expect("hnsw stats");
    assert!(stats.persisted && !stats.needs_rebuild, "loaded from the copied dump: {stats:?}");
    g.close().await?;
    Ok(())
}

/// The destination must not already carry indexes or an HNSW dump of its own.
#[tokio::test]
async fn refuses_a_destination_with_sidecars() -> Result<()> {
    let src = tempfile::tempdir()?;
    build_fixture(src.path()).await?;
    let dst = tempfile::tempdir()?;
    let dst_path = dst.path().join("redb");
    std::fs::create_dir_all(dst_path.join("indexes"))?;
    std::fs::write(dst_path.join("indexes").join("metadata.bin"), b"x")?;
    let err = Storage::copy_database(src.path(), opts(StorageEngine::Sled), &dst_path, opts(StorageEngine::Redb))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("indexes/"), "{err}");
    Ok(())
}
