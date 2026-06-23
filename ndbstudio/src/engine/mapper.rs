use nopaldb::{NqlResult, PropertyValue};

pub struct TabularResult {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

pub fn to_tabular_result(result: NqlResult) -> TabularResult {
    match result {
        NqlResult::Query(query_result) => {
            let headers = query_result.columns.clone();
            let rows = query_result
                .rows
                .iter()
                .map(|row| {
                    headers
                        .iter()
                        .map(|column| {
                            row.values
                                .get(column)
                                .map(property_value_to_string)
                                .unwrap_or_else(|| "null".to_string())
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();

            TabularResult { headers, rows }
        }
        NqlResult::Write(write) => TabularResult {
            headers: vec![
                "nodes_created".to_string(),
                "edges_created".to_string(),
                "nodes_deleted".to_string(),
                "edges_deleted".to_string(),
                "nodes_updated".to_string(),
                "properties_changed".to_string(),
            ],
            rows: vec![vec![
                write.nodes_created.to_string(),
                write.edges_created.to_string(),
                write.nodes_deleted.to_string(),
                write.edges_deleted.to_string(),
                write.nodes_updated.to_string(),
                write.properties_changed.to_string(),
            ]],
        },
        NqlResult::Index(message) | NqlResult::Message(message) => TabularResult {
            headers: vec!["result".to_string()],
            rows: vec![vec![message]],
        },
        NqlResult::Explain(plan) => TabularResult {
            headers: vec!["plan".to_string()],
            rows: vec![vec![plan]],
        },
        NqlResult::Export {
            format,
            data,
            rows_exported,
        } => TabularResult {
            headers: vec![
                "format".to_string(),
                "rows_exported".to_string(),
                "data".to_string(),
            ],
            rows: vec![vec![format, rows_exported.to_string(), data]],
        },
        NqlResult::Profile(profile) => TabularResult {
            headers: vec!["metric".to_string(), "value".to_string()],
            rows: vec![
                vec!["statement_type".to_string(), profile.statement_type],
                vec![
                    "execution_ms".to_string(),
                    format!("{:.3}", profile.execution_ms),
                ],
                vec![
                    "rows_returned".to_string(),
                    profile.rows_returned.to_string(),
                ],
                vec!["path_query".to_string(), profile.path_query.to_string()],
                vec!["plan".to_string(), profile.plan],
            ],
        },
    }
}

pub fn property_value_to_string(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Null => "null".to_string(),
        PropertyValue::Bool(v) => v.to_string(),
        PropertyValue::Int(v) => v.to_string(),
        PropertyValue::Float(v) => {
            if v.is_finite() {
                v.to_string()
            } else {
                "null".to_string()
            }
        }
        PropertyValue::String(v) => v.clone(),
        PropertyValue::Bytes(v) => format!("<{} bytes>", v.len()),
        PropertyValue::List(items) => {
            let inner: Vec<String> = items.iter().map(property_value_to_string).collect();
            format!("[{}]", inner.join(", "))
        }
        PropertyValue::Object(fields) => {
            let inner: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("{}: {}", k, property_value_to_string(v)))
                .collect();
            format!("{{{}}}", inner.join(", "))
        }
    }
}
