// src/query/nql/parser/ast.rs
//
// Abstract Syntax Tree for NQL v0.2
//
// Major changes from v0.1:
// - Statement enum wrapper (Query/Sketch/Commit/Delete/Update/Add)
// - HAVING clause support
// - Extended Projection (*, all(), expressions)
// - SortOrder enum (Asc/Desc)
// - SKETCH/COMMIT for conceptual operations
// - DELETE/UPDATE/ADD statements

use crate::types::PropertyValue;
use std::collections::HashMap;

// ═══════════════════════════════════════════════════════════════
// TOP-LEVEL STATEMENT
// ═══════════════════════════════════════════════════════════════

/// Top-level NQL statement
///
/// NQL v0.2 supports multiple statement types:
/// - Query: Read operations (FIND ... FROM ...)
/// - Sketch: Conceptual operations (preview before execution)
/// - Commit: Execute a previously defined sketch
/// - Delete: Remove nodes/edges from the graph
/// - Update: Modify properties of nodes/edges
/// - Create: Add new nodes/edges to the graph
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum Statement {
    /// Read query (FIND ... FROM ...)
    Query(Query),

    /// Sketch definition (SKETCH name = statement)
    Sketch(SketchStmt),

    /// Commit a sketch (COMMIT sketch_name)
    Commit(CommitStmt),

    /// Delete nodes/edges (DELETE pattern WHERE ...)
    Delete(DeleteStmt),

    /// Update properties (UPDATE pattern SET ... WHERE ...)
    Update(UpdateStmt),

    /// Add nodes/edges (Add pattern)
    Add(AddStmt),

    CreateIndex(CreateIndexStmt),
    DropIndex(DropIndexStmt),
    Explain(Box<Statement>),
    Profile(Box<Statement>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct CreateIndexStmt {
    pub label: String,
    pub property: String,
    pub index_type: IndexType,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IndexType {
    Hash,
    BTree,
    FullText,
    Taxonomy,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DropIndexStmt {
    pub index_name: String,
}

// ═══════════════════════════════════════════════════════════════
// QUERY (READ) - Extended from v0.1
// ═══════════════════════════════════════════════════════════════

/// Query statement (read-only)
///
/// Syntax:
/// ```nql
/// [EXPORT arrow | parquet "file.parquet"]
/// FIND <projections>
/// FROM <pattern>
/// [WHERE <condition>]
/// [GROUP BY <expressions>]
/// [HAVING <condition>]
/// [ORDER BY <expression> [ASC|DESC], ...]
/// [LIMIT <n> [OFFSET <m>]]
/// [AT <timestamp>]
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub export: Option<ExportClause>,
    pub find: FindClause,
    pub from: FromClause,
    pub filter: Option<WhereClause>,
    pub init: Vec<String>,
    pub gather: Vec<String>,
    pub return_expr: Option<String>, // F4-C: return "..." evaluated once per path
    pub group_by: Option<GroupByClause>,
    pub having: Option<HavingClause>, // NEW in v0.2
    pub order_by: Option<OrderByClause>,
    pub limit: Option<LimitClause>,
    pub time_travel: Option<TimeTravelClause>,
}

// ═══════════════════════════════════════════════════════════════
// SKETCH/COMMIT (Conceptual Operations)
// ═══════════════════════════════════════════════════════════════

/// Sketch statement - define operation without executing
///
/// Syntax:
/// ```nql
/// SKETCH <name> = <statement>
/// ```
///
/// Example:
/// ```nql
/// sketch cleanup =
///   delete (u:User)
///   where u.last_login < timestamp("2020-01-01")
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct SketchStmt {
    pub name: String,
    pub operation: Box<Statement>,
    pub description: Option<String>,
}

/// Commit statement - execute a sketch
///
/// Syntax:
/// ```nql
/// COMMIT <sketch_name>
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct CommitStmt {
    pub sketch_name: String,
}

// ═══════════════════════════════════════════════════════════════
// WRITE STATEMENTS (NEW in v0.2)
// ═══════════════════════════════════════════════════════════════

/// Delete statement - remove nodes/edges
///
/// Syntax:
/// ```nql
/// DELETE <pattern>
/// [WHERE <condition>]
/// [LIMIT <n>]
/// ```
///
/// Example:
/// ```nql
/// delete (u:User)
/// where u.last_login < timestamp("2020-01-01")
/// limit 1000
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct DeleteStmt {
    pub pattern: Pattern,
    pub filter: Option<WhereClause>,
    pub limit: Option<LimitClause>,
}

/// Update statement - modify properties
///
/// Syntax:
/// ```nql
/// UPDATE <pattern>
/// SET <variable>.<property> = <value>, ...
/// [WHERE <condition>]
/// [LIMIT <n>]
/// ```
///
/// Example:
/// ```nql
/// update (u:User)
/// set u.verified = true, u.verified_at = now()
/// where u.email_confirmed = true
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateStmt {
    pub pattern: Pattern,
    pub assignments: Vec<Assignment>,
    pub filter: Option<WhereClause>,
    pub limit: Option<LimitClause>,
}

/// Assignment in UPDATE statement
#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    pub variable: String,
    pub property: String,
    pub value: Expression,
}

/// Create statement - add nodes/edges
///
/// Syntax:
/// ```nql
/// CREATE <pattern>
/// ```
///
/// Example:
/// ```nql
/// create (p:Person {name: "Alice", age: 30})
/// create (a:Person)-[:KNOWS {since: 2020}]->(b:Person)
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct AddStmt {
    pub pattern: Pattern,
}

// ═══════════════════════════════════════════════════════════════
// CLAUSES
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq)]
pub struct ExportClause {
    pub format: ExportFormat,
    pub options: HashMap<String, PropertyValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExportFormat {
    Arrow,
    Csv,
    Json,
    Parquet(String), // file path
}

/// FIND clause - specify what to return
///
/// Supports:
/// - Wildcard: `find *`
/// - All function: `find all(p)`
/// - Property access: `find p.name, p.age`
/// - Expressions with alias: `find count(*) as total`
#[derive(Debug, Clone, PartialEq)]
pub struct FindClause {
    pub distinct: bool,
    pub projections: Vec<Projection>,
}

/// Projection in FIND clause
///
/// v0.2 supports multiple projection types for flexibility:
/// - Wildcard (*): All properties of all variables
/// - All(var): All properties of specific variable
/// - Expression: Property access, function call, etc.
#[derive(Debug, Clone, PartialEq)]
pub enum Projection {
    /// Wildcard (*) - all properties of all variables
    ///
    /// Example: `find *`
    Wildcard,

    /// All(variable) - all properties of specific variable
    ///
    /// Example: `find all(p), all(r)`
    All(String),

    /// Expression with optional alias
    ///
    /// Examples:
    /// - `find p.name`
    /// - `find count(*) as total`
    /// - `find p.age + 1 as next_age`
    Expression {
        expr: Expression,
        alias: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct FromClause {
    pub patterns: Vec<Pattern>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pattern {
    pub elements: Vec<PatternElement>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PatternElement {
    Node(NodePattern),
    Relationship(RelationshipPattern),
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodePattern {
    pub variable: Option<String>,
    pub label: Option<String>,
    pub properties: HashMap<String, PropertyValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RelationshipPattern {
    pub variable: Option<String>,
    pub rel_type: Option<String>,
    pub direction: Direction,
    pub quantifier: Option<Quantifier>,
    /// Filtros inline sobre propiedades de la arista, ej. -[r:TRANS {amount: 1000}]->
    pub properties: HashMap<String, PropertyValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Direction {
    Outgoing,      // ->
    Incoming,      // <-
    Bidirectional, // <->
}

#[derive(Debug, Clone, PartialEq)]
pub struct Quantifier {
    pub min: usize,
    pub max: Option<usize>, // None = unbounded
}

#[derive(Debug, Clone, PartialEq)]
pub struct VmAssignment {
    pub variable: String,
    pub expr: Expression,
}

/// WHERE clause - pre-aggregation filter
///
/// Applied BEFORE GROUP BY
#[derive(Debug, Clone, PartialEq)]
pub struct WhereClause {
    pub condition: Expression,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GroupByClause {
    pub expressions: Vec<Expression>,
}

/// HAVING clause - post-aggregation filter (NEW in v0.2)
///
/// Applied AFTER GROUP BY
///
/// Requirements:
/// - Can only be used with GROUP BY
/// - Can reference:
///   - Aggregation functions (count, sum, avg, min, max)
///   - Columns in GROUP BY clause
///
/// Example:
/// ```nql
/// find p.city, count(*) as total
/// from (p:Person)
/// group by p.city
/// having count(*) > 1000
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct HavingClause {
    pub condition: Expression,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrderByClause {
    pub items: Vec<OrderByItem>,
}

/// ORDER BY item with explicit sort order (improved in v0.2)
///
/// v0.1 used `descending: bool`
/// v0.2 uses `order: SortOrder` for clarity
#[derive(Debug, Clone, PartialEq)]
pub struct OrderByItem {
    pub expression: Expression,
    pub order: SortOrder,
}

/// Sort order for ORDER BY
///
/// Syntax:
/// ```nql
/// order by p.age asc
/// order by p.age desc
/// order by p.age        -- default: Asc
/// ```
#[derive(Debug, Clone, PartialEq, Default)]
pub enum SortOrder {
    #[default]
    Asc, // Ascending (default)
    Desc, // Descending
}

#[derive(Debug, Clone, PartialEq)]
pub struct LimitClause {
    pub limit: usize,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimeTravelClause {
    pub timestamp: u64,
}

// ═══════════════════════════════════════════════════════════════
// EXPRESSIONS
// ═══════════════════════════════════════════════════════════════

#[derive(Debug, Clone, PartialEq)]
pub enum Expression {
    // Literals
    Literal(PropertyValue),

    // Property access: node.property
    Property {
        variable: String,
        property: String,
    },

    // Binary operations
    BinaryOp {
        left: Box<Expression>,
        op: BinaryOperator,
        right: Box<Expression>,
    },

    // Unary operations
    UnaryOp {
        op: UnaryOperator,
        expr: Box<Expression>,
    },

    // Function call
    FunctionCall {
        name: String,
        args: Vec<Expression>,
    },

    // Wildcard * (used in function arguments like count(*))
    Wildcard,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BinaryOperator {
    // Comparison
    Eq,    // =
    NotEq, // !=
    Lt,    // <
    Gt,    // >
    LtEq,  // <=
    GtEq,  // >=

    // Logical
    And,
    Or,

    // Arithmetic
    Add, // +
    Sub, // -
    Mul, // *
    Div, // /
    Mod, // %
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UnaryOperator {
    Not, // !
    Neg, // -
}

// ═══════════════════════════════════════════════════════════════
// HELPERS & IMPLEMENTATIONS
// ═══════════════════════════════════════════════════════════════

impl Statement {
    /// Returns true if statement is read-only
    pub fn is_read_only(&self) -> bool {
        matches!(
            self,
            Statement::Query(_)
                | Statement::Sketch(_)
                | Statement::Explain(_)
                | Statement::Profile(_)
        )
    }

    /// Returns true if statement modifies the graph
    pub fn is_write(&self) -> bool {
        matches!(
            self,
            Statement::Delete(_) | Statement::Update(_) | Statement::Add(_) | Statement::Commit(_)
        )
    }

    /// Returns true if statement requires confirmation
    pub fn requires_confirmation(&self) -> bool {
        match self {
            Statement::Delete(del) => del.filter.is_none() && del.limit.is_none(),
            Statement::Update(upd) => upd.filter.is_none() && upd.limit.is_none(),
            Statement::Commit(_) => true,
            _ => false,
        }
    }
}

impl Query {
    /// Returns true if query is read-only (always true for Query)
    pub fn is_read_only(&self) -> bool {
        true
    }

    /// Returns true if query requires time travel
    pub fn requires_time_travel(&self) -> bool {
        self.time_travel.is_some()
    }

    /// Returns true if query exports results
    pub fn is_export(&self) -> bool {
        self.export.is_some()
    }

    /// Returns true if query has aggregations
    pub fn has_aggregations(&self) -> bool {
        self.group_by.is_some()
    }

    /// Returns true if query has HAVING clause
    pub fn has_having(&self) -> bool {
        self.having.is_some()
    }
}

impl SketchStmt {
    /// Create a new sketch
    pub fn new(name: String, operation: Statement) -> Self {
        Self {
            name,
            operation: Box::new(operation),
            description: None,
        }
    }

    /// Create a sketch with description
    pub fn with_description(name: String, operation: Statement, description: String) -> Self {
        Self {
            name,
            operation: Box::new(operation),
            description: Some(description),
        }
    }
}

impl DeleteStmt {
    /// Returns estimated danger level
    /// - High: No WHERE, no LIMIT (deletes everything)
    /// - Medium: No WHERE but has LIMIT
    /// - Low: Has WHERE
    pub fn danger_level(&self) -> DangerLevel {
        match (&self.filter, &self.limit) {
            (None, None) => DangerLevel::High,
            (None, Some(_)) => DangerLevel::Medium,
            (Some(_), _) => DangerLevel::Low,
        }
    }
}

impl UpdateStmt {
    /// Returns estimated danger level
    pub fn danger_level(&self) -> DangerLevel {
        match (&self.filter, &self.limit) {
            (None, None) => DangerLevel::High,
            (None, Some(_)) => DangerLevel::Medium,
            (Some(_), _) => DangerLevel::Low,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DangerLevel {
    Low,    // Has WHERE clause
    Medium, // No WHERE but has LIMIT
    High,   // No WHERE, no LIMIT - affects ALL nodes
}

impl Pattern {
    /// Extract all variable names from pattern
    pub fn variables(&self) -> Vec<String> {
        let mut vars = Vec::new();

        for element in &self.elements {
            match element {
                PatternElement::Node(node) => {
                    if let Some(var) = &node.variable {
                        vars.push(var.clone());
                    }
                }
                PatternElement::Relationship(rel) => {
                    if let Some(var) = &rel.variable {
                        vars.push(var.clone());
                    }
                }
            }
        }

        vars
    }

    /// Returns true if pattern has any nodes
    pub fn has_nodes(&self) -> bool {
        self.elements
            .iter()
            .any(|e| matches!(e, PatternElement::Node(_)))
    }

    /// Returns true if pattern has any relationships
    pub fn has_relationships(&self) -> bool {
        self.elements
            .iter()
            .any(|e| matches!(e, PatternElement::Relationship(_)))
    }
}

impl Projection {
    /// Create a wildcard projection
    pub fn wildcard() -> Self {
        Projection::Wildcard
    }

    /// Create an all(var) projection
    pub fn all(variable: impl Into<String>) -> Self {
        Projection::All(variable.into())
    }

    /// Create a property projection
    pub fn property(variable: impl Into<String>, property: impl Into<String>) -> Self {
        Projection::Expression {
            expr: Expression::property_access(&variable.into(), &property.into()),
            alias: None,
        }
    }

    /// Create a property projection with alias
    pub fn property_as(
        variable: impl Into<String>,
        property: impl Into<String>,
        alias: impl Into<String>,
    ) -> Self {
        Projection::Expression {
            expr: Expression::property_access(&variable.into(), &property.into()),
            alias: Some(alias.into()),
        }
    }
}

impl Expression {
    /// Create a property access expression
    pub fn property_access(variable: &str, property: &str) -> Self {
        Expression::Property {
            variable: variable.to_string(),
            property: property.to_string(),
        }
    }

    /// Create a literal expression
    pub fn literal(value: PropertyValue) -> Self {
        Expression::Literal(value)
    }

    /// Create an equals expression
    pub fn equals(left: Expression, right: Expression) -> Self {
        Expression::BinaryOp {
            left: Box::new(left),
            op: BinaryOperator::Eq,
            right: Box::new(right),
        }
    }

    /// Create a function call expression
    pub fn function(name: impl Into<String>, args: Vec<Expression>) -> Self {
        Expression::FunctionCall {
            name: name.into(),
            args,
        }
    }

    /// Create a wildcard expression
    pub fn wildcard() -> Self {
        Expression::Wildcard
    }

    /// Returns true if expression is a TRUE aggregation function (combines N rows → 1).
    ///
    /// Solo `count`, `sum`, `avg`, `min`, `max`. Las funciones algorítmicas
    /// (`degree`, `pagerank`, `community`, etc.) son per-node y se identifican
    /// con `is_algorithm()` — no son agregaciones.
    pub fn is_aggregation(&self) -> bool {
        matches!(
            self,
            Expression::FunctionCall { name, .. }
                if matches!(name.to_lowercase().as_str(),
                    "count" | "sum" | "avg" | "min" | "max")
        )
    }

    /// Returns true if expression is a per-node algorithm function.
    ///
    /// Estas funciones (degree, pagerank, betweenness, clustering, community,
    /// leiden, community_fast, shortestPath) computan un valor por nodo y se
    /// pre-calculan globalmente. Aunque comparten el path de pre-cómputo con
    /// agregaciones, semánticamente NO son agregaciones — son válidas en WHERE
    /// y en proyecciones non-aggregated.
    pub fn is_algorithm(&self) -> bool {
        matches!(
            self,
            Expression::FunctionCall { name, .. }
                if matches!(name.to_lowercase().as_str(),
                    "pagerank" | "betweenness" | "clustering" | "degree"
                    | "community" | "community_fast" | "leiden" | "shortestpath")
        )
    }
}

// ═══════════════════════════════════════════════════════════════
// DISPLAY IMPLEMENTATIONS (for debugging and error messages)
// ═══════════════════════════════════════════════════════════════

impl std::fmt::Display for Statement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Statement::Query(_) => write!(f, "Query"),
            Statement::Sketch(s) => write!(f, "Sketch({})", s.name),
            Statement::Commit(c) => write!(f, "Commit({})", c.sketch_name),
            Statement::Delete(_) => write!(f, "Delete"),
            Statement::Update(_) => write!(f, "Update"),
            Statement::Add(_) => write!(f, "Add"),
            Statement::CreateIndex(_) => write!(f, "CreateIndex"),
            Statement::DropIndex(_) => write!(f, "DropIndex"),
            Statement::Explain(_) => write!(f, "Explain"),
            Statement::Profile(_) => write!(f, "Profile"),
        }
    }
}

impl std::fmt::Display for DangerLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DangerLevel::Low => write!(f, "low"),
            DangerLevel::Medium => write!(f, "medium"),
            DangerLevel::High => write!(f, "HIGH"),
        }
    }
}

impl std::fmt::Display for SortOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SortOrder::Asc => write!(f, "ASC"),
            SortOrder::Desc => write!(f, "DESC"),
        }
    }
}
