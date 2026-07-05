// - COMMIT: Operational space (execution)
use crate::error::{NopalError, Result};

// src/query/sketch_manager.rs
//
// Sketch Manager - Manages conceptual operations (SKETCH/COMMIT)
//
// Philosophy:
// Sketches separate "thinking" from "executing". They allow users to:
// 1. Define complex operations conceptually
// 2. Preview their impact before execution
// 3. Iterate safely without modifying the graph
// 4. Commit only when ready
//
// This implements the "Bow-Tie Cognitive Model":
// - SKETCH: Conceptual space (no persistence)
// - PREVIEW: Reasoning space (analysis)
use std::collections::HashMap;
use std::time::SystemTime;
use crate::query::nql::parser::ast::{Statement, DeleteStmt, UpdateStmt, AddStmt, Query};
use crate::query::nql::executor::{Executor, UpdateResult};
use crate::query::nql::executor::result::QueryResult;
use crate::Transaction;

// ═══════════════════════════════════════════════════════════════
// SKETCH MANAGER
// ═══════════════════════════════════════════════════════════════

/// Manages sketches (conceptual operations)
///
/// Sketches are named operations that can be:
/// - Previewed (dry-run to see impact)
/// - Committed (executed on the graph)
/// - Discarded (removed without execution)
///
/// Example workflow:
/// ```nql
/// -- Define sketch
/// sketch cleanup =
///   delete (u:User)
///   where u.last_login < timestamp("2020-01-01")
///
/// -- Preview impact
/// find count(*) from cleanup
///
/// -- Execute
/// commit cleanup
/// ```
pub struct SketchManager {
    /// Active sketches (name -> sketch)
    sketches: HashMap<String, Sketch>,
}

/// A sketch - conceptual operation
///
/// Sketches can be:
/// - Query (read-only, safe to execute)
/// - Delete (needs preview)
/// - Update (needs preview)
/// - Add (safe to execute)
#[derive(Debug, Clone)]
pub struct Sketch {
    /// Sketch name
    pub name: String,

    /// The statement to execute
    pub statement: Statement,

    /// When the sketch was created
    pub created_at: SystemTime,

    /// Optional description
    pub description: Option<String>,

    /// Cached preview result (for efficiency)
    preview_cache: Option<SketchPreview>,
}

/// Preview result for a sketch
///
/// Different operations have different preview outputs:
/// - Query: Shows the result (read-only)
/// - Delete: Shows count of nodes/edges that would be deleted
/// - Update: Shows count of nodes/edges that would be updated + sample
/// - Add: Shows count of nodes/edges that would be added
#[derive(Debug, Clone)]
pub struct SketchPreview {
    /// Preview type
    pub preview_type: PreviewType,

    /// When preview was generated
    pub generated_at: SystemTime,
}

#[derive(Debug, Clone)]
pub enum PreviewType {
    /// Query result (read-only)
    QueryResult(QueryResult),

    /// Delete preview
    DeletePreview {
        nodes_affected: usize,
        edges_affected: usize,
        sample_nodes: Vec<String>, // Sample node IDs
    },

    /// Update preview
    UpdatePreview {
        nodes_affected: usize,
        edges_affected: usize,
        sample_changes: Vec<UpdateSample>,
    },

    /// Add preview
    AddPreview {
        nodes_to_add: usize,
        edges_to_add: usize,
    },
    Other(String),
}

#[derive(Debug, Clone)]
pub struct UpdateSample {
    pub node_id: String,
    pub property: String,
    pub old_value: String,
    pub new_value: String,
}

// ═══════════════════════════════════════════════════════════════
// IMPLEMENTATION
// ═══════════════════════════════════════════════════════════════

impl SketchManager {
    /// Create a new sketch manager
    pub fn new() -> Self {
        Self {
            sketches: HashMap::new(),
        }
    }

    /// Define a new sketch
    ///
    /// If a sketch with the same name exists, it will be replaced.
    ///
    /// # Arguments
    /// * `name` - Unique name for the sketch
    /// * `statement` - The statement to execute when committed
    ///
    /// # Returns
    /// Ok(()) if sketch was defined successfully
    /// Err if sketch is invalid
    pub fn define(
        &mut self,
        name: String,
        statement: Statement,
        description: Option<String>,
    ) -> Result<()> {
        // Validate sketch
        self.validate_sketch(&statement)?;

        // Add sketch
        let sketch = Sketch {
            name: name.clone(),
            statement,
            created_at: SystemTime::now(),
            description,
            preview_cache: None,
        };

        // Store sketch
        self.sketches.insert(name, sketch);

        Ok(())
    }

    /// Preview a sketch (dry-run)
    ///
    /// Shows what would happen if the sketch were committed,
    /// without actually modifying the graph.
    ///
    /// # Arguments
    /// * `name` - Name of the sketch to preview
    /// * `executor` - Query executor (for executing queries)
    ///
    /// # Returns
    /// Preview result showing impact of the sketch
    pub async fn preview(
        &mut self,
        name: &str,
        executor: &mut Executor<'_>,
    ) -> Result<SketchPreview> {
        // Check cache first (immutable borrow)
        if let Some(sketch) = self.sketches.get(name)
            && let Some(cached) = &sketch.preview_cache {
                return Ok(cached.clone());
        }

        // Clone statement to avoid borrow conflicts
        let statement = self.sketches.get(name)
            .ok_or_else(|| NopalError::SketchNotFound(name.to_string()))?
            .statement
            .clone();

        // Generate preview based on statement type
        let preview = match &statement {
            Statement::Query(query) => {
                self.preview_query(query, executor).await?
            }
            Statement::Delete(delete) => {
                self.preview_delete(delete, executor).await?
            }
            Statement::Update(update) => {
                self.preview_update(update, executor).await?
            }
            Statement::Add(add) => {
                self.preview_add(add)?
            }
            Statement::CreateIndex(_) => {
                SketchPreview{
                    preview_type: PreviewType::Other("CREATE INDEX operation".to_string()),
                    generated_at: SystemTime::now(),
                }
            }
            Statement::DropIndex(_) => {
                SketchPreview {
                    preview_type: PreviewType::Other("DROP INDEX operation".to_string()),
                    generated_at: SystemTime::now(),
                }
            }
            Statement::Explain(_) => {
                SketchPreview {
                    preview_type: PreviewType::Other("EXPLAIN query plan".to_string()),
                    generated_at: SystemTime::now(),
                }
            }
            Statement::Profile(_) => {
                SketchPreview {
                    preview_type: PreviewType::Other("PROFILE query execution".to_string()),
                    generated_at: SystemTime::now(),
                }
            }
            Statement::Sketch(_) => {
                return Err(NopalError::InvalidSketch(
                    "Cannot sketch a sketch".to_string()
                ));
            }
            Statement::Commit(_) => {
                return Err(NopalError::InvalidSketch(
                    "Cannot sketch a commit".to_string()
                ));
            }
        };

        // Cache preview (mutable borrow after match)
        if let Some(sketch) = self.sketches.get_mut(name) {
            sketch.preview_cache = Some(preview.clone());
        }

        Ok(preview)
    }

    /// Commit a sketch (execute it)
    ///
    /// Executes the sketch and modifies the graph.
    /// The sketch is removed from the manager after commit.
    ///
    /// # Arguments
    /// * `name` - Name of the sketch to commit
    /// * `executor` - Query executor
    /// * `tx` - Transaction to execute within
    ///
    /// # Returns
    /// Result of the execution
    pub async fn commit(
        &mut self,
        name: &str,
        executor: &mut Executor<'_>,
        _tx: &mut Transaction,
    ) -> Result<CommitResult> {
        let sketch = self.sketches.remove(name)
            .ok_or_else(|| NopalError::SketchNotFound(name.to_string()))?;

        // Execute based on statement type
        let result = match sketch.statement {
            Statement::Query(query) => {
                let result = executor.execute(query.clone()).await?;
                CommitResult::Query(result)
            }
            Statement::Delete(_delete) => {
                // TODO: Implement execute_delete in executor
                return Err(NopalError::query_error("DELETE not yet implemented in executor"));
            }
            Statement::Update(_update) => {
                // TODO: Implement execute_update in executor
                return Err(NopalError::query_error("UPDATE not yet implemented in executor"));
            }
            Statement::CreateIndex(_) => {
                return Err(NopalError::custom("CreateIndex not supported in sketches yet"));
            }
            Statement::DropIndex(_) => {
                return Err(NopalError::custom("DropIndex not supported in sketches yet"));
            }
            Statement::Explain(_) => {
                return Err(NopalError::custom("Explain not supported in sketches yet"));
            }
            Statement::Profile(_) => {
                return Err(NopalError::custom("Profile not supported in sketches yet"));
            }
            Statement::Add(_add) => {
                // TODO: Implement execute_add in executor
                return Err(NopalError::query_error("ADD not yet implemented in executor"));
            }
            Statement::Sketch(_) | Statement::Commit(_) => {
                return Err(NopalError::InvalidCommit(
                    "Cannot commit a sketch or commit statement".to_string()
                ));
            }
        };

        Ok(result)
    }

    /// Discard a sketch without executing
    pub fn discard(&mut self, name: &str) -> Result<()> {
        self.sketches.remove(name)
            .ok_or_else(|| NopalError::SketchNotFound(name.to_string()))?;
        Ok(())
    }

    /// List all active sketches
    pub fn list(&self) -> Vec<&Sketch> {
        self.sketches.values().collect()
    }

    /// Get a sketch by name
    pub fn get(&self, name: &str) -> Option<&Sketch> {
        self.sketches.get(name)
    }

    /// Clear all sketches
    pub fn clear(&mut self) {
        self.sketches.clear();
    }
}

// ═══════════════════════════════════════════════════════════════
// PREVIEW IMPLEMENTATIONS
// ═══════════════════════════════════════════════════════════════

impl SketchManager {
    /// Preview a query (just execute it read-only)
    async fn preview_query(
        &self,
        query: &Query,
        executor: &mut Executor<'_>,
    ) -> Result<SketchPreview> {
        let result = executor.execute(query.clone()).await?;

        Ok(SketchPreview {
            preview_type: PreviewType::QueryResult(result),
            generated_at: SystemTime::now(),
        })
    }

    /// Preview a delete operation
    async fn preview_delete(
        &self,
        delete: &DeleteStmt,
        executor: &mut Executor<'_>,
    ) -> Result<SketchPreview> {
        // Match pattern
        let matches = executor.match_pattern(&delete.pattern).await?;

        // Apply WHERE filter
        let to_delete = if let Some(where_clause) = &delete.filter {
            executor.filter_matches(matches, &where_clause.condition)?
        } else {
            matches
        };

        // Apply LIMIT
        let to_delete: Vec<_> = if let Some(limit_clause) = &delete.limit {
            to_delete.into_iter().take(limit_clause.limit).collect()
        } else {
            to_delete
        };

        // Count and sample
        let (nodes_affected, edges_affected) = executor.count_elements(&to_delete);
        let sample_nodes = executor.sample_node_ids(&to_delete, 10);

        Ok(SketchPreview {
            preview_type: PreviewType::DeletePreview {
                nodes_affected,
                edges_affected,
                sample_nodes,
            },
            generated_at: SystemTime::now(),
        })
    }

    /// Preview an update operation
    async fn preview_update(
        &self,
        update: &UpdateStmt,
        executor: &mut Executor<'_>,
    ) -> Result<SketchPreview> {
        // Match pattern
        let matches = executor.match_pattern(&update.pattern).await?;

        // Apply WHERE filter
        let to_update = if let Some(where_clause) = &update.filter {
            executor.filter_matches(matches, &where_clause.condition)?
        } else {
            matches
        };

        // Apply LIMIT
        let to_update: Vec<_> = if let Some(limit_clause) = &update.limit {
            to_update.into_iter().take(limit_clause.limit).collect()
        } else {
            to_update
        };

        // Count and sample
        let (nodes_affected, edges_affected) = executor.count_elements(&to_update);
        let sample_changes = executor.sample_updates(
            &to_update,
            &update.assignments,
            10
        );

        Ok(SketchPreview {
            preview_type: PreviewType::UpdatePreview {
                nodes_affected,
                edges_affected,
                sample_changes,
            },
            generated_at: SystemTime::now(),
        })
    }

    /// Preview a add operation
    fn preview_add(&self, add: &AddStmt) -> Result<SketchPreview> {
        // Count nodes and edges in pattern
        let mut nodes_to_add = 0;
        let mut edges_to_add = 0;

        for element in &add.pattern.elements {
            match element {
                crate::query::nql::parser::ast::PatternElement::Node(_) => {
                    nodes_to_add += 1;
                }
                crate::query::nql::parser::ast::PatternElement::Relationship(_) => {
                    edges_to_add += 1;
                }
            }
        }

        Ok(SketchPreview {
            preview_type: PreviewType::AddPreview {
                nodes_to_add,
                edges_to_add,
            },
            generated_at: SystemTime::now(),
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// VALIDATION
// ═══════════════════════════════════════════════════════════════

impl SketchManager {
    /// Validate a sketch before defining it
    fn validate_sketch(&self, statement: &Statement) -> Result<()> {
        match statement {
            Statement::Query(_) => Ok(()),
            Statement::Delete(_) => Ok(()),
            Statement::Update(_) => Ok(()),
            Statement::CreateIndex(_) => Ok(()),
            Statement::DropIndex(_) => Ok(()),
            Statement::Explain(_) => Ok(()),
            Statement::Profile(_) => Ok(()),
            Statement::Add(_) => Ok(()),
            Statement::Sketch(_) => {
                Err(NopalError::InvalidSketch(
                    "Cannot sketch a sketch (no nested sketches)".to_string()
                ))
            }
            Statement::Commit(_) => {
                Err(NopalError::InvalidSketch(
                    "Cannot sketch a commit".to_string()
                ))
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// COMMIT RESULTS
// ═══════════════════════════════════════════════════════════════

/// Result of committing a sketch
#[derive(Debug)]
pub enum CommitResult {
    Query(QueryResult),
    Delete(DeleteResult),
    Update(UpdateResult),
    Add(AddResult),
}

#[derive(Debug)]
pub struct DeleteResult {
    pub nodes_deleted: usize,
    pub edges_deleted: usize,
}

#[derive(Debug)]
pub struct AddResult {
    pub nodes_added: usize,
    pub edges_added: usize,
}

// ═══════════════════════════════════════════════════════════════
// DISPLAY IMPLEMENTATIONS
// ═══════════════════════════════════════════════════════════════

impl std::fmt::Display for SketchPreview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.preview_type {
            PreviewType::QueryResult(result) => {
                write!(f, "Query result: {} rows", result.rows.len())
            }
            PreviewType::DeletePreview { nodes_affected, edges_affected, .. } => {
                write!(
                    f,
                    "Will delete: {} nodes, {} edges",
                    nodes_affected, edges_affected
                )
            }
            PreviewType::UpdatePreview { nodes_affected, edges_affected, .. } => {
                write!(
                    f,
                    "Will update: {} nodes, {} edges",
                    nodes_affected, edges_affected
                )
            }
            PreviewType::AddPreview { nodes_to_add, edges_to_add } => {
                write!(
                    f,
                    "Will add: {} nodes, {} edges",
                    nodes_to_add, edges_to_add
                )
            }
            //TODO: revisar esta parte
            PreviewType::Other(desc) => {
                write!(f, "Preview: {}", desc)
            }
        }
    }
}

impl Default for SketchManager {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════
// TESTS
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sketch_manager_basics() {
        let manager = SketchManager::new();

        // Initially empty
        assert_eq!(manager.list().len(), 0);

        // TODO: Add more tests when we have full Statement constructors
    }

    #[test]
    fn test_sketch_validation() {
        let _manager = SketchManager::new();

        // Nested sketches should be invalid
        // TODO: Test when we have Statement constructors
    }
}
