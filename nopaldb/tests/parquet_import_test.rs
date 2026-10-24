//! `Graph::import_parquet`: the metadata Parquet import.
//!
//! Until 0.8.4 it read only the first batch of the file (1 024 rows), wrote
//! row by row outside a transaction (a bad row left the previous ones
//! written), and replaced an existing node with an empty one (its properties
//! gone). Now it reads every batch, validates the whole file before writing,
//! and writes all the nodes in one transaction or none.
#![cfg(feature = "analytics")]

use std::sync::Arc;

use arrow::array::{ArrayRef, Int64Array, StringArray};
use arrow::datatypes::{Field, Schema};
use arrow::record_batch::RecordBatch;
use nopaldb::types::{Node, NodeId, PropertyValue};
use nopaldb::Graph;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ArrowWriter;

/// Writes `columns` (name, values, nullable) as a Parquet file.
fn write(path: &std::path::Path, columns: Vec<(&str, ArrayRef)>) {
    let schema = Arc::new(Schema::new(
        columns
            .iter()
            .map(|(n, a)| Field::new(*n, a.data_type().clone(), true))
            .collect::<Vec<_>>(),
    ));
    let batch = RecordBatch::try_new(schema.clone(), columns.into_iter().map(|(_, a)| a).collect()).unwrap();
    let mut writer = ArrowWriter::try_new(std::fs::File::create(path).unwrap(), schema, None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
}

fn text(values: Vec<Option<String>>) -> ArrayRef {
    Arc::new(StringArray::from(values))
}

fn ids(n: usize) -> Vec<NodeId> {
    (0..n).map(|_| NodeId::new_v4()).collect()
}

async fn import_error(graph: &Graph, path: &std::path::Path) -> String {
    match graph.import_parquet(path).await {
        Ok(()) => panic!("import_parquet should fail"),
        Err(e) => e.to_string(),
    }
}

#[tokio::test]
async fn every_batch_is_imported_including_the_last() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plantas.parquet");
    let src = Graph::in_memory().await.unwrap();
    let mut created = Vec::new();
    for i in 0..2_600 {
        created.push(src.add_node(Node::new(if i % 2 == 0 { "Planta" } else { "Hongo" })).await.unwrap());
    }
    src.export_parquet(&path).await.unwrap();

    // The reader hands this file over in at least three batches.
    let batches = ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(&path).unwrap())
        .unwrap()
        .build()
        .unwrap()
        .count();
    assert!(batches >= 3, "{batches} batches");

    let dst = Graph::in_memory().await.unwrap();
    dst.import_parquet(&path).await.unwrap();
    assert_eq!(dst.node_count().await.unwrap(), 2_600);
    assert_eq!(dst.get_label_count("Planta").await.unwrap(), 1_300);
    let last = *created.last().unwrap();
    assert_eq!(dst.get_node(last).await.unwrap().label, "Hongo", "the last batch is there");
}

#[tokio::test]
async fn an_empty_file_imports_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vacio.parquet");
    write(&path, vec![("id", text(vec![])), ("label", text(vec![]))]);
    let g = Graph::in_memory().await.unwrap();
    g.import_parquet(&path).await.unwrap();
    assert_eq!(g.node_count().await.unwrap(), 0);
}

#[tokio::test]
async fn a_bad_row_in_the_last_batch_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("malo.parquet");
    let mut id_values: Vec<Option<String>> = ids(2_500).iter().map(|id| Some(id.to_string())).collect();
    id_values.push(Some("no-es-un-uuid".into()));
    let labels = vec![Some("Planta".to_string()); id_values.len()];
    write(&path, vec![("id", text(id_values)), ("label", text(labels))]);

    let g = Graph::in_memory().await.unwrap();
    let err = import_error(&g, &path).await;
    assert!(err.contains("row 2500") && err.contains("not a UUID"), "{err}");
    assert_eq!(g.node_count().await.unwrap(), 0, "all or nothing");
}

#[tokio::test]
async fn nulls_and_repeated_ids_are_errors_before_any_write() {
    let dir = tempfile::tempdir().unwrap();
    let g = Graph::in_memory().await.unwrap();

    let path = dir.path().join("nulo.parquet");
    let a = NodeId::new_v4().to_string();
    write(&path, vec![("id", text(vec![Some(a.clone()), None])), ("label", text(vec![Some("P".into()), Some("P".into())]))]);
    let err = import_error(&g, &path).await;
    assert!(err.contains("`id`") && err.contains("null"), "{err}");

    let path = dir.path().join("sin_label.parquet");
    write(&path, vec![("id", text(vec![Some(a.clone())])), ("label", text(vec![None]))]);
    let err = import_error(&g, &path).await;
    assert!(err.contains("`label`") && err.contains("null"), "{err}");

    let path = dir.path().join("repetido.parquet");
    write(&path, vec![("id", text(vec![Some(a.clone()), Some(a.clone())])), ("label", text(vec![Some("P".into()), Some("Q".into())]))]);
    let err = import_error(&g, &path).await;
    assert!(err.contains("more than once"), "{err}");

    let path = dir.path().join("sin_id.parquet");
    write(&path, vec![("label", text(vec![Some("P".into())]))]);
    let err = import_error(&g, &path).await;
    assert!(err.contains("missing column `id`"), "{err}");

    assert_eq!(g.node_count().await.unwrap(), 0);
}

#[tokio::test]
async fn property_columns_are_rejected_not_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("con_props.parquet");
    write(
        &path,
        vec![
            ("id", text(vec![Some(NodeId::new_v4().to_string())])),
            ("label", text(vec![Some("Planta".into())])),
            ("property_count", Arc::new(Int64Array::from(vec![2])) as ArrayRef),
            ("nombre", text(vec![Some("nopal".into())])),
        ],
    );
    let g = Graph::in_memory().await.unwrap();
    let err = import_error(&g, &path).await;
    assert!(err.contains("nombre") && err.contains("unsupported"), "{err}");
    assert_eq!(g.node_count().await.unwrap(), 0);

    // The metadata column of the export (`property_count`) is accepted.
    let path = dir.path().join("metadatos.parquet");
    write(
        &path,
        vec![
            ("id", text(vec![Some(NodeId::new_v4().to_string())])),
            ("label", text(vec![Some("Planta".into())])),
            ("property_count", Arc::new(Int64Array::from(vec![2])) as ArrayRef),
        ],
    );
    g.import_parquet(&path).await.unwrap();
    assert_eq!(g.node_count().await.unwrap(), 1);
}

#[tokio::test]
async fn an_existing_id_is_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existente.parquet");
    let g = Graph::in_memory().await.unwrap();
    let kept = g
        .add_node(Node::new("Planta").with_property("nombre", PropertyValue::String("nopal".into())))
        .await
        .unwrap();
    let fresh = NodeId::new_v4();
    write(
        &path,
        vec![
            ("id", text(vec![Some(fresh.to_string()), Some(kept.to_string())])),
            ("label", text(vec![Some("Planta".into()), Some("Otra".into())])),
        ],
    );
    let err = import_error(&g, &path).await;
    assert!(err.contains("already exist") && err.contains(&kept.to_string()), "{err}");

    let node = g.get_node(kept).await.unwrap();
    assert_eq!(node.label, "Planta");
    assert_eq!(node.properties.get("nombre"), Some(&PropertyValue::String("nopal".into())));
    assert!(g.get_node(fresh).await.is_err(), "nothing else was written either");
}

#[tokio::test]
async fn imported_nodes_are_durable_and_indexed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nodos.parquet");
    let id = NodeId::new_v4();
    write(&path, vec![("id", text(vec![Some(id.to_string())])), ("label", text(vec![Some("Planta".into())]))]);

    let db = dir.path().join("db");
    {
        let g = Graph::open(&db).await.unwrap();
        g.import_parquet(&path).await.unwrap();
        // Written as a transaction: it has MVCC history like any write.
        assert_eq!(g.history(id).await.unwrap().len(), 1);
        g.close().await.unwrap();
    }
    let g = Graph::open(&db).await.unwrap();
    assert_eq!(g.get_node(id).await.unwrap().label, "Planta");
    assert_eq!(g.get_label_count("Planta").await.unwrap(), 1);
}
