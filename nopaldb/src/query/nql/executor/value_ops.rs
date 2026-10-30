//! What the NQL operators do to values (#269): arithmetic, negation and
//! comparison, one definition for every evaluator.
//!
//! Until 0.10.4 each evaluator had its own idea, or none: the path VM
//! computed `+ - * / %` (with `Int / Int` giving a `Float`), the condition
//! evaluators let `!(…)` through as `true`, the scalar ones turned
//! arithmetic into "no value", and `=` used the derived `PartialEq`
//! (`Int(3) != Float(3.0)`) while `<`/`>` used `Ord`, which equates them.
//! The evaluators still resolve properties and functions their own way
//! (a node, a match, a path); what the operators mean lives here.
//!
//! - Integer arithmetic is checked: an overflow is an error, not a panic in
//!   debug and a wrap in release. `Int / Int` and `Int % Int` are integers
//!   (truncated, as in SQL and Cypher); by zero is an error.
//! - `Int` with `Float` is a `Float`.
//! - `Null` in arithmetic gives `Null`, as in SQL.
//! - `=`, `!=`, `in`, `not in` compare numbers by value: `3 = 3.0`.

use crate::error::{NopalError, Result};
use crate::query::nql::parser::ast::{BinaryOperator, Expression, UnaryOperator};
use crate::types::PropertyValue;
use std::cmp::Ordering;

pub(crate) fn is_arithmetic(op: &BinaryOperator) -> bool {
    matches!(
        op,
        BinaryOperator::Add | BinaryOperator::Sub | BinaryOperator::Mul | BinaryOperator::Div | BinaryOperator::Mod
    )
}

fn symbol(op: &BinaryOperator) -> &'static str {
    match op {
        BinaryOperator::Add => "+",
        BinaryOperator::Sub => "-",
        BinaryOperator::Mul => "*",
        BinaryOperator::Div => "/",
        BinaryOperator::Mod => "%",
        _ => "?",
    }
}

fn error(msg: String) -> NopalError {
    NopalError::QueryExecutionError(msg)
}

/// `left op right` for `+ - * / %`.
pub(crate) fn arith(op: &BinaryOperator, left: &PropertyValue, right: &PropertyValue) -> Result<PropertyValue> {
    use PropertyValue::{Float, Int, Null};
    let s = symbol(op);
    match (left, right) {
        (Null, _) | (_, Null) => Ok(Null),
        (Int(a), Int(b)) => {
            let (a, b) = (*a, *b);
            let value = match op {
                BinaryOperator::Add => a.checked_add(b),
                BinaryOperator::Sub => a.checked_sub(b),
                BinaryOperator::Mul => a.checked_mul(b),
                BinaryOperator::Div | BinaryOperator::Mod if b == 0 => {
                    return Err(error(format!("{a} {s} 0: division by zero")));
                }
                // `i64::MIN / -1` overflows; `i64::MIN % -1` is 0.
                BinaryOperator::Div => a.checked_div(b),
                BinaryOperator::Mod => Some(a.wrapping_rem(b)),
                _ => return Err(error(format!("`{s}` is not an arithmetic operator"))),
            };
            value
                .map(Int)
                .ok_or_else(|| error(format!("{a} {s} {b}: integer overflow")))
        }
        (Int(_) | Float(_), Int(_) | Float(_)) => {
            let (a, b) = (as_f64(left), as_f64(right));
            match op {
                BinaryOperator::Add => Ok(Float(a + b)),
                BinaryOperator::Sub => Ok(Float(a - b)),
                BinaryOperator::Mul => Ok(Float(a * b)),
                BinaryOperator::Div | BinaryOperator::Mod if b == 0.0 => {
                    Err(error(format!("{a:?} {s} 0: division by zero")))
                }
                BinaryOperator::Div => Ok(Float(a / b)),
                BinaryOperator::Mod => Ok(Float(a % b)),
                _ => Err(error(format!("`{s}` is not an arithmetic operator"))),
            }
        }
        _ => Err(error(format!(
            "`{s}` needs two numbers, got {} and {}",
            type_name(left),
            type_name(right)
        ))),
    }
}

/// `-value`.
pub(crate) fn negate(value: &PropertyValue) -> Result<PropertyValue> {
    match value {
        PropertyValue::Null => Ok(PropertyValue::Null),
        PropertyValue::Int(i) => i
            .checked_neg()
            .map(PropertyValue::Int)
            .ok_or_else(|| error(format!("-({i}): integer overflow"))),
        PropertyValue::Float(f) => Ok(PropertyValue::Float(-f)),
        other => Err(error(format!("`-` needs a number, got {}", type_name(other)))),
    }
}

/// `!value` / `not value`.
pub(crate) fn not(value: &PropertyValue) -> Result<PropertyValue> {
    match value {
        PropertyValue::Null => Ok(PropertyValue::Null),
        PropertyValue::Bool(b) => Ok(PropertyValue::Bool(!b)),
        other => Err(error(format!("`not` needs a boolean, got {}", type_name(other)))),
    }
}

pub(crate) fn unary(op: &UnaryOperator, value: &PropertyValue) -> Result<PropertyValue> {
    match op {
        UnaryOperator::Neg => negate(value),
        UnaryOperator::Not => not(value),
    }
}

/// Equality as NQL means it: numbers by value (`3 = 3.0`), lists element by
/// element with the same rule, everything else as stored.
pub(crate) fn equal(left: &PropertyValue, right: &PropertyValue) -> bool {
    use PropertyValue::{Float, Int, List};
    match (left, right) {
        (Int(_) | Float(_), Int(_) | Float(_)) => left.cmp(right) == Ordering::Equal,
        (List(a), List(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| equal(x, y)),
        _ => left == right,
    }
}

/// `left op right` for the comparison operators. Ordering uses
/// `PropertyValue`'s `Ord` (numbers by value, as before).
pub(crate) fn compare(left: &PropertyValue, op: &BinaryOperator, right: &PropertyValue) -> bool {
    match op {
        BinaryOperator::Eq => equal(left, right),
        BinaryOperator::NotEq => !equal(left, right),
        BinaryOperator::Gt => left > right,
        BinaryOperator::Lt => left < right,
        BinaryOperator::GtEq => left >= right,
        BinaryOperator::LtEq => left <= right,
        // `x in [..]`: membership with the equality of `=`. A right side
        // that is not a list is `false` (the parser does not produce it).
        BinaryOperator::In => matches!(right, PropertyValue::List(xs) if xs.iter().any(|x| equal(left, x))),
        BinaryOperator::NotIn => !matches!(right, PropertyValue::List(xs) if xs.iter().any(|x| equal(left, x))),
        _ => false,
    }
}

/// The value of `expr`, with the operators of NQL and `leaf` for what each
/// evaluator resolves its own way (literals, properties, functions).
/// `Ok(None)` is "no value" (a missing property): arithmetic over it has no
/// value either, and a comparison with it is false.
pub(crate) fn scalar<F>(expr: &Expression, leaf: &mut F) -> Result<Option<PropertyValue>>
where
    F: FnMut(&Expression) -> Result<Option<PropertyValue>>,
{
    match expr {
        Expression::BinaryOp { left, op, right } if is_arithmetic(op) => {
            match (scalar(left, leaf)?, scalar(right, leaf)?) {
                (Some(l), Some(r)) => arith(op, &l, &r).map(Some),
                _ => Ok(None),
            }
        }
        Expression::BinaryOp { .. } | Expression::UnaryOp { op: UnaryOperator::Not, .. } => {
            condition(expr, leaf).map(|b| Some(PropertyValue::Bool(b)))
        }
        Expression::UnaryOp { op, expr: inner } => match scalar(inner, leaf)? {
            Some(v) => unary(op, &v).map(Some),
            None => Ok(None),
        },
        _ => leaf(expr),
    }
}

/// Whether `expr` holds, with the operators of NQL and `leaf` as in
/// [`scalar`]. Two-valued: a comparison with a missing value is false, and
/// `not` negates whether the condition holds.
pub(crate) fn condition<F>(expr: &Expression, leaf: &mut F) -> Result<bool>
where
    F: FnMut(&Expression) -> Result<Option<PropertyValue>>,
{
    match expr {
        Expression::BinaryOp { left, op: BinaryOperator::And, right } => {
            Ok(condition(left, leaf)? && condition(right, leaf)?)
        }
        Expression::BinaryOp { left, op: BinaryOperator::Or, right } => {
            Ok(condition(left, leaf)? || condition(right, leaf)?)
        }
        Expression::UnaryOp { op: UnaryOperator::Not, expr: inner } => Ok(!condition(inner, leaf)?),
        Expression::BinaryOp { left, op, right } if !is_arithmetic(op) => {
            match (scalar(left, leaf)?, scalar(right, leaf)?) {
                (Some(l), Some(r)) => Ok(compare(&l, op, &r)),
                _ => Ok(false),
            }
        }
        _ => match scalar(expr, leaf)? {
            Some(PropertyValue::Bool(b)) => Ok(b),
            None | Some(PropertyValue::Null) => Ok(false),
            Some(other) => Err(error(format!("`{expr}` is not a condition: it gives {}", type_name(&other)))),
        },
    }
}

fn as_f64(v: &PropertyValue) -> f64 {
    match v {
        PropertyValue::Int(i) => *i as f64,
        PropertyValue::Float(f) => *f,
        _ => f64::NAN,
    }
}

fn type_name(v: &PropertyValue) -> &'static str {
    match v {
        PropertyValue::Null => "null",
        PropertyValue::Bool(_) => "a boolean",
        PropertyValue::Int(_) => "an integer",
        PropertyValue::Float(_) => "a float",
        PropertyValue::String(_) => "a string",
        PropertyValue::Bytes(_) => "bytes",
        PropertyValue::Object(_) => "an object",
        PropertyValue::List(_) => "a list",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use BinaryOperator as B;
    use PropertyValue::{Float, Int, Null};

    fn err(r: Result<PropertyValue>) -> String {
        r.unwrap_err().to_string()
    }

    #[test]
    fn integer_arithmetic_is_exact_and_checked() {
        assert_eq!(arith(&B::Add, &Int(2), &Int(3)).unwrap(), Int(5));
        assert_eq!(arith(&B::Div, &Int(7), &Int(2)).unwrap(), Int(3));
        assert_eq!(arith(&B::Div, &Int(-7), &Int(2)).unwrap(), Int(-3));
        assert_eq!(arith(&B::Mod, &Int(7), &Int(-2)).unwrap(), Int(1));
        assert_eq!(arith(&B::Mod, &Int(i64::MIN), &Int(-1)).unwrap(), Int(0));
        assert!(err(arith(&B::Add, &Int(i64::MAX), &Int(1))).contains("overflow"));
        assert!(err(arith(&B::Mul, &Int(i64::MAX), &Int(2))).contains("overflow"));
        assert!(err(arith(&B::Div, &Int(i64::MIN), &Int(-1))).contains("overflow"));
        assert!(err(arith(&B::Div, &Int(1), &Int(0))).contains("division by zero"));
        assert!(err(arith(&B::Mod, &Int(1), &Int(0))).contains("division by zero"));
        assert!(err(negate(&Int(i64::MIN))).contains("overflow"));
    }

    #[test]
    fn mixed_numbers_are_floats_and_null_propagates() {
        assert_eq!(arith(&B::Mul, &Int(2), &Float(1.5)).unwrap(), Float(3.0));
        assert_eq!(arith(&B::Div, &Float(7.0), &Int(2)).unwrap(), Float(3.5));
        assert!(err(arith(&B::Div, &Float(1.0), &Float(0.0))).contains("division by zero"));
        assert_eq!(arith(&B::Add, &Null, &Int(1)).unwrap(), Null);
        assert_eq!(negate(&Null).unwrap(), Null);
        assert!(err(arith(&B::Add, &PropertyValue::String("a".into()), &Int(1))).contains("needs two numbers"));
        assert!(err(not(&Int(1))).contains("needs a boolean"));
    }

    #[test]
    fn numbers_are_equal_by_value_like_the_order() {
        assert!(compare(&Int(3), &B::Eq, &Float(3.0)));
        assert!(!compare(&Int(3), &B::NotEq, &Float(3.0)));
        assert!(compare(&Int(3), &B::GtEq, &Float(3.0)) && compare(&Int(3), &B::LtEq, &Float(3.0)));
        assert!(compare(&Float(2.0), &B::In, &PropertyValue::List(vec![Int(1), Int(2)])));
        assert!(!compare(&PropertyValue::String("3".into()), &B::Eq, &Int(3)));
    }
}
