// tests/nql_export_honest_test.rs
//
// #295: EXPORT (and SKETCH/COMMIT) either do what was asked or fail with a
// named error (docs/NQL_SEMANTICS.md § 5e). Until 0.9.3:
// - `export parquet "path"` answered "Exported N rows as Parquet" and wrote
//   no file; `export arrow` answered success with a text summary;
// - unknown options (`delimiter=";"`) were silently ignored;
// - `sketch` / `commit` answered an OK message and did nothing.

use nopaldb::{Graph, Node, NqlResult, PropertyValue, Result};

async fn plants() -> Result<Graph> {
    let graph = Graph::in_memory().await?;
    let mut tx = graph.begin_transaction().await?;
    for (name, height) in [("Nopal", 120i64), ("Maguey", 180), ("Biznaga", 40)] {
        tx.add_node(
            Node::new("Plant")
                .with_property("name", PropertyValue::String(name.to_string()))
                .with_property("height", PropertyValue::Int(height)),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(graph)
}

async fn export_error(graph: &Graph, nql: &str) -> String {
    match graph.execute_statement(nql).await {
        Ok(result) => panic!("expected an error for {nql:?}, got: {}", result.summary()),
        Err(e) => e.to_string(),
    }
}

// ─── options ─────────────────────────────────────────────

#[tokio::test]
async fn csv_unknown_option_delimiter_is_an_error() -> Result<()> {
    let graph = plants().await?;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("plants.csv");
    let nql = format!(
        r#"find p.name, p.height from (p:Plant) export csv with path="{}", delimiter=";""#,
        path.display()
    );

    let err = export_error(&graph, &nql).await;
    assert!(err.contains("delimiter"), "{err}");
    assert!(err.contains("path, separator, header"), "{err}");
    assert!(err.contains("`separator`"), "names the right option: {err}");
    assert!(!path.exists(), "nothing is written when the options are wrong");
    Ok(())
}

#[tokio::test]
async fn json_unknown_option_lists_valid_ones() -> Result<()> {
    let graph = plants().await?;
    let err = export_error(&graph, r#"find p.name from (p:Plant) export json with indent=true"#).await;
    assert!(err.contains("indent"), "{err}");
    assert!(err.contains("path, pretty, jsonl"), "{err}");
    Ok(())
}

#[tokio::test]
async fn csv_separator_still_works() -> Result<()> {
    let graph = plants().await?;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("plants.csv");
    let nql = format!(
        r#"find p.name, p.height from (p:Plant) where p.name = "Nopal" export csv with path="{}", separator=";""#,
        path.display()
    );

    match graph.execute_statement(&nql).await? {
        NqlResult::Export { data, rows_exported, .. } => {
            assert_eq!(rows_exported, 1);
            assert_eq!(data, "p.name;p.height\nNopal;120\n");
        }
        other => panic!("expected Export, got: {}", other.summary()),
    }
    assert_eq!(std::fs::read_to_string(&path)?, "p.name;p.height\nNopal;120\n");
    Ok(())
}

#[tokio::test]
async fn non_string_path_is_an_error() -> Result<()> {
    let graph = plants().await?;
    let err = export_error(&graph, "find p.name from (p:Plant) export json with path=3").await;
    assert!(err.contains("'path' must be a string"), "{err}");
    Ok(())
}

// ─── arrow / parquet ─────────────────────────────────────

#[tokio::test]
async fn arrow_without_path_is_an_error() -> Result<()> {
    let graph = plants().await?;
    let err = export_error(&graph, "find p.name from (p:Plant) export arrow").await;
    assert!(err.contains("export arrow needs a file"), "{err}");
    Ok(())
}

#[tokio::test]
async fn parquet_path_option_is_rejected_with_a_hint() -> Result<()> {
    let graph = plants().await?;
    let err = export_error(
        &graph,
        r#"find p.name from (p:Plant) export parquet "a.parquet" with path="b.parquet""#,
    )
    .await;
    assert!(err.contains("valid options: none"), "{err}");
    assert!(err.contains("the path goes in the syntax"), "{err}");
    Ok(())
}

#[cfg(feature = "analytics")]
#[tokio::test]
async fn parquet_export_writes_the_file() -> Result<()> {
    use arrow::array::{Array, Int64Array, StringArray};

    let graph = plants().await?;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("plants.parquet");
    let nql = format!(
        r#"find p.name, p.height from (p:Plant) order by p.height export parquet "{}""#,
        path.display()
    );

    match graph.execute_statement(&nql).await? {
        NqlResult::Export { format, data, rows_exported } => {
            assert_eq!(format, "Parquet");
            assert_eq!(rows_exported, 3);
            assert_eq!(data, path.display().to_string());
        }
        other => panic!("expected Export, got: {}", other.summary()),
    }

    let batch = nopaldb::arrow_export::read_parquet(&path)?;
    assert_eq!(batch.num_rows(), 3);
    let schema = batch.schema();
    let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
    assert_eq!(names, ["p.name", "p.height"], "result columns, in order");

    let name = batch.column(0).as_any().downcast_ref::<StringArray>().expect("p.name is Utf8");
    let height = batch.column(1).as_any().downcast_ref::<Int64Array>().expect("p.height is Int64");
    assert_eq!(name.value(0), "Biznaga");
    assert_eq!(height.value(0), 40);
    assert_eq!(name.value(2), "Maguey");
    assert_eq!(height.value(2), 180);
    assert_eq!(height.null_count(), 0);
    Ok(())
}

#[cfg(feature = "analytics")]
#[tokio::test]
async fn arrow_export_with_path_writes_an_ipc_file() -> Result<()> {
    use arrow::array::StringArray;

    let graph = plants().await?;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("plants.arrow");
    let nql = format!(
        r#"find p.name from (p:Plant) where p.height > 100 order by p.name export arrow with path="{}""#,
        path.display()
    );

    match graph.execute_statement(&nql).await? {
        NqlResult::Export { format, rows_exported, .. } => {
            assert_eq!(format, "Arrow");
            assert_eq!(rows_exported, 2);
        }
        other => panic!("expected Export, got: {}", other.summary()),
    }

    let reader = arrow::ipc::reader::FileReader::try_new(std::fs::File::open(&path)?, None)
        .expect("an Arrow IPC file");
    let batches: Vec<_> = reader.collect::<std::result::Result<_, _>>().expect("readable batches");
    assert_eq!(batches.len(), 1);
    let names = batches[0].column(0).as_any().downcast_ref::<StringArray>().expect("Utf8");
    assert_eq!(names.iter().flatten().collect::<Vec<_>>(), ["Maguey", "Nopal"]);
    Ok(())
}

#[cfg(feature = "analytics")]
#[tokio::test]
async fn execute_nql_reports_where_parquet_went() -> Result<()> {
    let graph = plants().await?;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("plants.parquet");
    let nql = format!(r#"find p.name from (p:Plant) export parquet "{}""#, path.display());

    let result = graph.execute_nql(&nql).await?;
    let row = &result.rows()[0];
    assert_eq!(
        row.get("exported_to"),
        Some(&PropertyValue::String(path.display().to_string()))
    );
    assert_eq!(row.get("rows"), Some(&PropertyValue::Int(3)));
    assert!(path.exists());
    Ok(())
}

#[cfg(not(feature = "analytics"))]
#[tokio::test]
async fn parquet_without_analytics_is_a_named_error() -> Result<()> {
    let graph = plants().await?;
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("plants.parquet");
    let nql = format!(r#"find p.name from (p:Plant) export parquet "{}""#, path.display());

    let err = export_error(&graph, &nql).await;
    assert!(err.contains("requires the `analytics` feature"), "{err}");
    assert!(!path.exists());
    Ok(())
}

// ─── sketch / commit ─────────────────────────────────────

#[tokio::test]
async fn sketch_statement_is_an_error() -> Result<()> {
    let graph = plants().await?;
    let err = export_error(&graph, "sketch tall = find p.name from (p:Plant) where p.height > 100").await;
    assert!(err.contains("SKETCH is not supported"), "{err}");
    assert!(err.contains("SketchManager"), "{err}");
    Ok(())
}

#[tokio::test]
async fn commit_statement_is_an_error() -> Result<()> {
    let graph = plants().await?;
    let err = export_error(&graph, "commit tall").await;
    assert!(err.contains("COMMIT <sketch> is not supported"), "{err}");
    // execute_nql routes non-queries through execute_statement: same error.
    assert!(graph.execute_nql("commit tall").await.is_err());
    Ok(())
}
