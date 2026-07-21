// Helpers: PropertyValue → serde_json, NqlResult → CallToolResult.
use nopaldb::types::PropertyValue;
use nopaldb::query::nql::{NqlResult, QueryResult};
use rmcp::model::{CallToolResult, Content};
use serde_json::{Value, json};

// ─── PropertyValue serialization ───────────────────────────────────────────

pub fn pv_to_json(pv: &PropertyValue) -> Value {
    match pv {
        PropertyValue::String(s)  => Value::String(s.clone()),
        PropertyValue::Int(n)     => json!(n),
        PropertyValue::Float(f)   => json!(f),
        PropertyValue::Bool(b)    => json!(b),
        PropertyValue::List(vs)   => Value::Array(vs.iter().map(pv_to_json).collect()),
        PropertyValue::Object(m)  => Value::Object(
            m.iter().map(|(k, v)| (k.clone(), pv_to_json(v))).collect(),
        ),
        _ => Value::Null,
    }
}

/// Convert a serde_json Value to a PropertyValue (inverse of `pv_to_json`).
pub fn json_to_pv(v: &Value) -> PropertyValue {
    match v {
        Value::Null => PropertyValue::Null,
        Value::Bool(b) => PropertyValue::Bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                PropertyValue::Int(i)
            } else {
                PropertyValue::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(s) => PropertyValue::String(s.clone()),
        Value::Array(a) => PropertyValue::List(a.iter().map(json_to_pv).collect()),
        Value::Object(m) => {
            PropertyValue::Object(m.iter().map(|(k, v)| (k.clone(), json_to_pv(v))).collect())
        }
    }
}

// ─── QueryResult → JSON ────────────────────────────────────────────────────

/// Convert a QueryResult to a JSON Value (array of row objects).
pub fn query_result_to_value(result: &QueryResult) -> Value {
    let rows: Vec<Value> = result.rows().iter().map(|row| {
        let mut obj = serde_json::Map::new();
        for col in &result.columns {
            if let Some(pv) = row.get(col) {
                obj.insert(col.clone(), pv_to_json(pv));
            }
        }
        Value::Object(obj)
    }).collect();
    Value::Array(rows)
}

// ─── NqlResult → CallToolResult ────────────────────────────────────────────

pub fn nql_result_to_tool(result: NqlResult, max_rows: usize) -> CallToolResult {
    match result {
        NqlResult::Query(mut qr) => {
            let total = qr.rows.len();
            let truncated = total > max_rows;
            if truncated {
                qr.rows.truncate(max_rows);
            }
            let v = query_result_to_value(&qr);
            // MCP `structured` requiere un objeto JSON, no un array.
            // Siempre envolvemos en {rows, total_returned, truncated?, note?}.
            let returned = match &v {
                Value::Array(arr) => arr.len(),
                _ => 0,
            };
            let mut obj = serde_json::Map::new();
            obj.insert("rows".to_string(), v);
            obj.insert("total_returned".to_string(), json!(returned));
            if truncated {
                obj.insert("truncated".to_string(), json!(true));
                obj.insert(
                    "note".to_string(),
                    json!(format!(
                        "{} rows total; only {} returned (set limit <= {} to see more)",
                        total, max_rows, max_rows
                    )),
                );
            }
            CallToolResult::structured(Value::Object(obj))
        }
        NqlResult::Write(w) => {
            CallToolResult::structured(json!({
                "nodes_created":  w.nodes_created,
                "edges_created":  w.edges_created,
                "nodes_deleted":  w.nodes_deleted,
                "edges_deleted":  w.edges_deleted,
                "nodes_updated":  w.nodes_updated,
                "edges_updated":  w.edges_updated,
                "created_ids":    w.created_ids,
            }))
        }
        NqlResult::Index(msg)  => CallToolResult::success(vec![Content::text(msg)]),
        NqlResult::Explain(p)  => CallToolResult::success(vec![Content::text(p)]),
        NqlResult::Profile(p)  => CallToolResult::structured(json!({
            "statement_type": p.statement_type,
            "execution_ms":   p.execution_ms,
            "rows_returned":  p.rows_returned,
            "plan":           p.plan,
        })),
        NqlResult::Export { format, data, rows_exported } => {
            CallToolResult::structured(json!({
                "format":        format,
                "rows_exported": rows_exported,
                "data":          data,
            }))
        }
        NqlResult::Message(msg) => CallToolResult::success(vec![Content::text(msg)]),
    }
}

// ─── Error helpers ─────────────────────────────────────────────────────────

pub fn tool_error(msg: impl std::fmt::Display) -> CallToolResult {
    CallToolResult::error(vec![Content::text(format!("{}", msg))])
}

pub fn readonly_error() -> CallToolResult {
    tool_error("Server is in read-only mode. Write operations are not allowed.")
}

/// Returns true if the NQL statement looks like a write operation.
pub fn is_write_statement(nql: &str) -> bool {
    let first = nql.split_whitespace().next().unwrap_or("").to_lowercase();
    matches!(first.as_str(), "add" | "update" | "delete" | "create" | "drop")
}
