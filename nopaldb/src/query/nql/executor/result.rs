// src/query/nql/executor/result.rs
//
// Query Result types

use crate::types::PropertyValue;
use std::collections::HashMap;

// ═══════════════════════════════════════════════════════════
// NQL UNIFIED RESULT (for execute_statement)
// ═══════════════════════════════════════════════════════════

/// Unified result type for all NQL statements
#[derive(Debug, Clone)]
pub enum NqlResult {
    /// Result of a FIND query
    Query(QueryResult),
    /// Result of ADD/DELETE/UPDATE
    Write(WriteResult),
    /// Result of CREATE INDEX / DROP INDEX
    Index(String),
    /// Result of EXPLAIN
    Explain(String),
    /// Result of PROFILE
    Profile(ProfileResult),
    /// Result of EXPORT (format name, serialized data)
    Export {
        format: String,
        data: String,
        rows_exported: usize,
    },
    /// Message for unsupported or informational responses
    Message(String),
}

impl NqlResult {
    /// Extract QueryResult if this is a Query result, error otherwise
    pub fn into_query(self) -> crate::error::Result<QueryResult> {
        match self {
            NqlResult::Query(r) => Ok(r),
            other => Err(crate::error::NopalError::QueryExecutionError(format!(
                "Expected Query result, got {:?}",
                std::mem::discriminant(&other)
            ))),
        }
    }

    /// Get a human-readable summary of the result
    pub fn summary(&self) -> String {
        match self {
            NqlResult::Query(r) => format!("{} rows returned", r.len()),
            NqlResult::Write(w) => format!(
                "{} nodes created, {} edges created, {} nodes deleted, {} edges deleted, {} nodes updated, {} edges updated",
                w.nodes_created,
                w.edges_created,
                w.nodes_deleted,
                w.edges_deleted,
                w.nodes_updated,
                w.edges_updated
            ),
            NqlResult::Index(msg) => msg.clone(),
            NqlResult::Explain(plan) => plan.clone(),
            NqlResult::Profile(profile) => format!(
                "PROFILE query: {} rows, {:.3} ms",
                profile.rows_returned, profile.execution_ms
            ),
            NqlResult::Export {
                format,
                rows_exported,
                ..
            } => {
                format!("Exported {} rows as {}", rows_exported, format)
            }
            NqlResult::Message(msg) => msg.clone(),
        }
    }
}

/// Combined write operation result
#[derive(Debug, Clone)]
pub struct WriteResult {
    pub nodes_created: usize,
    pub edges_created: usize,
    pub nodes_deleted: usize,
    pub edges_deleted: usize,
    pub nodes_updated: usize,
    pub edges_updated: usize,
    pub properties_changed: usize,
    pub created_ids: Vec<String>,
}

impl WriteResult {
    pub fn from_add(add: &AddResult) -> Self {
        WriteResult {
            nodes_created: add.nodes_created,
            edges_created: add.edges_created,
            nodes_deleted: 0,
            edges_deleted: 0,
            nodes_updated: 0,
            edges_updated: 0,
            properties_changed: 0,
            created_ids: add.created_ids.clone(),
        }
    }

    pub fn from_delete(del: &DeleteResult) -> Self {
        WriteResult {
            nodes_created: 0,
            edges_created: 0,
            nodes_deleted: del.nodes_deleted,
            edges_deleted: del.edges_deleted,
            nodes_updated: 0,
            edges_updated: 0,
            properties_changed: 0,
            created_ids: Vec::new(),
        }
    }

    pub fn from_update(upd: &UpdateResult) -> Self {
        WriteResult {
            nodes_created: 0,
            edges_created: 0,
            nodes_deleted: 0,
            edges_deleted: 0,
            nodes_updated: upd.nodes_updated,
            edges_updated: upd.edges_updated,
            properties_changed: upd.properties_changed,
            created_ids: Vec::new(),
        }
    }
}

// ═══════════════════════════════════════════════════════════
// INDIVIDUAL RESULT TYPES
// ═══════════════════════════════════════════════════════════

/// Query execution result
#[derive(Debug, Clone)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Row>,
}

/// Result of PROFILE statement
#[derive(Debug, Clone)]
pub struct ProfileResult {
    pub plan: String,
    pub statement_type: String,
    pub execution_ms: f64,
    pub rows_returned: i64,
    pub columns: Vec<String>,
    pub path_query: bool,
    pub path_metrics: Option<PropertyValue>,
}

/// Result of ADD operation
#[derive(Debug, Clone)]
pub struct AddResult {
    pub nodes_created: usize,
    pub edges_created: usize,
    pub created_ids: Vec<String>,
}

impl AddResult {
    pub fn new() -> Self {
        AddResult {
            nodes_created: 0,
            edges_created: 0,
            created_ids: Vec::new(),
        }
    }
}

impl Default for AddResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of DELETE operation
#[derive(Debug, Clone)]
pub struct DeleteResult {
    pub nodes_deleted: usize,
    pub edges_deleted: usize,
}

impl DeleteResult {
    pub fn new() -> Self {
        DeleteResult {
            nodes_deleted: 0,
            edges_deleted: 0,
        }
    }
}

impl Default for DeleteResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of UPDATE operation
#[derive(Debug, Clone)]
pub struct UpdateResult {
    pub nodes_updated: usize,
    pub edges_updated: usize,
    pub properties_changed: usize,
}

impl UpdateResult {
    pub fn new() -> Self {
        UpdateResult {
            nodes_updated: 0,
            edges_updated: 0,
            properties_changed: 0,
        }
    }
}

impl Default for UpdateResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Single result row
#[derive(Debug, Clone)]
pub struct Row {
    pub values: HashMap<String, PropertyValue>,
}

impl QueryResult {
    /// Create empty result
    pub fn empty() -> Self {
        QueryResult {
            columns: vec![],
            rows: vec![],
        }
    }

    /// Create result with columns
    pub fn new(columns: Vec<String>) -> Self {
        QueryResult {
            columns,
            rows: vec![],
        }
    }

    /// Add row to result
    pub fn add_row(&mut self, row: Row) {
        self.rows.push(row);
    }

    /// Get rows
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Number of rows
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Is empty
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    // ═══════════════════════════════════════════════════════════
    // EXPORT API — direct methods for Rust API usage
    // ═══════════════════════════════════════════════════════════

    /// Export result as CSV string with default comma separator and header
    ///
    /// ```no_run
    /// # use nopaldb::Graph;
    /// # async fn example() -> nopaldb::Result<()> {
    /// let graph = Graph::in_memory().await?;
    /// let result = graph.execute_nql("find p.name, p.age from (p:Person)").await?;
    /// let csv = result.to_csv();
    /// println!("{}", csv);
    /// // Output:
    /// // p.name,p.age
    /// // Alice,30
    /// // Bob,25
    /// # Ok(())
    /// # }
    /// ```
    pub fn to_csv(&self) -> String {
        super::export::query_result_to_csv(self, ",", true)
    }

    /// Export result as CSV with custom separator and optional header
    ///
    /// ```no_run
    /// # use nopaldb::Graph;
    /// # async fn example() -> nopaldb::Result<()> {
    /// let graph = Graph::in_memory().await?;
    /// let result = graph.execute_nql("find p.name from (p:Person)").await?;
    /// let tsv = result.to_csv_custom("\t", false);
    /// # Ok(())
    /// # }
    /// ```
    pub fn to_csv_custom(&self, separator: &str, include_header: bool) -> String {
        super::export::query_result_to_csv(self, separator, include_header)
    }

    /// Export result as JSON string
    ///
    /// ```no_run
    /// # use nopaldb::Graph;
    /// # async fn example() -> nopaldb::Result<()> {
    /// let graph = Graph::in_memory().await?;
    /// let result = graph.execute_nql("find p.name, p.age from (p:Person)").await?;
    /// let json = result.to_json();
    /// println!("{}", json);
    /// // Output: [{"p.name":"Alice","p.age":30},{"p.name":"Bob","p.age":25}]
    /// # Ok(())
    /// # }
    /// ```
    pub fn to_json(&self) -> String {
        super::export::query_result_to_json(self, false)
    }

    /// Export result as pretty-printed JSON string
    pub fn to_json_pretty(&self) -> String {
        super::export::query_result_to_json(self, true)
    }
}

impl Row {
    /// Create new row
    pub fn new() -> Self {
        Row {
            values: HashMap::new(),
        }
    }

    /// Set value
    pub fn set(&mut self, key: impl Into<String>, value: PropertyValue) {
        self.values.insert(key.into(), value);
    }

    /// Get value
    pub fn get(&self, key: &str) -> Option<&PropertyValue> {
        self.values.get(key)
    }

    /// Get as string
    pub fn get_string(&self, key: &str) -> Option<String> {
        match self.get(key)? {
            PropertyValue::String(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// Get as int
    pub fn get_int(&self, key: &str) -> Option<i64> {
        match self.get(key)? {
            PropertyValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    /// Get all keys in the row
    pub fn keys(&self) -> Vec<&String> {
        self.values.keys().collect()
    }
}

impl Default for Row {
    fn default() -> Self {
        Self::new()
    }
}

// Implement indexing for convenience (returns Null for missing keys instead of panicking)
impl std::ops::Index<&str> for Row {
    type Output = PropertyValue;

    fn index(&self, key: &str) -> &Self::Output {
        static NULL: PropertyValue = PropertyValue::Null;
        self.values.get(key).unwrap_or(&NULL)
    }
}
