// examples/arrow/quickstart.rs

#[cfg(feature = "analytics")]
use nopaldb::{Graph, Node, PropertyValue};

#[cfg(feature = "analytics")]
#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    env_logger::init();

    println!("🚀 NopalDB Arrow Quickstart\n");

    // 1. Crear grafo
    let graph = Graph::in_memory().await?;

    // 2. Insertar datos
    let mut tx = graph.begin_transaction().await?;
    for i in 0..100 {
        let node = Node::new("Person")
            .with_property("name", PropertyValue::String(format!("User{}", i)))
            .with_property("age", PropertyValue::Int(20 + (i % 50)));
        tx.add_node(node).await?;
    }
    tx.commit().await?;
    println!("✅ Inserted 100 nodes\n");

    // 3. Export a Arrow
    let batch = graph.to_arrow().await?;
    println!("📦 Arrow RecordBatch:");
    println!("   Rows: {}", batch.num_rows());
    println!("   Columns: {}", batch.num_columns());

    let schema = batch.schema();
    println!("\n📋 Schema:");
    for field in schema.fields() {
        println!("   - {}: {:?}", field.name(), field.data_type());
    }

    // 4. Export a Parquet
    let temp_dir = tempfile::tempdir()?;
    let parquet_path = temp_dir.path().join("quickstart.parquet");

    graph.export_parquet(&parquet_path).await?;
    println!("\n💾 Exported to Parquet: {:?}", parquet_path);

    let metadata = std::fs::metadata(&parquet_path)?;
    println!("   Size: {} bytes", metadata.len());

    println!("\n✅ Quickstart complete!");

    Ok(())
}

#[cfg(not(feature = "analytics"))]
fn main() {
    println!("⚠️  This example requires the 'analytics' feature");
    println!("   Run: cargo run --example quickstart --features analytics");
}