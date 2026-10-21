//! `Graph::import_parquet` reads files written by other tools, so the parquet
//! dependency keeps the usual codecs: Snappy (what NopalDB writes), gzip and
//! zstd. brotli and lz4 were dropped in 0.8.1 to compile less.
#![cfg(feature = "analytics")]

use std::sync::Arc;

use arrow::array::StringArray;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use nopaldb::{Graph, NodeId};
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, GzipLevel, ZstdLevel};
use parquet::file::properties::WriterProperties;

/// Writes the two columns `import_parquet` reads (`id`, `label`) with `codec`.
fn write_plants(path: &std::path::Path, codec: Compression) -> Vec<NodeId> {
    let ids: Vec<NodeId> = (0..3).map(|_| NodeId::new_v4()).collect();
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("label", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from_iter_values(ids.iter().map(|id| id.to_string()))),
            Arc::new(StringArray::from(vec!["Planta"; 3])),
        ],
    )
    .unwrap();
    let props = WriterProperties::builder().set_compression(codec).build();
    let file = std::fs::File::create(path).unwrap();
    let mut writer = ArrowWriter::try_new(file, schema, Some(props)).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
    ids
}

async fn import_reads(codec: Compression) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plantas.parquet");
    let ids = write_plants(&path, codec);

    let graph = Graph::in_memory().await.unwrap();
    graph.import_parquet(&path).await.unwrap();
    for id in ids {
        assert_eq!(graph.get_node(id).await.unwrap().label, "Planta", "{codec:?}");
    }
}

#[tokio::test]
async fn imports_snappy() {
    import_reads(Compression::SNAPPY).await;
}

#[tokio::test]
async fn imports_gzip() {
    import_reads(Compression::GZIP(GzipLevel::default())).await;
}

#[tokio::test]
async fn imports_zstd() {
    import_reads(Compression::ZSTD(ZstdLevel::default())).await;
}
