// src/query/cypher.rs
//
//! Cypher de solo lectura, traducido a NQL (feature `cypher`).
//!
//! [`to_nql`] lee una consulta Cypher con el parser `open-cypher` y devuelve
//! el texto NQL equivalente; [`Graph::execute_cypher`](crate::Graph::execute_cypher)
//! ejecuta ese texto con `execute_nql`. El subconjunto es `MATCH … WHERE …
//! RETURN … ORDER BY … SKIP … LIMIT`; todo lo demás (escrituras, `OPTIONAL
//! MATCH`, `WITH`, `UNWIND`, `UNION`, parámetros, funciones sin equivalente)
//! es un error que nombra la parte rechazada. Nada se descarta en silencio.
//!
//! Se traduce a texto NQL y no al AST de NQL: el texto pasa por el mismo
//! parser, validador, planner y ejecutor que una consulta escrita a mano, y
//! la tabla "Cypher aceptado → NQL equivalente" se puede comprobar en los
//! tests (lo que `to_nql` devuelve es exactamente lo que se ejecuta). Armar
//! el AST directamente se saltaría las reglas que hoy viven en el parser de
//! NQL (cuantificadores, literales, `-[]-`) y obligaría a duplicarlas aquí.
//!
//! Diferencias que el traductor no puede cerrar, documentadas en
//! `docs/CYPHER.md`:
//! - `RETURN n` da las columnas de `all(n)` (`n.id`, `n.label` y una por
//!   propiedad), no un objeto nodo.
//! - Una propiedad ausente no es `null` en NQL: `n.x = 1` es falso, y
//!   `NOT n.x = 1` es verdadero, donde Cypher da `null` y descarta la fila.
//! - `(n:Label)` no admite nodos de clase (0.11.5, #411).

use open_cypher::ast::{
    BinaryOperator, ClauseKind, Expr, ExprKind, IntegerRadix, IsPredicate, LabelExpression,
    LabelExpressionKind, LiteralKind, NodePattern, PathFactorKind, PatternPart, ProjectionClause,
    ProjectionItemKind, QuantifierKind, QueryKind, RelationshipDirection, RelationshipPattern,
    SetQuantifier, SortDirection, StatementKind, UnaryOperator,
};
use open_cypher::Span;

use crate::error::{NopalError, Result};

/// Traduce una consulta Cypher de solo lectura a NQL.
///
/// ```
/// let nql = nopaldb::query::cypher::to_nql(
///     "MATCH (p:Persona)-[:CONOCE]->(q) WHERE p.edad > 30 RETURN q.nombre LIMIT 5",
/// ).unwrap();
/// assert_eq!(nql, "find q.nombre from (p:Persona)-[:CONOCE]->(q) where p.edad > 30 limit 5");
/// ```
pub fn to_nql(cypher: &str) -> Result<String> {
    Translator { src: cypher }.query()
}

struct Translator<'a> {
    src: &'a str,
}

/// Lo que un `MATCH` aporta: patrones y condiciones (su `WHERE` y los
/// `WHERE` dentro de los patrones).
#[derive(Default)]
struct Matched {
    patterns: Vec<String>,
    conditions: Vec<String>,
}

impl Translator<'_> {
    fn text(&self, span: Span) -> &str {
        span.text(self.src).unwrap_or("").trim()
    }

    /// Error para una parte que no se soporta: cita el texto de la consulta.
    fn unsupported(&self, what: &str, span: Span) -> NopalError {
        NopalError::QueryParseError(format!(
            "Cypher: {what} is not supported (only MATCH … WHERE … RETURN … ORDER BY … SKIP … LIMIT): `{}`",
            self.text(span)
        ))
    }

    fn query(&self) -> Result<String> {
        let parsed = open_cypher::parse(self.src)
            .map_err(|e| NopalError::QueryParseError(format!("Cypher: {}", e.render(self.src))))?;
        let [statement] = parsed.program.statements.as_slice() else {
            return Err(NopalError::QueryParseError("Cypher: one statement per query".into()));
        };
        let StatementKind::Query(query) = &statement.kind else {
            return Err(self.unsupported("this statement", statement.span));
        };
        if let Some(mode) = &query.mode {
            return Err(self.unsupported("EXPLAIN/PROFILE", mode.span));
        }
        let QueryKind::Regular(regular) = &query.query.kind else {
            return Err(self.unsupported("this query", query.query.span));
        };
        if let Some(union) = regular.unions.first() {
            return Err(self.unsupported("UNION", union.operator.span));
        }

        let clauses = &regular.head.kind.clauses;
        let mut matched = Matched::default();
        let mut projection = None;
        for (i, clause) in clauses.iter().enumerate() {
            match &clause.kind {
                ClauseKind::Match(m) if projection.is_none() => {
                    if m.optional {
                        return Err(self.unsupported("OPTIONAL MATCH", clause.span));
                    }
                    if m.mode.is_some() || !m.hints.is_empty() {
                        return Err(self.unsupported("MATCH modes and hints", clause.span));
                    }
                    for part in &m.pattern.parts {
                        matched.patterns.push(self.pattern(part, &mut matched.conditions)?);
                    }
                    if let Some(w) = &m.where_clause {
                        matched.conditions.push(self.expr(w)?);
                    }
                }
                ClauseKind::Return(r) if i + 1 == clauses.len() && !matched.patterns.is_empty() => {
                    projection = Some(&r.projection);
                }
                ClauseKind::Return(_) => {
                    return Err(self.unsupported("RETURN without a MATCH before it", clause.span));
                }
                ClauseKind::Match(_) => return Err(self.unsupported("MATCH after RETURN", clause.span)),
                ClauseKind::Create(_)
                | ClauseKind::Insert(_)
                | ClauseKind::Merge(_)
                | ClauseKind::Delete(_)
                | ClauseKind::Set(_)
                | ClauseKind::Remove(_)
                | ClauseKind::Foreach(_)
                | ClauseKind::LoadCsv(_) => {
                    return Err(NopalError::QueryParseError(format!(
                        "Cypher: read-only; writes go through NQL (add/update/delete/upsert): `{}`",
                        self.text(clause.span)
                    )));
                }
                ClauseKind::With(_) => return Err(self.unsupported("WITH", clause.span)),
                ClauseKind::Unwind(_) => return Err(self.unsupported("UNWIND", clause.span)),
                ClauseKind::Call(_) => return Err(self.unsupported("CALL", clause.span)),
                _ => return Err(self.unsupported("this clause", clause.span)),
            }
        }
        let Some(projection) = projection else {
            return Err(NopalError::QueryParseError("Cypher: the query must end with RETURN".into()));
        };

        let mut nql = format!("find {} from {}", self.projection(projection)?, matched.patterns.join(", "));
        match matched.conditions.as_slice() {
            [] => {}
            [one] => nql.push_str(&format!(" where {one}")),
            many => {
                let joined: Vec<String> = many.iter().map(|c| format!("({c})")).collect();
                nql.push_str(&format!(" where {}", joined.join(" and ")));
            }
        }
        nql.push_str(&self.tail(projection)?);
        Ok(nql)
    }

    // ── RETURN ────────────────────────────────────────────────────────

    fn projection(&self, p: &ProjectionClause) -> Result<String> {
        let aggregates = p.items.iter().any(|i| match &i.kind {
            ProjectionItemKind::Expression { expression, .. } => has_aggregate(expression),
            _ => false,
        });
        let mut items = Vec::with_capacity(p.items.len());
        for item in &p.items {
            items.push(match &item.kind {
                ProjectionItemKind::Wildcard => "*".to_string(),
                ProjectionItemKind::Expression { expression, alias } => {
                    let alias = alias.as_ref().map(|a| self.identifier(&a.kind.text, a.span)).transpose()?;
                    match (&expression.kind, alias) {
                        // `RETURN n`: las columnas del nodo; NQL no tiene un
                        // valor "nodo" con alias.
                        (ExprKind::Variable(_), None) if aggregates => {
                            return Err(self.unsupported(
                                "a whole node next to an aggregate (group by its properties instead)",
                                item.span,
                            ));
                        }
                        (ExprKind::Variable(v), None) => format!("all({})", self.identifier(&v.kind.text, v.span)?),
                        (ExprKind::Variable(_), Some(_)) => {
                            return Err(self.unsupported("an alias on a whole node or relationship", item.span));
                        }
                        (_, None) => self.expr(expression)?,
                        (_, Some(a)) => format!("{} as {a}", self.expr(expression)?),
                    }
                }
                _ => return Err(self.unsupported("this projection", item.span)),
            });
        }
        let distinct = match p.quantifier.as_ref().map(|q| &q.kind) {
            Some(SetQuantifier::Distinct) => "distinct ",
            _ => "",
        };
        Ok(format!("{distinct}{}", items.join(", ")))
    }

    /// `ORDER BY`, `SKIP`/`OFFSET` y `LIMIT`.
    fn tail(&self, p: &ProjectionClause) -> Result<String> {
        let mut out = String::new();
        if let Some(order) = &p.order_by {
            let mut items = Vec::with_capacity(order.items.len());
            for item in &order.items {
                if item.nulls.is_some() {
                    return Err(self.unsupported("NULLS FIRST/LAST", item.span));
                }
                let dir = match item.direction.as_ref().map(|d| &d.kind) {
                    Some(SortDirection::Descending) => " desc",
                    _ => "",
                };
                items.push(format!("{}{dir}", self.expr(&item.expression)?));
            }
            out.push_str(&format!(" order by {}", items.join(", ")));
        }
        let skip = match (&p.skip, &p.offset) {
            (Some(e), None) | (None, Some(e)) => Some(self.count(e)?),
            (None, None) => None,
            (Some(_), Some(e)) => return Err(self.unsupported("SKIP and OFFSET together", e.span)),
        };
        match (&p.limit, skip) {
            (Some(limit), skip) => {
                out.push_str(&format!(" limit {}", self.count(limit)?));
                if let Some(skip) = skip {
                    out.push_str(&format!(" offset {skip}"));
                }
            }
            // NQL no tiene `offset` sin `limit`, y un tope inventado
            // cambiaría el resultado.
            (None, Some(_)) => return Err(self.unsupported("SKIP without LIMIT", p.span)),
            (None, None) => {}
        }
        Ok(out)
    }

    /// Un entero literal no negativo (`SKIP`, `LIMIT`).
    fn count(&self, e: &Expr) -> Result<u64> {
        if let ExprKind::Literal(lit) = &e.kind
            && let LiteralKind::Integer(i) = &lit.kind
            && let Some(n) = integer(&i.text, &i.radix).and_then(|n| u64::try_from(n).ok())
        {
            return Ok(n);
        }
        Err(self.unsupported("a non-literal SKIP/LIMIT", e.span))
    }

    // ── MATCH ─────────────────────────────────────────────────────────

    fn pattern(&self, part: &PatternPart, conditions: &mut Vec<String>) -> Result<String> {
        if part.kind.binding.is_some() {
            return Err(self.unsupported("a path variable (`p = …`)", part.span));
        }
        if part.kind.selector.is_some() {
            return Err(self.unsupported("a path selector", part.span));
        }
        let mut out = String::new();
        for factor in &part.kind.path.kind.factors {
            match &factor.kind {
                PathFactorKind::Node(n) => out.push_str(&self.node(n, factor.span, conditions)?),
                PathFactorKind::Relationship(r) => out.push_str(&self.relationship(r, factor.span, conditions)?),
                PathFactorKind::LegacyShortest { .. } => {
                    return Err(NopalError::QueryParseError(format!(
                        "Cypher: shortestPath is not supported; NQL has `find path(a, b) from (a), (b) where …`: `{}`",
                        self.text(factor.span)
                    )));
                }
                _ => return Err(self.unsupported("this pattern", factor.span)),
            }
        }
        Ok(out)
    }

    fn node(&self, n: &NodePattern, span: Span, conditions: &mut Vec<String>) -> Result<String> {
        let mut out = String::from("(");
        if let Some(v) = &n.variable {
            out.push_str(&self.identifier(&v.kind.text, v.span)?);
        }
        if let Some(labels) = &n.labels {
            match &labels.kind {
                LabelExpressionKind::Name(l) => {
                    out.push(':');
                    out.push_str(&self.identifier(&l.kind.text, l.span)?);
                }
                _ => return Err(self.unsupported("a node with several labels or a label expression", labels.span)),
            }
        }
        if let Some(props) = &n.properties {
            out.push_str(&self.property_map(props)?);
        }
        if let Some(w) = &n.where_clause {
            if n.variable.is_none() {
                return Err(self.unsupported("WHERE in a node without a variable", span));
            }
            conditions.push(self.expr(w)?);
        }
        out.push(')');
        Ok(out)
    }

    fn relationship(&self, r: &RelationshipPattern, span: Span, conditions: &mut Vec<String>) -> Result<String> {
        let mut inner = String::new();
        if let Some(v) = &r.variable {
            inner.push_str(&self.identifier(&v.kind.text, v.span)?);
        }
        if let Some(labels) = &r.labels {
            inner.push(':');
            inner.push_str(&self.rel_types(labels)?.join("|"));
        }
        if let Some(props) = &r.properties {
            inner.push_str(&self.property_map(props)?);
        }
        if let Some(w) = &r.where_clause {
            if r.variable.is_none() {
                return Err(self.unsupported("WHERE in a relationship without a variable", span));
            }
            conditions.push(self.expr(w)?);
        }
        let quantifier = match (&r.legacy_quantifier, &r.graph_quantifier) {
            (None, None) => String::new(),
            (Some(q), None) | (None, Some(q)) => {
                if r.variable.is_some() {
                    // En Cypher la variable sería la lista de aristas.
                    return Err(self.unsupported("a variable on a variable-length relationship", span));
                }
                let (min, max) = match &q.kind {
                    QuantifierKind::Fixed(n) => (bound(n), bound(n)),
                    QuantifierKind::Range { lower, upper: Some(upper) } => {
                        (lower.as_deref().map_or(Some(1), bound), bound(upper))
                    }
                    _ => (None, None),
                };
                match (min, max) {
                    // NQL exige al menos un salto y un tope (Path Queries F1).
                    (Some(min), Some(max)) if min >= 1 && min <= max => format!("{{{min},{max}}}"),
                    _ => {
                        return Err(self.unsupported(
                            "this length (NQL needs at least 1 hop and an upper bound, e.g. *1..3)",
                            q.span,
                        ));
                    }
                }
            }
            (Some(_), Some(q)) => return Err(self.unsupported("two quantifiers", q.span)),
        };
        let body = if inner.is_empty() { String::new() } else { format!("[{inner}]") };
        Ok(match r.direction.kind {
            RelationshipDirection::Right => format!("-{}->{quantifier}", if body.is_empty() { "[]" } else { &body }),
            RelationshipDirection::Left => format!("<-{}-{quantifier}", if body.is_empty() { "[]" } else { &body }),
            RelationshipDirection::Undirected | RelationshipDirection::Both => {
                format!("-{}-{quantifier}", if body.is_empty() { "[]" } else { &body })
            }
            _ => return Err(self.unsupported("this direction", r.direction.span)),
        })
    }

    /// `:A` o `:A|B` (también `:A|:B`).
    fn rel_types(&self, labels: &LabelExpression) -> Result<Vec<String>> {
        match &labels.kind {
            LabelExpressionKind::Name(l) => Ok(vec![self.identifier(&l.kind.text, l.span)?]),
            LabelExpressionKind::Or(alternatives) => {
                let mut out = Vec::new();
                for a in alternatives {
                    out.extend(self.rel_types(a)?);
                }
                Ok(out)
            }
            _ => Err(self.unsupported("this relationship type expression", labels.span)),
        }
    }

    /// `{k: literal, …}` dentro de un patrón: NQL solo admite literales.
    fn property_map(&self, props: &Expr) -> Result<String> {
        let ExprKind::Map(entries) = &props.kind else {
            return Err(self.unsupported("a property map that is not a literal map", props.span));
        };
        let mut items = Vec::with_capacity(entries.len());
        for entry in entries {
            let value = self.literal(&entry.value).ok_or_else(|| {
                self.unsupported("a non-literal value in a pattern property map", entry.value.span)
            })?;
            items.push(format!("{}: {value}", self.identifier(&entry.key.kind.text, entry.key.span)?));
        }
        Ok(format!(" {{{}}}", items.join(", ")))
    }

    // ── Expresiones ───────────────────────────────────────────────────

    fn expr(&self, e: &Expr) -> Result<String> {
        if let Some(lit) = self.literal(e) {
            return Ok(lit);
        }
        match &e.kind {
            ExprKind::Variable(v) => self.identifier(&v.kind.text, v.span),
            ExprKind::Property { expression, key } => match &expression.kind {
                ExprKind::Variable(v) => Ok(format!(
                    "{}.{}",
                    self.identifier(&v.kind.text, v.span)?,
                    self.identifier(&key.kind.text, key.span)?
                )),
                _ => Err(self.unsupported("a property of something that is not a variable", e.span)),
            },
            ExprKind::Parenthesized(inner) => self.expr(inner),
            ExprKind::Unary { operator, operand } => match operator.kind {
                UnaryOperator::Not => Ok(format!("!({})", self.expr(operand)?)),
                UnaryOperator::Minus => Ok(format!("-{}", self.operand(operand)?)),
                UnaryOperator::Plus => self.expr(operand),
                _ => Err(self.unsupported("this operator", operator.span)),
            },
            ExprKind::Binary { left, operator, right } => {
                let op = match operator.kind {
                    BinaryOperator::Or => "or",
                    BinaryOperator::And => "and",
                    BinaryOperator::Equal => "=",
                    BinaryOperator::NotEqual => "!=",
                    BinaryOperator::Less => "<",
                    BinaryOperator::LessEqual => "<=",
                    BinaryOperator::Greater => ">",
                    BinaryOperator::GreaterEqual => ">=",
                    BinaryOperator::Add => "+",
                    BinaryOperator::Subtract => "-",
                    BinaryOperator::Multiply => "*",
                    BinaryOperator::Divide => "/",
                    BinaryOperator::Remainder => "%",
                    BinaryOperator::In => {
                        let list = self.literal(right).filter(|l| l.starts_with('['));
                        let Some(list) = list else {
                            return Err(self.unsupported("IN with something other than a literal list", right.span));
                        };
                        return Ok(format!("{} in {list}", self.operand(left)?));
                    }
                    _ => return Err(self.unsupported("this operator", operator.span)),
                };
                Ok(format!("{} {op} {}", self.operand(left)?, self.operand(right)?))
            }
            // Una propiedad ausente no es `= null` en NQL, pero sí falla
            // `!= null`: `IS NOT NULL` es `!= null` e `IS NULL` su negación.
            ExprKind::Is { expression, negated, predicate: IsPredicate::Null } => {
                let x = self.operand(expression)?;
                Ok(if *negated { format!("{x} != null") } else { format!("!({x} != null)") })
            }
            ExprKind::Function(f) => self.function(e, f),
            ExprKind::Parameter(_) => Err(self.unsupported("a parameter", e.span)),
            _ => Err(self.unsupported("this expression", e.span)),
        }
    }

    /// Un operando de un operador: entre paréntesis si es a su vez una
    /// operación, para no depender de la precedencia de NQL.
    fn operand(&self, e: &Expr) -> Result<String> {
        let mut inner = e;
        while let ExprKind::Parenthesized(x) = &inner.kind {
            inner = x;
        }
        let text = self.expr(inner)?;
        Ok(match &inner.kind {
            ExprKind::Binary { .. } | ExprKind::Is { .. } => format!("({text})"),
            _ => text,
        })
    }

    fn function(&self, e: &Expr, f: &open_cypher::ast::FunctionInvocation) -> Result<String> {
        let [name] = f.name.parts.as_slice() else {
            return Err(self.unsupported("a namespaced function", e.span));
        };
        if f.quantifier.is_some() {
            return Err(self.unsupported("DISTINCT inside an aggregate", e.span));
        }
        let name = name.kind.text.to_ascii_lowercase();
        let var = |f: &open_cypher::ast::FunctionInvocation| match f.arguments.as_slice() {
            [a] => match &a.kind {
                ExprKind::Variable(v) => Some(v.kind.text.clone()),
                _ => None,
            },
            _ => None,
        };
        match name.as_str() {
            "count" | "sum" | "avg" | "min" | "max" => {
                let args = match f.arguments.as_slice() {
                    [a] if matches!(a.kind, ExprKind::Wildcard) => "*".to_string(),
                    [a] => self.expr(a)?,
                    _ => return Err(self.unsupported("this aggregate call", e.span)),
                };
                Ok(format!("{name}({args})"))
            }
            "id" | "elementid" => match var(f) {
                Some(v) => Ok(format!("{}.id", self.identifier(&v, e.span)?)),
                None => Err(self.unsupported("id() of something that is not a variable", e.span)),
            },
            "type" => match var(f) {
                Some(v) => Ok(format!("{}.type", self.identifier(&v, e.span)?)),
                None => Err(self.unsupported("type() of something that is not a variable", e.span)),
            },
            _ => Err(self.unsupported(&format!("the function `{}`", name), e.span)),
        }
    }

    /// El literal NQL de `e`, si `e` es un literal (o una lista de ellos, o
    /// un número negado).
    fn literal(&self, e: &Expr) -> Option<String> {
        match &e.kind {
            ExprKind::Literal(lit) => match &lit.kind {
                LiteralKind::Null => Some("null".into()),
                LiteralKind::Boolean(b) => Some(b.to_string()),
                LiteralKind::Integer(i) => integer(&i.text, &i.radix).map(|n| n.to_string()),
                LiteralKind::Float(x) => x.text.replace('_', "").parse::<f64>().ok().map(|x| format!("{x:?}")),
                LiteralKind::String(s) => Some(nql_string(&s.value)),
                _ => None,
            },
            ExprKind::List(items) => {
                let items: Option<Vec<String>> = items.iter().map(|i| self.literal(i)).collect();
                Some(format!("[{}]", items?.join(", ")))
            }
            ExprKind::Unary { operator, operand } if operator.kind == UnaryOperator::Minus => {
                match &operand.kind {
                    ExprKind::Literal(l) if matches!(l.kind, LiteralKind::Integer(_) | LiteralKind::Float(_)) => {
                        self.literal(operand).map(|x| format!("-{x}"))
                    }
                    _ => None,
                }
            }
            ExprKind::Parenthesized(inner) => self.literal(inner),
            _ => None,
        }
    }

    /// Un nombre de variable, etiqueta, tipo o propiedad tal como lo acepta
    /// NQL; uno con espacios o símbolos (posible en Cypher con backticks)
    /// no tiene forma en NQL.
    fn identifier(&self, name: &str, span: Span) -> Result<String> {
        let mut chars = name.chars();
        let ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
        if ok {
            Ok(name.to_string())
        } else {
            Err(self.unsupported("this name (NQL names are letters, digits and `_`)", span))
        }
    }
}

/// Si `e` contiene una llamada a un agregado.
fn has_aggregate(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Function(f) => {
            let aggregate = matches!(
                f.name.parts.as_slice(),
                [n] if ["count", "sum", "avg", "min", "max"].iter().any(|a| n.kind.text.eq_ignore_ascii_case(a))
            );
            aggregate || f.arguments.iter().any(has_aggregate)
        }
        ExprKind::Binary { left, right, .. } => has_aggregate(left) || has_aggregate(right),
        ExprKind::Unary { operand, .. } => has_aggregate(operand),
        ExprKind::Parenthesized(inner) => has_aggregate(inner),
        _ => false,
    }
}

fn integer(text: &str, radix: &IntegerRadix) -> Option<i64> {
    let text = text.replace('_', "");
    match radix {
        IntegerRadix::Decimal => text.parse().ok(),
        IntegerRadix::Hexadecimal => i64::from_str_radix(text.trim_start_matches("0x").trim_start_matches("0X"), 16).ok(),
        IntegerRadix::Octal => i64::from_str_radix(text.trim_start_matches("0o").trim_start_matches("0O"), 8).ok(),
        _ => None,
    }
}

fn bound(text: &str) -> Option<usize> {
    text.replace('_', "").parse().ok()
}

/// Un string NQL entre comillas dobles; NQL reconoce `\"`, `\\`, `\n`, `\r`
/// y `\t`, y todo lo demás va tal cual.
fn nql_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
