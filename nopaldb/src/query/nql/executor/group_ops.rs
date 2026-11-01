//! The aggregation core that every path of NQL shares (#364): the SQL
//! aggregates over the rows of a group, and grouping in the order the groups
//! first appear.
//!
//! Until 0.10.10 each path had its own: the node path counted every node in
//! `count(x)`, divided `avg(x)` by every row, failed `min`/`max` with no
//! values, took only a property as argument and ordered the groups by the
//! text of the key (`10` before `9`), giving the keys back as text; the
//! one-hop path grouped with a `HashMap` (no order). A path only says how to
//! evaluate an expression over one of its rows.

use std::collections::HashMap;

use crate::error::{NopalError, Result};
use crate::query::nql::parser::ast::Expression;
use crate::types::PropertyValue;

/// Whether `name` is one of the SQL aggregates this module computes.
pub(crate) fn is_sql_aggregate(name: &str) -> bool {
    matches!(name.to_lowercase().as_str(), "count" | "sum" | "avg" | "min" | "max")
}

/// `name(args)` over `rows`, SQL's way: `count(*)` counts rows and `count(x)`
/// the values present; `sum` and `avg` take the numbers (`sum` of none is 0,
/// `avg` of none is null); `min` and `max` order as `order by` does and are
/// null with no values. `None` if `name` is not one of them. A missing value
/// and a stored null are both absent.
pub(crate) fn sql_aggregate<R>(
    name: &str,
    args: &[Expression],
    rows: &[R],
    eval: &mut dyn FnMut(&R, &Expression) -> Result<Option<PropertyValue>>,
) -> Result<Option<PropertyValue>> {
    let lower = name.to_lowercase();
    if !is_sql_aggregate(&lower) {
        return Ok(None);
    }
    let star = args.is_empty() || matches!(args.first(), Some(Expression::Wildcard));
    if lower == "count" && star {
        return Ok(Some(PropertyValue::Int(rows.len() as i64)));
    }
    let arg = match args.first() {
        Some(a) if !star => a,
        _ => return Err(NopalError::QueryExecutionError(format!("{name}() requires an argument"))),
    };
    let mut values = Vec::with_capacity(rows.len());
    for row in rows {
        match eval(row, arg)? {
            None | Some(PropertyValue::Null) => {}
            Some(v) => values.push(v),
        }
    }
    Ok(Some(match lower.as_str() {
        "count" => PropertyValue::Int(values.len() as i64),
        "sum" | "avg" => {
            let numbers: Vec<f64> = values
                .iter()
                .filter_map(|v| match v {
                    PropertyValue::Int(i) => Some(*i as f64),
                    PropertyValue::Float(f) => Some(*f),
                    _ => None,
                })
                .collect();
            let total: f64 = numbers.iter().sum();
            match (lower.as_str(), numbers.len()) {
                ("sum", _) => PropertyValue::Float(total),
                (_, 0) => PropertyValue::Null,
                (_, n) => PropertyValue::Float(total / n as f64),
            }
        }
        // The first of the smallest (largest), as a strict comparison gives.
        "min" => values.into_iter().reduce(|a, b| if b < a { b } else { a }).unwrap_or(PropertyValue::Null),
        _ => values.into_iter().reduce(|a, b| if b > a { b } else { a }).unwrap_or(PropertyValue::Null),
    }))
}

/// `rows` in groups by the values of `keys`, in the order each group first
/// appears, with the values of its key (a missing one is null). Two keys are
/// the same group when their values are written the same (`Int(3)` and
/// `Float(3.0)` are two groups, as before).
pub(crate) fn group_rows<R>(
    rows: Vec<R>,
    keys: &[Expression],
    eval: &mut dyn FnMut(&R, &Expression) -> Result<Option<PropertyValue>>,
) -> Result<Vec<(Vec<PropertyValue>, Vec<R>)>> {
    let mut groups: Vec<(Vec<PropertyValue>, Vec<R>)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for row in rows {
        let mut values = Vec::with_capacity(keys.len());
        let mut text = String::new();
        for key in keys {
            let v = eval(&row, key)?.unwrap_or(PropertyValue::Null);
            text.push_str(&format!("{v:?}\u{1f}"));
            values.push(v);
        }
        match index.get(&text) {
            Some(&i) => groups[i].1.push(row),
            None => {
                index.insert(text, groups.len());
                groups.push((values, vec![row]));
            }
        }
    }
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prop(p: &str) -> Expression {
        Expression::Property { variable: "n".into(), property: p.into() }
    }

    fn rows() -> Vec<Vec<(&'static str, PropertyValue)>> {
        vec![
            vec![("k", PropertyValue::String("b".into())), ("x", PropertyValue::Int(3))],
            vec![("k", PropertyValue::String("a".into())), ("x", PropertyValue::Float(1.5))],
            vec![("k", PropertyValue::String("b".into()))],
            vec![("k", PropertyValue::String("b".into())), ("x", PropertyValue::Null)],
            vec![("k", PropertyValue::String("a".into())), ("x", PropertyValue::String("t".into()))],
        ]
    }

    fn eval(row: &Vec<(&'static str, PropertyValue)>, e: &Expression) -> Result<Option<PropertyValue>> {
        let Expression::Property { property, .. } = e else { return Ok(None) };
        Ok(row.iter().find(|(k, _)| k == property).map(|(_, v)| v.clone()))
    }

    fn agg(name: &str, arg: Option<&str>) -> PropertyValue {
        let args: Vec<Expression> = arg.map(prop).into_iter().collect();
        sql_aggregate(name, &args, &rows(), &mut eval).unwrap().unwrap()
    }

    #[test]
    fn the_aggregates_are_sqls() {
        assert_eq!(agg("count", None), PropertyValue::Int(5));
        // Present values: 3, 1.5, "t" (a missing one and a null are absent).
        assert_eq!(agg("count", Some("x")), PropertyValue::Int(3));
        assert_eq!(agg("sum", Some("x")), PropertyValue::Float(4.5));
        assert_eq!(agg("avg", Some("x")), PropertyValue::Float(2.25));
        assert_eq!(agg("min", Some("x")), PropertyValue::Float(1.5));
        assert_eq!(agg("max", Some("x")), PropertyValue::String("t".into()));
        assert_eq!(agg("avg", Some("nada")), PropertyValue::Null);
        assert_eq!(agg("sum", Some("nada")), PropertyValue::Float(0.0));
        assert_eq!(agg("min", Some("nada")), PropertyValue::Null);
        assert!(sql_aggregate("degree", &[], &rows(), &mut eval).unwrap().is_none());
    }

    #[test]
    fn groups_come_in_the_order_they_first_appear() {
        let groups = group_rows(rows(), &[prop("k")], &mut eval).unwrap();
        let keys: Vec<_> = groups.iter().map(|(k, rows)| (k[0].clone(), rows.len())).collect();
        assert_eq!(keys, [(PropertyValue::String("b".into()), 3), (PropertyValue::String("a".into()), 2)]);
        // A missing key is a null group.
        let groups = group_rows(rows(), &[prop("x")], &mut eval).unwrap();
        assert!(groups.iter().any(|(k, rows)| k[0] == PropertyValue::Null && rows.len() == 2));
    }
}
