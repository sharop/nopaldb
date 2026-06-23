// src/query/nql/parser/mod.rs

pub mod ast;

use crate::error::{NopalError, Result};
use crate::types::PropertyValue;
use ast::*;
use pest::Parser;
use pest_derive::Parser;

#[derive(Parser)]
#[grammar = "query/nql/parser/nql.pest"] // ← ACTUALIZAR PATH
struct NQLParser;

/// Parse NQL string into Statement AST
pub fn parse(input: &str) -> Result<Statement> {
    let pairs = NQLParser::parse(Rule::statement, input)
        .map_err(|e| NopalError::QueryParseError(format!("Parse error: {}", e)))?;

    let mut ast_builder = AstBuilder::new();

    for pair in pairs {
        if pair.as_rule() == Rule::statement {
            return ast_builder.build_statement(pair);
        }
    }

    Err(NopalError::QueryParseError(
        "Invalid statement structure".into(),
    ))
}

/// Parse NQL query (legacy, for backward compatibility)
pub fn parse_query(query: &str) -> Result<Query> {
    match parse(query)? {
        Statement::Query(q) => Ok(q),
        _ => Err(NopalError::QueryParseError(
            "Expected Query statement".into(),
        )),
    }
}

pub fn parse_vm_assignment(input: &str) -> Result<VmAssignment> {
    let mut pairs = NQLParser::parse(Rule::vm_assignment, input)
        .map_err(|e| NopalError::QueryParseError(format!("VM assignment parse error: {}", e)))?;
    let pair = pairs
        .next()
        .ok_or_else(|| NopalError::QueryParseError("Invalid VM assignment".into()))?;
    let mut builder = AstBuilder::new();
    builder.build_vm_assignment(pair)
}

pub fn parse_vm_expression(input: &str) -> Result<Expression> {
    let mut pairs = NQLParser::parse(Rule::vm_expression, input)
        .map_err(|e| NopalError::QueryParseError(format!("VM expression parse error: {}", e)))?;
    let pair = pairs
        .next()
        .ok_or_else(|| NopalError::QueryParseError("Invalid VM expression".into()))?;
    let mut builder = AstBuilder::new();
    builder.build_vm_expression(pair)
}

// ═══════════════════════════════════════════════════════════
// AST BUILDER
// ═══════════════════════════════════════════════════════════

struct AstBuilder;

impl AstBuilder {
    fn new() -> Self {
        AstBuilder
    }

    fn build_statement(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<Statement> {
        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::profile_stmt => {
                    return self.build_profile_stmt(inner);
                }
                Rule::explain_stmt => {
                    return self.build_explain_stmt(inner);
                }
                Rule::create_index_stmt => {
                    return Ok(Statement::CreateIndex(self.build_create_index_stmt(inner)?));
                }
                Rule::drop_index_stmt => {
                    return Ok(Statement::DropIndex(self.build_drop_index_stmt(inner)?));
                }
                Rule::sketch_stmt => {
                    return Ok(Statement::Sketch(self.build_sketch_stmt(inner)?));
                }
                Rule::commit_stmt => {
                    return Ok(Statement::Commit(self.build_commit_stmt(inner)?));
                }
                Rule::delete_stmt => {
                    return Ok(Statement::Delete(self.build_delete_stmt(inner)?));
                }
                Rule::update_stmt => {
                    return Ok(Statement::Update(self.build_update_stmt(inner)?));
                }
                Rule::add_stmt => {
                    return Ok(Statement::Add(self.build_add_stmt(inner)?));
                }
                Rule::query => {
                    return Ok(Statement::Query(self.build_query(inner)?));
                }
                _ => {}
            }
        }

        Err(NopalError::QueryParseError("Invalid statement".into()))
    }

    fn build_sketch_stmt(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<SketchStmt> {
        let mut name = None;
        let mut operation = None;

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::identifier => {
                    if name.is_none() {
                        name = Some(inner.as_str().to_string());
                    }
                }
                Rule::delete_stmt => {
                    operation = Some(Box::new(Statement::Delete(self.build_delete_stmt(inner)?)));
                }
                Rule::update_stmt => {
                    operation = Some(Box::new(Statement::Update(self.build_update_stmt(inner)?)));
                }
                Rule::add_stmt => {
                    operation = Some(Box::new(Statement::Add(self.build_add_stmt(inner)?)));
                }
                Rule::query => {
                    operation = Some(Box::new(Statement::Query(self.build_query(inner)?)));
                }
                _ => {}
            }
        }

        Ok(SketchStmt {
            name: name.ok_or_else(|| NopalError::QueryParseError("Missing sketch name".into()))?,
            operation: operation
                .ok_or_else(|| NopalError::QueryParseError("Missing sketch operation".into()))?,
            description: None,
        })
    }

    fn build_commit_stmt(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<CommitStmt> {
        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::identifier {
                return Ok(CommitStmt {
                    sketch_name: inner.as_str().to_string(),
                });
            }
        }

        Err(NopalError::QueryParseError(
            "Missing sketch name in COMMIT".into(),
        ))
    }

    fn build_delete_stmt(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<DeleteStmt> {
        let mut pattern = None;
        let mut filter = None;
        let mut limit = None;

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::pattern => {
                    pattern = Some(self.build_pattern(inner)?);
                }
                Rule::where_clause => {
                    filter = Some(self.build_where_clause(inner)?);
                }
                Rule::limit_clause => {
                    limit = Some(self.build_limit_clause(inner)?);
                }
                _ => {}
            }
        }

        Ok(DeleteStmt {
            pattern: pattern
                .ok_or_else(|| NopalError::QueryParseError("Missing pattern in DELETE".into()))?,
            filter,
            limit,
        })
    }

    fn build_update_stmt(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<UpdateStmt> {
        let mut pattern = None;
        let mut assignments = Vec::new();
        let mut filter = None;
        let mut limit = None;

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::pattern => {
                    pattern = Some(self.build_pattern(inner)?);
                }
                Rule::assignment_list => {
                    for assignment_pair in inner.into_inner() {
                        if assignment_pair.as_rule() == Rule::assignment {
                            assignments.push(self.build_assignment(assignment_pair)?);
                        }
                    }
                }
                Rule::where_clause => {
                    filter = Some(self.build_where_clause(inner)?);
                }
                Rule::limit_clause => {
                    limit = Some(self.build_limit_clause(inner)?);
                }
                _ => {}
            }
        }

        Ok(UpdateStmt {
            pattern: pattern
                .ok_or_else(|| NopalError::QueryParseError("Missing pattern in UPDATE".into()))?,
            assignments,
            filter,
            limit,
        })
    }

    fn build_assignment(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<Assignment> {
        let mut identifiers = Vec::new();
        let mut value = None;

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::identifier => {
                    identifiers.push(inner.as_str().to_string());
                }
                Rule::expression => {
                    value = Some(self.build_expression(inner)?);
                }
                _ => {}
            }
        }

        if identifiers.len() < 2 {
            return Err(NopalError::QueryParseError(
                "Invalid assignment format".into(),
            ));
        }

        Ok(Assignment {
            variable: identifiers[0].clone(),
            property: identifiers[1].clone(),
            value: value
                .ok_or_else(|| NopalError::QueryParseError("Missing value in assignment".into()))?,
        })
    }

    fn build_add_stmt(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<AddStmt> {
        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::pattern {
                return Ok(AddStmt {
                    pattern: self.build_pattern(inner)?,
                });
            }
        }

        Err(NopalError::QueryParseError("Missing pattern in ADD".into()))
    }

    fn build_query(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<Query> {
        let mut export = None;
        let mut find = None;
        let mut from = None;
        let mut filter = None;
        let mut init = Vec::new();
        let mut gather = Vec::new();
        let mut return_expr = None;
        let mut group_by = None;
        let mut having = None;
        let mut order_by = None;
        let mut limit = None;
        let mut time_travel = None;

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::query_body => {
                    for clause in inner.into_inner() {
                        match clause.as_rule() {
                            Rule::find_clause => {
                                find = Some(self.build_find_clause(clause)?);
                            }
                            Rule::from_clause => {
                                from = Some(self.build_from_clause(clause)?);
                            }
                            Rule::where_clause => {
                                filter = Some(self.build_where_clause(clause)?);
                            }
                            Rule::init_clause => {
                                init.push(self.build_string_clause(
                                    clause,
                                    Rule::init_clause,
                                    "INIT",
                                )?);
                            }
                            Rule::gather_clause => {
                                gather.push(self.build_string_clause(
                                    clause,
                                    Rule::gather_clause,
                                    "GATHER",
                                )?);
                            }
                            Rule::return_clause => {
                                return_expr = Some(self.build_string_clause(
                                    clause,
                                    Rule::return_clause,
                                    "RETURN",
                                )?);
                            }
                            Rule::group_clause => {
                                group_by = Some(self.build_group_by_clause(clause)?);
                            }
                            Rule::having_clause => {
                                having = Some(self.build_having_clause(clause)?);
                            }
                            Rule::order_clause => {
                                order_by = Some(self.build_order_by_clause(clause)?);
                            }
                            Rule::limit_clause => {
                                limit = Some(self.build_limit_clause(clause)?);
                            }
                            Rule::time_clause => {
                                time_travel = Some(self.build_time_clause(clause)?);
                            }
                            Rule::export_clause => {
                                export = Some(self.build_export_clause(clause)?);
                            }

                            _ => {}
                        }
                    }
                }
                Rule::EOI => {}
                _ => {}
            }
        }

        Ok(Query {
            export,
            find: find.ok_or_else(|| NopalError::QueryParseError("Missing FIND clause".into()))?,
            from: from.ok_or_else(|| NopalError::QueryParseError("Missing FROM clause".into()))?,
            filter,
            init,
            gather,
            return_expr,
            group_by,
            having,
            order_by,
            limit,
            time_travel,
        })
    }

    fn build_string_clause(
        &mut self,
        pair: pest::iterators::Pair<Rule>,
        expected_rule: Rule,
        clause_name: &str,
    ) -> Result<String> {
        if pair.as_rule() != expected_rule {
            return Err(NopalError::QueryParseError(format!(
                "Invalid {} clause",
                clause_name
            )));
        }

        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::string {
                return Ok(Self::unquote_string(inner.as_str()));
            }
        }

        Err(NopalError::QueryParseError(format!(
            "Missing string payload in {} clause",
            clause_name
        )))
    }

    fn build_vm_assignment(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<VmAssignment> {
        let mut variable = None;
        let mut expr = None;

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::identifier if variable.is_none() => {
                    variable = Some(inner.as_str().to_string());
                }
                Rule::vm_expression => {
                    expr = Some(self.build_vm_expression(inner)?);
                }
                _ => {}
            }
        }

        Ok(VmAssignment {
            variable: variable.ok_or_else(|| {
                NopalError::QueryParseError("Missing target variable in VM assignment".into())
            })?,
            expr: expr.ok_or_else(|| {
                NopalError::QueryParseError("Missing expression in VM assignment".into())
            })?,
        })
    }

    fn build_vm_expression(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<Expression> {
        match pair.as_rule() {
            Rule::vm_expression => {
                let inner = pair
                    .into_inner()
                    .next()
                    .ok_or_else(|| NopalError::QueryParseError("Invalid VM expression".into()))?;
                self.build_vm_expression(inner)
            }
            Rule::vm_or_expression => {
                let mut inner = pair
                    .into_inner()
                    .filter(|p| p.as_rule() == Rule::vm_and_expression);
                let first = inner.next().ok_or_else(|| {
                    NopalError::QueryParseError("Invalid VM OR expression".into())
                })?;
                let mut result = self.build_vm_expression(first)?;
                for next in inner {
                    let right = self.build_vm_expression(next)?;
                    result = Expression::BinaryOp {
                        left: Box::new(result),
                        op: BinaryOperator::Or,
                        right: Box::new(right),
                    };
                }
                Ok(result)
            }
            Rule::vm_and_expression => {
                let mut inner = pair
                    .into_inner()
                    .filter(|p| p.as_rule() == Rule::vm_comparison_expression);
                let first = inner.next().ok_or_else(|| {
                    NopalError::QueryParseError("Invalid VM AND expression".into())
                })?;
                let mut result = self.build_vm_expression(first)?;
                for next in inner {
                    let right = self.build_vm_expression(next)?;
                    result = Expression::BinaryOp {
                        left: Box::new(result),
                        op: BinaryOperator::And,
                        right: Box::new(right),
                    };
                }
                Ok(result)
            }
            Rule::vm_comparison_expression => {
                let mut left = None;
                let mut op = None;
                let mut right = None;

                for inner in pair.into_inner() {
                    match inner.as_rule() {
                        Rule::vm_additive_expression => {
                            if left.is_none() {
                                left = Some(self.build_vm_expression(inner)?);
                            } else {
                                right = Some(self.build_vm_expression(inner)?);
                            }
                        }
                        Rule::comparison_op => {
                            op = Some(self.parse_comparison_op(inner.as_str())?);
                        }
                        _ => {}
                    }
                }

                match (left, op, right) {
                    (Some(l), Some(o), Some(r)) => Ok(Expression::BinaryOp {
                        left: Box::new(l),
                        op: o,
                        right: Box::new(r),
                    }),
                    (Some(l), None, None) => Ok(l),
                    _ => Err(NopalError::QueryParseError(
                        "Invalid VM comparison expression".into(),
                    )),
                }
            }
            Rule::vm_additive_expression => {
                let mut inner = pair.into_inner();
                let first = inner.next().ok_or_else(|| {
                    NopalError::QueryParseError("Invalid VM additive expression".into())
                })?;
                let mut result = self.build_vm_expression(first)?;

                while let Some(op_pair) = inner.next() {
                    let right_pair = inner.next().ok_or_else(|| {
                        NopalError::QueryParseError("Missing RHS in VM additive expression".into())
                    })?;
                    let op = match op_pair.as_rule() {
                        Rule::add_op => BinaryOperator::Add,
                        Rule::sub_op => BinaryOperator::Sub,
                        _ => {
                            return Err(NopalError::QueryParseError(
                                "Invalid additive operator in VM expression".into(),
                            ));
                        }
                    };
                    let right = self.build_vm_expression(right_pair)?;
                    result = Expression::BinaryOp {
                        left: Box::new(result),
                        op,
                        right: Box::new(right),
                    };
                }

                Ok(result)
            }
            Rule::vm_multiplicative_expression => {
                let mut inner = pair.into_inner();
                let first = inner.next().ok_or_else(|| {
                    NopalError::QueryParseError("Invalid VM multiplicative expression".into())
                })?;
                let mut result = self.build_vm_expression(first)?;

                while let Some(op_pair) = inner.next() {
                    let right_pair = inner.next().ok_or_else(|| {
                        NopalError::QueryParseError(
                            "Missing RHS in VM multiplicative expression".into(),
                        )
                    })?;
                    let op = match op_pair.as_rule() {
                        Rule::mul_op => BinaryOperator::Mul,
                        Rule::div_op => BinaryOperator::Div,
                        Rule::mod_op => BinaryOperator::Mod,
                        _ => {
                            return Err(NopalError::QueryParseError(
                                "Invalid multiplicative operator in VM expression".into(),
                            ));
                        }
                    };
                    let right = self.build_vm_expression(right_pair)?;
                    result = Expression::BinaryOp {
                        left: Box::new(result),
                        op,
                        right: Box::new(right),
                    };
                }

                Ok(result)
            }
            Rule::vm_unary_expression => {
                let raw = pair.as_str().trim_start();
                let inner = pair.into_inner().last().ok_or_else(|| {
                    NopalError::QueryParseError("Invalid VM unary expression".into())
                })?;
                let expr = self.build_vm_expression(inner)?;

                if raw.starts_with('!') || raw.to_ascii_lowercase().starts_with("not") {
                    Ok(Expression::UnaryOp {
                        op: UnaryOperator::Not,
                        expr: Box::new(expr),
                    })
                } else if raw.starts_with('-') {
                    Ok(Expression::UnaryOp {
                        op: UnaryOperator::Neg,
                        expr: Box::new(expr),
                    })
                } else {
                    Ok(expr)
                }
            }
            Rule::vm_primary_expression => {
                let inner = pair.into_inner().next().ok_or_else(|| {
                    NopalError::QueryParseError("Invalid VM primary expression".into())
                })?;
                match inner.as_rule() {
                    Rule::value => self.build_value_expression(inner),
                    Rule::property_access => self.build_property_access(inner),
                    Rule::vm_expression => self.build_vm_expression(inner),
                    _ => Err(NopalError::QueryParseError(
                        "Invalid VM primary expression".into(),
                    )),
                }
            }
            _ => Err(NopalError::QueryParseError("Invalid VM expression".into())),
        }
    }

    fn build_export_clause(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<ExportClause> {
        let mut format = ExportFormat::Arrow;
        let mut options = std::collections::HashMap::new();

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::export_format => {
                    for fmt_inner in inner.into_inner() {
                        match fmt_inner.as_rule() {
                            Rule::export_arrow => {
                                format = ExportFormat::Arrow;
                            }
                            Rule::export_csv => {
                                format = ExportFormat::Csv;
                            }
                            Rule::export_json => {
                                format = ExportFormat::Json;
                            }
                            Rule::export_parquet => {
                                for p in fmt_inner.into_inner() {
                                    if p.as_rule() == Rule::string {
                                        format =
                                            ExportFormat::Parquet(Self::unquote_string(p.as_str()));
                                        break;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Rule::export_options => {
                    for opt in inner.into_inner() {
                        if opt.as_rule() == Rule::export_option {
                            let mut key = None;
                            let mut value = None;
                            for opt_inner in opt.into_inner() {
                                match opt_inner.as_rule() {
                                    Rule::identifier => {
                                        if key.is_none() {
                                            key = Some(opt_inner.as_str().to_string());
                                        }
                                    }
                                    Rule::value => {
                                        value = Some(self.parse_property_value(opt_inner)?);
                                    }
                                    _ => {}
                                }
                            }
                            if let (Some(k), Some(v)) = (key, value) {
                                options.insert(k, v);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(ExportClause { format, options })
    }

    fn build_find_clause(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<FindClause> {
        let mut distinct = false;
        let mut projections = Vec::new();

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::distinct_kw => {
                    distinct = true;
                }
                Rule::projection_list => {
                    for proj_pair in inner.into_inner() {
                        match proj_pair.as_rule() {
                            Rule::projection => {
                                projections.push(self.build_projection(proj_pair)?);
                            }
                            _ => {
                                // Wildcard * (cuando NO es projection)
                                // Esto captura el caso de "*" directamente
                                if proj_pair.as_str().trim() == "*" {
                                    projections.push(Projection::Wildcard);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Si no hay projections pero sí encontramos el clause, agregar wildcard
        if projections.is_empty() {
            projections.push(Projection::Wildcard);
        }

        Ok(FindClause {
            distinct,
            projections,
        })
    }

    fn build_projection(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<Projection> {
        let mut expression = None;
        let mut alias = None;

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::expression => {
                    expression = Some(self.build_expression(inner)?);
                }
                Rule::identifier => {
                    alias = Some(inner.as_str().to_string());
                }
                _ => {}
            }
        }

        Ok(Projection::Expression {
            expr: expression
                .ok_or_else(|| NopalError::QueryParseError("Invalid projection".into()))?,
            alias,
        })
    }

    fn build_from_clause(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<FromClause> {
        let mut patterns = Vec::new();

        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::pattern_list {
                for pattern_pair in inner.into_inner() {
                    if pattern_pair.as_rule() == Rule::pattern {
                        patterns.push(self.build_pattern(pattern_pair)?);
                    }
                }
            }
        }

        Ok(FromClause { patterns })
    }

    fn build_pattern(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<Pattern> {
        let mut elements = Vec::new();

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::node => {
                    elements.push(PatternElement::Node(self.build_node_pattern(inner)?));
                }
                Rule::relationship => {
                    elements.push(PatternElement::Relationship(
                        self.build_relationship_pattern(inner)?,
                    ));
                }
                _ => {}
            }
        }

        Ok(Pattern { elements })
    }

    fn build_node_pattern(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<NodePattern> {
        let mut variable = None;
        let mut label = None;
        let mut properties = std::collections::HashMap::new();

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::identifier => {
                    if variable.is_none() {
                        variable = Some(inner.as_str().to_string());
                    }
                }
                Rule::label_spec => {
                    for label_inner in inner.into_inner() {
                        if label_inner.as_rule() == Rule::identifier {
                            label = Some(label_inner.as_str().to_string());
                        }
                    }
                }
                Rule::property_map => {
                    for prop in inner.into_inner() {
                        if prop.as_rule() == Rule::property {
                            let mut key = None;
                            let mut value = None;
                            for prop_inner in prop.into_inner() {
                                match prop_inner.as_rule() {
                                    Rule::identifier => {
                                        if key.is_none() {
                                            key = Some(prop_inner.as_str().to_string());
                                        }
                                    }
                                    Rule::value => {
                                        value = Some(self.parse_property_value(prop_inner)?);
                                    }
                                    _ => {}
                                }
                            }
                            if let (Some(k), Some(v)) = (key, value) {
                                properties.insert(k, v);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(NodePattern {
            variable,
            label,
            properties,
        })
    }

    fn build_relationship_pattern(
        &mut self,
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<RelationshipPattern> {
        let mut arrows: Vec<String> = Vec::new();
        let mut variable = None;
        let mut rel_type = None;
        let mut properties = std::collections::HashMap::new();
        let mut quantifier: Option<Quantifier> = None;

        for inner in pair.into_inner() {
            match inner.as_rule() {
                // Simple arrow pattern: ->, <-, <->, --
                Rule::arrow_pattern => {
                    arrows.push(inner.as_str().to_string());
                }
                // Arrow with spec: -[...]->
                Rule::arrow_with_spec => {
                    let mut arrow_left = None;
                    let mut arrow_right = None;

                    for spec_part in inner.into_inner() {
                        match spec_part.as_rule() {
                            Rule::arrow_left => {
                                arrow_left = Some(spec_part.as_str().to_string());
                            }
                            Rule::arrow_right => {
                                arrow_right = Some(spec_part.as_str().to_string());
                            }
                            Rule::quantifier => {
                                quantifier = Some(build_quantifier(spec_part));
                            }
                            Rule::relationship_spec => {
                                // Detectar si el spec empieza con ':' (sin variable)
                                let spec_str = spec_part.as_str();
                                let has_colon_prefix = spec_str.starts_with(':');

                                let mut identifiers = Vec::new();

                                for spec_inner in spec_part.into_inner() {
                                    match spec_inner.as_rule() {
                                        Rule::identifier => {
                                            identifiers.push(spec_inner.as_str().to_string());
                                        }
                                        // Parsear property_map inline: {k: v, ...}
                                        Rule::property_map => {
                                            for prop in spec_inner.into_inner() {
                                                if prop.as_rule() == Rule::property {
                                                    let mut key = None;
                                                    let mut value = None;
                                                    for prop_inner in prop.into_inner() {
                                                        match prop_inner.as_rule() {
                                                            Rule::identifier => {
                                                                if key.is_none() {
                                                                    key = Some(
                                                                        prop_inner
                                                                            .as_str()
                                                                            .to_string(),
                                                                    );
                                                                }
                                                            }
                                                            Rule::value => {
                                                                value = Some(
                                                                    self.parse_property_value(
                                                                        prop_inner,
                                                                    )?,
                                                                );
                                                            }
                                                            _ => {}
                                                        }
                                                    }
                                                    if let (Some(k), Some(v)) = (key, value) {
                                                        properties.insert(k, v);
                                                    }
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }

                                // Asignar variable y tipo según el patrón detectado
                                if has_colon_prefix {
                                    // [:TYPE] o [:TYPE {props}]
                                    if !identifiers.is_empty() {
                                        rel_type = Some(identifiers[0].clone());
                                    }
                                } else if identifiers.len() >= 2 {
                                    // [variable:TYPE] o [variable:TYPE {props}]
                                    variable = Some(identifiers[0].clone());
                                    rel_type = Some(identifiers[1].clone());
                                } else if identifiers.len() == 1 {
                                    // [variable] - solo variable
                                    variable = Some(identifiers[0].clone());
                                }
                            }
                            _ => {}
                        }
                    }

                    // Combinar flechas para determinar dirección
                    if let (Some(left), Some(right)) = (arrow_left, arrow_right) {
                        let combined = format!("{}{}", left, right);
                        arrows.push(combined);
                    }
                }
                _ => {}
            }
        }

        // Determinar dirección desde las flechas acumuladas
        let direction = if !arrows.is_empty() {
            let arrow = &arrows[0];
            if arrow.contains("<-") && arrow.contains("->") {
                Direction::Bidirectional
            } else if arrow.ends_with("->") {
                Direction::Outgoing
            } else if arrow.starts_with("<-") {
                Direction::Incoming
            } else {
                Direction::Outgoing
            }
        } else {
            Direction::Outgoing
        };

        Ok(RelationshipPattern {
            variable,
            rel_type,
            direction,
            quantifier,
            properties,
        })
    }

    fn build_where_clause(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<WhereClause> {
        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::expression {
                return Ok(WhereClause {
                    condition: self.build_expression(inner)?,
                });
            }
        }

        Err(NopalError::QueryParseError("Invalid WHERE clause".into()))
    }

    fn build_group_by_clause(
        &mut self,
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<GroupByClause> {
        let mut expressions = Vec::new();

        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::expression_list {
                for expr_pair in inner.into_inner() {
                    if expr_pair.as_rule() == Rule::expression {
                        expressions.push(self.build_expression(expr_pair)?);
                    }
                }
            }
        }

        Ok(GroupByClause { expressions })
    }

    fn build_having_clause(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<HavingClause> {
        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::expression {
                return Ok(HavingClause {
                    condition: self.build_expression(inner)?,
                });
            }
        }

        Err(NopalError::QueryParseError("Invalid HAVING clause".into()))
    }

    fn build_order_by_clause(
        &mut self,
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<OrderByClause> {
        let mut items = Vec::new();

        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::order_item {
                items.push(self.build_order_by_item(inner)?);
            }
        }

        Ok(OrderByClause { items })
    }

    fn build_order_by_item(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<OrderByItem> {
        let mut expression = None;
        let mut order = SortOrder::Asc; // default

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::expression => {
                    expression = Some(self.build_expression(inner)?);
                }
                Rule::order_direction => {
                    let direction = inner.as_str().to_lowercase();
                    order = if direction == "desc" {
                        SortOrder::Desc
                    } else {
                        SortOrder::Asc
                    };
                }
                _ => {}
            }
        }

        Ok(OrderByItem {
            expression: expression.ok_or_else(|| {
                NopalError::QueryParseError("Missing expression in ORDER BY".into())
            })?,
            order,
        })
    }

    fn build_limit_clause(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<LimitClause> {
        let mut limit = 0;
        let mut offset = None;

        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::number {
                let num: usize = inner
                    .as_str()
                    .parse()
                    .map_err(|_| NopalError::QueryParseError("Invalid number in LIMIT".into()))?;

                if limit == 0 {
                    limit = num;
                } else {
                    offset = Some(num);
                }
            }
        }

        Ok(LimitClause { limit, offset })
    }

    fn build_time_clause(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<TimeTravelClause> {
        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::number {
                let timestamp: u64 = inner
                    .as_str()
                    .parse()
                    .map_err(|_| NopalError::QueryParseError("Invalid timestamp".into()))?;

                return Ok(TimeTravelClause { timestamp });
            }
        }

        Err(NopalError::QueryParseError("Invalid TIME clause".into()))
    }

    fn build_expression(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<Expression> {
        self.build_or_expression(pair)
    }

    fn build_or_expression(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<Expression> {
        let mut children: Vec<pest::iterators::Pair<Rule>> = pair
            .into_inner()
            .filter(|p| matches!(p.as_rule(), Rule::and_expression | Rule::or_expression))
            .collect();

        if children.is_empty() {
            return Err(NopalError::QueryParseError("Invalid expression".into()));
        }

        // Build first child
        let first = children.remove(0);
        let mut result = match first.as_rule() {
            Rule::and_expression => self.build_and_expression(first),
            Rule::or_expression => self.build_or_expression(first),
            _ => self.build_and_expression(first),
        }?;

        // Chain remaining children with OR
        for child in children {
            let right = match child.as_rule() {
                Rule::and_expression => self.build_and_expression(child),
                _ => self.build_and_expression(child),
            }?;
            result = Expression::BinaryOp {
                left: Box::new(result),
                op: BinaryOperator::Or,
                right: Box::new(right),
            };
        }

        Ok(result)
    }

    fn build_and_expression(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<Expression> {
        let mut children: Vec<pest::iterators::Pair<Rule>> = pair
            .into_inner()
            .filter(|p| p.as_rule() == Rule::comparison_expression)
            .collect();

        if children.is_empty() {
            return Err(NopalError::QueryParseError("Invalid expression".into()));
        }

        // Build first child
        let first = children.remove(0);
        let mut result = self.build_comparison_expression(first)?;

        // Chain remaining children with AND
        for child in children {
            let right = self.build_comparison_expression(child)?;
            result = Expression::BinaryOp {
                left: Box::new(result),
                op: BinaryOperator::And,
                right: Box::new(right),
            };
        }

        Ok(result)
    }

    fn build_comparison_expression(
        &mut self,
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Expression> {
        let mut left = None;
        let mut op = None;
        let mut right = None;

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::additive_expression => {
                    if left.is_none() {
                        left = Some(self.build_primary_from_additive(inner)?);
                    } else {
                        right = Some(self.build_primary_from_additive(inner)?);
                    }
                }
                Rule::comparison_op => {
                    op = Some(self.parse_comparison_op(inner.as_str())?);
                }
                _ => {}
            }
        }

        // ✅ FIX: Clone left antes de moverlo
        match (left.clone(), op, right) {
            (Some(l), Some(o), Some(r)) => Ok(Expression::BinaryOp {
                left: Box::new(l),
                op: o,
                right: Box::new(r),
            }),
            (Some(l), None, None) => Ok(l),
            _ => Err(NopalError::QueryParseError(
                "Invalid comparison expression".into(),
            )),
        }
    }

    fn build_primary_from_additive(
        &mut self,
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Expression> {
        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::multiplicative_expression {
                return self.build_primary_from_multiplicative(inner);
            }
        }
        Err(NopalError::QueryParseError("Invalid expression".into()))
    }

    fn build_primary_from_multiplicative(
        &mut self,
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Expression> {
        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::unary_expression {
                return self.build_primary_from_unary(inner);
            }
        }
        Err(NopalError::QueryParseError("Invalid expression".into()))
    }

    fn build_primary_from_unary(
        &mut self,
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Expression> {
        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::primary_expression {
                return self.build_primary_expression(inner);
            }
        }
        Err(NopalError::QueryParseError("Invalid expression".into()))
    }

    fn build_primary_expression(
        &mut self,
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<Expression> {
        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::property_access => {
                    return self.build_property_access(inner);
                }
                Rule::value => {
                    return self.build_value_expression(inner);
                }
                Rule::function_call => {
                    return self.build_function_call(inner);
                }
                _ => {}
            }
        }

        Err(NopalError::QueryParseError(
            "Invalid primary expression".into(),
        ))
    }

    fn build_property_access(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<Expression> {
        let parts: Vec<String> = pair
            .into_inner()
            .filter(|p| p.as_rule() == Rule::identifier)
            .map(|p| p.as_str().to_string())
            .collect();

        if parts.len() == 2 {
            Ok(Expression::Property {
                variable: parts[0].clone(),
                property: parts[1].clone(),
            })
        } else if parts.len() == 1 {
            // Just a variable reference
            Ok(Expression::Property {
                variable: parts[0].clone(),
                property: String::new(),
            })
        } else {
            Err(NopalError::QueryParseError(
                "Invalid property access".into(),
            ))
        }
    }

    fn build_value_expression(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<Expression> {
        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::string => {
                    let s = inner.as_str();
                    let s = Self::unquote_string(s);
                    return Ok(Expression::Literal(PropertyValue::String(s)));
                }
                Rule::number => {
                    let num_str = inner.as_str();
                    if num_str.contains('.') {
                        let f: f64 = num_str
                            .parse()
                            .map_err(|_| NopalError::QueryParseError("Invalid float".into()))?;
                        return Ok(Expression::Literal(PropertyValue::Float(f)));
                    } else {
                        let i: i64 = num_str
                            .parse()
                            .map_err(|_| NopalError::QueryParseError("Invalid integer".into()))?;
                        return Ok(Expression::Literal(PropertyValue::Int(i)));
                    }
                }
                Rule::boolean => {
                    let b = inner.as_str().to_lowercase() == "true";
                    return Ok(Expression::Literal(PropertyValue::Bool(b)));
                }
                Rule::null => {
                    return Ok(Expression::Literal(PropertyValue::Null));
                }
                _ => {}
            }
        }

        Err(NopalError::QueryParseError("Invalid value".into()))
    }

    fn build_function_call(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<Expression> {
        let mut name = None;
        let mut args = Vec::new();

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::identifier => {
                    if name.is_none() {
                        name = Some(inner.as_str().to_string());
                    }
                }
                Rule::function_arg => {
                    // function_arg = { "*" | expression }
                    // Check if it's wildcard directly
                    if inner.as_str().trim() == "*" {
                        args.push(Expression::Wildcard);
                    } else {
                        // Extract expression from inside function_arg
                        if let Some(expr_pair) = inner.into_inner().next()
                            && expr_pair.as_rule() == Rule::expression
                        {
                            args.push(self.build_expression(expr_pair)?);
                        }
                    }
                }
                // Legacy support or direct wildcard matching if previous grammar logic persists
                Rule::expression => {
                    args.push(self.build_expression(inner)?);
                }
                _ => {}
            }
        }

        Ok(Expression::FunctionCall {
            name: name
                .ok_or_else(|| NopalError::QueryParseError("Missing function name".into()))?,
            args,
        })
    }

    fn parse_comparison_op(&self, op: &str) -> Result<BinaryOperator> {
        match op {
            "=" => Ok(BinaryOperator::Eq),
            "!=" => Ok(BinaryOperator::NotEq),
            "<" => Ok(BinaryOperator::Lt),
            ">" => Ok(BinaryOperator::Gt),
            "<=" => Ok(BinaryOperator::LtEq),
            ">=" => Ok(BinaryOperator::GtEq),
            _ => Err(NopalError::QueryParseError(format!(
                "Unknown operator: {}",
                op
            ))),
        }
    }

    /// Parse a value to PropertyValue
    fn parse_property_value(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<PropertyValue> {
        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::string => {
                    let s = inner.as_str();
                    let s = Self::unquote_string(s);
                    return Ok(PropertyValue::String(s));
                }
                Rule::number => {
                    let num_str = inner.as_str();
                    if num_str.contains('.') {
                        let f: f64 = num_str
                            .parse()
                            .map_err(|_| NopalError::QueryParseError("Invalid float".into()))?;
                        return Ok(PropertyValue::Float(f));
                    } else {
                        let i: i64 = num_str
                            .parse()
                            .map_err(|_| NopalError::QueryParseError("Invalid integer".into()))?;
                        return Ok(PropertyValue::Int(i));
                    }
                }
                Rule::boolean => {
                    let b = inner.as_str().to_lowercase() == "true";
                    return Ok(PropertyValue::Bool(b));
                }
                Rule::null => {
                    return Ok(PropertyValue::Null);
                }
                _ => {}
            }
        }
        Err(NopalError::QueryParseError("Invalid property value".into()))
    }

    fn build_create_index_stmt(
        &mut self,
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<CreateIndexStmt> {
        let mut label = None;
        let mut property = None;
        let mut index_type = IndexType::Hash; // default

        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::identifier => {
                    if label.is_none() {
                        label = Some(inner.as_str().to_string());
                    } else if property.is_none() {
                        property = Some(inner.as_str().to_string());
                    }
                }
                Rule::index_type_spec => {
                    // Iterate over the named index_type_keyword sub-rule.
                    for type_inner in inner.into_inner() {
                        if type_inner.as_rule() == Rule::index_type_keyword {
                            let type_str = type_inner.as_str().to_lowercase();
                            index_type = match type_str.as_str() {
                                "hash" => IndexType::Hash,
                                "btree" => IndexType::BTree,
                                "fulltext" => IndexType::FullText,
                                "taxonomy" => IndexType::Taxonomy,
                                _ => IndexType::Hash,
                            };
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(CreateIndexStmt {
            label: label.ok_or_else(|| {
                NopalError::QueryParseError("Missing label in CREATE INDEX".into())
            })?,
            property: property.ok_or_else(|| {
                NopalError::QueryParseError("Missing property in CREATE INDEX".into())
            })?,
            index_type,
        })
    }

    fn unquote_string(input: &str) -> String {
        let bytes = input.as_bytes();
        if bytes.len() >= 2 {
            let first = bytes[0];
            let last = bytes[bytes.len() - 1];
            if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
                return input[1..bytes.len() - 1].to_string();
            }
        }
        input.to_string()
    }

    fn build_drop_index_stmt(
        &mut self,
        pair: pest::iterators::Pair<Rule>,
    ) -> Result<DropIndexStmt> {
        let mut index_name = None;

        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::identifier {
                index_name = Some(inner.as_str().to_string());
                break;
            }
        }

        Ok(DropIndexStmt {
            index_name: index_name.ok_or_else(|| {
                NopalError::QueryParseError("Missing index name in DROP INDEX".into())
            })?,
        })
    }

    fn build_explain_stmt(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<Statement> {
        for inner in pair.into_inner() {
            match inner.as_rule() {
                Rule::query => {
                    let query = self.build_query(inner)?;
                    return Ok(Statement::Explain(Box::new(Statement::Query(query))));
                }
                Rule::delete_stmt => {
                    let delete = self.build_delete_stmt(inner)?;
                    return Ok(Statement::Explain(Box::new(Statement::Delete(delete))));
                }
                Rule::update_stmt => {
                    let update = self.build_update_stmt(inner)?;
                    return Ok(Statement::Explain(Box::new(Statement::Update(update))));
                }
                Rule::add_stmt => {
                    let add = self.build_add_stmt(inner)?;
                    return Ok(Statement::Explain(Box::new(Statement::Add(add))));
                }
                _ => {}
            }
        }

        Err(NopalError::QueryParseError(
            "EXPLAIN requires a statement".into(),
        ))
    }

    fn build_profile_stmt(&mut self, pair: pest::iterators::Pair<Rule>) -> Result<Statement> {
        for inner in pair.into_inner() {
            if inner.as_rule() == Rule::query {
                let query = self.build_query(inner)?;
                return Ok(Statement::Profile(Box::new(Statement::Query(query))));
            }
        }

        Err(NopalError::QueryParseError(
            "PROFILE requires a query".into(),
        ))
    }
}

/// Convierte un par `Rule::quantifier` en un `Quantifier`.
/// Casos:
///   `{n}`   → min=n, max=Some(n)  (exact)
///   `{n,m}` → min=n, max=Some(m)  (bounded range)
///   `{n,}`  → min=n, max=None     (unbounded — F1 rechaza en el executor)
fn build_quantifier(pair: pest::iterators::Pair<Rule>) -> Quantifier {
    let raw = pair.as_str();
    let has_comma = raw.contains(',');

    let numbers: Vec<usize> = pair
        .into_inner()
        .filter(|p| p.as_rule() == Rule::number)
        .filter_map(|p| p.as_str().parse().ok())
        .collect();

    match (has_comma, numbers.as_slice()) {
        (false, [n]) => Quantifier {
            min: *n,
            max: Some(*n),
        },
        (true, [n, m]) => Quantifier {
            min: *n,
            max: Some(*m),
        },
        (true, [n]) => Quantifier { min: *n, max: None },
        _ => Quantifier {
            min: 1,
            max: Some(1),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_query() {
        let query = r#"
            find p.name, p.age
            from (p:Person)
            where p.age > 25
            limit 10
        "#;

        let stmt = parse(query).unwrap();

        // parse() ahora retorna Statement::Query
        if let Statement::Query(ast) = stmt {
            assert_eq!(ast.find.projections.len(), 2);
            assert_eq!(ast.from.patterns.len(), 1);
            assert!(ast.filter.is_some());
            assert!(ast.limit.is_some());
        } else {
            panic!("Expected Query statement");
        }
    }

    #[test]
    fn test_pattern_matching() {
        let query = r#"
            find a.name, b.name
            from (a:Person)-[:KNOWS]->(b:Person)
        "#;

        let stmt = parse(query).unwrap();

        if let Statement::Query(ast) = stmt {
            assert_eq!(ast.from.patterns.len(), 1);
            let pattern = &ast.from.patterns[0];
            assert_eq!(pattern.elements.len(), 3); // node, rel, node
        } else {
            panic!("Expected Query statement");
        }
    }

    #[test]
    fn test_time_travel() {
        let query = r#"
            find n.name
            from (n:Person)
            at timestamp 1234567890
        "#;

        let stmt = parse(query).unwrap();

        if let Statement::Query(ast) = stmt {
            assert!(ast.time_travel.is_some());
            assert_eq!(ast.time_travel.unwrap().timestamp, 1234567890);
        } else {
            panic!("Expected Query statement");
        }
    }

    #[test]
    fn test_pattern_matching_untyped() {
        // Test untyped relationships (no brackets) per documentation
        let query = r#"
            find src.node_id, tgt.node_id
            from (src:Entity) -> (tgt:Entity)
        "#;

        let stmt = parse(query).unwrap();

        if let Statement::Query(ast) = stmt {
            assert_eq!(ast.from.patterns.len(), 1);
            let pattern = &ast.from.patterns[0];
            assert_eq!(pattern.elements.len(), 3); // node, rel, node
        } else {
            panic!("Expected Query statement");
        }
    }

    #[test]
    fn test_parse_query_directly() {
        // Test parse_query() which returns Query directly
        let query = r#"
            find p.name
            from (p:Person)
        "#;

        let ast = parse_query(query).unwrap();
        assert_eq!(ast.find.projections.len(), 1);
        assert_eq!(ast.from.patterns.len(), 1);
    }

    #[test]
    fn test_quantifier_exact_parsed() {
        let query = r#"
            find b.name
            from (a:Person) -[:KNOWS]->{2} (b:Person)
        "#;

        let stmt = parse(query).unwrap();

        if let Statement::Query(ast) = stmt {
            let pattern = &ast.from.patterns[0];
            // elements: [node, rel, node]
            assert_eq!(pattern.elements.len(), 3, "Expected 3 pattern elements");
            if let PatternElement::Relationship(rel) = &pattern.elements[1] {
                let q = rel
                    .quantifier
                    .as_ref()
                    .expect("Quantifier must be Some({2})");
                assert_eq!(q.min, 2, "min should be 2");
                assert_eq!(q.max, Some(2), "max should be Some(2)");
            } else {
                panic!("Expected Relationship at index 1");
            }
        } else {
            panic!("Expected Query statement");
        }
    }

    #[test]
    fn test_quantifier_range_parsed() {
        let query = r#"
            find b.name
            from (a:Person) -[:KNOWS]->{1,3} (b:Person)
        "#;

        let stmt = parse(query).unwrap();

        if let Statement::Query(ast) = stmt {
            let pattern = &ast.from.patterns[0];
            if let PatternElement::Relationship(rel) = &pattern.elements[1] {
                let q = rel
                    .quantifier
                    .as_ref()
                    .expect("Quantifier must be Some({1,3})");
                assert_eq!(q.min, 1);
                assert_eq!(q.max, Some(3));
            } else {
                panic!("Expected Relationship");
            }
        } else {
            panic!("Expected Query");
        }
    }

    #[test]
    fn test_quantifier_unbounded_parsed() {
        let query = r#"
            find b.name
            from (a:Person) -[:KNOWS]->{1,} (b:Person)
        "#;

        let stmt = parse(query).unwrap();

        if let Statement::Query(ast) = stmt {
            let pattern = &ast.from.patterns[0];
            if let PatternElement::Relationship(rel) = &pattern.elements[1] {
                let q = rel
                    .quantifier
                    .as_ref()
                    .expect("Quantifier must be Some({1,})");
                assert_eq!(q.min, 1);
                assert_eq!(q.max, None, "Unbounded max should be None");
            } else {
                panic!("Expected Relationship");
            }
        } else {
            panic!("Expected Query");
        }
    }

    #[test]
    fn test_profile_statement_parsed() {
        let stmt = parse("profile find a.name from (a:Person)").expect("PROFILE should parse");

        match stmt {
            Statement::Profile(inner) => match *inner {
                Statement::Query(query) => {
                    assert_eq!(query.find.projections.len(), 1);
                    assert_eq!(query.from.patterns.len(), 1);
                }
                other => panic!("Expected Statement::Query inside PROFILE, got {:?}", other),
            },
            other => panic!("Expected Statement::Profile, got {:?}", other),
        }
    }

    #[test]
    fn test_init_and_gather_clauses_parsed() {
        let stmt = parse(
            r#"
            find b.name, path_eval("sum") as total
            from (a:Account)-[:TRANSFER]->{1,3}(b:Account)
            where path_eval("sum") > 100
            init "sum = 0"
            gather "sum = sum + edge.amount"
        "#,
        )
        .expect("F4-B query should parse");

        match stmt {
            Statement::Query(query) => {
                assert_eq!(query.init, vec!["sum = 0".to_string()]);
                assert_eq!(query.gather, vec!["sum = sum + edge.amount".to_string()]);
            }
            other => panic!("Expected Query, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_vm_assignment_arithmetic() {
        let assignment =
            parse_vm_assignment("sum = sum + edge.amount").expect("VM assignment should parse");

        assert_eq!(assignment.variable, "sum");
        match assignment.expr {
            Expression::BinaryOp {
                op: BinaryOperator::Add,
                ..
            } => {}
            other => panic!("Expected additive VM expression, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_vm_expression_boolean_and_path_depth() {
        let expr = parse_vm_expression("path.depth > 2").expect("VM expression should parse");

        match expr {
            Expression::BinaryOp {
                op: BinaryOperator::Gt,
                ..
            } => {}
            other => panic!("Expected comparison VM expression, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_vm_expression_boolean_composition() {
        let expr =
            parse_vm_expression("risky or path.depth > 2").expect("VM OR expression should parse");
        match expr {
            Expression::BinaryOp {
                op: BinaryOperator::Or,
                ..
            } => {}
            other => panic!("Expected OR VM expression, got {:?}", other),
        }

        let expr = parse_vm_expression("risky and edge.amount > 10")
            .expect("VM AND expression should parse");
        match expr {
            Expression::BinaryOp {
                op: BinaryOperator::And,
                ..
            } => {}
            other => panic!("Expected AND VM expression, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_vm_expression_unary_not_forms() {
        let expr = parse_vm_expression("!risky").expect("Unary bang NOT should parse");
        match expr {
            Expression::UnaryOp {
                op: UnaryOperator::Not,
                ..
            } => {}
            other => panic!("Expected unary NOT expression, got {:?}", other),
        }

        let expr = parse_vm_expression("not risky").expect("Unary keyword NOT should parse");
        match expr {
            Expression::UnaryOp {
                op: UnaryOperator::Not,
                ..
            } => {}
            other => panic!("Expected unary NOT expression, got {:?}", other),
        }
    }

    // ─── F4-C parser tests ───────────────────────────────────────────────────

    #[test]
    fn test_return_clause_parsed() {
        let stmt = parse(
            r#"find b.name from (a:Account)-[:TRANSFER]->(b:Account) init "sum = 0" gather "sum = sum + edge.amount" return "sum""#
        ).expect("Query with RETURN should parse");

        if let Statement::Query(q) = stmt {
            assert_eq!(q.return_expr, Some("sum".to_string()));
        } else {
            panic!("Expected Query");
        }
    }

    #[test]
    fn test_no_return_clause_is_none() {
        let stmt = parse(r#"find b.name from (a:Account)-[:TRANSFER]->(b:Account)"#)
            .expect("Query without RETURN should parse");

        if let Statement::Query(q) = stmt {
            assert_eq!(q.return_expr, None);
        } else {
            panic!("Expected Query");
        }
    }

    #[test]
    fn test_return_clause_roundtrip() {
        let stmt = parse(
            r#"find b.name, path.result as score from (a:Account)-[:TRANSFER]->{1,3}(b:Account) where path.result > 100 init "sum = 0" gather "sum = sum + edge.amount" return "sum * path.depth""#
        ).expect("Full F4-C query should parse");

        if let Statement::Query(q) = stmt {
            assert_eq!(q.return_expr, Some("sum * path.depth".to_string()));
            assert_eq!(q.init, vec!["sum = 0"]);
            assert_eq!(q.gather, vec!["sum = sum + edge.amount"]);
        } else {
            panic!("Expected Query");
        }
    }
}
