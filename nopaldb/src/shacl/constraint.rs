// src/shacl/constraint.rs
//! Evaluacion de constraints SHACL Core sobre valores y nodos.

use crate::types::{Node, NodeId, PropertyValue};
use super::shape::ConstraintType;
use super::report::{ConstraintViolation, Severity};

/// Evalua una lista de constraints sobre un conjunto de valores resueltos.
///
/// `focus_node` es el nodo que se esta validando.
/// `shape_id` identifica el shape para el reporte.
/// `path` es la propiedad o edge_type (para mensajes), `None` si es constraint directa.
/// `values` son los valores resueltos del path (puede ser vacio).
pub fn evaluate_constraints(
    constraints: &[ConstraintType],
    values: &[PropertyValue],
    focus_node: NodeId,
    shape_id: NodeId,
    path: Option<&str>,
) -> Vec<ConstraintViolation> {
    let mut violations = Vec::new();

    for constraint in constraints {
        if let Some(v) = evaluate_constraint(constraint, values, focus_node, shape_id, path) {
            violations.push(v);
        }
    }

    violations
}

/// Evalua un constraint individual. Retorna `Some(violation)` si no conforma.
fn evaluate_constraint(
    constraint: &ConstraintType,
    values: &[PropertyValue],
    focus_node: NodeId,
    shape_id: NodeId,
    path: Option<&str>,
) -> Option<ConstraintViolation> {
    let path_str = path.map(|s| s.to_string());

    match constraint {
        // --- Cardinalidad ---
        ConstraintType::MinCount(min) => {
            if values.len() < *min {
                Some(ConstraintViolation::violation(
                    focus_node,
                    shape_id,
                    path_str,
                    format!(
                        "sh:minCount {min}: se encontraron {} valor(es), se requieren al menos {min}",
                        values.len()
                    ),
                ))
            } else {
                None
            }
        }

        ConstraintType::MaxCount(max) => {
            if values.len() > *max {
                Some(ConstraintViolation::violation(
                    focus_node,
                    shape_id,
                    path_str,
                    format!(
                        "sh:maxCount {max}: se encontraron {} valor(es), maximo permitido {max}",
                        values.len()
                    ),
                ))
            } else {
                None
            }
        }

        // --- Tipo de dato ---
        ConstraintType::Datatype(dtype) => {
            let bad: Vec<_> = values
                .iter()
                .filter(|v| !dtype.matches(v))
                .collect();
            if !bad.is_empty() {
                Some(ConstraintViolation::violation(
                    focus_node,
                    shape_id,
                    path_str,
                    format!(
                        "sh:datatype {:?}: {} valor(es) no cumplen el tipo requerido",
                        dtype,
                        bad.len()
                    ),
                ))
            } else {
                None
            }
        }

        // --- Rangos numericos ---
        ConstraintType::MinInclusive(min) => {
            let bad: Vec<_> = values
                .iter()
                .filter(|v| !numeric_ge(v, *min))
                .collect();
            if !bad.is_empty() {
                Some(ConstraintViolation::violation(
                    focus_node,
                    shape_id,
                    path_str,
                    format!("sh:minInclusive {min}: valor fuera de rango"),
                ))
            } else {
                None
            }
        }

        ConstraintType::MaxInclusive(max) => {
            let bad: Vec<_> = values
                .iter()
                .filter(|v| !numeric_le(v, *max))
                .collect();
            if !bad.is_empty() {
                Some(ConstraintViolation::violation(
                    focus_node,
                    shape_id,
                    path_str,
                    format!("sh:maxInclusive {max}: valor fuera de rango"),
                ))
            } else {
                None
            }
        }

        ConstraintType::MinExclusive(min) => {
            let bad: Vec<_> = values
                .iter()
                .filter(|v| !numeric_gt(v, *min))
                .collect();
            if !bad.is_empty() {
                Some(ConstraintViolation::violation(
                    focus_node,
                    shape_id,
                    path_str,
                    format!("sh:minExclusive {min}: valor fuera de rango"),
                ))
            } else {
                None
            }
        }

        ConstraintType::MaxExclusive(max) => {
            let bad: Vec<_> = values
                .iter()
                .filter(|v| !numeric_lt(v, *max))
                .collect();
            if !bad.is_empty() {
                Some(ConstraintViolation::violation(
                    focus_node,
                    shape_id,
                    path_str,
                    format!("sh:maxExclusive {max}: valor fuera de rango"),
                ))
            } else {
                None
            }
        }

        // --- Longitud de strings ---
        ConstraintType::MinLength(min) => {
            let bad: Vec<_> = values
                .iter()
                .filter(|v| {
                    if let PropertyValue::String(s) = v {
                        s.chars().count() < *min
                    } else {
                        false
                    }
                })
                .collect();
            if !bad.is_empty() {
                Some(ConstraintViolation::violation(
                    focus_node,
                    shape_id,
                    path_str,
                    format!("sh:minLength {min}: cadena demasiado corta"),
                ))
            } else {
                None
            }
        }

        ConstraintType::MaxLength(max) => {
            let bad: Vec<_> = values
                .iter()
                .filter(|v| {
                    if let PropertyValue::String(s) = v {
                        s.chars().count() > *max
                    } else {
                        false
                    }
                })
                .collect();
            if !bad.is_empty() {
                Some(ConstraintViolation::violation(
                    focus_node,
                    shape_id,
                    path_str,
                    format!("sh:maxLength {max}: cadena demasiado larga"),
                ))
            } else {
                None
            }
        }

        // --- Patron regex ---
        ConstraintType::Pattern(pattern) => {
            #[cfg(feature = "shacl")]
            {
                use regex::Regex;
                match Regex::new(pattern) {
                    Ok(re) => {
                        let bad: Vec<_> = values
                            .iter()
                            .filter(|v| {
                                if let PropertyValue::String(s) = v {
                                    !re.is_match(s)
                                } else {
                                    true // no-string no conforma
                                }
                            })
                            .collect();
                        if !bad.is_empty() {
                            Some(ConstraintViolation::violation(
                                focus_node,
                                shape_id,
                                path_str,
                                format!("sh:pattern '{pattern}': valor no coincide con el patron"),
                            ))
                        } else {
                            None
                        }
                    }
                    Err(e) => Some(ConstraintViolation {
                        focus_node,
                        shape_id,
                        path: path_str,
                        message: format!("sh:pattern: patron regex invalido '{pattern}': {e}"),
                        severity: Severity::Warning,
                    }),
                }
            }
        }

        // --- Enumeracion ---
        ConstraintType::In(allowed) => {
            let bad: Vec<_> = values
                .iter()
                .filter(|v| !allowed.contains(v))
                .collect();
            if !bad.is_empty() {
                Some(ConstraintViolation::violation(
                    focus_node,
                    shape_id,
                    path_str,
                    format!(
                        "sh:in: {} valor(es) no estan en la lista de valores permitidos",
                        bad.len()
                    ),
                ))
            } else {
                None
            }
        }

        // --- Valor exacto ---
        ConstraintType::HasValue(expected) => {
            if !values.contains(expected) {
                Some(ConstraintViolation::violation(
                    focus_node,
                    shape_id,
                    path_str,
                    "sh:hasValue: valor requerido no encontrado".to_string(),
                ))
            } else {
                None
            }
        }

        // --- NodeKind y Class se evaluan en mod.rs sobre el nodo directamente ---
        // Estos constraints no operan sobre "valores" sino sobre el nodo mismo.
        // Se retorna None aqui; mod.rs los evalua por separado.
        ConstraintType::NodeKindConstraint(_) | ConstraintType::Class(_) => None,
    }
}

/// Evalua constraints de NodeKind directamente sobre un nodo.
pub fn evaluate_node_kind_constraint(
    node: &Node,
    constraint: &ConstraintType,
    shape_id: NodeId,
) -> Option<ConstraintViolation> {
    match constraint {
        ConstraintType::NodeKindConstraint(expected_kind) => {
            if node.kind != *expected_kind {
                Some(ConstraintViolation::violation(
                    node.id,
                    shape_id,
                    None,
                    format!(
                        "sh:nodeKind: se esperaba {:?}, se encontro {:?}",
                        expected_kind, node.kind
                    ),
                ))
            } else {
                None
            }
        }
        ConstraintType::Class(expected_label) => {
            if node.label != *expected_label {
                Some(ConstraintViolation::violation(
                    node.id,
                    shape_id,
                    None,
                    format!(
                        "sh:class: se esperaba label '{}', se encontro '{}'",
                        expected_label, node.label
                    ),
                ))
            } else {
                None
            }
        }
        _ => None,
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
    to_f64(v).map(|n| n >= limit).unwrap_or(false)
}

fn numeric_le(v: &PropertyValue, limit: f64) -> bool {
    to_f64(v).map(|n| n <= limit).unwrap_or(false)
}

fn numeric_gt(v: &PropertyValue, limit: f64) -> bool {
    to_f64(v).map(|n| n > limit).unwrap_or(false)
}

fn numeric_lt(v: &PropertyValue, limit: f64) -> bool {
    to_f64(v).map(|n| n < limit).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PropertyValue;
    use crate::shacl::DatatypeKind;
    use uuid::Uuid;

    fn node_id() -> NodeId { Uuid::new_v4() }
    fn shape_id() -> NodeId { Uuid::new_v4() }

    #[test]
    fn test_min_count_pass() {
        let vs = vec![PropertyValue::Int(1)];
        let result = evaluate_constraints(
            &[ConstraintType::MinCount(1)], &vs, node_id(), shape_id(), Some("age")
        );
        assert!(result.is_empty());
    }

    #[test]
    fn test_min_count_fail() {
        let vs: Vec<PropertyValue> = vec![];
        let result = evaluate_constraints(
            &[ConstraintType::MinCount(1)], &vs, node_id(), shape_id(), Some("age")
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_max_count_fail() {
        let vs = vec![PropertyValue::Int(1), PropertyValue::Int(2)];
        let result = evaluate_constraints(
            &[ConstraintType::MaxCount(1)], &vs, node_id(), shape_id(), Some("age")
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_datatype_int_pass() {
        let vs = vec![PropertyValue::Int(42)];
        let result = evaluate_constraints(
            &[ConstraintType::Datatype(DatatypeKind::Int)], &vs, node_id(), shape_id(), Some("age")
        );
        assert!(result.is_empty());
    }

    #[test]
    fn test_datatype_int_fail() {
        let vs = vec![PropertyValue::String("hello".into())];
        let result = evaluate_constraints(
            &[ConstraintType::Datatype(DatatypeKind::Int)], &vs, node_id(), shape_id(), Some("age")
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_min_inclusive_pass() {
        let vs = vec![PropertyValue::Int(10)];
        let result = evaluate_constraints(
            &[ConstraintType::MinInclusive(5.0)], &vs, node_id(), shape_id(), Some("score")
        );
        assert!(result.is_empty());
    }

    #[test]
    fn test_min_inclusive_fail() {
        let vs = vec![PropertyValue::Int(3)];
        let result = evaluate_constraints(
            &[ConstraintType::MinInclusive(5.0)], &vs, node_id(), shape_id(), Some("score")
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_in_pass() {
        let allowed = vec![
            PropertyValue::String("admin".into()),
            PropertyValue::String("user".into()),
        ];
        let vs = vec![PropertyValue::String("admin".into())];
        let result = evaluate_constraints(
            &[ConstraintType::In(allowed)], &vs, node_id(), shape_id(), Some("role")
        );
        assert!(result.is_empty());
    }

    #[test]
    fn test_in_fail() {
        let allowed = vec![
            PropertyValue::String("admin".into()),
            PropertyValue::String("user".into()),
        ];
        let vs = vec![PropertyValue::String("superuser".into())];
        let result = evaluate_constraints(
            &[ConstraintType::In(allowed)], &vs, node_id(), shape_id(), Some("role")
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_has_value_pass() {
        let vs = vec![PropertyValue::Bool(true)];
        let result = evaluate_constraints(
            &[ConstraintType::HasValue(PropertyValue::Bool(true))],
            &vs, node_id(), shape_id(), Some("active")
        );
        assert!(result.is_empty());
    }

    #[test]
    fn test_has_value_fail() {
        let vs = vec![PropertyValue::Bool(false)];
        let result = evaluate_constraints(
            &[ConstraintType::HasValue(PropertyValue::Bool(true))],
            &vs, node_id(), shape_id(), Some("active")
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_pattern_pass() {
        let vs = vec![PropertyValue::String("user@example.com".into())];
        let result = evaluate_constraints(
            &[ConstraintType::Pattern(r"^[^@]+@[^@]+\.[^@]+$".into())],
            &vs, node_id(), shape_id(), Some("email")
        );
        assert!(result.is_empty());
    }

    #[test]
    fn test_pattern_fail() {
        let vs = vec![PropertyValue::String("not-an-email".into())];
        let result = evaluate_constraints(
            &[ConstraintType::Pattern(r"^[^@]+@[^@]+\.[^@]+$".into())],
            &vs, node_id(), shape_id(), Some("email")
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_min_length_fail() {
        let vs = vec![PropertyValue::String("ab".into())];
        let result = evaluate_constraints(
            &[ConstraintType::MinLength(5)],
            &vs, node_id(), shape_id(), Some("name")
        );
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_max_length_fail() {
        let vs = vec![PropertyValue::String("toolongstring".into())];
        let result = evaluate_constraints(
            &[ConstraintType::MaxLength(5)],
            &vs, node_id(), shape_id(), Some("code")
        );
        assert_eq!(result.len(), 1);
    }
}
