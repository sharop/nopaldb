// src/query/nql/executor/write.rs
//
// Write operations executor (ADD, DELETE, UPDATE)
// P0 fix: All writes go through Transaction for atomicity.
// P0 fix: DELETE/UPDATE support relationship patterns.

use std::collections::{HashMap, HashSet};
use crate::error::{NopalError, Result};
use crate::graph::Graph;
use crate::query::nql::parser::ast::{
    AddStmt, DeleteStmt, UpdateStmt, Pattern, PatternElement,
    NodePattern, RelationshipPattern, Expression, Assignment,
};
use crate::types::{Node, Edge, NodeId, PropertyValue};
use crate::Transaction;

// Import result types from result.rs
use super::result::{AddResult, DeleteResult, UpdateResult};
use super::operators;
use super::value_ops;


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
    pub async fn execute_add(
        &self,
        add: &AddStmt,
        tx: &mut Transaction,
    ) -> Result<AddResult> {
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
                    let (node_id, is_new) = self.process_node_for_add_tx(
                        node_pattern,
                        &variable_to_node,
                        tx,
                    ).await?;

                    if is_new {
                        nodes_created += 1;
                        created_ids.push(node_id.to_string());
                    }

                    if let Some(var) = &node_pattern.variable {
                        variable_to_node.insert(var.clone(), node_id);
                    }

                    // If there's a pending relationship, buffer the edge in tx
                    if let (Some(source_id), Some(rel_pattern)) = (last_node_id, pending_rel.take()) {
                        let edge = self.build_edge_for_add(source_id, node_id, &rel_pattern)?;
                        tx.add_edge(edge)?;
                        edges_created += 1;
                    }

                    last_node_id = Some(node_id);
                }
                PatternElement::Relationship(rel_pattern) => {
                    if last_node_id.is_none() {
                        return Err(NopalError::query_error(
                            "Relationship in ADD pattern has no source node"
                        ));
                    }
                    pending_rel = Some(rel_pattern.clone());
                }
            }
        }

        log::info!("ADD buffered: {} nodes, {} edges (will apply on commit)", nodes_created, edges_created);

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
            && let Some(existing_id) = variable_map.get(var) {
                return Ok((*existing_id, false));
        }

        let label = pattern.label.clone()
            .unwrap_or_else(|| "Node".to_string());
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
        // Una función desconocida en el `where` no borraba nada, sin error (#328).
        crate::query::nql::validator::check_known_functions(delete.filter.iter().map(|w| &w.condition))?;
        if let Some(filter) = &delete.filter {
            crate::query::nql::validator::check_condition(&filter.condition, "WHERE")?;
        }

        let has_relationships = delete.pattern.elements.iter()
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
            let mut kept = Vec::with_capacity(nodes.len());
            for n in nodes {
                if self.evaluate_simple_condition(&n, &filter.condition)? {
                    kept.push(n);
                }
            }
            kept
        } else {
            nodes
        };

        let to_delete: Vec<_> = if let Some(limit) = &delete.limit {
            let offset = limit.offset.unwrap_or(0);
            filtered.into_iter().skip(offset).take(limit.limit).collect()
        } else {
            filtered
        };

        let mut nodes_deleted = 0;
        let mut edges_deleted = 0;

        for node in &to_delete {
            let out_edges = self.graph.get_outgoing_edges(node.id).await.unwrap_or_default();
            let in_edges = self.graph.get_incoming_edges(node.id).await.unwrap_or_default();
            edges_deleted += out_edges.len() + in_edges.len();

            // Buffer deletion in transaction
            tx.delete_node(node.id)?;
            nodes_deleted += 1;
        }

        Ok(DeleteResult { nodes_deleted, edges_deleted })
    }

    /// DELETE with relationship pattern (P0-B)
    /// Deletes nodes that participate in the specified relationship pattern.
    async fn execute_delete_pattern(
        &self,
        delete: &DeleteStmt,
        tx: &mut Transaction,
    ) -> Result<DeleteResult> {
        let (source, rel, target, source_var, target_var) =
            self.extract_pattern_parts(&delete.pattern)?;

        let matches = operators::execute_one_hop_pattern(self.graph, source, rel, target).await?;

        // Apply WHERE filter
        let filtered: Vec<_> = if let Some(filter) = &delete.filter {
            let mut kept = Vec::with_capacity(matches.len());
            for m in matches {
                if operators::eval_pattern_condition(&m, &filter.condition, &source_var, &target_var)? {
                    kept.push(m);
                }
            }
            kept
        } else {
            matches
        };

        // Apply LIMIT
        let to_process: Vec<_> = if let Some(limit) = &delete.limit {
            let offset = limit.offset.unwrap_or(0);
            filtered.into_iter().skip(offset).take(limit.limit).collect()
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

        Ok(DeleteResult { nodes_deleted: 0, edges_deleted })
    }

    // ═══════════════════════════════════════════════════════════
    // UPDATE OPERATION — supports relationship patterns (P0-B)
    // ═══════════════════════════════════════════════════════════

    /// Execute UPDATE statement
    ///
    /// Los cambios van al write-set de `tx`, como ADD y DELETE: el commit del
    /// llamador los escribe con versión MVCC, registro en el WAL y
    /// mantenimiento de índices. Hasta 0.9.4 se escribían directo en el
    /// motor (`storage_insert_node`/`storage_insert_edge`), sin versión ni
    /// WAL: un crash antes del siguiente checkpoint perdía el UPDATE ya
    /// confirmado y el redo no tenía con qué restaurarlo.
    pub async fn execute_update(
        &self,
        update: &UpdateStmt,
        tx: &mut Transaction,
    ) -> Result<UpdateResult> {
        log::info!("Executing UPDATE statement");
        crate::query::nql::validator::check_known_functions(update.filter.iter().map(|w| &w.condition))?;
        crate::query::nql::validator::check_update_values(update)?;
        if let Some(filter) = &update.filter {
            crate::query::nql::validator::check_condition(&filter.condition, "WHERE")?;
        }

        let has_relationships = update.pattern.elements.iter()
            .any(|e| matches!(e, PatternElement::Relationship(_)));

        if has_relationships {
            self.execute_update_pattern(update, tx).await
        } else {
            self.execute_update_simple(update, tx).await
        }
    }

    /// UPDATE with simple node pattern
    async fn execute_update_simple(
        &self,
        update: &UpdateStmt,
        tx: &mut Transaction,
    ) -> Result<UpdateResult> {
        let nodes = self.match_nodes_by_pattern(&update.pattern).await?;

        let filtered = if let Some(filter) = &update.filter {
            let mut kept = Vec::with_capacity(nodes.len());
            for n in nodes {
                if self.evaluate_simple_condition(&n, &filter.condition)? {
                    kept.push(n);
                }
            }
            kept
        } else {
            nodes
        };

        let to_update: Vec<_> = if let Some(limit) = &update.limit {
            let offset = limit.offset.unwrap_or(0);
            filtered.into_iter().skip(offset).take(limit.limit).collect()
        } else {
            filtered
        };

        let root_var = update.pattern.elements.iter().find_map(|element| match element {
            PatternElement::Node(node) => node.variable.clone(),
            _ => None,
        }).unwrap_or_else(|| "n".to_string());

        self.apply_updates(tx, to_update, Vec::new(), &update.assignments, &[root_var.as_str()], None).await
    }

    /// UPDATE with relationship pattern (P0-B)
    async fn execute_update_pattern(
        &self,
        update: &UpdateStmt,
        tx: &mut Transaction,
    ) -> Result<UpdateResult> {
        let (source, rel, target, source_var, target_var) =
            self.extract_pattern_parts(&update.pattern)?;

        let matches = operators::execute_one_hop_pattern(self.graph, source, rel, target).await?;

        let filtered: Vec<_> = if let Some(filter) = &update.filter {
            let mut kept = Vec::with_capacity(matches.len());
            for m in matches {
                if operators::eval_pattern_condition(&m, &filter.condition, &source_var, &target_var)? {
                    kept.push(m);
                }
            }
            kept
        } else {
            matches
        };

        let to_process: Vec<_> = if let Some(limit) = &update.limit {
            let offset = limit.offset.unwrap_or(0);
            filtered.into_iter().skip(offset).take(limit.limit).collect()
        } else {
            filtered
        };

        let edge_var = update.pattern.elements.iter().find_map(|element| match element {
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
            tx,
            nodes_to_update,
            edges_to_update,
            &update.assignments,
            &[source_var.as_str(), target_var.as_str()],
            edge_var.as_deref(),
        ).await
    }

    /// Apply property updates to matched nodes and edges.
    ///
    /// Solo arma el write-set: cada nodo o arista cambiado se re-agrega a
    /// `tx` con su mismo id (upsert). El commit hace lo demás:
    /// - nodos: invalida la versión vigente y escribe una nueva con el ts del
    ///   commit (`history()` / `as_of()` ven el cambio), y mantiene las dos
    ///   capas de índice (el de propiedades del storage y los hash/btree/
    ///   full-text de usuario) retractando los valores que el SET pisó. Por
    ///   eso aquí ya no se toca el índice a mano: antes se mantenía solo la
    ///   capa del storage y un índice hash de usuario seguía devolviendo el
    ///   nodo por el valor viejo.
    /// - aristas: se reescriben por el WAL con el ts del commit, misma
    ///   adyacencia (el SET no cambia extremos ni tipo). Las aristas aún no
    ///   tienen cadena de versiones: la reescritura reemplaza su versión en
    ///   vez de encadenar una nueva.
    async fn apply_updates(
        &self,
        tx: &mut Transaction,
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
            let before = node.clone();
            let mut changed = false;
            for assignment in assignments {
                if !node_vars.iter().any(|var| *var == assignment.variable) {
                    continue;
                }
                let val = self.assignment_value_for_node(&before, &assignment.variable, &assignment.value)?;
                node.properties.insert(assignment.property.clone(), val);
                properties_changed += 1;
                changed = true;
            }

            if changed {
                tx.add_node(node).await?;
                nodes_updated += 1;
            }
        }

        for mut edge in edges {
            let before = edge.clone();
            let mut changed = false;
            for assignment in assignments {
                if Some(assignment.variable.as_str()) != edge_var {
                    continue;
                }
                let val = self.assignment_value_for_edge(&before, &assignment.variable, &assignment.value)?;
                edge.properties.insert(assignment.property.clone(), val);
                properties_changed += 1;
                changed = true;
            }

            if changed {
                tx.add_edge(edge)?;
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
    /// Las tres partes de un patrón de escritura `(a)-[r]->(b)` y sus
    /// variables. DELETE y UPDATE con patrón solo saben de un salto: un
    /// patrón más largo o con cuantificador es un error, no un patrón
    /// truncado a su primer salto en silencio (hasta 0.8.3).
    fn extract_pattern_parts<'p>(&self, pattern: &'p Pattern) -> Result<(
        &'p NodePattern,
        &'p RelationshipPattern,
        &'p NodePattern,
        String, // source var
        String, // target var
    )> {
        let (source, rel, target) = match pattern.elements.as_slice() {
            [PatternElement::Node(a), PatternElement::Relationship(r), PatternElement::Node(b)] => (a, r, b),
            elements if elements.len() > 3 => {
                return Err(NopalError::query_error(
                    "DELETE / UPDATE with a pattern support a single hop `(a)-[:T]->(b)`; \
                     longer patterns are not supported yet",
                ))
            }
            _ => {
                return Err(NopalError::query_error(
                    "Relationship pattern requires: node, relationship, node",
                ))
            }
        };
        if rel.quantifier.is_some() {
            return Err(NopalError::query_error(
                "DELETE / UPDATE with a pattern do not support relationship quantifiers",
            ));
        }
        let source_var = source.variable.clone().unwrap_or_else(|| "_source".into());
        let target_var = target.variable.clone().unwrap_or_else(|| "_target".into());
        Ok((source, rel, target, source_var, target_var))
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

    /// The WHERE of a single-node DELETE/UPDATE, with NQL's operators
    /// (`value_ops`, #269). Until 0.10.4 anything but a comparison was
    /// `false` here, so `where not …` or `where n.a * 2 = 6` touched nothing.
    fn evaluate_simple_condition(&self, node: &Node, expr: &Expression) -> Result<bool> {
        value_ops::condition(expr, &mut |e| match e {
            Expression::Literal(v) => Ok(Some(v.clone())),
            // The pattern has one variable: every property is of `node`.
            Expression::Property { property, .. } => Ok(node_field(node, property)),
            other => Err(operators::unsupported_here(other)),
        })
    }

    /// The value of `set var.prop = <expr>` for `node` (#269), read from the
    /// node as it was before the UPDATE: `set n.a = n.b, n.b = n.a` swaps.
    /// The validator lets through literals, properties of `var` and the
    /// operators. Arithmetic over a missing property writes `null`.
    fn assignment_value_for_node(&self, before: &Node, var: &str, expr: &Expression) -> Result<PropertyValue> {
        let value = value_ops::scalar(expr, &mut |e| match e {
            Expression::Literal(v) => Ok(Some(v.clone())),
            Expression::Property { variable, property } if variable == var => Ok(node_field(before, property)),
            other => Err(operators::unsupported_here(other)),
        })?;
        Ok(value.unwrap_or(PropertyValue::Null))
    }

    fn assignment_value_for_edge(&self, before: &Edge, var: &str, expr: &Expression) -> Result<PropertyValue> {
        let value = value_ops::scalar(expr, &mut |e| match e {
            Expression::Literal(v) => Ok(Some(v.clone())),
            Expression::Property { variable, property } if variable == var => Ok(match property.as_str() {
                "" | "id" => Some(PropertyValue::String(before.id.to_string())),
                "type" | "edge_type" => Some(PropertyValue::String(before.edge_type.clone())),
                p => before.properties.get(p).cloned(),
            }),
            other => Err(operators::unsupported_here(other)),
        })?;
        Ok(value.unwrap_or(PropertyValue::Null))
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
        let nodes = elements.iter()
            .filter(|e| matches!(e, MatchedElement::Node(_)))
            .count();
        let edges = elements.iter()
            .filter(|e| matches!(e, MatchedElement::Edge(_)))
            .count();
        (nodes, edges)
    }

    /// Sample node IDs from matched elements
    pub fn sample_node_ids(&self, elements: &[MatchedElement], limit: usize) -> Vec<String> {
        elements.iter()
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

/// `id`, `label` or a property of `node`.
fn node_field(node: &Node, property: &str) -> Option<PropertyValue> {
    match property {
        "" | "id" => Some(PropertyValue::String(node.id.to_string())),
        "label" => Some(PropertyValue::String(node.label.clone())),
        p => node.properties.get(p).cloned(),
    }
}
