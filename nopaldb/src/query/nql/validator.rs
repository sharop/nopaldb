// src/query/nql/validator.rs
//
// Semantic Validator for NQL
//
// Validates semantic rules that go beyond syntax:
// - HAVING requires GROUP BY
// - HAVING can only use aggregations or GROUP BY columns
// - Aggregation functions not allowed in WHERE
// - ORDER BY columns must be in SELECT (optional rule)
// - DELETE/UPDATE without WHERE warns about danger

use crate::error::{NopalError, Result};
use crate::query::nql::parser::ast::*;
use crate::types::PropertyValue;
use std::collections::HashSet;

// ═══════════════════════════════════════════════════════════════
// SEMANTIC VALIDATOR
// ═══════════════════════════════════════════════════════════════

/// Validates semantic rules for NQL statements
pub struct SemanticValidator {
    /// Strict mode (more validations)
    strict: bool,
}

impl SemanticValidator {
    /// Create a new validator
    pub fn new() -> Self {
        Self { strict: false }
    }

    /// Create a strict validator
    pub fn strict() -> Self {
        Self { strict: true }
    }

    /// Validate a statement
    pub fn validate(&self, statement: &Statement) -> Result<Vec<ValidationWarning>> {
        let mut warnings = Vec::new();

        match statement {
            Statement::Query(query) => {
                self.validate_query(query)?;
                warnings.extend(self.check_query_warnings(query));
            }
            Statement::Sketch(sketch) => {
                // Validate the inner statement
                let inner_warnings = self.validate(&sketch.operation)?;
                warnings.extend(inner_warnings);
            }
            Statement::Commit(_) => {
                // Commits are validated when the sketch is defined
            }
            Statement::Delete(delete) => {
                warnings.extend(self.check_delete_warnings(delete));
            }
            Statement::Update(update) => {
                self.validate_update(update)?;
                warnings.extend(self.check_update_warnings(update));
            }
            Statement::Add(create) => {
                self.validate_add(create)?;
            }
            Statement::CreateIndex(_) => {} // Always valid
            Statement::DropIndex(_) => {}   // Always valid
            Statement::Explain(inner) => {
                let inner_validator = SemanticValidator::new();
                inner_validator.validate(inner)?;
            }
            Statement::Profile(inner) => {
                let inner_validator = SemanticValidator::new();
                inner_validator.validate(inner)?;
            }
        }

        Ok(warnings)
    }
}

// ═══════════════════════════════════════════════════════════════
// QUERY VALIDATION
// ═══════════════════════════════════════════════════════════════

impl SemanticValidator {
    /// Validate a query
    fn validate_query(&self, query: &Query) -> Result<()> {
        // Rule 1: HAVING normalmente requiere GROUP BY, EXCEPTO cuando sólo
        // referencia funciones algorítmicas (degree, pagerank, etc.) que son
        // per-node y no necesitan agrupación.
        if let Some(having) = query.having.as_ref()
            && query.group_by.is_none()
            && !having_only_uses_algorithms(&having.condition)
        {
            return Err(NopalError::SemanticError(
                "HAVING clause requires GROUP BY".to_string(),
            ));
        }

        // Rule 2: HAVING puede usar:
        //  - funciones de agregación (count, sum, avg, min, max)
        //  - funciones algorítmicas (degree, pagerank, etc.) — per-node
        //  - columnas presentes en GROUP BY
        if let Some(having) = &query.having {
            let empty_group_by: Vec<Expression> = Vec::new();
            let group_by_exprs = query
                .group_by
                .as_ref()
                .map(|gb| &gb.expressions)
                .unwrap_or(&empty_group_by);

            self.validate_having_expression(&having.condition, group_by_exprs)?;
        }

        // Rule 3: WHERE no permite agregaciones (count/sum/avg/etc.).
        // Sí permite funciones algorítmicas (degree, pagerank, etc.) que son
        // per-node — son perfectamente válidas como predicados de filtrado.
        if let Some(where_clause) = &query.filter {
            self.validate_no_aggregations_in_where(&where_clause.condition)?;
        }

        // Rule 3b: INIT/GATHER only supported on a single pattern with relationships
        if (!query.init.is_empty() || !query.gather.is_empty())
            && !self.query_is_single_pattern_with_relationships(query)
        {
            return Err(NopalError::SemanticError(
                "INIT/GATHER are only supported for a single pattern with at least one relationship in Path Queries F4-B".to_string()
            ));
        }

        // Rule 3c: path_eval only in FIND and WHERE in F4-B
        if let Some(order_by) = &query.order_by {
            for item in &order_by.items {
                if expr_contains_function_validator(&item.expression, &["path_eval"]) {
                    return Err(NopalError::SemanticError(
                        "path_eval(\"...\") is not supported in ORDER BY in Path Queries F4-B"
                            .to_string(),
                    ));
                }
            }
        }

        if let Some(group_by) = &query.group_by {
            for expr in &group_by.expressions {
                if expr_contains_function_validator(expr, &["path_eval"]) {
                    return Err(NopalError::SemanticError(
                        "path_eval(\"...\") is not supported in GROUP BY in Path Queries F4-B"
                            .to_string(),
                    ));
                }
            }
        }

        if let Some(having) = &query.having
            && expr_contains_function_validator(&having.condition, &["path_eval"])
        {
            return Err(NopalError::SemanticError(
                "path_eval(\"...\") is not supported in HAVING in Path Queries F4-B".to_string(),
            ));
        }

        // Rule 4: If strict, ORDER BY columns must be in SELECT
        if self.strict
            && let Some(order_by) = &query.order_by
        {
            self.validate_order_by_in_select(order_by, &query.find)?;
        }

        // Rule 5: Path reducers not allowed in ORDER BY (F3 scope: FIND + WHERE only)
        if let Some(order_by) = &query.order_by {
            for item in &order_by.items {
                if expr_contains_path_reducer_validator(&item.expression) {
                    return Err(NopalError::SemanticError(
                        "Path reducers (path_sum, path_min, path_max, path_avg) are not supported in ORDER BY in Path Queries F3".to_string()
                    ));
                }
            }
        }

        // Rule 6: Path reducers not allowed in GROUP BY (F3 scope: FIND + WHERE only)
        if let Some(group_by) = &query.group_by {
            for expr in &group_by.expressions {
                if expr_contains_path_reducer_validator(expr) {
                    return Err(NopalError::SemanticError(
                        "Path reducers (path_sum, path_min, path_max, path_avg) are not supported in GROUP BY in Path Queries F3".to_string()
                    ));
                }
            }
        }

        // Rule 7: RETURN requires a single pattern with at least one relationship
        if query.return_expr.is_some() && !self.query_is_single_pattern_with_relationships(query) {
            return Err(NopalError::SemanticError(
                "RETURN requires a path pattern with at least one relationship in Path Queries F4-C".to_string()
            ));
        }

        // Rule 8: RETURN not allowed with ORDER BY, GROUP BY, or HAVING
        if query.return_expr.is_some() {
            if query.order_by.is_some() {
                return Err(NopalError::SemanticError(
                    "RETURN is not supported with ORDER BY in Path Queries F4-C".to_string(),
                ));
            }
            if query.group_by.is_some() {
                return Err(NopalError::SemanticError(
                    "RETURN is not supported with GROUP BY in Path Queries F4-C".to_string(),
                ));
            }
            if query.having.is_some() {
                return Err(NopalError::SemanticError(
                    "RETURN is not supported with HAVING in Path Queries F4-C".to_string(),
                ));
            }
        }

        // Rule 9: path.result requires a RETURN clause
        let uses_path_result = projections_use_path_property(&query.find.projections, "result")
            || query
                .filter
                .as_ref()
                .is_some_and(|f| expr_uses_path_property(&f.condition, "result"));
        if uses_path_result && query.return_expr.is_none() {
            return Err(NopalError::SemanticError(
                "path.result requires a RETURN clause in Path Queries F4-C".to_string(),
            ));
        }

        // Rule 10: path.start, path.end, path.state not allowed in WHERE
        if let Some(filter) = &query.filter {
            for prop in ["start", "end", "state"] {
                if expr_uses_path_property(&filter.condition, prop) {
                    return Err(NopalError::SemanticError(format!(
                        "path.{} is not allowed in WHERE in Path Queries F4-C; use it in FIND",
                        prop
                    )));
                }
            }
        }

        // Rule 11: path.result not allowed in ORDER BY or GROUP BY
        if let Some(order_by) = &query.order_by {
            for item in &order_by.items {
                if expr_uses_path_property(&item.expression, "result") {
                    return Err(NopalError::SemanticError(
                        "path.result is not supported in ORDER BY in Path Queries F4-C".to_string(),
                    ));
                }
            }
        }
        if let Some(group_by) = &query.group_by {
            for expr in &group_by.expressions {
                if expr_uses_path_property(expr, "result") {
                    return Err(NopalError::SemanticError(
                        "path.result is not supported in GROUP BY in Path Queries F4-C".to_string(),
                    ));
                }
            }
        }

        // Rule 12: semantic path filters are only supported in WHERE over a single path-aware pattern
        let uses_semantic_path_filters_in_where = query.filter.as_ref().is_some_and(|filter| {
            expr_contains_function_validator(&filter.condition, PATH_SEMANTIC_FILTERS)
        });
        if uses_semantic_path_filters_in_where
            && !self.query_is_single_pattern_with_relationships(query)
        {
            return Err(NopalError::SemanticError(
                "Semantic path filters are only supported for a single path pattern with at least one relationship in Path Queries F4-D.1".to_string()
            ));
        }

        for projection in &query.find.projections {
            if let Projection::Expression { expr, .. } = projection
                && expr_contains_function_validator(expr, PATH_SEMANTIC_FILTERS)
            {
                return Err(NopalError::SemanticError(
                    "Semantic path filters are only supported in WHERE in Path Queries F4-D.1"
                        .to_string(),
                ));
            }
        }

        if let Some(order_by) = &query.order_by {
            for item in &order_by.items {
                if expr_contains_function_validator(&item.expression, PATH_SEMANTIC_FILTERS) {
                    return Err(NopalError::SemanticError(
                        "Semantic path filters are not supported in ORDER BY in Path Queries F4-D.1".to_string()
                    ));
                }
            }
        }

        if let Some(group_by) = &query.group_by {
            for expr in &group_by.expressions {
                if expr_contains_function_validator(expr, PATH_SEMANTIC_FILTERS) {
                    return Err(NopalError::SemanticError(
                        "Semantic path filters are not supported in GROUP BY in Path Queries F4-D.1".to_string()
                    ));
                }
            }
        }

        if let Some(having) = &query.having
            && expr_contains_function_validator(&having.condition, PATH_SEMANTIC_FILTERS)
        {
            return Err(NopalError::SemanticError(
                "Semantic path filters are not supported in HAVING in Path Queries F4-D.1"
                    .to_string(),
            ));
        }

        for projection in &query.find.projections {
            if let Projection::Expression { expr, .. } = projection {
                self.validate_pattern_embedding_usage(expr, EmbeddingExprContext::Find, query)?;
                self.validate_path_embedding_usage(expr, EmbeddingExprContext::Find, query)?;
            }
        }
        if let Some(filter) = &query.filter {
            self.validate_pattern_embedding_usage(
                &filter.condition,
                EmbeddingExprContext::Where,
                query,
            )?;
            self.validate_path_embedding_usage(
                &filter.condition,
                EmbeddingExprContext::Where,
                query,
            )?;
        }
        if let Some(order_by) = &query.order_by {
            for item in &order_by.items {
                self.validate_pattern_embedding_usage(
                    &item.expression,
                    EmbeddingExprContext::OrderBy,
                    query,
                )?;
                self.validate_path_embedding_usage(
                    &item.expression,
                    EmbeddingExprContext::OrderBy,
                    query,
                )?;
            }
        }
        if let Some(group_by) = &query.group_by {
            for expr in &group_by.expressions {
                self.validate_pattern_embedding_usage(expr, EmbeddingExprContext::GroupBy, query)?;
                self.validate_path_embedding_usage(expr, EmbeddingExprContext::GroupBy, query)?;
            }
        }
        if let Some(having) = &query.having {
            self.validate_pattern_embedding_usage(
                &having.condition,
                EmbeddingExprContext::Having,
                query,
            )?;
            self.validate_path_embedding_usage(
                &having.condition,
                EmbeddingExprContext::Having,
                query,
            )?;
        }

        Ok(())
    }

    /// Validate HAVING expression
    ///
    /// HAVING can reference:
    /// 1. Aggregation functions (count, sum, avg, min, max)
    /// 2. Algorithm functions (degree, pagerank, betweenness, community, etc.)
    /// 3. Columns in GROUP BY
    fn validate_having_expression(
        &self,
        expr: &Expression,
        group_by_exprs: &[Expression],
    ) -> Result<()> {
        match expr {
            Expression::FunctionCall { name, args } => {
                // Aceptamos agregaciones y funciones algorítmicas.
                // Las algorítmicas son per-node y no requieren que sus
                // argumentos estén en GROUP BY (la pre-computación corre
                // sobre todo el grafo).
                let is_agg = is_aggregation_function(name);
                let is_algo = is_algorithm_function(name);
                if !is_agg && !is_algo {
                    return Err(NopalError::SemanticError(format!(
                        "HAVING can only use aggregation or algorithm functions, got: {}",
                        name
                    )));
                }

                // Para funciones algorítmicas, los argumentos son típicamente
                // referencias a nodos (variable-only, e.g. `degree(e)`); no
                // las validamos contra GROUP BY.
                if !is_algo {
                    for arg in args {
                        self.validate_having_expression(arg, group_by_exprs)?;
                    }
                }
            }
            Expression::Property { variable, property } => {
                // Variable-only (e.g. `degree(e)` parsed as Property{e, ""})
                // se permite — referencia al nodo, no a una columna.
                if property.is_empty() {
                    return Ok(());
                }
                // Otherwise must be in GROUP BY
                let prop_expr = Expression::property_access(variable, property);
                if !group_by_exprs.contains(&prop_expr) {
                    return Err(NopalError::SemanticError(format!(
                        "Column {}.{} must appear in GROUP BY to use in HAVING",
                        variable, property
                    )));
                }
            }
            Expression::BinaryOp { left, right, .. } => {
                self.validate_having_expression(left, group_by_exprs)?;
                self.validate_having_expression(right, group_by_exprs)?;
            }
            Expression::UnaryOp { expr, .. } => {
                self.validate_having_expression(expr, group_by_exprs)?;
            }
            Expression::Literal(_) | Expression::Wildcard => {
                // Literals and wildcards are OK
            }
        }

        Ok(())
    }

    /// Validate that WHERE doesn't use aggregations
    fn validate_no_aggregations_in_where(&self, expr: &Expression) -> Result<()> {
        if expr.is_aggregation() {
            return Err(NopalError::SemanticError(
                "Aggregation functions not allowed in WHERE clause. Use HAVING instead."
                    .to_string(),
            ));
        }

        // Recursively check nested expressions
        match expr {
            Expression::BinaryOp { left, right, .. } => {
                self.validate_no_aggregations_in_where(left)?;
                self.validate_no_aggregations_in_where(right)?;
            }
            Expression::UnaryOp { expr, .. } => {
                self.validate_no_aggregations_in_where(expr)?;
            }
            Expression::FunctionCall { args, .. } => {
                for arg in args {
                    self.validate_no_aggregations_in_where(arg)?;
                }
            }
            _ => {}
        }

        Ok(())
    }

    fn query_is_single_pattern_with_relationships(&self, query: &Query) -> bool {
        query.from.patterns.len() == 1
            && query.from.patterns[0]
                .elements
                .iter()
                .any(|e| matches!(e, PatternElement::Relationship(_)))
    }

    fn validate_pattern_embedding_usage(
        &self,
        expr: &Expression,
        context: EmbeddingExprContext,
        query: &Query,
    ) -> Result<()> {
        match expr {
            Expression::FunctionCall { name, args } => {
                match name.to_lowercase().as_str() {
                    "pattern_has_embeddings" => {
                        if !self.query_is_single_pattern_with_relationships(query) {
                            return Err(NopalError::SemanticError(
                                "Pattern embedding functions are only supported for a single path pattern with at least one relationship in PatternEmbedding E-3".to_string()
                            ));
                        }
                        match context {
                            EmbeddingExprContext::Find | EmbeddingExprContext::Where => {}
                            EmbeddingExprContext::OrderBy => {
                                return Err(NopalError::SemanticError(
                                    "pattern_has_embeddings(node_model, edge_model) is not supported in ORDER BY in PatternEmbedding E-3".to_string()
                                ));
                            }
                            EmbeddingExprContext::GroupBy => {
                                return Err(NopalError::SemanticError(
                                    "pattern_has_embeddings(node_model, edge_model) is not supported in GROUP BY in PatternEmbedding E-3".to_string()
                                ));
                            }
                            EmbeddingExprContext::Having => {
                                return Err(NopalError::SemanticError(
                                    "pattern_has_embeddings(node_model, edge_model) is not supported in HAVING in PatternEmbedding E-3".to_string()
                                ));
                            }
                        }

                        if args.len() == 1 {
                            return Err(NopalError::SemanticError(
                                "pattern_has_embeddings(\"model\") was replaced in PatternEmbedding E-3; use pattern_has_embeddings(node_model, edge_model)".to_string()
                            ));
                        }
                        if args.len() != 2
                            || !args.iter().all(|arg| {
                                matches!(arg, Expression::Literal(PropertyValue::String(_)))
                            })
                        {
                            return Err(NopalError::SemanticError(
                                "pattern_has_embeddings(node_model, edge_model) requires exactly 2 string literal model names in PatternEmbedding E-3".to_string()
                            ));
                        }
                    }
                    "pattern_embedding" => {
                        if !self.query_is_single_pattern_with_relationships(query) {
                            return Err(NopalError::SemanticError(
                                "Pattern embedding functions are only supported for a single path pattern with at least one relationship in PatternEmbedding E-3".to_string()
                            ));
                        }
                        match context {
                            EmbeddingExprContext::Find => {}
                            EmbeddingExprContext::Where => {
                                return Err(NopalError::SemanticError(
                                    "pattern_embedding(node_model, edge_model) is only supported in FIND in PatternEmbedding E-3".to_string()
                                ));
                            }
                            EmbeddingExprContext::OrderBy => {
                                return Err(NopalError::SemanticError(
                                    "pattern_embedding(node_model, edge_model) is not supported in ORDER BY in PatternEmbedding E-3".to_string()
                                ));
                            }
                            EmbeddingExprContext::GroupBy => {
                                return Err(NopalError::SemanticError(
                                    "pattern_embedding(node_model, edge_model) is not supported in GROUP BY in PatternEmbedding E-3".to_string()
                                ));
                            }
                            EmbeddingExprContext::Having => {
                                return Err(NopalError::SemanticError(
                                    "pattern_embedding(node_model, edge_model) is not supported in HAVING in PatternEmbedding E-3".to_string()
                                ));
                            }
                        }

                        if args.len() != 2
                            || !args.iter().all(|arg| {
                                matches!(arg, Expression::Literal(PropertyValue::String(_)))
                            })
                        {
                            return Err(NopalError::SemanticError(
                                "pattern_embedding(node_model, edge_model) requires exactly 2 string literal model names in PatternEmbedding E-3".to_string()
                            ));
                        }
                    }
                    "pattern_embedding_similarity" => {
                        return Err(NopalError::SemanticError(
                            "pattern_embedding_similarity(...) is no longer the official PatternEmbedding surface in E-3; use pattern_embedding(node_model, edge_model)".to_string()
                        ));
                    }
                    _ => {}
                }

                for arg in args {
                    self.validate_pattern_embedding_usage(arg, context, query)?;
                }
            }
            Expression::BinaryOp { left, right, .. } => {
                self.validate_pattern_embedding_usage(left, context, query)?;
                self.validate_pattern_embedding_usage(right, context, query)?;
            }
            Expression::UnaryOp { expr, .. } => {
                self.validate_pattern_embedding_usage(expr, context, query)?;
            }
            _ => {}
        }

        Ok(())
    }

    fn validate_path_embedding_usage(
        &self,
        expr: &Expression,
        context: EmbeddingExprContext,
        query: &Query,
    ) -> Result<()> {
        match expr {
            Expression::FunctionCall { name, args } => {
                match name.to_lowercase().as_str() {
                    "path_has_embeddings" => {
                        if !self.query_is_single_pattern_with_relationships(query) {
                            return Err(NopalError::SemanticError(
                                "Path embedding functions are only supported for a single path pattern with at least one relationship in PathEmbedding E-7".to_string()
                            ));
                        }
                        match context {
                            EmbeddingExprContext::Find | EmbeddingExprContext::Where => {}
                            EmbeddingExprContext::OrderBy => {
                                return Err(NopalError::SemanticError(
                                    "path_has_embeddings(node_model, edge_model) is not supported in ORDER BY in PathEmbedding E-7".to_string()
                                ));
                            }
                            EmbeddingExprContext::GroupBy => {
                                return Err(NopalError::SemanticError(
                                    "path_has_embeddings(node_model, edge_model) is not supported in GROUP BY in PathEmbedding E-7".to_string()
                                ));
                            }
                            EmbeddingExprContext::Having => {
                                return Err(NopalError::SemanticError(
                                    "path_has_embeddings(node_model, edge_model) is not supported in HAVING in PathEmbedding E-7".to_string()
                                ));
                            }
                        }

                        if args.len() == 1 {
                            return Err(NopalError::SemanticError(
                                "path_has_embeddings(\"model\") was replaced in PathEmbedding E-7; use path_has_embeddings(node_model, edge_model)".to_string()
                            ));
                        }
                        if args.len() != 2
                            || !args.iter().all(|arg| {
                                matches!(arg, Expression::Literal(PropertyValue::String(_)))
                            })
                        {
                            return Err(NopalError::SemanticError(
                                "path_has_embeddings(node_model, edge_model) requires exactly 2 string literal model names in PathEmbedding E-7".to_string()
                            ));
                        }
                    }
                    "path_embedding" => {
                        if !self.query_is_single_pattern_with_relationships(query) {
                            return Err(NopalError::SemanticError(
                                "Path embedding functions are only supported for a single path pattern with at least one relationship in PathEmbedding E-7".to_string()
                            ));
                        }
                        match context {
                            EmbeddingExprContext::Find => {}
                            EmbeddingExprContext::Where => {
                                return Err(NopalError::SemanticError(
                                    "path_embedding(node_model, edge_model) is only supported in FIND in PathEmbedding E-7".to_string()
                                ));
                            }
                            EmbeddingExprContext::OrderBy => {
                                return Err(NopalError::SemanticError(
                                    "path_embedding(node_model, edge_model) is not supported in ORDER BY in PathEmbedding E-7".to_string()
                                ));
                            }
                            EmbeddingExprContext::GroupBy => {
                                return Err(NopalError::SemanticError(
                                    "path_embedding(node_model, edge_model) is not supported in GROUP BY in PathEmbedding E-7".to_string()
                                ));
                            }
                            EmbeddingExprContext::Having => {
                                return Err(NopalError::SemanticError(
                                    "path_embedding(node_model, edge_model) is not supported in HAVING in PathEmbedding E-7".to_string()
                                ));
                            }
                        }

                        if args.len() != 2
                            || !args.iter().all(|arg| {
                                matches!(arg, Expression::Literal(PropertyValue::String(_)))
                            })
                        {
                            return Err(NopalError::SemanticError(
                                "path_embedding(node_model, edge_model) requires exactly 2 string literal model names in PathEmbedding E-7".to_string()
                            ));
                        }
                    }
                    "path_embedding_similarity" => {
                        // E-8: path_embedding_similarity(ref_name, node_model, edge_model)
                        if args.len() != 3 {
                            return Err(NopalError::SemanticError(format!(
                                "path_embedding_similarity requires exactly 3 arguments: \
                                 path_embedding_similarity(ref_name, node_model, edge_model), got {}. \
                                 The old 1- or 2-argument form is no longer valid — see PathSimilarity E-8.",
                                args.len()
                            )));
                        }
                        if !self.query_is_single_pattern_with_relationships(query) {
                            return Err(NopalError::SemanticError(
                                "path_embedding_similarity requires a single linear path-aware pattern with at least one relationship (PathSimilarity E-8)".to_string()
                            ));
                        }
                        match context {
                            EmbeddingExprContext::Find | EmbeddingExprContext::Where => {}
                            EmbeddingExprContext::OrderBy => {
                                return Err(NopalError::SemanticError(
                                    "path_embedding_similarity cannot be used directly in ORDER BY in PathSimilarity E-8; \
                                     project it with an alias in FIND and ORDER BY the alias".to_string()
                                ));
                            }
                            EmbeddingExprContext::GroupBy => {
                                return Err(NopalError::SemanticError(
                                    "path_embedding_similarity cannot be used in GROUP BY in PathSimilarity E-8".to_string()
                                ));
                            }
                            EmbeddingExprContext::Having => {
                                return Err(NopalError::SemanticError(
                                    "path_embedding_similarity cannot be used in HAVING in PathSimilarity E-8".to_string()
                                ));
                            }
                        }
                    }
                    "path_knn_references" => {
                        // E-9/E-10: path_knn_references(node_model, edge_model, k, min_score)
                        if args.len() != 4 {
                            return Err(NopalError::SemanticError(format!(
                                "path_knn_references requires exactly 4 arguments: \
                                 path_knn_references(node_model, edge_model, k, min_score), got {} (PathKNN E-9/E-10)",
                                args.len()
                            )));
                        }
                        if !self.query_is_single_pattern_with_relationships(query) {
                            return Err(NopalError::SemanticError(
                                "path_knn_references requires a single linear path-aware pattern \
                                 with at least one relationship (PathKNN E-9/E-10)"
                                    .to_string(),
                            ));
                        }
                        match context {
                            EmbeddingExprContext::Find => {}
                            EmbeddingExprContext::Where => {
                                return Err(NopalError::SemanticError(
                                    "path_knn_references returns a List and cannot be used in WHERE; \
                                     project it with an alias in FIND (PathKNN E-9)".to_string()
                                ));
                            }
                            EmbeddingExprContext::OrderBy => {
                                return Err(NopalError::SemanticError(
                                    "path_knn_references cannot be used directly in ORDER BY; \
                                     project it with an alias in FIND"
                                        .to_string(),
                                ));
                            }
                            EmbeddingExprContext::GroupBy => {
                                return Err(NopalError::SemanticError(
                                    "path_knn_references cannot be used in GROUP BY".to_string(),
                                ));
                            }
                            EmbeddingExprContext::Having => {
                                return Err(NopalError::SemanticError(
                                    "path_knn_references cannot be used in HAVING".to_string(),
                                ));
                            }
                        }
                    }
                    "path_anomaly_score" => {
                        // E-10: path_anomaly_score(node_model, edge_model)
                        if args.len() != 2 {
                            return Err(NopalError::SemanticError(format!(
                                "path_anomaly_score requires exactly 2 arguments: \
                                 path_anomaly_score(node_model, edge_model), got {} (PathAnomaly E-10)",
                                args.len()
                            )));
                        }
                        if !self.query_is_single_pattern_with_relationships(query) {
                            return Err(NopalError::SemanticError(
                                "path_anomaly_score requires a single linear path-aware pattern \
                                 with at least one relationship (PathAnomaly E-10)"
                                    .to_string(),
                            ));
                        }
                        match context {
                            EmbeddingExprContext::Find | EmbeddingExprContext::Where => {}
                            EmbeddingExprContext::OrderBy => {
                                return Err(NopalError::SemanticError(
                                    "path_anomaly_score cannot be used directly in ORDER BY in PathAnomaly E-10; \
                                     project it with an alias in FIND".to_string()
                                ));
                            }
                            EmbeddingExprContext::GroupBy => {
                                return Err(NopalError::SemanticError(
                                    "path_anomaly_score cannot be used in GROUP BY in PathAnomaly E-10".to_string()
                                ));
                            }
                            EmbeddingExprContext::Having => {
                                return Err(NopalError::SemanticError(
                                    "path_anomaly_score cannot be used in HAVING in PathAnomaly E-10".to_string()
                                ));
                            }
                        }
                    }
                    _ => {}
                }

                for arg in args {
                    self.validate_path_embedding_usage(arg, context, query)?;
                }
            }
            Expression::BinaryOp { left, right, .. } => {
                self.validate_path_embedding_usage(left, context, query)?;
                self.validate_path_embedding_usage(right, context, query)?;
            }
            Expression::UnaryOp { expr, .. } => {
                self.validate_path_embedding_usage(expr, context, query)?;
            }
            _ => {}
        }

        Ok(())
    }

    /// Validate ORDER BY columns are in SELECT (strict mode)
    fn validate_order_by_in_select(
        &self,
        _order_by: &OrderByClause,
        _find: &FindClause,
    ) -> Result<()> {
        // TODO: Re-enable after Expression implements Eq + Hash
        // For now, skip this validation
        Ok(())

        /*
        // Extract expressions from SELECT
        let select_exprs: HashSet<_> = find.projections.iter()
            .filter_map(|proj| match proj {
                Projection::Expression { expr, .. } => Some(expr),
                _ => None,
            })
            .collect();

        // Check each ORDER BY expression
        for item in &order_by.items {
            if !select_exprs.contains(&item.expression) {
                return Err(NopalError::semantic(
                    "ORDER BY expression must appear in SELECT clause (strict mode)"
                ));
            }
        }

        Ok(())
        */
    }

    /// Check for query warnings
    fn check_query_warnings(&self, query: &Query) -> Vec<ValidationWarning> {
        let mut warnings = Vec::new();

        // Warn if GROUP BY without aggregations in SELECT
        if query.group_by.is_some() {
            let has_aggregations = query.find.projections.iter().any(|proj| match proj {
                Projection::Expression { expr, .. } => expr.is_aggregation(),
                _ => false,
            });

            if !has_aggregations {
                warnings.push(ValidationWarning {
                    level: WarningLevel::Info,
                    message:
                        "GROUP BY without aggregations in SELECT - consider if this is intended"
                            .to_string(),
                });
            }
        }

        warnings
    }
}

// ═══════════════════════════════════════════════════════════════
// UPDATE VALIDATION
// ═══════════════════════════════════════════════════════════════

impl SemanticValidator {
    /// Validate UPDATE statement
    fn validate_update(&self, update: &UpdateStmt) -> Result<()> {
        // Rule 1: Must have at least one assignment
        if update.assignments.is_empty() {
            return Err(NopalError::SemanticError(
                "UPDATE requires at least one SET assignment".to_string(),
            ));
        }

        // Rule 2: Variables in assignments must be in pattern
        let pattern_vars: HashSet<_> = update.pattern.variables().into_iter().collect();

        for assignment in &update.assignments {
            if !pattern_vars.contains(&assignment.variable) {
                return Err(NopalError::SemanticError(format!(
                    "Variable '{}' in SET not found in pattern",
                    assignment.variable
                )));
            }
        }

        Ok(())
    }

    /// Check UPDATE warnings
    fn check_update_warnings(&self, update: &UpdateStmt) -> Vec<ValidationWarning> {
        let mut warnings = Vec::new();

        // Warn if no WHERE and no LIMIT
        if update.filter.is_none() && update.limit.is_none() {
            warnings.push(ValidationWarning {
                level: WarningLevel::Critical,
                message: "UPDATE without WHERE will modify ALL matching nodes".to_string(),
            });
        }

        warnings
    }
}

// ═══════════════════════════════════════════════════════════════
// DELETE VALIDATION
// ═══════════════════════════════════════════════════════════════

impl SemanticValidator {
    /// Check DELETE warnings
    fn check_delete_warnings(&self, delete: &DeleteStmt) -> Vec<ValidationWarning> {
        let mut warnings = Vec::new();

        // Warn based on danger level
        match delete.danger_level() {
            DangerLevel::High => {
                warnings.push(ValidationWarning {
                    level: WarningLevel::Critical,
                    message: "DELETE without WHERE will delete ALL matching nodes".to_string(),
                });
            }
            DangerLevel::Medium => {
                warnings.push(ValidationWarning {
                    level: WarningLevel::Warning,
                    message: "DELETE without WHERE but with LIMIT - still dangerous".to_string(),
                });
            }
            DangerLevel::Low => {
                // No warning
            }
        }

        warnings
    }
}

// ═══════════════════════════════════════════════════════════════
// CREATE VALIDATION
// ═══════════════════════════════════════════════════════════════

impl SemanticValidator {
    /// Validate CREATE statement
    fn validate_add(&self, create: &AddStmt) -> Result<()> {
        // Rule 1: Must have at least one node or edge
        if create.pattern.elements.is_empty() {
            return Err(NopalError::SemanticError(
                "CREATE requires at least one node or edge".to_string(),
            ));
        }

        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════
// HELPERS
// ═══════════════════════════════════════════════════════════════

/// Verifica si un nombre de función es un path reducer de F3.
fn is_path_reducer_fn(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "path_sum" | "path_min" | "path_max" | "path_avg"
    )
}

/// Verifica recursivamente si una expresión contiene un path reducer.
fn expr_contains_path_reducer_validator(expr: &Expression) -> bool {
    match expr {
        Expression::FunctionCall { name, .. } => is_path_reducer_fn(name),
        Expression::BinaryOp { left, right, .. } => {
            expr_contains_path_reducer_validator(left)
                || expr_contains_path_reducer_validator(right)
        }
        Expression::UnaryOp { expr, .. } => expr_contains_path_reducer_validator(expr),
        _ => false,
    }
}

fn expr_contains_function_validator(expr: &Expression, names: &[&str]) -> bool {
    match expr {
        Expression::FunctionCall { name, args } => {
            if names
                .iter()
                .any(|candidate| name.eq_ignore_ascii_case(candidate))
            {
                return true;
            }
            args.iter()
                .any(|arg| expr_contains_function_validator(arg, names))
        }
        Expression::BinaryOp { left, right, .. } => {
            expr_contains_function_validator(left, names)
                || expr_contains_function_validator(right, names)
        }
        Expression::UnaryOp { expr, .. } => expr_contains_function_validator(expr, names),
        _ => false,
    }
}

const PATH_SEMANTIC_FILTERS: &[&str] = &[
    "path_start_instanceof",
    "path_end_instanceof",
    "path_any_instanceof",
    "path_all_instanceof",
    "path_start_subclassof",
    "path_end_subclassof",
    "path_any_subclassof",
    "path_all_subclassof",
];

#[derive(Clone, Copy)]
enum EmbeddingExprContext {
    Find,
    Where,
    OrderBy,
    GroupBy,
    Having,
}

/// Verifica recursivamente si una expresión usa `path.<prop>` (F4-C).
fn expr_uses_path_property(expr: &Expression, prop: &str) -> bool {
    match expr {
        Expression::Property { variable, property } => variable == "path" && property == prop,
        Expression::BinaryOp { left, right, .. } => {
            expr_uses_path_property(left, prop) || expr_uses_path_property(right, prop)
        }
        Expression::UnaryOp { expr, .. } => expr_uses_path_property(expr, prop),
        Expression::FunctionCall { args, .. } => {
            args.iter().any(|a| expr_uses_path_property(a, prop))
        }
        _ => false,
    }
}

/// Verifica si alguna proyección usa `path.<prop>` (F4-C).
fn projections_use_path_property(projections: &[Projection], prop: &str) -> bool {
    projections.iter().any(|p| match p {
        Projection::Expression { expr, .. } => expr_uses_path_property(expr, prop),
        _ => false,
    })
}

/// Returns true if the HAVING expression references ONLY algorithm functions
/// (and literals, comparisons, logical ops over them) — without any true
/// aggregation function nor property reference. En ese caso, HAVING puede
/// usarse sin GROUP BY porque los algoritmos son per-node.
fn having_only_uses_algorithms(expr: &Expression) -> bool {
    match expr {
        Expression::FunctionCall { name, args } => {
            if !is_algorithm_function(name) {
                return false;
            }
            args.iter().all(|a| {
                matches!(a, Expression::Property { .. } | Expression::Literal(_))
                    || having_only_uses_algorithms(a)
            })
        }
        Expression::BinaryOp { left, right, op } => {
            // Comparisons and logical ops are fine if both sides only have
            // algos or literals.
            let side_ok = |e: &Expression| match e {
                Expression::Literal(_) => true,
                Expression::FunctionCall { .. } => having_only_uses_algorithms(e),
                Expression::BinaryOp { .. } | Expression::UnaryOp { .. } => {
                    having_only_uses_algorithms(e)
                }
                _ => false,
            };
            // Disallow if either side is a property reference (those need GROUP BY).
            let _ = op; // op type doesn't matter — we just check the sides
            side_ok(left) && side_ok(right)
        }
        Expression::UnaryOp { expr, .. } => having_only_uses_algorithms(expr),
        Expression::Literal(_) => true,
        _ => false,
    }
}

/// Check if function name is a TRUE aggregation (combines N rows → 1).
///
/// Solo `count`, `sum`, `avg`, `min`, `max`. Las funciones algorítmicas se
/// identifican con `is_algorithm_function`.
fn is_aggregation_function(name: &str) -> bool {
    let lower = name.to_lowercase();
    if matches!(lower.as_str(), "count" | "sum" | "avg" | "min" | "max") {
        return true;
    }
    #[cfg(feature = "embeddings")]
    if matches!(lower.as_str(), "embedding_similarity" | "knn_nodes") {
        return true;
    }
    false
}

/// Check if function name is a per-node algorithm function.
///
/// Estas funciones operan por nodo individual (degree de cada nodo, PageRank
/// de cada nodo, etc.) y son válidas en WHERE, HAVING y proyecciones
/// non-aggregated. Comparten el path de pre-cómputo con agregaciones porque
/// requieren correr el algoritmo una vez sobre el grafo.
#[cfg(feature = "algorithms")]
fn is_algorithm_function(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "pagerank"
            | "betweenness"
            | "clustering"
            | "degree"
            | "community"
            | "community_fast"
            | "leiden"
            | "shortestpath"
    )
}

#[cfg(not(feature = "algorithms"))]
fn is_algorithm_function(_name: &str) -> bool {
    false
}

// ═══════════════════════════════════════════════════════════════
// WARNINGS
// ═══════════════════════════════════════════════════════════════

/// Validation warning
#[derive(Debug, Clone)]
pub struct ValidationWarning {
    pub level: WarningLevel,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningLevel {
    Info,     // Informational
    Warning,  // Should review
    Critical, // Dangerous operation
}

impl std::fmt::Display for ValidationWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.level {
            WarningLevel::Info => write!(f, "[INFO] {}", self.message),
            WarningLevel::Warning => write!(f, "[WARNING] {}", self.message),
            WarningLevel::Critical => write!(f, "[CRITICAL] {}", self.message),
        }
    }
}

impl Default for SemanticValidator {
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
    use crate::types::PropertyValue;
    use std::collections::HashMap;

    #[test]
    fn test_having_without_group_by() {
        let validator = SemanticValidator::new();

        // Create query with HAVING but no GROUP BY
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![Projection::Expression {
                    expr: Expression::property_access("p", "name"),
                    alias: None,
                }],
            },
            from: FromClause { patterns: vec![] },
            filter: None,
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: Some(HavingClause {
                condition: Expression::function("count", vec![Expression::wildcard()]),
            }),
            order_by: None,
            limit: None,
            time_travel: None,
        };

        let result = validator.validate_query(&query);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("HAVING clause requires GROUP BY")
        );
    }

    #[test]
    fn test_aggregation_in_where() {
        let validator = SemanticValidator::new();

        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![],
            },
            from: FromClause { patterns: vec![] },
            filter: Some(WhereClause {
                condition: Expression::function("count", vec![Expression::wildcard()]),
            }),
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };

        let result = validator.validate_query(&query);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not allowed in WHERE")
        );
    }

    #[test]
    fn test_init_requires_relationship_pattern() {
        let validator = SemanticValidator::new();

        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![],
            },
            from: FromClause {
                patterns: vec![Pattern {
                    elements: vec![PatternElement::Node(NodePattern {
                        variable: Some("n".to_string()),
                        label: Some("Person".to_string()),
                        properties: HashMap::new(),
                    })],
                }],
            },
            filter: None,
            init: vec!["sum = 0".to_string()],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };

        let result = validator.validate_query(&query);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("INIT/GATHER"));
    }

    #[test]
    fn test_path_eval_rejected_in_order_by() {
        let validator = SemanticValidator::new();

        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![Projection::Expression {
                    expr: Expression::property_access("b", "name"),
                    alias: None,
                }],
            },
            from: FromClause {
                patterns: vec![Pattern {
                    elements: vec![
                        PatternElement::Node(NodePattern {
                            variable: Some("a".to_string()),
                            label: Some("Account".to_string()),
                            properties: HashMap::new(),
                        }),
                        PatternElement::Relationship(RelationshipPattern {
                            variable: None,
                            rel_type: Some("TRANSFER".to_string()),
                            properties: HashMap::new(),
                            direction: Direction::Outgoing,
                            quantifier: None,
                        }),
                        PatternElement::Node(NodePattern {
                            variable: Some("b".to_string()),
                            label: Some("Account".to_string()),
                            properties: HashMap::new(),
                        }),
                    ],
                }],
            },
            filter: None,
            init: vec!["sum = 0".to_string()],
            gather: vec!["sum = sum + edge.amount".to_string()],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: Some(OrderByClause {
                items: vec![OrderByItem {
                    expression: Expression::function(
                        "path_eval",
                        vec![Expression::Literal(PropertyValue::String(
                            "sum".to_string(),
                        ))],
                    ),
                    order: SortOrder::Asc,
                }],
            }),
            limit: None,
            time_travel: None,
        };

        let result = validator.validate_query(&query);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("path_eval(\"...\") is not supported in ORDER BY")
        );
    }

    // ─── F4-C validator tests ────────────────────────────────────────────────

    fn path_pattern() -> FromClause {
        FromClause {
            patterns: vec![Pattern {
                elements: vec![
                    PatternElement::Node(NodePattern {
                        variable: Some("a".to_string()),
                        label: Some("Account".to_string()),
                        properties: HashMap::new(),
                    }),
                    PatternElement::Relationship(RelationshipPattern {
                        variable: None,
                        rel_type: Some("TRANSFER".to_string()),
                        properties: HashMap::new(),
                        direction: Direction::Outgoing,
                        quantifier: None,
                    }),
                    PatternElement::Node(NodePattern {
                        variable: Some("b".to_string()),
                        label: Some("Account".to_string()),
                        properties: HashMap::new(),
                    }),
                ],
            }],
        }
    }

    #[test]
    fn test_return_requires_relationship() {
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![],
            },
            from: FromClause {
                patterns: vec![Pattern {
                    elements: vec![PatternElement::Node(NodePattern {
                        variable: Some("n".to_string()),
                        label: None,
                        properties: HashMap::new(),
                    })],
                }],
            },
            filter: None,
            init: vec![],
            gather: vec![],
            return_expr: Some("1".to_string()),
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };
        let result = validator.validate_query(&query);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("RETURN requires a path pattern")
        );
    }

    #[test]
    fn test_return_rejected_with_order_by() {
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![],
            },
            from: path_pattern(),
            filter: None,
            init: vec![],
            gather: vec![],
            return_expr: Some("1".to_string()),
            group_by: None,
            having: None,
            order_by: Some(OrderByClause {
                items: vec![OrderByItem {
                    expression: Expression::property_access("b", "name"),
                    order: SortOrder::Asc,
                }],
            }),
            limit: None,
            time_travel: None,
        };
        let result = validator.validate_query(&query);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("RETURN is not supported with ORDER BY")
        );
    }

    #[test]
    fn test_return_rejected_with_group_by() {
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![],
            },
            from: path_pattern(),
            filter: None,
            init: vec![],
            gather: vec![],
            return_expr: Some("1".to_string()),
            group_by: Some(GroupByClause {
                expressions: vec![Expression::property_access("b", "label")],
            }),
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };
        let result = validator.validate_query(&query);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("RETURN is not supported with GROUP BY")
        );
    }

    #[test]
    fn test_path_result_requires_return() {
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![Projection::Expression {
                    expr: Expression::Property {
                        variable: "path".to_string(),
                        property: "result".to_string(),
                    },
                    alias: None,
                }],
            },
            from: path_pattern(),
            filter: None,
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };
        let result = validator.validate_query(&query);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("path.result requires a RETURN clause")
        );
    }

    #[test]
    fn test_path_state_in_where_rejected() {
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![],
            },
            from: path_pattern(),
            filter: Some(WhereClause {
                condition: Expression::Property {
                    variable: "path".to_string(),
                    property: "state".to_string(),
                },
            }),
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };
        let result = validator.validate_query(&query);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("path.state is not allowed in WHERE")
        );
    }

    #[test]
    fn test_path_start_in_where_rejected() {
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![],
            },
            from: path_pattern(),
            filter: Some(WhereClause {
                condition: Expression::Property {
                    variable: "path".to_string(),
                    property: "start".to_string(),
                },
            }),
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };
        let result = validator.validate_query(&query);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("path.start is not allowed in WHERE")
        );
    }

    #[test]
    fn test_path_result_in_order_by_rejected() {
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![],
            },
            from: path_pattern(),
            filter: None,
            init: vec![],
            gather: vec![],
            return_expr: Some("1".to_string()),
            group_by: None,
            having: None,
            order_by: Some(OrderByClause {
                items: vec![OrderByItem {
                    expression: Expression::Property {
                        variable: "path".to_string(),
                        property: "result".to_string(),
                    },
                    order: SortOrder::Asc,
                }],
            }),
            limit: None,
            time_travel: None,
        };
        // Rule 8 fires first (return + order_by)
        let result = validator.validate_query(&query);
        assert!(result.is_err());
    }

    #[test]
    fn test_semantic_path_filter_allowed_in_where_for_path_pattern() {
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![Projection::Expression {
                    expr: Expression::property_access("b", "name"),
                    alias: None,
                }],
            },
            from: path_pattern(),
            filter: Some(WhereClause {
                condition: Expression::function(
                    "path_end_instanceOf",
                    vec![Expression::Literal(PropertyValue::String(
                        "FinancialEntity".into(),
                    ))],
                ),
            }),
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };

        assert!(validator.validate_query(&query).is_ok());
    }

    #[test]
    fn test_semantic_path_filter_rejected_in_find() {
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![Projection::Expression {
                    expr: Expression::function(
                        "path_end_instanceOf",
                        vec![Expression::Literal(PropertyValue::String(
                            "FinancialEntity".into(),
                        ))],
                    ),
                    alias: Some("ok".into()),
                }],
            },
            from: path_pattern(),
            filter: None,
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };

        let result = validator.validate_query(&query);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("only supported in WHERE")
        );
    }

    #[test]
    fn test_semantic_path_filter_rejected_without_path_pattern() {
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![],
            },
            from: FromClause {
                patterns: vec![Pattern {
                    elements: vec![PatternElement::Node(NodePattern {
                        variable: Some("n".to_string()),
                        label: Some("Account".to_string()),
                        properties: HashMap::new(),
                    })],
                }],
            },
            filter: Some(WhereClause {
                condition: Expression::function(
                    "path_any_instanceOf",
                    vec![Expression::Literal(PropertyValue::String(
                        "FinancialEntity".into(),
                    ))],
                ),
            }),
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };

        let result = validator.validate_query(&query);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("single path pattern")
        );
    }

    #[test]
    fn test_path_embedding_two_models_allowed_in_find_and_where() {
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![Projection::Expression {
                    expr: Expression::function(
                        "path_embedding",
                        vec![
                            Expression::Literal(PropertyValue::String("node-minilm".into())),
                            Expression::Literal(PropertyValue::String("edge-relbert".into())),
                        ],
                    ),
                    alias: Some("path_vec".into()),
                }],
            },
            from: path_pattern(),
            filter: Some(WhereClause {
                condition: Expression::BinaryOp {
                    left: Box::new(Expression::function(
                        "path_has_embeddings",
                        vec![
                            Expression::Literal(PropertyValue::String("node-minilm".into())),
                            Expression::Literal(PropertyValue::String("edge-relbert".into())),
                        ],
                    )),
                    op: BinaryOperator::And,
                    right: Box::new(Expression::Literal(PropertyValue::Bool(true))),
                },
            }),
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };

        assert!(validator.validate_query(&query).is_ok());
    }

    #[test]
    fn test_path_embedding_rejected_in_where() {
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![],
            },
            from: path_pattern(),
            filter: Some(WhereClause {
                condition: Expression::function(
                    "path_embedding",
                    vec![
                        Expression::Literal(PropertyValue::String("node-minilm".into())),
                        Expression::Literal(PropertyValue::String("edge-relbert".into())),
                    ],
                ),
            }),
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };

        let result = validator.validate_query(&query);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("only supported in FIND")
        );
    }

    #[test]
    fn test_path_has_embeddings_single_model_rejected_with_migration_error() {
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![Projection::Expression {
                    expr: Expression::function(
                        "path_has_embeddings",
                        vec![Expression::Literal(PropertyValue::String("minilm".into()))],
                    ),
                    alias: Some("legacy".into()),
                }],
            },
            from: path_pattern(),
            filter: None,
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };

        let result = validator.validate_query(&query);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("was replaced"));
    }

    #[test]
    fn test_path_embedding_similarity_old_two_arg_form_rejected() {
        // E-8: 2-arg form (old baseline) must fail with arity error
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![Projection::Expression {
                    expr: Expression::function(
                        "path_embedding_similarity",
                        vec![
                            Expression::Literal(PropertyValue::String("ref-name".into())),
                            Expression::Literal(PropertyValue::String("node-minilm".into())),
                        ],
                    ),
                    alias: Some("legacy_sim".into()),
                }],
            },
            from: path_pattern(),
            filter: None,
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };

        let result = validator.validate_query(&query);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("exactly 3 arguments"),
            "expected arity error, got: {}",
            msg
        );
    }

    #[test]
    fn test_path_embedding_similarity_three_arg_form_accepted_in_find() {
        // E-8: 3-arg form in FIND is valid
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![Projection::Expression {
                    expr: Expression::function(
                        "path_embedding_similarity",
                        vec![
                            Expression::Literal(PropertyValue::String("fraud_ring_v1".into())),
                            Expression::Literal(PropertyValue::String("node-minilm".into())),
                            Expression::Literal(PropertyValue::String("edge-relbert".into())),
                        ],
                    ),
                    alias: Some("score".into()),
                }],
            },
            from: path_pattern(),
            filter: None,
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };

        let result = validator.validate_query(&query);
        assert!(result.is_ok(), "expected Ok, got: {:?}", result);
    }

    #[test]
    fn test_path_embedding_similarity_rejected_in_order_by_direct() {
        // E-8: ORDER BY path_embedding_similarity(...) directly must fail
        let validator = SemanticValidator::new();
        let order_expr = Expression::function(
            "path_embedding_similarity",
            vec![
                Expression::Literal(PropertyValue::String("ref".into())),
                Expression::Literal(PropertyValue::String("node-m".into())),
                Expression::Literal(PropertyValue::String("edge-m".into())),
            ],
        );
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![Projection::Expression {
                    expr: Expression::Property {
                        variable: "n".into(),
                        property: "id".into(),
                    },
                    alias: None,
                }],
            },
            from: path_pattern(),
            filter: None,
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: Some(crate::query::nql::parser::ast::OrderByClause {
                items: vec![crate::query::nql::parser::ast::OrderByItem {
                    expression: order_expr,
                    order: crate::query::nql::parser::ast::SortOrder::Asc,
                }],
            }),
            limit: None,
            time_travel: None,
        };

        let result = validator.validate_query(&query);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("ORDER BY") || msg.contains("alias"),
            "expected ORDER BY rejection, got: {}",
            msg
        );
    }

    // ────────────────────────────────────────────────────────────
    // E-9 PathKNN validator tests
    // ────────────────────────────────────────────────────────────

    #[test]
    fn test_path_knn_references_wrong_arity_rejected() {
        // E-9: 3 args instead of 4 must fail
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![Projection::Expression {
                    expr: Expression::function(
                        "path_knn_references",
                        vec![
                            Expression::Literal(PropertyValue::String("nm".into())),
                            Expression::Literal(PropertyValue::String("em".into())),
                            Expression::Literal(PropertyValue::Int(3)),
                        ],
                    ),
                    alias: Some("refs".into()),
                }],
            },
            from: path_pattern(),
            filter: None,
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };
        let result = validator.validate_query(&query);
        assert!(result.is_err(), "expected arity error");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("4 arguments") || msg.contains("E-9"),
            "expected arity error, got: {}",
            msg
        );
    }

    #[test]
    fn test_path_knn_references_four_arg_form_accepted_in_find() {
        // E-9: 4-arg form in FIND is valid
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![Projection::Expression {
                    expr: Expression::function(
                        "path_knn_references",
                        vec![
                            Expression::Literal(PropertyValue::String("nm".into())),
                            Expression::Literal(PropertyValue::String("em".into())),
                            Expression::Literal(PropertyValue::Int(5)),
                            Expression::Literal(PropertyValue::Float(0.8)),
                        ],
                    ),
                    alias: Some("refs".into()),
                }],
            },
            from: path_pattern(),
            filter: None,
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };
        let result = validator.validate_query(&query);
        assert!(result.is_ok(), "expected Ok, got: {:?}", result);
    }

    #[test]
    fn test_path_knn_references_rejected_in_where() {
        // E-9: path_knn_references in WHERE must fail
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![Projection::Expression {
                    expr: Expression::Property {
                        variable: "n".into(),
                        property: "id".into(),
                    },
                    alias: None,
                }],
            },
            from: path_pattern(),
            filter: Some(crate::query::nql::parser::ast::WhereClause {
                condition: Expression::function(
                    "path_knn_references",
                    vec![
                        Expression::Literal(PropertyValue::String("nm".into())),
                        Expression::Literal(PropertyValue::String("em".into())),
                        Expression::Literal(PropertyValue::Int(3)),
                        Expression::Literal(PropertyValue::Float(0.5)),
                    ],
                ),
            }),
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };
        let result = validator.validate_query(&query);
        assert!(
            result.is_err(),
            "expected SemanticError for path_knn_references in WHERE"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("WHERE") || msg.contains("E-9"),
            "expected WHERE rejection, got: {}",
            msg
        );
    }

    #[test]
    fn test_pattern_has_embeddings_two_models_allowed_in_find_and_where() {
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![Projection::Expression {
                    expr: Expression::function(
                        "pattern_has_embeddings",
                        vec![
                            Expression::Literal(PropertyValue::String("node-minilm".into())),
                            Expression::Literal(PropertyValue::String("edge-relbert".into())),
                        ],
                    ),
                    alias: Some("has_pat".into()),
                }],
            },
            from: path_pattern(),
            filter: Some(WhereClause {
                condition: Expression::function(
                    "pattern_has_embeddings",
                    vec![
                        Expression::Literal(PropertyValue::String("node-minilm".into())),
                        Expression::Literal(PropertyValue::String("edge-relbert".into())),
                    ],
                ),
            }),
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };

        assert!(validator.validate_query(&query).is_ok());
    }

    #[test]
    fn test_pattern_embedding_allowed_in_find() {
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![Projection::Expression {
                    expr: Expression::function(
                        "pattern_embedding",
                        vec![
                            Expression::Literal(PropertyValue::String("node-minilm".into())),
                            Expression::Literal(PropertyValue::String("edge-relbert".into())),
                        ],
                    ),
                    alias: Some("pat_vec".into()),
                }],
            },
            from: path_pattern(),
            filter: None,
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };

        assert!(validator.validate_query(&query).is_ok());
    }

    #[test]
    fn test_pattern_embedding_rejected_in_where() {
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![],
            },
            from: path_pattern(),
            filter: Some(WhereClause {
                condition: Expression::function(
                    "pattern_embedding",
                    vec![
                        Expression::Literal(PropertyValue::String("node-minilm".into())),
                        Expression::Literal(PropertyValue::String("edge-relbert".into())),
                    ],
                ),
            }),
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };

        let result = validator.validate_query(&query);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("only supported in FIND")
        );
    }

    #[test]
    fn test_pattern_has_embeddings_single_model_rejected_with_migration_error() {
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![Projection::Expression {
                    expr: Expression::function(
                        "pattern_has_embeddings",
                        vec![Expression::Literal(PropertyValue::String("minilm".into()))],
                    ),
                    alias: Some("legacy".into()),
                }],
            },
            from: path_pattern(),
            filter: None,
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };

        let result = validator.validate_query(&query);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("was replaced"));
    }

    #[test]
    fn test_pattern_embedding_similarity_rejected_with_migration_error() {
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![Projection::Expression {
                    expr: Expression::function(
                        "pattern_embedding_similarity",
                        vec![
                            Expression::Literal(PropertyValue::String("ref-uuid".into())),
                            Expression::Literal(PropertyValue::String("minilm".into())),
                        ],
                    ),
                    alias: Some("legacy_sim".into()),
                }],
            },
            from: path_pattern(),
            filter: None,
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };

        let result = validator.validate_query(&query);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("no longer the official PatternEmbedding surface")
        );
    }

    #[test]
    fn test_pattern_embedding_rejected_without_relationship_pattern() {
        let validator = SemanticValidator::new();
        let query = Query {
            export: None,
            find: FindClause {
                distinct: false,
                projections: vec![Projection::Expression {
                    expr: Expression::function(
                        "pattern_embedding",
                        vec![
                            Expression::Literal(PropertyValue::String("node-minilm".into())),
                            Expression::Literal(PropertyValue::String("edge-relbert".into())),
                        ],
                    ),
                    alias: Some("pat_vec".into()),
                }],
            },
            from: FromClause {
                patterns: vec![Pattern {
                    elements: vec![PatternElement::Node(NodePattern {
                        variable: Some("n".to_string()),
                        label: Some("Account".to_string()),
                        properties: HashMap::new(),
                    })],
                }],
            },
            filter: None,
            init: vec![],
            gather: vec![],
            return_expr: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            time_travel: None,
        };

        let result = validator.validate_query(&query);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("single path pattern")
        );
    }
}
