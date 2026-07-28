// Helpers: PropertyValue → serde_json, NqlResult → CallToolResult.
use nopaldb::types::PropertyValue;
use nopaldb::query::nql::{NqlResult, QueryResult};
use rmcp::model::{CallToolResult, Content};
use serde_json::{Value, json};

// ─── PropertyValue serialization ───────────────────────────────────────────
// Wrappers delgados sobre el puente canónico de nopaldb (types.rs), que usa
// matches exhaustivos: una variante nueva de PropertyValue rompe compilación
// en el core en vez de serializar null en silencio (el match local anterior
// tenía `_ => Value::Null` y colapsaba Null Y Bytes).

pub fn pv_to_json(pv: &PropertyValue) -> Value {
    Value::from(pv)
}

/// Convert a serde_json Value to a PropertyValue (inverse of `pv_to_json`).
pub fn json_to_pv(v: &Value) -> PropertyValue {
    PropertyValue::from(v)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pv_json_roundtrip_all_variants() {
        // Escalares y estructuras roundtripean tipo-exacto
        for pv in [
            PropertyValue::Null,
            PropertyValue::Bool(true),
            PropertyValue::Int(-7),
            PropertyValue::Float(2.5),
            PropertyValue::String("a".into()),
            PropertyValue::List(vec![PropertyValue::Int(1), PropertyValue::Bool(false)]),
            PropertyValue::Object(vec![("k".to_string(), PropertyValue::String("v".into()))]),
        ] {
            assert_eq!(json_to_pv(&pv_to_json(&pv)), pv, "roundtrip de {pv:?}");
        }
    }

    #[test]
    fn bytes_now_serialize_as_array_not_null() {
        // Bugfix pinneado: el match local anterior colapsaba Bytes en null.
        let pv = PropertyValue::Bytes(vec![1, 2, 3]);
        assert_eq!(pv_to_json(&pv), serde_json::json!([1, 2, 3]));
        // Lossiness documentada del puente: regresa como List(Int)
        assert_eq!(
            json_to_pv(&pv_to_json(&pv)),
            PropertyValue::List(vec![
                PropertyValue::Int(1),
                PropertyValue::Int(2),
                PropertyValue::Int(3)
            ])
        );
    }
}
