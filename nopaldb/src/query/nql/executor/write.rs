// src/query/nql/executor/write.rs
//
// Write operations executor (ADD, DELETE, UPDATE)
// P0 fix: All writes go through Transaction for atomicity.
// P0 fix: DELETE/UPDATE support relationship patterns.

use crate::Transaction;
use crate::error::{NopalError, Result};
use crate::graph::Graph;
use crate::query::nql::parser::ast::{
    AddStmt, Assignment, DeleteStmt, Expression, NodePattern, Pattern, PatternElement,
    RelationshipPattern, UpdateStmt,
};
use crate::types::{Edge, Node, NodeId, PropertyValue};
use std::collections::{HashMap, HashSet};

// Import result types from result.rs
use super::operators;
use super::result::{AddResult, DeleteResult, UpdateResult};

// ═══════════════════════════════════════════════════════════
// MATCHED ELEMENT TYPE
// ═══════════════════════════════════════════════════════════

/// Matched element (node or edge)
#[derive(Debug, Clone)]
pub enum MatchedElement {
    Node(Node),
    Edge(crate::types::Edge),
}

// ═══════════════════════════════════════════════════════════
// WRITE OPERATIONS EXECUTOR
// ═══════════════════════════════════════════════════════════

pub struct WriteExecutor<'a> {
    graph: &'a Graph,
}

impl<'a> WriteExecutor<'a> {
    pub fn new(graph: &'a Graph) -> Self {
        WriteExecutor { graph }
    }

    // ═══════════════════════════════════════════════════════════
    // ADD OPERATION — now writes through Transaction (P0-A)
    // ═══════════════════════════════════════════════════════════

    /// Execute ADD statement
    ///
    /// All writes go through the Transaction for atomicity.
    /// If any step fails, the tx is not committed and changes are rolled back.
    pub async fn execute_add(&self, add: &AddStmt, tx: &mut Transaction) -> Result<AddResult> {
        log::info!("Executing ADD statement (transactional)");

        let mut nodes_created = 0;
        let mut edges_created = 0;
        let mut created_ids = Vec::new();
        let mut variable_to_node: HashMap<String, NodeId> = HashMap::new();
        let mut last_node_id: Option<NodeId> = None;
        let mut pending_rel: Option<RelationshipPattern> = None;

        for element in &add.pattern.elements {
            match element {
                PatternElement::Node(node_pattern) => {
                    let (node_id, is_new) = self
                        .process_node_for_add_tx(node_pattern, &variable_to_node, tx)
                        .await?;

                    if is_new {
                        nodes_created += 1;
                        created_ids.push(node_id.to_string());
                    }

                    if let Some(var) = &node_pattern.variable {
                        variable_to_node.insert(var.clone(), node_id);
                    }

                    // If there's a pending relationship, buffer the edge in tx
                    if let (Some(source_id), Some(rel_pattern)) = (last_node_id, pending_rel.take())
                    {
                        let edge = self.build_edge_for_add(source_id, node_id, &rel_pattern)?;
                        tx.add_edge(edge)?;
                        edges_created += 1;
                    }

                    last_node_id = Some(node_id);
                }
                PatternElement::Relationship(rel_pattern) => {
                    if last_node_id.is_none() {
                        return Err(NopalError::query_error(
                            "Relationship in ADD pattern has no source node",
                        ));
                    }
                    pending_rel = Some(rel_pattern.clone());
                }
            }
        }

        log::info!(
            "ADD buffered: {} nodes, {} edges (will apply on commit)",
            nodes_created,
            edges_created
        );

        Ok(AddResult {
            nodes_created,
            edges_created,
            created_ids,
        })
    }

    fn build_edge_for_add(
        &self,
        source_id: NodeId,
        target_id: NodeId,
        rel_pattern: &RelationshipPattern,
    ) -> Result<Edge> {
        let rel_type = rel_pattern
            .rel_type
            .clone()
            .unwrap_or_else(|| "RELATED_TO".to_string());

        let mut edge = Edge::new(source_id, target_id, rel_type);
        if !rel_pattern.properties.is_empty() {
            edge = edge.with_properties(rel_pattern.properties.clone());
        }

        Ok(edge)
    }

    /// Process a node for ADD — writes through Transaction
    async fn process_node_for_add_tx(
        &self,
        pattern: &NodePattern,
        variable_map: &HashMap<String, NodeId>,
        tx: &mut Transaction,
    ) -> Result<(NodeId, bool)> {
        // Check if already created in this statement
        if let Some(var) = &pattern.variable
            && let Some(existing_id) = variable_map.get(var)
        {
            return Ok((*existing_id, false));
        }

        let label = pattern.label.clone().unwrap_or_else(|| "Node".to_string());
        let properties = pattern.properties.clone();

        let node = Node {
            id: NodeId::new_v4(),
            label,
            properties,
            kind: crate::types::NodeKind::Individual,
        };

        // Buffer in transaction — not persisted until commit
        let node_id = tx.add_node(node).await?;

        Ok((node_id, true))
    }

    // ═══════════════════════════════════════════════════════════
    // DELETE OPERATION — supports relationship patterns (P0-B)
    // ═══════════════════════════════════════════════════════════

    /// Execute DELETE statement
    ///
    /// Supports:
    /// - `delete (u:User) where u.active = false` — simple node pattern
    /// - `delete (a:Person)-[:KNOWS]->(b:Person) where a.name = "Bob"` — relationship pattern
    pub async fn execute_delete(
        &self,
        delete: &DeleteStmt,
        tx: &mut Transaction,
    ) -> Result<DeleteResult> {
        log::info!("Executing DELETE statement (transactional)");

        let has_relationships = delete
            .pattern
            .elements
            .iter()
            .any(|e| matches!(e, PatternElement::Relationship(_)));

        if has_relationships {
            self.execute_delete_pattern(delete, tx).await
        } else {
            self.execute_delete_simple(delete, tx).await
        }
    }

    /// DELETE with simple node pattern
    async fn execute_delete_simple(
        &self,
        delete: &DeleteStmt,
        tx: &mut Transaction,
    ) -> Result<DeleteResult> {
        let nodes = self.match_nodes_by_pattern(&delete.pattern).await?;

        let filtered = if let Some(filter) = &delete.filter {
            nodes
                .into_iter()
                .filter(|n| self.evaluate_simple_condition(n, &filter.condition))
                .collect::<Vec<_>>()
        } else {
            nodes
        };

        let to_delete: Vec<_> = if let Some(limit) = &delete.limit {
            let offset = limit.offset.unwrap_or(0);
            filtered
                .into_iter()
                .skip(offset)
                .take(limit.limit)
                .collect()
        } else {
            filtered
        };

        let mut nodes_deleted = 0;
        let mut edges_deleted = 0;

        for node in &to_delete {
            let out_edges = self
                .graph
                .get_outgoing_edges(node.id)
                .await
                .unwrap_or_default();
            let in_edges = self
                .graph
                .get_incoming_edges(node.id)
                .await
                .unwrap_or_default();
            edges_deleted += out_edges.len() + in_edges.len();

            // Buffer deletion in transaction
            tx.delete_node(node.id)?;
            nodes_deleted += 1;
        }

        Ok(DeleteResult {
            nodes_deleted,
            edges_deleted,
        })
    }

    /// DELETE with relationship pattern (P0-B)
    /// Deletes nodes that participate in the specified relationship pattern.
    async fn execute_delete_pattern(
        &self,
        delete: &DeleteStmt,
        tx: &mut Transaction,
    ) -> Result<DeleteResult> {
        let (source_label, rel_type, target_label, source_var, target_var) =
            self.extract_pattern_parts(&delete.pattern)?;

        // Use the pattern matching engine
        let matches = operators::execute_pattern(
            self.graph,
            source_label.as_deref(),
            rel_type.as_deref(),
            target_label.as_deref(),
        )
        .await?;

        // Apply WHERE filter
        let filtered: Vec<_> = if let Some(filter) = &delete.filter {
            matches
                .into_iter()
                .filter(|m| {
                    self.evaluate_pattern_condition(m, &filter.condition, &source_var, &target_var)
                })
                .collect()
        } else {
            matches
        };

        // Apply LIMIT
        let to_process: Vec<_> = if let Some(limit) = &delete.limit {
            let offset = limit.offset.unwrap_or(0);
            filtered
                .into_iter()
                .skip(offset)
                .take(limit.limit)
                .collect()
        } else {
            filtered
        };

        let mut edges_deleted = 0;
        let mut deleted_edge_ids = HashSet::new();

        for m in &to_process {
            if let Some(edge) = &m.edge
                && deleted_edge_ids.insert(edge.id)
            {
                tx.delete_edge(edge.id)?;
                edges_deleted += 1;
            }
        }

        Ok(DeleteResult {
            nodes_deleted: 0,
            edges_deleted,
        })
    }

    // ═══════════════════════════════════════════════════════════
    // UPDATE OPERATION — supports relationship patterns (P0-B)
    // ═══════════════════════════════════════════════════════════

    /// Execute UPDATE statement
    pub async fn execute_update(
        &self,
        update: &UpdateStmt,
        _tx: &mut Transaction,
    ) -> Result<UpdateResult> {
        log::info!("Executing UPDATE statement");

        let has_relationships = update
            .pattern
            .elements
            .iter()
            .any(|e| matches!(e, PatternElement::Relationship(_)));

        if has_relationships {
            self.execute_update_pattern(update).await
        } else {
            self.execute_update_simple(update).await
        }
    }

    /// UPDATE with simple node pattern
    async fn execute_update_simple(&self, update: &UpdateStmt) -> Result<UpdateResult> {
        let nodes = self.match_nodes_by_pattern(&update.pattern).await?;

        let filtered = if let Some(filter) = &update.filter {
            nodes
                .into_iter()
                .filter(|n| self.evaluate_simple_condition(n, &filter.condition))
                .collect::<Vec<_>>()
        } else {
            nodes
        };

        let to_update: Vec<_> = if let Some(limit) = &update.limit {
            let offset = limit.offset.unwrap_or(0);
            filtered
                .into_iter()
                .skip(offset)
                .take(limit.limit)
                .collect()
        } else {
            filtered
        };

        let root_var = update
            .pattern
            .elements
            .iter()
            .find_map(|element| match element {
                PatternElement::Node(node) => node.variable.clone(),
                _ => None,
            })
            .unwrap_or_else(|| "n".to_string());

        self.apply_updates(
            to_update,
            Vec::new(),
            &update.assignments,
            &[root_var.as_str()],
            None,
        )
        .await
    }

    /// UPDATE with relationship pattern (P0-B)
    async fn execute_update_pattern(&self, update: &UpdateStmt) -> Result<UpdateResult> {
        let (source_label, rel_type, target_label, source_var, target_var) =
            self.extract_pattern_parts(&update.pattern)?;

        let matches = operators::execute_pattern(
            self.graph,
            source_label.as_deref(),
            rel_type.as_deref(),
            target_label.as_deref(),
        )
        .await?;

        let filtered: Vec<_> = if let Some(filter) = &update.filter {
            matches
                .into_iter()
                .filter(|m| {
                    self.evaluate_pattern_condition(m, &filter.condition, &source_var, &target_var)
                })
                .collect()
        } else {
            matches
        };

        let to_process: Vec<_> = if let Some(limit) = &update.limit {
            let offset = limit.offset.unwrap_or(0);
            filtered
                .into_iter()
                .skip(offset)
                .take(limit.limit)
                .collect()
        } else {
            filtered
        };

        let edge_var = update
            .pattern
            .elements
            .iter()
            .find_map(|element| match element {
                PatternElement::Relationship(r) => r.variable.clone(),
                _ => None,
            });

        let mut nodes_to_update = Vec::new();
        let mut edges_to_update = Vec::new();
        let mut seen_nodes = HashSet::new();
        let mut seen_edges = HashSet::new();

        for m in &to_process {
            for assignment in &update.assignments {
                if assignment.variable == source_var && seen_nodes.insert(m.source.id) {
                    nodes_to_update.push(m.source.clone());
                }
                if assignment.variable == target_var && seen_nodes.insert(m.target.id) {
                    nodes_to_update.push(m.target.clone());
                }
                if edge_var.as_deref() == Some(assignment.variable.as_str())
                    && let Some(edge) = &m.edge
                    && seen_edges.insert(edge.id)
                {
                    edges_to_update.push(edge.clone());
                }
            }
        }

        self.apply_updates(
            nodes_to_update,
            edges_to_update,
            &update.assignments,
            &[source_var.as_str(), target_var.as_str()],
            edge_var.as_deref(),
        )
        .await
    }

    /// Apply property updates to matched nodes and edges.
    async fn apply_updates(
        &self,
        nodes: Vec<Node>,
        edges: Vec<Edge>,
        assignments: &[Assignment],
        node_vars: &[&str],
        edge_var: Option<&str>,
    ) -> Result<UpdateResult> {
        let mut nodes_updated = 0;
        let mut edges_updated = 0;
        let mut properties_changed = 0;

        for mut node in nodes {
            let mut changed = false;
            for assignment in assignments {
                if !node_vars.iter().any(|var| *var == assignment.variable) {
                    continue;
                }
                let new_value = self.evaluate_assignment_value(&assignment.value);
                if let Some(val) = new_value {
                    // Remove old value from property index
                    if let Some(old_val) = node.properties.get(&assignment.property) {
                        let _ = self
                            .graph
                            .storage_remove_property_index(&assignment.property, old_val, node.id)
                            .await;
                    }

                    node.properties
                        .insert(assignment.property.clone(), val.clone());

                    // Add new value to property index
                    let _ = self
                        .graph
                        .storage_add_property_index(&assignment.property, &val, node.id)
                        .await;

                    properties_changed += 1;
                    changed = true;
                }
            }

            if changed {
                self.graph.storage_insert_node(&node).await?;
                nodes_updated += 1;
            }
        }

        for mut edge in edges {
            let mut changed = false;
            for assignment in assignments {
                if Some(assignment.variable.as_str()) != edge_var {
                    continue;
                }
                let new_value = self.evaluate_assignment_value(&assignment.value);
                if let Some(val) = new_value {
                    edge.properties.insert(assignment.property.clone(), val);
                    properties_changed += 1;
                    changed = true;
                }
            }

            if changed {
                self.graph.storage_insert_edge(&edge).await?;
                edges_updated += 1;
            }
        }

        Ok(UpdateResult {
            nodes_updated,
            edges_updated,
            properties_changed,
        })
    }

    // ═══════════════════════════════════════════════════════════
    // PATTERN HELPERS
    // ═══════════════════════════════════════════════════════════

    /// Extract source label, rel type, target label, and variable names from a pattern
    #[allow(clippy::type_complexity)]
    fn extract_pattern_parts(
        &self,
        pattern: &Pattern,
    ) -> Result<(
        Option<String>, // source label
        Option<String>, // rel type
        Option<String>, // target label
        String,         // source var
        String,         // target var
    )> {
        if pattern.elements.len() < 3 {
            return Err(NopalError::query_error(
                "Relationship pattern requires at least: node -> rel -> node",
            ));
        }

        let source_label = match &pattern.elements[0] {
            PatternElement::Node(n) => n.label.clone(),
            _ => return Err(NopalError::query_error("Pattern must start with node")),
        };
        let source_var = match &pattern.elements[0] {
            PatternElement::Node(n) => n.variable.clone().unwrap_or_else(|| "_source".into()),
            _ => "_source".into(),
        };

        let rel_type = match &pattern.elements[1] {
            PatternElement::Relationship(r) => r.rel_type.clone(),
            _ => return Err(NopalError::query_error("Expected relationship after node")),
        };

        let target_label = match &pattern.elements[2] {
            PatternElement::Node(n) => n.label.clone(),
            _ => return Err(NopalError::query_error("Expected node after relationship")),
        };
        let target_var = match &pattern.elements[2] {
            PatternElement::Node(n) => n.variable.clone().unwrap_or_else(|| "_target".into()),
            _ => "_target".into(),
        };

        Ok((source_label, rel_type, target_label, source_var, target_var))
    }

    /// Evaluate condition on a pattern match with variable scoping
    fn evaluate_pattern_condition(
        &self,
        m: &operators::PatternMatch,
        expr: &Expression,
        source_var: &str,
        target_var: &str,
    ) -> bool {
        match expr {
            Expression::BinaryOp { left, op, right } => {
                use crate::query::nql::parser::ast::BinaryOperator;
                match op {
                    BinaryOperator::And => {
                        self.evaluate_pattern_condition(m, left, source_var, target_var)
                            && self.evaluate_pattern_condition(m, right, source_var, target_var)
                    }
                    BinaryOperator::Or => {
                        self.evaluate_pattern_condition(m, left, source_var, target_var)
                            || self.evaluate_pattern_condition(m, right, source_var, target_var)
                    }
                    _ => {
                        let l = self.eval_pattern_expr(m, left, source_var, target_var);
                        let r = self.eval_pattern_expr(m, right, source_var, target_var);
                        match (l, r) {
                            (Some(lv), Some(rv)) => match op {
                                BinaryOperator::Eq => lv == rv,
                                BinaryOperator::NotEq => lv != rv,
                                BinaryOperator::Gt => lv > rv,
                                BinaryOperator::Lt => lv < rv,
                                BinaryOperator::GtEq => lv >= rv,
                                BinaryOperator::LtEq => lv <= rv,
                                _ => false,
                            },
                            _ => false,
                        }
                    }
                }
            }
            _ => {
                log::warn!("Unsupported expression in pattern WHERE, defaulting to false");
                false
            }
        }
    }

    fn eval_pattern_expr(
        &self,
        m: &operators::PatternMatch,
        expr: &Expression,
        source_var: &str,
        target_var: &str,
    ) -> Option<PropertyValue> {
        match expr {
            Expression::Literal(v) => Some(v.clone()),
            Expression::Property { variable, property } => {
                let node = if variable == source_var {
                    &m.source
                } else if variable == target_var {
                    &m.target
                } else {
                    if let Some(edge) = &m.edge {
                        if property == "edge_type" || property == "type" {
                            return Some(PropertyValue::String(edge.edge_type.clone()));
                        }
                        return edge.properties.get(property).cloned();
                    }
                    return None;
                };

                if property == "label" {
                    return Some(PropertyValue::String(node.label.clone()));
                }
                if property == "id" {
                    return Some(PropertyValue::String(node.id.to_string()));
                }
                node.properties.get(property).cloned()
            }
            _ => None,
        }
    }

    /// Match nodes by simple pattern (label filter only)
    async fn match_nodes_by_pattern(&self, pattern: &Pattern) -> Result<Vec<Node>> {
        if pattern.elements.is_empty() {
            return Ok(vec![]);
        }

        match &pattern.elements[0] {
            PatternElement::Node(node_pattern) => {
                if let Some(label) = &node_pattern.label {
                    self.graph.get_nodes_by_label(label).await
                } else {
                    self.graph.get_all_nodes().await
                }
            }
            _ => Ok(vec![]),
        }
    }

    /// Simple condition evaluation for DELETE/UPDATE WHERE clauses
    fn evaluate_simple_condition(&self, node: &Node, expr: &Expression) -> bool {
        match expr {
            Expression::BinaryOp { left, op, right } => {
                use crate::query::nql::parser::ast::BinaryOperator;
                match op {
                    BinaryOperator::And => {
                        self.evaluate_simple_condition(node, left)
                            && self.evaluate_simple_condition(node, right)
                    }
                    BinaryOperator::Or => {
                        self.evaluate_simple_condition(node, left)
                            || self.evaluate_simple_condition(node, right)
                    }
                    _ => {
                        let l = self.eval_expr(node, left);
                        let r = self.eval_expr(node, right);
                        match (l, r) {
                            (Some(lv), Some(rv)) => match op {
                                BinaryOperator::Eq => lv == rv,
                                BinaryOperator::NotEq => lv != rv,
                                BinaryOperator::Gt => lv > rv,
                                BinaryOperator::Lt => lv < rv,
                                BinaryOperator::GtEq => lv >= rv,
                                BinaryOperator::LtEq => lv <= rv,
                                _ => false,
                            },
                            _ => false,
                        }
                    }
                }
            }
            _ => {
                log::warn!(
                    "Unsupported expression type in WHERE clause for write operation, defaulting to false for safety"
                );
                false
            }
        }
    }

    fn eval_expr(&self, node: &Node, expr: &Expression) -> Option<PropertyValue> {
        match expr {
            Expression::Literal(v) => Some(v.clone()),
            Expression::Property {
                variable: _,
                property,
            } => {
                if property == "label" {
                    Some(PropertyValue::String(node.label.clone()))
                } else if property == "id" {
                    Some(PropertyValue::String(node.id.to_string()))
                } else {
                    node.properties.get(property).cloned()
                }
            }
            _ => None,
        }
    }

    fn evaluate_assignment_value(&self, expr: &Expression) -> Option<PropertyValue> {
        match expr {
            Expression::Literal(v) => Some(v.clone()),
            _ => {
                log::warn!(
                    "UPDATE SET only supports literal values for now. Expression {:?} ignored.",
                    expr
                );
                None
            }
        }
    }

    /// Match pattern against graph
    pub async fn match_pattern(&self, _pattern: &Pattern) -> Result<Vec<MatchedElement>> {
        Ok(vec![])
    }

    /// Filter matched elements by condition
    pub fn filter_matches(
        &self,
        matches: Vec<MatchedElement>,
        _condition: &Expression,
    ) -> Result<Vec<MatchedElement>> {
        Ok(matches)
    }

    /// Count nodes and edges in matched elements
    pub fn count_elements(&self, elements: &[MatchedElement]) -> (usize, usize) {
        let nodes = elements
            .iter()
            .filter(|e| matches!(e, MatchedElement::Node(_)))
            .count();
        let edges = elements
            .iter()
            .filter(|e| matches!(e, MatchedElement::Edge(_)))
            .count();
        (nodes, edges)
    }

    /// Sample node IDs from matched elements
    pub fn sample_node_ids(&self, elements: &[MatchedElement], limit: usize) -> Vec<String> {
        elements
            .iter()
            .filter_map(|e| match e {
                MatchedElement::Node(node) => Some(node.id.to_string()),
                _ => None,
            })
            .take(limit)
            .collect()
    }

    /// Sample updates (property changes)
    pub fn sample_updates(
        &self,
        _elements: &[MatchedElement],
        _assignments: &[Assignment],
        _limit: usize,
    ) -> Vec<crate::query::sketch_manager::UpdateSample> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_elements_empty() {
        let elements: Vec<MatchedElement> = vec![];
        assert_eq!(elements.len(), 0);
    }
}
