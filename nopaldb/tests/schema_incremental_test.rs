//! #164: el esquema derivado se mantiene por operación y se persiste en cada
//! checkpoint. Cada camino de escritura lo actualiza sin reconstruir, un
//! `open` limpio lo carga sin recorrer nada, y solo una recuperación de
//! crash (o una base sin snapshot) reconstruye, una vez. `schema_cache_test`
//! sigue afirmando los valores; aquí se afirma además CUÁNTAS veces se
//! reconstruyó (`Graph::schema_rebuild_count`), que es lo que #164 cambia.

use std::collections::BTreeMap;

use nopaldb::{Edge, Graph, Node, PropertyValue, Storage, StorageOptions};

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

async fn counts(g: &Graph) -> (usize, usize, BTreeMap<String, usize>, BTreeMap<String, usize>) {
    let sc = g.get_schema().await.unwrap();
    (
        sc.total_nodes,
        sc.total_edges,
        sc.node_counts.into_iter().collect(),
        sc.edge_counts.into_iter().collect(),
    )
}

async fn labels(g: &Graph) -> Vec<String> {
    let mut l = g.get_labels().await.unwrap();
    l.sort();
    l
}

/// Oráculo: lo que dice el esquema mantenido coincide con una reconstrucción
/// desde cero y con el conteo de claves.
async fn assert_matches_rebuild(g: &Graph) {
    let before = counts(g).await;
    let rebuilds = g.schema_rebuild_count();
    g.rebuild_schema().await.unwrap();
    assert_eq!(g.schema_rebuild_count(), rebuilds + 1);
    let after = counts(g).await;
    assert_eq!(before, after, "incremental schema must equal a full rebuild");
    assert_eq!(g.storage().count_nodes().await.unwrap(), after.0, "total_nodes vs storage keys");
    assert_eq!(g.storage().count_edges().await.unwrap(), after.1, "total_edges vs storage keys");
}

#[tokio::test]
async fn every_write_path_updates_the_schema_without_rebuilding() {
    let g = Graph::in_memory().await.unwrap();
    assert_eq!(g.schema_rebuild_count(), 0, "a fresh graph starts with a valid empty schema");
    assert_eq!(counts(&g).await.0, 0);

    // Escritura directa con propiedad.
    let a = g.add_node(Node::new("Planta").with_property("n", PropertyValue::Int(1))).await.unwrap();
    // Commit transaccional con nodo y arista con propiedad.
    let mut tx = g.begin_transaction().await.unwrap();
    let b = tx.add_node(Node::new("Riego").with_property("litros", PropertyValue::Int(3))).await.unwrap();
    tx.add_edge(Edge::new(a, b, "RECIBE").with_property("desde", PropertyValue::Int(2020))).unwrap();
    tx.commit().await.unwrap();
    // Bulk loader: nodos y aristas.
    let mut loader = g.bulk_loader(2);
    let mut bulk_ids = Vec::new();
    for i in 0..5 {
        let n = Node::new("Bulk").with_property("i", PropertyValue::Int(i));
        bulk_ids.push(n.id);
        loader.add_node(n).await.unwrap();
    }
    for w in bulk_ids.windows(2) {
        loader.add_edge(Edge::new(w[0], w[1], "SIGUE")).await.unwrap();
    }
    loader.finish().await.unwrap();

    let (n, e, nc, ec) = counts(&g).await;
    assert_eq!((n, e), (7, 5));
    assert_eq!(nc, [("Bulk".into(), 5), ("Planta".into(), 1), ("Riego".into(), 1)].into_iter().collect());
    assert_eq!(ec, [("RECIBE".into(), 1), ("SIGUE".into(), 4)].into_iter().collect());
    assert!(g.get_edge_type_properties("RECIBE").await.unwrap().contains(&"desde".to_string()));
    assert_eq!(g.node_count().await.unwrap(), 7);
    assert_eq!(g.edge_count().await.unwrap(), 5);

    // delete_edge y delete_node (con arista incidente).
    let sigue: Vec<Edge> = g.get_all_edges().await.unwrap().into_iter().filter(|e| e.edge_type == "SIGUE").collect();
    g.delete_edge(sigue[0].id).await.unwrap();
    g.delete_node(b).await.unwrap(); // se lleva RECIBE
    let (n, e, _, ec) = counts(&g).await;
    assert_eq!((n, e), (6, 3));
    assert_eq!(ec, [("SIGUE".into(), 3)].into_iter().collect());
    assert_eq!(labels(&g).await, vec!["Bulk", "Planta"], "Riego disappears at count 0");

    // Sobrescritura: misma etiqueta, propiedad nueva.
    let mut over = Node::new("Planta").with_property("n", PropertyValue::Int(1)).with_property("nueva", s("x"));
    over.id = a;
    g.add_node(over).await.unwrap();
    assert!(g.get_label_properties("Planta").await.unwrap().contains(&"nueva".to_string()));
    assert_eq!(g.get_label_count("Planta").await.unwrap(), 1);

    // Sobrescritura con cambio de etiqueta mueve el conteo.
    let mut moved = Node::new("Arbol");
    moved.id = a;
    g.add_node(moved).await.unwrap();
    assert_eq!(labels(&g).await, vec!["Arbol", "Bulk"]);

    // NQL UPDATE (camino `storage_insert_node`) añade una propiedad.
    g.execute_statement("update (b:Bulk) set b.riego = true where b.i = 1").await.unwrap();
    assert!(g.get_label_properties("Bulk").await.unwrap().contains(&"riego".to_string()));

    // Inserción directa en storage (camino `storage_insert_node`).
    g.storage_insert_node(&Node::new("Importado")).await.unwrap();
    assert_eq!(g.get_label_count("Importado").await.unwrap(), 1);

    assert_eq!(g.schema_rebuild_count(), 0, "nothing above needed a rebuild");
    assert_matches_rebuild(&g).await;
}

#[tokio::test]
async fn deleting_a_node_discounts_incident_edges_of_every_type() {
    let g = Graph::in_memory().await.unwrap();
    let a = g.add_node(Node::new("P")).await.unwrap();
    let b = g.add_node(Node::new("P")).await.unwrap();
    let c = g.add_node(Node::new("P")).await.unwrap();
    let d = g.add_node(Node::new("P")).await.unwrap();
    g.add_edge(Edge::new(a, b, "KNOWS")).await.unwrap();
    g.add_edge(Edge::new(c, a, "LIKES")).await.unwrap();
    g.add_edge(Edge::new(a, d, "LIKES")).await.unwrap();
    g.add_edge(Edge::new(d, b, "KNOWS")).await.unwrap();
    assert_eq!(counts(&g).await.1, 4);

    g.delete_node(a).await.unwrap();
    let (n, e, _, ec) = counts(&g).await;
    assert_eq!((n, e), (3, 1));
    assert_eq!(ec, [("KNOWS".into(), 1)].into_iter().collect());
    assert_eq!(g.get_edge_types().await.unwrap(), vec!["KNOWS"]);
    assert_eq!(g.schema_rebuild_count(), 0);
    assert_matches_rebuild(&g).await;
}

#[tokio::test]
async fn edge_overwrite_with_type_change_moves_the_count() {
    let g = Graph::in_memory().await.unwrap();
    let a = g.add_node(Node::new("P")).await.unwrap();
    let b = g.add_node(Node::new("P")).await.unwrap();
    let e = Edge::new(a, b, "KNOWS");
    g.add_edge(e.clone()).await.unwrap();
    let mut e2 = Edge::new(a, b, "LIKES").with_property("w", PropertyValue::Float(0.5));
    e2.id = e.id;
    g.add_edge(e2).await.unwrap();
    let (_, total, _, ec) = counts(&g).await;
    assert_eq!(total, 1);
    assert_eq!(ec, [("LIKES".into(), 1)].into_iter().collect());
    assert!(g.get_edge_type_properties("LIKES").await.unwrap().contains(&"w".to_string()));
    g.delete_edge(e.id).await.unwrap();
    assert!(g.get_edge_types().await.unwrap().is_empty());
    assert_eq!(g.schema_rebuild_count(), 0);
    assert_matches_rebuild(&g).await;
}

#[tokio::test]
async fn batches_with_repeated_ids_do_not_overcount() {
    let g = Graph::in_memory().await.unwrap();
    let n1 = Node::new("A").with_property("v", PropertyValue::Int(1));
    let mut n1b = Node::new("B").with_property("v", PropertyValue::Int(2));
    n1b.id = n1.id; // mismo id, etiqueta distinta: último gana
    let n2 = Node::new("A");
    g.add_nodes_batch(vec![n1.clone(), n2.clone(), n1b.clone()]).await.unwrap();
    let (n, _, nc, _) = counts(&g).await;
    assert_eq!(n, 2);
    assert_eq!(nc, [("A".into(), 1), ("B".into(), 1)].into_iter().collect());

    // Segundo lote que repite un id existente: sigue siendo 2.
    g.add_nodes_batch(vec![n2.clone()]).await.unwrap();
    assert_eq!(counts(&g).await.0, 2);

    let e1 = Edge::new(n1.id, n2.id, "X");
    let mut e1b = Edge::new(n1.id, n2.id, "Y");
    e1b.id = e1.id;
    let e2 = Edge::new(n2.id, n1.id, "X");
    g.add_edges_batch(vec![e1.clone(), e2.clone(), e1b]).await.unwrap();
    let (_, e, _, ec) = counts(&g).await;
    assert_eq!(e, 2);
    assert_eq!(ec, [("X".into(), 1), ("Y".into(), 1)].into_iter().collect());
    g.add_edges_batch(vec![e2]).await.unwrap();
    assert_eq!(counts(&g).await.1, 2);
    assert_eq!(g.schema_rebuild_count(), 0);
    assert_matches_rebuild(&g).await;
}

async fn seed(dir: &std::path::Path, rows: usize) -> Graph {
    let g = Graph::open(dir).await.unwrap();
    for i in 0..rows {
        let mut tx = g.begin_transaction().await.unwrap();
        tx.add_node(Node::new("Planta").with_property("n", PropertyValue::Int(i as i64))).await.unwrap();
        tx.commit().await.unwrap();
    }
    g
}

#[tokio::test]
async fn a_clean_reopen_loads_the_snapshot_and_never_rebuilds() {
    let dir = tempfile::tempdir().unwrap();
    let g = seed(dir.path(), 20).await;
    assert_eq!(g.schema_rebuild_count(), 0, "a new database starts clean");
    g.close().await.unwrap();
    drop(g);

    let g = Graph::open(dir.path()).await.unwrap();
    assert_eq!(g.stats().await.unwrap().recovery.operations_replayed, 0);
    assert_eq!(g.get_label_count("Planta").await.unwrap(), 20);
    g.add_node(Node::new("Planta")).await.unwrap();
    assert_eq!(g.node_count().await.unwrap(), 21);
    assert_eq!(g.schema_rebuild_count(), 0, "loaded from the snapshot, then maintained");
    g.close().await.unwrap();
}

#[tokio::test]
async fn a_crash_style_reopen_rebuilds_once_and_is_correct() {
    let dir = tempfile::tempdir().unwrap();
    let g = seed(dir.path(), 40).await;
    drop(g); // sin close: el WAL queda con commits ⇒ crash recovery

    let g = Graph::open(dir.path()).await.unwrap();
    assert!(g.stats().await.unwrap().recovery.crash_recovery);
    // stats() ya leyó el esquema: exactamente una reconstrucción.
    assert_eq!(g.schema_rebuild_count(), 1);
    assert_eq!(g.get_label_count("Planta").await.unwrap(), 40);
    g.add_node(Node::new("Planta")).await.unwrap();
    assert_eq!(g.get_label_count("Planta").await.unwrap(), 41);
    assert_eq!(g.schema_rebuild_count(), 1, "the rebuilt schema is maintained afterwards");
    g.close().await.unwrap();
    drop(g);

    let g = Graph::open(dir.path()).await.unwrap();
    assert_eq!(g.get_label_count("Planta").await.unwrap(), 41);
    assert_eq!(g.schema_rebuild_count(), 0, "close persisted it");
}

#[tokio::test]
async fn the_snapshot_travels_with_copy_database() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    let g = seed(&src, 7).await;
    g.close().await.unwrap();
    drop(g);

    let report = Storage::copy_database(&src, StorageOptions::default(), &dst, StorageOptions::default()).await.unwrap();
    assert!(report.verified);
    let g = Graph::open(&dst).await.unwrap();
    assert_eq!(g.get_label_count("Planta").await.unwrap(), 7);
    assert_eq!(g.schema_rebuild_count(), 0, "the catalog keyspace carried the snapshot");
}

#[tokio::test]
async fn read_only_open_uses_the_snapshot_or_rebuilds_without_persisting() {
    let dir = tempfile::tempdir().unwrap();
    let g = seed(dir.path(), 5).await;
    g.close().await.unwrap();
    drop(g);

    let ro = Graph::open_read_only(dir.path()).await.unwrap();
    assert_eq!(ro.get_label_count("Planta").await.unwrap(), 5);
    assert_eq!(ro.schema_rebuild_count(), 0);
    ro.close().await.unwrap();
    drop(ro);

    // Sin snapshot (base "legacy"): el handle de solo lectura reconstruye y
    // cierra sin intentar persistir.
    let g = Graph::open(dir.path()).await.unwrap();
    g.storage().delete_meta(nopaldb::storage::META_SCHEMA_SNAPSHOT).await.unwrap();
    g.invalidate_schema(); // que close() no lo vuelva a escribir
    g.storage().flush().await.unwrap();
    drop(g); // sin close, pero el WAL solo tiene el Checkpoint: no es crash recovery
    let ro = Graph::open_read_only(dir.path()).await.unwrap();
    assert_eq!(ro.get_label_count("Planta").await.unwrap(), 5);
    assert_eq!(ro.schema_rebuild_count(), 1);
    ro.close().await.unwrap();
}

#[tokio::test]
async fn a_legacy_database_rebuilds_once_then_close_persists() {
    let dir = tempfile::tempdir().unwrap();
    let g = seed(dir.path(), 9).await;
    g.close().await.unwrap();
    drop(g);

    // Simular 0.6.6: base sin snapshot.
    let g = Graph::open(dir.path()).await.unwrap();
    g.storage().delete_meta(nopaldb::storage::META_SCHEMA_SNAPSHOT).await.unwrap();
    g.invalidate_schema();
    g.storage().flush().await.unwrap();
    drop(g);

    let g = Graph::open(dir.path()).await.unwrap();
    assert_eq!(g.get_label_count("Planta").await.unwrap(), 9);
    assert_eq!(g.schema_rebuild_count(), 1, "no snapshot: one lazy rebuild");
    g.close().await.unwrap();
    drop(g);

    let g = Graph::open(dir.path()).await.unwrap();
    assert_eq!(g.get_label_count("Planta").await.unwrap(), 9);
    assert_eq!(g.schema_rebuild_count(), 0, "close persisted the rebuilt schema");
}

#[tokio::test]
async fn a_hundred_thousand_nodes_then_an_upsert_reads_without_rebuilding() {
    let g = Graph::in_memory().await.unwrap();
    let mut loader = g.bulk_loader(10_000);
    let mut first = None;
    for i in 0..100_000 {
        let n = Node::new("N").with_property("i", PropertyValue::Int(i));
        if first.is_none() {
            first = Some(n.id);
        }
        loader.add_node(n).await.unwrap();
    }
    loader.finish().await.unwrap();
    assert_eq!(g.get_label_count("N").await.unwrap(), 100_000);

    let mut over = Node::new("N").with_property("i", PropertyValue::Int(0)).with_property("nueva", s("x"));
    over.id = first.unwrap();
    g.add_node(over).await.unwrap();
    assert_eq!(g.get_label_count("N").await.unwrap(), 100_000);
    assert!(g.get_label_properties("N").await.unwrap().contains(&"nueva".to_string()));
    assert_eq!(g.node_count().await.unwrap(), 100_000);
    assert_eq!(g.schema_rebuild_count(), 0);
}

#[cfg(feature = "analytics")]
#[tokio::test]
async fn import_parquet_feeds_the_schema() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("nodes.parquet");
    let src = Graph::in_memory().await.unwrap();
    for _ in 0..3 {
        src.add_node(Node::new("Exportado")).await.unwrap();
    }
    src.export_parquet(&file).await.unwrap();

    let dst = Graph::in_memory().await.unwrap();
    dst.import_parquet(&file).await.unwrap();
    assert_eq!(dst.get_label_count("Exportado").await.unwrap(), 3);
    assert_eq!(dst.schema_rebuild_count(), 0);
}
