// src/arrow_export/mod.rs
//
// Apache Arrow integration for NopalDB
// Export graphs to columnar format for analytics

use arrow::array::{ArrayRef, StringArray, Int64Array, UInt64Array, BooleanArray,
                   RecordBatch, StringBuilder, Int64Builder, Float64Builder, BooleanBuilder};
use arrow::datatypes::{Schema, Field, DataType};
use std::sync::Arc;
//use arrow::record_batch::RecordBatch;
use std::collections::HashMap;
use crate::error::{NopalError, Result};
use crate::types::{Node, Edge, PropertyValue};
use crate::mvcc::VersionedNode;

/// Convierte nodos a Arrow RecordBatch (columnar format)
///
/// Schema:
/// - id: String (UUID)
/// - label: String
/// - property_count: Int64
///
/// # Example
/// ```no_run
/// use nopaldb::{Graph, Node, PropertyValue};
/// use nopaldb::arrow_export;
///
/// # async fn example() -> nopaldb::Result<()> {
/// let nodes = vec![
///     Node::new("Person")
///         .with_property("name", PropertyValue::String("Alice".into())),
/// ];
///
/// let batch = arrow_export::nodes_to_arrow(&nodes)?;
/// println!("Converted {} nodes to Arrow", batch.num_rows());
/// # Ok(())
/// # }
/// ```
pub fn nodes_to_arrow(nodes: &[Node]) -> Result<RecordBatch> {
    if nodes.is_empty() {
        return Err(NopalError::Custom("Cannot convert empty node list to Arrow".into()));
    }

    // Define schema (columnar layout)
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("label", DataType::Utf8, false),
        Field::new("property_count", DataType::Int64, false),
    ]));

    // Extract columns (this is the magic of columnar format!)
    // Instead of row-by-row, we extract column-by-column
    let ids: StringArray = nodes.iter()
        .map(|n| Some(n.id.to_string()))
        .collect();

    let labels: StringArray = nodes.iter()
        .map(|n| Some(n.label.as_str()))
        .collect();

    let prop_counts: Int64Array = nodes.iter()
        .map(|n| Some(n.properties.len() as i64))
        .collect();

    // Create RecordBatch (columnar data structure)
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(ids) as ArrayRef,
            Arc::new(labels) as ArrayRef,
            Arc::new(prop_counts) as ArrayRef,
        ],
    )
        .map_err(|e| NopalError::Custom(format!("Arrow conversion error: {}", e)))?;

    log::info!("Converted {} nodes to Arrow RecordBatch", nodes.len());

    Ok(batch)
}

/// Export nodes with properties (schema inferred from label)
///
/// This exports individual properties as columns, providing a clean
/// columnar format perfect for ML/analytics.
pub fn nodes_to_arrow_with_properties(nodes: &[Node], label_filter: Option<&str>) -> Result<RecordBatch> {
    if nodes.is_empty() {
        return Err(NopalError::Custom("No nodes to export".into()));
    }

    // Filter by label if specified
    let filtered_nodes: Vec<&Node> = if let Some(label) = label_filter {
        nodes.iter().filter(|n| n.label == label).collect()
    } else {
        nodes.iter().collect()
    };

    if filtered_nodes.is_empty() {
        return Err(NopalError::Custom(format!(
            "No nodes found with label '{}'",
            label_filter.unwrap_or("any")
        )));
    }

    // Step 1: Discover all properties and their types
    let property_schema = infer_property_schema(&filtered_nodes);

    // Step 2: Build Arrow schema
    let mut fields = vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("label", DataType::Utf8, false),
    ];

    for (prop_name, prop_type) in &property_schema {
        let arrow_type = match prop_type {
            PropertyType::String => DataType::Utf8,
            PropertyType::Int => DataType::Int64,
            PropertyType::Float => DataType::Float64,
            PropertyType::Bool => DataType::Boolean,
            PropertyType::Mixed => DataType::Utf8, // Fallback to string
        };
        fields.push(Field::new(prop_name, arrow_type, true)); // nullable
    }

    let schema = Arc::new(Schema::new(fields));

    // Step 3: Build columns
    let mut id_builder = StringBuilder::new();
    let mut label_builder = StringBuilder::new();
    let mut property_builders: HashMap<String, PropertyBuilder> = HashMap::new();

    for (prop_name, prop_type) in &property_schema {
        property_builders.insert(
            prop_name.clone(),
            PropertyBuilder::new(*prop_type, filtered_nodes.len())
        );
    }

    // Step 4: Populate data
    for node in filtered_nodes {
        id_builder.append_value(node.id.to_string());
        label_builder.append_value(&node.label);

        for (prop_name, builder) in &mut property_builders {
            if let Some(value) = node.properties.get(prop_name) {
                builder.append_value(value);
            } else {
                builder.append_null();
            }
        }
    }

    // Step 5: Build arrays
    let mut columns: Vec<ArrayRef> = vec![
        Arc::new(id_builder.finish()),
        Arc::new(label_builder.finish()),
    ];

    for prop_name in property_schema.keys() {
        let builder = property_builders.remove(prop_name)
            .ok_or_else(|| NopalError::Custom(format!("Missing property builder for '{}'", prop_name)))?;
        columns.push(builder.finish());
    }

    // Step 6: Create RecordBatch
    RecordBatch::try_new(schema, columns)
        .map_err(|e| NopalError::Custom(format!("Failed to create RecordBatch: {}", e)))
}

/// Export edges básico
pub fn edges_to_arrow(edges: &[Edge]) -> Result<RecordBatch> {
    if edges.is_empty() {
        return Err(NopalError::Custom("Cannot convert empty edge list to Arrow".into()));
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("source", DataType::Utf8, false),
        Field::new("target", DataType::Utf8, false),
        Field::new("edge_type", DataType::Utf8, false),
        Field::new("property_count", DataType::Int64, false),
    ]));

    let ids: StringArray = edges.iter()
        .map(|e| Some(e.id.to_string()))
        .collect();

    let sources: StringArray = edges.iter()
        .map(|e| Some(e.source.to_string()))
        .collect();

    let targets: StringArray = edges.iter()
        .map(|e| Some(e.target.to_string()))
        .collect();

    let types: StringArray = edges.iter()
        .map(|e| Some(e.edge_type.as_str()))
        .collect();

    let prop_counts: Int64Array = edges.iter()
        .map(|e| Some(e.properties.len() as i64))
        .collect();

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(ids) as ArrayRef,
            Arc::new(sources) as ArrayRef,
            Arc::new(targets) as ArrayRef,
            Arc::new(types) as ArrayRef,
            Arc::new(prop_counts) as ArrayRef,
        ],
    )
        .map_err(|e| NopalError::Custom(format!("Arrow conversion error: {}", e)))?;

    log::info!("Converted {} edges to Arrow RecordBatch", edges.len());
    Ok(batch)
}

/// Helper: Convert PropertyValue to string representation
fn property_value_to_string(value: &PropertyValue) -> String {
    match value {
        PropertyValue::String(s) => s.clone(),
        PropertyValue::Int(i) => i.to_string(),
        PropertyValue::Float(f) => f.to_string(),
        PropertyValue::Bool(b) => b.to_string(),
        PropertyValue::Null => "null".to_string(),
        PropertyValue::Bytes(bytes) => format!("<bytes:{}>", bytes.len()),
        PropertyValue::Object(_) | PropertyValue::List(_) => "<complex>".to_string(),
    }
}

/// Export edges con propiedades dinámicas
pub fn edges_to_arrow_with_properties(edges: &[Edge]) -> Result<RecordBatch> {
    if edges.is_empty() {
        return Err(NopalError::Custom("Cannot convert empty edge list to Arrow".into()));
    }

    // Collect all property keys
    let mut property_keys = std::collections::HashSet::new();
    for edge in edges {
        for key in edge.properties.keys() {
            property_keys.insert(key.clone());
        }
    }

    let mut property_keys: Vec<String> = property_keys.into_iter().collect();
    property_keys.sort();

    // Build schema
    let mut fields = vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("source", DataType::Utf8, false),
        Field::new("target", DataType::Utf8, false),
        Field::new("edge_type", DataType::Utf8, false),
    ];

    for key in &property_keys {
        fields.push(Field::new(key, DataType::Utf8, true));
    }

    let schema = Arc::new(Schema::new(fields));

    // Build columns
    let mut columns: Vec<ArrayRef> = Vec::new();

    // Fixed columns
    let ids: StringArray = edges.iter()
        .map(|e| Some(e.id.to_string()))
        .collect();
    columns.push(Arc::new(ids));

    let sources: StringArray = edges.iter()
        .map(|e| Some(e.source.to_string()))
        .collect();
    columns.push(Arc::new(sources));

    let targets: StringArray = edges.iter()
        .map(|e| Some(e.target.to_string()))
        .collect();
    columns.push(Arc::new(targets));

    let types: StringArray = edges.iter()
        .map(|e| Some(e.edge_type.as_str()))
        .collect();
    columns.push(Arc::new(types));

    // Property columns
    for key in &property_keys {
        let mut builder = StringBuilder::new();

        for edge in edges {
            if let Some(value) = edge.properties.get(key) {
                let value_str = property_value_to_string(value);
                builder.append_value(&value_str);
            } else {
                builder.append_null();
            }
        }

        columns.push(Arc::new(builder.finish()));
    }

    let batch = RecordBatch::try_new(schema, columns)
        .map_err(|e| NopalError::Custom(format!("Arrow conversion error: {}", e)))?;

    log::info!(
        "Converted {} edges to Arrow RecordBatch with {} property columns",
        edges.len(),
        property_keys.len()
    );

    Ok(batch)
}

/// Export graph completo (nodes + edges)
pub async fn graph_to_arrow(
    graph: &crate::graph::Graph,
    label_filter: Option<&str>,
) -> Result<(RecordBatch, RecordBatch)> {
    // Get nodes
    let nodes = if let Some(label) = label_filter {
        graph.get_nodes_by_label(label).await?
    } else {
        graph.get_all_nodes().await?
    };

    // Get edges
    let edges = graph.get_all_edges().await?;

    log::info!(
        "Exporting graph: {} nodes, {} edges",
        nodes.len(),
        edges.len()
    );

    // Convert
    let nodes_batch = nodes_to_arrow_with_properties(&nodes, label_filter)?;

    let edges_batch = if edges.is_empty() {
        // Empty batch with correct schema
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("source", DataType::Utf8, false),
            Field::new("target", DataType::Utf8, false),
            Field::new("edge_type", DataType::Utf8, false),
        ]));
        RecordBatch::new_empty(schema)
    } else {
        edges_to_arrow_with_properties(&edges)?
    };

    Ok((nodes_batch, edges_batch))
}

/// Infer property schema from nodes
fn infer_property_schema(nodes: &[&Node]) -> HashMap<String, PropertyType> {
    let mut schema = HashMap::new();

    for node in nodes {
        for (key, value) in &node.properties {
            let prop_type = match value {
                PropertyValue::String(_) => PropertyType::String,
                PropertyValue::Int(_) => PropertyType::Int,
                PropertyValue::Float(_) => PropertyType::Float,
                PropertyValue::Bool(_) => PropertyType::Bool,
                PropertyValue::Null => continue,
                PropertyValue::Bytes(_) => continue,
                PropertyValue::Object(_) | PropertyValue::List(_) => continue,
            };

            schema.entry(key.clone())
                .and_modify(|existing| {
                    // If types don't match, mark as Mixed
                    if *existing != prop_type {
                        *existing = PropertyType::Mixed;
                    }
                })
                .or_insert(prop_type);
        }
    }

    schema
}

/// Property type for schema inference
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PropertyType {
    String,
    Int,
    Float,
    Bool,
    Mixed, // When same property has different types
}

/// Dynamic property builder
enum PropertyBuilder {
    String(StringBuilder),
    Int(Int64Builder),
    Float(Float64Builder),
    Bool(BooleanBuilder),
}

impl PropertyBuilder {
    fn new(prop_type: PropertyType, capacity: usize) -> Self {
        match prop_type {
            PropertyType::String | PropertyType::Mixed => {
                PropertyBuilder::String(StringBuilder::new())
            }
            PropertyType::Int => {
                PropertyBuilder::Int(Int64Builder::with_capacity(capacity))
            }
            PropertyType::Float => {
                PropertyBuilder::Float(Float64Builder::with_capacity(capacity))
            }
            PropertyType::Bool => {
                PropertyBuilder::Bool(BooleanBuilder::with_capacity(capacity))
            }
        }
    }

    fn append_value(&mut self, value: &PropertyValue) {
        match self {
            PropertyBuilder::String(b) => {
                match value {
                    PropertyValue::String(s) => b.append_value(s),
                    PropertyValue::Int(i) => b.append_value(i.to_string()),
                    PropertyValue::Float(f) => b.append_value(f.to_string()),
                    PropertyValue::Bool(v) => b.append_value(v.to_string()),
                    PropertyValue::Null => b.append_value("null"),
                    PropertyValue::Bytes(bytes) => b.append_value(format!("<bytes:{}>", bytes.len())),
                    PropertyValue::Object(_) | PropertyValue::List(_) => b.append_value("<complex>"),
                }
            }
            PropertyBuilder::Int(b) => {
                match value {
                    PropertyValue::Int(i) => b.append_value(*i),
                    _ => b.append_null(),
                }
            }
            PropertyBuilder::Float(b) => {
                match value {
                    PropertyValue::Float(f) => b.append_value(*f),
                    _ => b.append_null(),
                }
            }
            PropertyBuilder::Bool(b) => {
                match value {
                    PropertyValue::Bool(v) => b.append_value(*v),
                    _ => b.append_null(),
                }
            }
        }
    }

    fn append_null(&mut self) {
        match self {
            PropertyBuilder::String(b) => b.append_null(),
            PropertyBuilder::Int(b) => b.append_null(),
            PropertyBuilder::Float(b) => b.append_null(),
            PropertyBuilder::Bool(b) => b.append_null(),
        }
    }

    fn finish(self) -> ArrayRef {
        match self {
            PropertyBuilder::String(mut b) => Arc::new(b.finish()),
            PropertyBuilder::Int(mut b) => Arc::new(b.finish()),
            PropertyBuilder::Float(mut b) => Arc::new(b.finish()),
            PropertyBuilder::Bool(mut b) => Arc::new(b.finish()),
        }
    }
}

/// Convierte nodos versionados a Arrow RecordBatch (MVCC + Arrow)
///
/// Schema:
/// - id: String
/// - label: String
/// - version: UInt64
/// - timestamp: UInt64
/// - valid_from: UInt64
/// - valid_to: UInt64 (nullable)
/// - is_current: Boolean
pub fn versioned_nodes_to_arrow(nodes: &[VersionedNode]) -> Result<RecordBatch> {
    if nodes.is_empty() {
        return Err(NopalError::Custom("Cannot convert empty versioned node list to Arrow".into()));
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("label", DataType::Utf8, false),
        Field::new("version", DataType::UInt64, false),
        Field::new("timestamp", DataType::UInt64, false),
        Field::new("valid_from", DataType::UInt64, false),
        Field::new("valid_to", DataType::UInt64, true), // Nullable
        Field::new("is_current", DataType::Boolean, false),
    ]));

    let ids: StringArray = nodes.iter()
        .map(|n| Some(n.id.to_string()))
        .collect();

    let labels: StringArray = nodes.iter()
        .map(|n| Some(n.node_data.label.as_str()))
        .collect();

    let versions: UInt64Array = nodes.iter()
        .map(|n| Some(n.version))
        .collect();

    let timestamps: UInt64Array = nodes.iter()
        .map(|n| Some(n.timestamp))
        .collect();

    let valid_from: UInt64Array = nodes.iter()
        .map(|n| Some(n.valid_from))
        .collect();

    let valid_to: UInt64Array = nodes.iter()
        .map(|n| n.valid_to)
        .collect();

    let is_current: BooleanArray = nodes.iter()
        .map(|n| Some(n.valid_to.is_none()))
        .collect();

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(ids) as ArrayRef,
            Arc::new(labels) as ArrayRef,
            Arc::new(versions) as ArrayRef,
            Arc::new(timestamps) as ArrayRef,
            Arc::new(valid_from) as ArrayRef,
            Arc::new(valid_to) as ArrayRef,
            Arc::new(is_current) as ArrayRef,
        ],
    )
        .map_err(|e| NopalError::Custom(format!("Arrow conversion error: {}", e)))?;

    log::info!("Converted {} versioned nodes to Arrow RecordBatch", nodes.len());

    Ok(batch)
}

/// Escribe RecordBatch a archivo Parquet
///
/// Parquet provides:
/// - Efficient compression (SNAPPY)
/// - Fast columnar queries
/// - Industry standard format
pub fn write_parquet(
    batch: &RecordBatch,
    path: impl AsRef<std::path::Path>,
) -> Result<()> {
    use parquet::arrow::ArrowWriter;
    use parquet::file::properties::WriterProperties;
    use std::fs::File;

    let file = File::create(path.as_ref())
        .map_err(NopalError::IoError)?;

    let props = WriterProperties::builder()
        .set_compression(parquet::basic::Compression::SNAPPY)
        .build();

    let mut writer = ArrowWriter::try_new(file, batch.schema(), Some(props))
        .map_err(|e| NopalError::Custom(format!("Parquet writer error: {}", e)))?;

    writer.write(batch)
        .map_err(|e| NopalError::Custom(format!("Parquet write error: {}", e)))?;

    writer.close()
        .map_err(|e| NopalError::Custom(format!("Parquet close error: {}", e)))?;

    log::info!("Wrote Arrow RecordBatch to Parquet: {:?}", path.as_ref());

    Ok(())
}

/// Lee RecordBatch desde archivo Parquet
pub fn read_parquet(
    path: impl AsRef<std::path::Path>,
) -> Result<RecordBatch> {
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use std::fs::File;

    let file = File::open(path.as_ref())
        .map_err(NopalError::IoError)?;

    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| NopalError::Custom(format!("Parquet reader error: {}", e)))?;

    let mut reader = builder.build()
        .map_err(|e| NopalError::Custom(format!("Parquet build error: {}", e)))?;

    // Read first batch (assuming single batch for simplicity)
    let batch = reader.next()
        .ok_or_else(|| NopalError::Custom("No data in Parquet file".into()))?
        .map_err(|e| NopalError::Custom(format!("Parquet read error: {}", e)))?;

    log::info!("Read Arrow RecordBatch from Parquet: {:?}", path.as_ref());

    Ok(batch)
}



#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PropertyValue;

    #[test]
    fn test_nodes_to_arrow() {
        let nodes = vec![
            Node::new("Person")
                .with_property("name", PropertyValue::String("Alice".into()))
                .with_property("age", PropertyValue::Int(30)),
            Node::new("Person")
                .with_property("name", PropertyValue::String("Bob".into()))
                .with_property("age", PropertyValue::Int(25)),
        ];

        let batch = nodes_to_arrow(&nodes).unwrap();

        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 3);
        assert_eq!(batch.schema().fields().len(), 3);

        // Verify schema
        let schema = batch.schema();
        assert_eq!(schema.field(0).name(), "id");
        assert_eq!(schema.field(1).name(), "label");
        assert_eq!(schema.field(2).name(), "property_count");
    }

    #[test]
    fn test_versioned_nodes_to_arrow() {
        let node1 = Node::new("Test");
        let v1 = VersionedNode::new(node1.clone(), 100);
        let v2 = VersionedNode::new_version(&v1, node1, 200);

        let nodes = vec![v1, v2];
        let batch = versioned_nodes_to_arrow(&nodes).unwrap();

        assert_eq!(batch.num_rows(), 2);
        assert_eq!(batch.num_columns(), 7);

        // Verify columns exist
        let schema = batch.schema();
        assert_eq!(schema.field(0).name(), "id");
        assert_eq!(schema.field(2).name(), "version");
        assert_eq!(schema.field(6).name(), "is_current");
    }

    #[test]
    fn test_empty_nodes_error() {
        let nodes: Vec<Node> = vec![];
        let result = nodes_to_arrow(&nodes);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("empty"));
    }
}