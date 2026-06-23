// tests/arrow_integration_test.rs

#[cfg(feature = "analytics")]
mod arrow_tests {
    use nopaldb::{Graph, Node, PropertyValue, Result};

    #[tokio::test]
    async fn test_export_to_arrow() -> Result<()> {
        let graph = Graph::in_memory().await?;

        println!("📊 Testing Arrow export...\n");

        // Create some nodes
        let mut tx = graph.begin_transaction().await?;

        for i in 0..10 {
            let node = Node::new("Person")
                .with_property("name", PropertyValue::String(format!("User{}", i)))
                .with_property("age", PropertyValue::Int(20 + i));

            tx.add_node(node).await?;
        }

        tx.commit().await?;

        println!("✅ Created 10 nodes");

        // Export to Arrow
        let batch = graph.to_arrow().await?;

        println!("\n📦 Arrow RecordBatch:");
        println!("   Rows: {}", batch.num_rows());
        println!("   Columns: {}", batch.num_columns());

        assert_eq!(batch.num_rows(), 10);
        assert_eq!(batch.num_columns(), 3);

        // Verify schema
        let schema = batch.schema();
        println!("\n📋 Schema:");
        for field in schema.fields() {
            println!("   - {}: {:?}", field.name(), field.data_type());
        }

        assert_eq!(schema.field(0).name(), "id");
        assert_eq!(schema.field(1).name(), "label");
        assert_eq!(schema.field(2).name(), "property_count");

        println!("\n✅ Arrow export test PASSED!");

        Ok(())
    }

    #[tokio::test]
    async fn test_export_to_parquet() -> Result<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let parquet_path = temp_dir.path().join("test.parquet");

        let graph = Graph::in_memory().await?;

        println!("📦 Testing Parquet export...\n");

        // Create nodes
        let mut tx = graph.begin_transaction().await?;

        for i in 0..5 {
            let node = Node::new("Test").with_property("value", PropertyValue::Int(i));
            tx.add_node(node).await?;
        }

        tx.commit().await?;

        println!("✅ Created 5 nodes");

        // Export to Parquet
        graph.export_parquet(&parquet_path).await?;

        println!("✅ Exported to Parquet: {:?}", parquet_path);

        // Verify file exists
        assert!(parquet_path.exists());

        let metadata = std::fs::metadata(&parquet_path).unwrap();
        println!("   File size: {} bytes", metadata.len());

        assert!(metadata.len() > 0);

        // Read back (basic verification)
        let batch = nopaldb::arrow_export::read_parquet(&parquet_path)?;
        println!("✅ Read back from Parquet");
        println!("   Rows: {}", batch.num_rows());

        assert_eq!(batch.num_rows(), 5);

        println!("\n✅ Parquet export test PASSED!");

        Ok(())
    }

    #[tokio::test]
    async fn test_mvcc_history_to_arrow() -> Result<()> {
        let graph = Graph::in_memory().await?;

        println!("⏰ Testing MVCC history export to Arrow...\n");

        // Create node with multiple versions
        let node_id = {
            let mut tx = graph.begin_transaction().await?;
            let node = Node::new("Counter").with_property("value", PropertyValue::Int(0));
            let id = tx.add_node(node).await?;
            tx.commit().await?;
            id
        };

        println!("✅ Created v1 (value=0)");

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Update (creates v2)
        {
            let mut tx = graph.begin_transaction().await?;
            let mut node = graph.get_node(node_id).await?;
            node.properties
                .insert("value".into(), PropertyValue::Int(100));
            tx.add_node(node).await?;
            tx.commit().await?;
        }

        println!("✅ Created v2 (value=100)");

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Update (creates v3)
        {
            let mut tx = graph.begin_transaction().await?;
            let mut node = graph.get_node(node_id).await?;
            node.properties
                .insert("value".into(), PropertyValue::Int(200));
            tx.add_node(node).await?;
            tx.commit().await?;
        }

        println!("✅ Created v3 (value=200)");

        // Export history to Arrow
        let batch = graph.history_to_arrow().await?;

        println!("\n📦 Arrow RecordBatch (MVCC History):");
        println!("   Rows (versions): {}", batch.num_rows());
        println!("   Columns: {}", batch.num_columns());

        assert_eq!(batch.num_rows(), 3); // 3 versions
        assert_eq!(batch.num_columns(), 7); // id, label, version, timestamp, valid_from, valid_to, is_current

        // Verify schema
        let schema = batch.schema();
        println!("\n📋 MVCC Schema:");
        for field in schema.fields() {
            println!("   - {}: {:?}", field.name(), field.data_type());
        }

        assert_eq!(schema.field(0).name(), "id");
        assert_eq!(schema.field(2).name(), "version");
        assert_eq!(schema.field(6).name(), "is_current");

        println!("\n✅ MVCC history to Arrow test PASSED!");

        Ok(())
    }
}

#[cfg(not(feature = "analytics"))]
#[test]
fn analytics_feature_required() {
    println!("⚠️  Arrow tests require 'analytics' feature");
    println!("   Run: cargo test --features analytics");
}
