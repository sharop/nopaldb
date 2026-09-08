// src/shacl/constraint.rs
//! Evaluacion de constraints SHACL Core sobre los valores de un path.
//!
//! Un solo evaluador para los dos niveles: las constraints de nodo
//! (`sh:nodeKind`, `sh:class` sobre el focus node) se evalúan con el propio
//! focus node como único valor, y las de propiedad sobre los valores del
//! path. Antes eran dos caminos, y `sh:class` dentro de un `sh:property` no
//! se evaluaba nunca.

use std::collections::HashMap;

use crate::index::TaxonomyIndex;
use crate::rdf_owl::importer::local_name;
use crate::types::{Node, NodeId, PropertyValue};
use super::shape::{ConstraintType, DatatypeKind, PathValue, ShaclNodeKind};
use super::report::{ConstraintViolation, Severity};

/// Lo que el evaluador (síncrono) necesita del grafo para juzgar valores-nodo:
/// los nodos destino ya leídos y un snapshot de la taxonomía para `sh:class`.
/// Quien valida los carga antes (async) y los pasa aquí.
#[derive(Default)]
pub struct EvalContext {
    /// Nodos referidos por `PathValue::Node`, más el focus node.
    pub nodes: HashMap<NodeId, Node>,
    /// Snapshot de la taxonomía; `None` cuando el grafo no tiene clases.
    pub taxonomy: Option<TaxonomyIndex>,
}

impl EvalContext {
    /// `true` si `node` es instancia de `class` (label, `prefix:Local` o IRI):
    /// por la taxonomía cuando existe (directa o heredada, todos los tipos
    /// declarados), y por `label == local name` cuando no.
    fn is_instance_of(&mut self, node: &Node, class: &str) -> bool {
        if let Some(tax) = self.taxonomy.as_mut()
            && let Some(class_id) = tax.resolve_class(class)
        {
            return tax.is_instance_of(node.id, &node.label, class_id);
        }
        node.label == local_name(class)
    }

    fn describe(&self, id: NodeId) -> PropertyValue {
        match self.nodes.get(&id).and_then(|n| n.properties.get("iri")) {
            Some(v) => v.clone(),
            None => PropertyValue::String(id.to_string()),
        }
    }
}

/// Componente SHACL de una constraint, para el reporte.
pub fn component(constraint: &ConstraintType) -> &'static str {
    match constraint {
        ConstraintType::MinCount(_) => "sh:MinCountConstraintComponent",
        ConstraintType::MaxCount(_) => "sh:MaxCountConstraintComponent",
        ConstraintType::Datatype(_) => "sh:DatatypeConstraintComponent",
        ConstraintType::MinInclusive(_) => "sh:MinInclusiveConstraintComponent",
        ConstraintType::MaxInclusive(_) => "sh:MaxInclusiveConstraintComponent",
        ConstraintType::MinExclusive(_) => "sh:MinExclusiveConstraintComponent",
        ConstraintType::MaxExclusive(_) => "sh:MaxExclusiveConstraintComponent",
        ConstraintType::MinLength(_) => "sh:MinLengthConstraintComponent",
        ConstraintType::MaxLength(_) => "sh:MaxLengthConstraintComponent",
        ConstraintType::Pattern(_) => "sh:PatternConstraintComponent",
        ConstraintType::In(_) => "sh:InConstraintComponent",
        ConstraintType::HasValue(_) => "sh:HasValueConstraintComponent",
        ConstraintType::NodeKindConstraint(_) | ConstraintType::NodeKindShacl(_) => "sh:NodeKindConstraintComponent",
        ConstraintType::Class(_) => "sh:ClassConstraintComponent",
    }
}

/// Evalua una lista de constraints sobre los valores resueltos de un path.
///
/// `focus_node` es el nodo que se esta validando, `shape_id` identifica el
/// shape, `path` es el predicado (`None` para constraints de nodo). Cada
/// violación sale con su componente; el valor culpable cuando lo hay.
pub fn evaluate_constraints(
    constraints: &[ConstraintType],
    values: &[PathValue],
    focus_node: NodeId,
    shape_id: NodeId,
    path: Option<&str>,
    ctx: &mut EvalContext,
) -> Vec<ConstraintViolation> {
    let mut violations = Vec::new();
    for constraint in constraints {
        if let Some(mut v) = evaluate_constraint(constraint, values, focus_node, shape_id, path, ctx) {
            v.constraint = component(constraint).to_string();
            violations.push(v);
        }
    }
    violations
}

/// Primer valor que no cumple `ok`, descrito para el reporte.
fn first_bad<'a>(
    values: &'a [PathValue],
    ctx: &EvalContext,
    mut ok: impl FnMut(&'a PathValue) -> bool,
) -> Option<PropertyValue> {
    values.iter().find(|v| !ok(v)).map(|v| match v {
        PathValue::Literal(l) => l.clone(),
        PathValue::Node(id) => ctx.describe(*id),
    })
}

fn literal(v: &PathValue) -> Option<&PropertyValue> {
    match v {
        PathValue::Literal(l) => Some(l),
        PathValue::Node(_) => None,
    }
}

/// Evalua un constraint individual. Retorna `Some(violation)` si no conforma.
fn evaluate_constraint(
    constraint: &ConstraintType,
    values: &[PathValue],
    focus_node: NodeId,
    shape_id: NodeId,
    path: Option<&str>,
    ctx: &mut EvalContext,
) -> Option<ConstraintViolation> {
    let path_str = path.map(|s| s.to_string());
    let fail = |message: String| ConstraintViolation::violation(focus_node, shape_id, path_str.clone(), message);

    match constraint {
        // --- Cardinalidad ---
        ConstraintType::MinCount(min) => (values.len() < *min).then(|| {
            fail(format!(
                "sh:minCount {min}: se encontraron {} valor(es), se requieren al menos {min}",
                values.len()
            ))
        }),
        ConstraintType::MaxCount(max) => (values.len() > *max).then(|| {
            fail(format!(
                "sh:maxCount {max}: se encontraron {} valor(es), maximo permitido {max}",
                values.len()
            ))
        }),

        // --- Tipo de dato: un nodo nunca es un literal ---
        ConstraintType::Datatype(dtype) => first_bad(values, ctx, |v| literal(v).is_some_and(|l| dtype.matches(l)))
            .map(|bad| fail(format!("sh:datatype {dtype:?}: el valor no cumple el tipo requerido")).with_value(bad)),

        // --- Rangos numericos: un no-número no cumple ---
        ConstraintType::MinInclusive(min) => first_bad(values, ctx, |v| literal(v).is_some_and(|l| numeric_ge(l, *min)))
            .map(|bad| fail(format!("sh:minInclusive {min}: valor fuera de rango")).with_value(bad)),
        ConstraintType::MaxInclusive(max) => first_bad(values, ctx, |v| literal(v).is_some_and(|l| numeric_le(l, *max)))
            .map(|bad| fail(format!("sh:maxInclusive {max}: valor fuera de rango")).with_value(bad)),
        ConstraintType::MinExclusive(min) => first_bad(values, ctx, |v| literal(v).is_some_and(|l| numeric_gt(l, *min)))
            .map(|bad| fail(format!("sh:minExclusive {min}: valor fuera de rango")).with_value(bad)),
        ConstraintType::MaxExclusive(max) => first_bad(values, ctx, |v| literal(v).is_some_and(|l| numeric_lt(l, *max)))
            .map(|bad| fail(format!("sh:maxExclusive {max}: valor fuera de rango")).with_value(bad)),

        // --- Longitud de strings: solo se juzgan las cadenas ---
        ConstraintType::MinLength(min) => first_bad(values, ctx, |v| match literal(v) {
            Some(PropertyValue::String(s)) => s.chars().count() >= *min,
            _ => true,
        })
        .map(|bad| fail(format!("sh:minLength {min}: cadena demasiado corta")).with_value(bad)),
        ConstraintType::MaxLength(max) => first_bad(values, ctx, |v| match literal(v) {
            Some(PropertyValue::String(s)) => s.chars().count() <= *max,
            _ => true,
        })
        .map(|bad| fail(format!("sh:maxLength {max}: cadena demasiado larga")).with_value(bad)),

        // --- Patron regex: un no-string no conforma ---
        ConstraintType::Pattern(pattern) => match regex::Regex::new(pattern) {
            Ok(re) => first_bad(values, ctx, |v| matches!(literal(v), Some(PropertyValue::String(s)) if re.is_match(s)))
                .map(|bad| fail(format!("sh:pattern '{pattern}': valor no coincide con el patron")).with_value(bad)),
            Err(e) => Some(
                fail(format!("sh:pattern: patron regex invalido '{pattern}': {e}")).with_severity(Severity::Warning),
            ),
        },

        // --- Enumeracion ---
        ConstraintType::In(allowed) => first_bad(values, ctx, |v| literal(v).is_some_and(|l| allowed.contains(l)))
            .map(|bad| fail("sh:in: el valor no esta en la lista de valores permitidos".to_string()).with_value(bad)),

        // --- Valor exacto: literal, o el `iri` de un valor-nodo ---
        ConstraintType::HasValue(expected) => {
            let present = values.iter().any(|v| match v {
                PathValue::Literal(l) => l == expected,
                PathValue::Node(id) => &ctx.describe(*id) == expected,
            });
            (!present).then(|| fail("sh:hasValue: valor requerido no encontrado".to_string()).with_value(expected.clone()))
        }

        // --- Kind de NopalDB del nodo (programático): un literal siempre falla ---
        ConstraintType::NodeKindConstraint(expected_kind) => {
            let bad = values.iter().find(|v| match v {
                PathValue::Node(id) => ctx.nodes.get(id).is_none_or(|n| n.kind != *expected_kind),
                PathValue::Literal(_) => true,
            })?;
            let found = match bad {
                PathValue::Node(id) => ctx.nodes.get(id).map(|n| format!("{:?}", n.kind)).unwrap_or("desconocido".into()),
                PathValue::Literal(_) => "un literal".to_string(),
            };
            let value = match bad {
                PathValue::Node(id) => ctx.describe(*id),
                PathValue::Literal(l) => l.clone(),
            };
            Some(fail(format!("sh:nodeKind: se esperaba {expected_kind:?}, se encontro {found}")).with_value(value))
        }

        // --- sh:nodeKind de SHACL: nodo (IRI/blank) o literal ---
        ConstraintType::NodeKindShacl(kind) => {
            let ok = |v: &PathValue| -> bool {
                let is_blank = |id: &NodeId| {
                    matches!(ctx.nodes.get(id).and_then(|n| n.properties.get("iri")), Some(PropertyValue::String(s)) if s.starts_with("_:"))
                };
                match (kind, v) {
                    (ShaclNodeKind::Literal, PathValue::Literal(_)) => true,
                    (ShaclNodeKind::Literal, PathValue::Node(_)) => false,
                    (ShaclNodeKind::Iri, PathValue::Node(id)) => !is_blank(id),
                    (ShaclNodeKind::BlankNode, PathValue::Node(id)) => is_blank(id),
                    (ShaclNodeKind::BlankNodeOrIri, PathValue::Node(_)) => true,
                    (ShaclNodeKind::Iri | ShaclNodeKind::BlankNode | ShaclNodeKind::BlankNodeOrIri, PathValue::Literal(_)) => false,
                    (ShaclNodeKind::IriOrLiteral, PathValue::Node(id)) => !is_blank(id),
                    (ShaclNodeKind::IriOrLiteral, PathValue::Literal(_)) => true,
                    (ShaclNodeKind::BlankNodeOrLiteral, PathValue::Node(id)) => is_blank(id),
                    (ShaclNodeKind::BlankNodeOrLiteral, PathValue::Literal(_)) => true,
                }
            };
            first_bad(values, ctx, ok)
                .map(|bad| fail(format!("sh:nodeKind {}: el valor no es de ese kind", kind.as_str())).with_value(bad))
        }

        // --- sh:class: cada valor-nodo es instancia de la clase ---
        ConstraintType::Class(class) => {
            let mut bad: Option<PropertyValue> = None;
            for v in values {
                let ok = match v {
                    PathValue::Literal(_) => false,
                    PathValue::Node(id) => match ctx.nodes.get(id).cloned() {
                        Some(node) => ctx.is_instance_of(&node, class),
                        None => false,
                    },
                };
                if !ok {
                    bad = Some(match v {
                        PathValue::Literal(l) => l.clone(),
                        PathValue::Node(id) => ctx.describe(*id),
                    });
                    break;
                }
            }
            bad.map(|bad| fail(format!("sh:class {class}: el valor no es instancia de la clase")).with_value(bad))
        }
    }
}

// --- Helpers numericos ---

fn to_f64(v: &PropertyValue) -> Option<f64> {
    match v {
        PropertyValue::Int(i) => Some(*i as f64),
        PropertyValue::Float(f) => Some(*f),
        _ => None,
    }
}

fn numeric_ge(v: &PropertyValue, limit: f64) -> bool {
    to_f64(v).is_some_and(|n| n >= limit)
}

fn numeric_le(v: &PropertyValue, limit: f64) -> bool {
    to_f64(v).is_some_and(|n| n <= limit)
}

fn numeric_gt(v: &PropertyValue, limit: f64) -> bool {
    to_f64(v).is_some_and(|n| n > limit)
}

fn numeric_lt(v: &PropertyValue, limit: f64) -> bool {
    to_f64(v).is_some_and(|n| n < limit)
}

/// El `DatatypeKind` que se muestra en mensajes.
impl std::fmt::Display for DatatypeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::NodeKind;
    use uuid::Uuid;

    fn node_id() -> NodeId { Uuid::new_v4() }
    fn shape_id() -> NodeId { Uuid::new_v4() }
    fn lits(vs: Vec<PropertyValue>) -> Vec<PathValue> { vs.into_iter().map(PathValue::Literal).collect() }
    fn eval(c: ConstraintType, vs: Vec<PropertyValue>) -> Vec<ConstraintViolation> {
        let mut ctx = EvalContext::default();
        evaluate_constraints(&[c], &lits(vs), node_id(), shape_id(), Some("age"), &mut ctx)
    }

    #[test]
    fn test_min_count_pass_and_fail() {
        assert!(eval(ConstraintType::MinCount(1), vec![PropertyValue::Int(1)]).is_empty());
        let v = eval(ConstraintType::MinCount(1), vec![]);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].constraint, "sh:MinCountConstraintComponent");
        assert_eq!(v[0].value, None);
    }

    #[test]
    fn test_max_count_fail() {
        assert_eq!(eval(ConstraintType::MaxCount(1), vec![PropertyValue::Int(1), PropertyValue::Int(2)]).len(), 1);
    }

    #[test]
    fn test_datatype_reports_the_offending_value() {
        assert!(eval(ConstraintType::Datatype(DatatypeKind::Int), vec![PropertyValue::Int(42)]).is_empty());
        let v = eval(ConstraintType::Datatype(DatatypeKind::Int), vec![PropertyValue::Int(1), PropertyValue::String("hello".into())]);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].value, Some(PropertyValue::String("hello".into())));
        assert_eq!(v[0].constraint, "sh:DatatypeConstraintComponent");
    }

    #[test]
    fn test_numeric_ranges() {
        assert!(eval(ConstraintType::MinInclusive(10.0), vec![PropertyValue::Int(10)]).is_empty());
        assert_eq!(eval(ConstraintType::MinInclusive(10.0), vec![PropertyValue::Int(9)]).len(), 1);
        assert!(eval(ConstraintType::MaxInclusive(10.0), vec![PropertyValue::Float(10.0)]).is_empty());
        assert_eq!(eval(ConstraintType::MaxInclusive(10.0), vec![PropertyValue::Float(10.5)]).len(), 1);
        assert!(eval(ConstraintType::MinExclusive(10.0), vec![PropertyValue::Int(11)]).is_empty());
        assert_eq!(eval(ConstraintType::MinExclusive(10.0), vec![PropertyValue::Int(10)]).len(), 1);
        assert!(eval(ConstraintType::MaxExclusive(10.0), vec![PropertyValue::Int(9)]).is_empty());
        assert_eq!(eval(ConstraintType::MaxExclusive(10.0), vec![PropertyValue::Int(10)]).len(), 1);
        // A string is not in any numeric range.
        assert_eq!(eval(ConstraintType::MinInclusive(0.0), vec![PropertyValue::String("x".into())]).len(), 1);
    }

    #[test]
    fn test_string_lengths_ignore_non_strings() {
        assert!(eval(ConstraintType::MinLength(3), vec![PropertyValue::String("abc".into())]).is_empty());
        assert_eq!(eval(ConstraintType::MinLength(3), vec![PropertyValue::String("ab".into())]).len(), 1);
        assert_eq!(eval(ConstraintType::MaxLength(3), vec![PropertyValue::String("abcd".into())]).len(), 1);
        assert!(eval(ConstraintType::MaxLength(3), vec![PropertyValue::Int(12345)]).is_empty());
    }

    #[test]
    fn test_pattern() {
        assert!(eval(ConstraintType::Pattern("^[a-z]+$".into()), vec![PropertyValue::String("abc".into())]).is_empty());
        assert_eq!(eval(ConstraintType::Pattern("^[a-z]+$".into()), vec![PropertyValue::String("ABC".into())]).len(), 1);
        assert_eq!(eval(ConstraintType::Pattern("^[a-z]+$".into()), vec![PropertyValue::Int(1)]).len(), 1);
        let v = eval(ConstraintType::Pattern("(".into()), vec![PropertyValue::String("x".into())]);
        assert_eq!(v[0].severity, Severity::Warning);
    }

    #[test]
    fn test_in_and_has_value() {
        let allowed = vec![PropertyValue::String("a".into()), PropertyValue::String("b".into())];
        assert!(eval(ConstraintType::In(allowed.clone()), vec![PropertyValue::String("a".into())]).is_empty());
        let v = eval(ConstraintType::In(allowed), vec![PropertyValue::String("z".into())]);
        assert_eq!(v[0].value, Some(PropertyValue::String("z".into())));
        assert!(eval(ConstraintType::HasValue(PropertyValue::Int(1)), vec![PropertyValue::Int(1)]).is_empty());
        assert_eq!(eval(ConstraintType::HasValue(PropertyValue::Int(1)), vec![PropertyValue::Int(2)]).len(), 1);
    }

    #[test]
    fn test_node_constraints_on_node_values() {
        let mut ctx = EvalContext::default();
        let mut ingrediente = Node::new("Ingrediente");
        ingrediente.properties.insert("iri".into(), PropertyValue::String("http://cocina.example/canela".into()));
        let mut blank = Node::new("Cosa");
        blank.properties.insert("iri".into(), PropertyValue::String("_:abc-b0".into()));
        let (ing_id, blank_id) = (ingrediente.id, blank.id);
        ctx.nodes.insert(ing_id, ingrediente);
        ctx.nodes.insert(blank_id, blank);
        let values = vec![PathValue::Node(ing_id), PathValue::Node(blank_id), PathValue::Literal(PropertyValue::Int(3))];

        // sh:class by label without taxonomy: canela is an Ingrediente, the blank is not.
        let v = evaluate_constraints(&[ConstraintType::Class("Ingrediente".into())], &values[..1], node_id(), shape_id(), Some("usa"), &mut ctx);
        assert!(v.is_empty());
        let v = evaluate_constraints(&[ConstraintType::Class("http://cocina.example/Ingrediente".into())], &values[..1], node_id(), shape_id(), Some("usa"), &mut ctx);
        assert!(v.is_empty(), "IRI falls back to its local name without a taxonomy");
        let v = evaluate_constraints(&[ConstraintType::Class("Ingrediente".into())], &values, node_id(), shape_id(), Some("usa"), &mut ctx);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].value, Some(PropertyValue::String("_:abc-b0".into())));
        assert_eq!(v[0].constraint, "sh:ClassConstraintComponent");

        // sh:nodeKind
        let v = evaluate_constraints(&[ConstraintType::NodeKindShacl(ShaclNodeKind::Iri)], &values[..1], node_id(), shape_id(), None, &mut ctx);
        assert!(v.is_empty());
        let v = evaluate_constraints(&[ConstraintType::NodeKindShacl(ShaclNodeKind::Iri)], &values[1..2], node_id(), shape_id(), None, &mut ctx);
        assert_eq!(v.len(), 1, "a blank node is not an IRI");
        let v = evaluate_constraints(&[ConstraintType::NodeKindShacl(ShaclNodeKind::Literal)], &values[2..], node_id(), shape_id(), None, &mut ctx);
        assert!(v.is_empty());
        let v = evaluate_constraints(&[ConstraintType::NodeKindShacl(ShaclNodeKind::BlankNodeOrIri)], &values[2..], node_id(), shape_id(), None, &mut ctx);
        assert_eq!(v.len(), 1);

        // NopalDB kind
        let v = evaluate_constraints(&[ConstraintType::NodeKindConstraint(NodeKind::Individual)], &values[..1], node_id(), shape_id(), None, &mut ctx);
        assert!(v.is_empty());
        let v = evaluate_constraints(&[ConstraintType::NodeKindConstraint(NodeKind::Class)], &values[..1], node_id(), shape_id(), None, &mut ctx);
        assert_eq!(v.len(), 1);
        // A literal is never a node of any kind, and never a class instance.
        let v = evaluate_constraints(&[ConstraintType::Datatype(DatatypeKind::Int)], &values[..1], node_id(), shape_id(), None, &mut ctx);
        assert_eq!(v.len(), 1, "a node is not an integer");
    }

    #[test]
    fn test_datatype_from_xsd_matches_the_importer_table() {
        assert_eq!(DatatypeKind::from_xsd("http://www.w3.org/2001/XMLSchema#integer"), DatatypeKind::Int);
        assert_eq!(DatatypeKind::from_xsd("http://www.w3.org/2001/XMLSchema#nonNegativeInteger"), DatatypeKind::Int);
        assert_eq!(DatatypeKind::from_xsd("http://www.w3.org/2001/XMLSchema#decimal"), DatatypeKind::Float);
        assert_eq!(DatatypeKind::from_xsd("http://www.w3.org/2001/XMLSchema#boolean"), DatatypeKind::Bool);
        assert_eq!(DatatypeKind::from_xsd("http://www.w3.org/2001/XMLSchema#string"), DatatypeKind::Str);
        assert_eq!(DatatypeKind::from_xsd("http://www.w3.org/2001/XMLSchema#date"), DatatypeKind::Str);
    }
}
