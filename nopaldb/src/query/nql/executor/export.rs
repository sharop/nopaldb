// src/query/nql/executor/export.rs
//
// Export executor: converts QueryResult to CSV, JSON, Arrow IPC or Parquet
// Triggered by the EXPORT clause in NQL queries.
//
// Syntax:
//   find p.name, p.age from (p:Person) export csv
//   find p.name, p.age from (p:Person) export json
//   find p.name, p.age from (p:Person) export arrow with path="out.arrow"
//   find p.name, p.age from (p:Person) export parquet "out.parquet"
//   find * from (p:Person) export csv with separator="|"

use crate::error::{NopalError, Result};
use crate::types::PropertyValue;
use super::result::{QueryResult, NqlResult};
use crate::query::nql::parser::ast::{ExportClause, ExportFormat};

/// Opciones válidas de cada formato. Una opción fuera de la lista es error:
/// hasta 0.9.3 se ignoraban en silencio, así que `delimiter=";"` (el nombre
/// que documentaba la referencia) exportaba con comas sin avisar. No se
/// acepta `delimiter` como alias de `separator`: dos nombres para lo mismo
/// son superficie que hay que documentar y probar para siempre; el error
/// sugiere el nombre bueno.
const CSV_OPTIONS: &[&str] = &["path", "separator", "header"];
const JSON_OPTIONS: &[&str] = &["path", "pretty", "jsonl"];
const ARROW_OPTIONS: &[&str] = &["path"];
/// La ruta va en la sintaxis (`export parquet "ruta"`), no como opción.
const PARQUET_OPTIONS: &[&str] = &[];

/// Execute export: convert a QueryResult into the requested format
///
/// Contrato (docs/NQL_SEMANTICS.md § 5e): o se hace lo pedido o es error.
/// `Export.data` lleva el contenido exportado (CSV/JSON) o, en los formatos
/// binarios (que siempre van a archivo), la ruta escrita.
pub fn execute_export(result: &QueryResult, export: &ExportClause) -> Result<NqlResult> {
    let rows_exported = result.len();

    let (format_name, data) = match &export.format {
        ExportFormat::Csv => {
            check_options(export, "csv", CSV_OPTIONS)?;
            let sep = option_as_str(export, "separator")?.unwrap_or(",");
            let include_header = option_as_bool(export, "header", true);
            let csv = export_csv(result, sep, include_header);

            if let Some(path) = option_as_str(export, "path")? {
                std::fs::write(path, &csv)
                    .map_err(|e| NopalError::QueryExecutionError(format!("Failed to write CSV export to '{}': {}", path, e)))?;
            }

            ("CSV".to_string(), csv)
        }
        ExportFormat::Json => {
            check_options(export, "json", JSON_OPTIONS)?;
            let pretty = option_as_bool(export, "pretty", false);
            let jsonl = option_as_bool(export, "jsonl", false);
            let json = if jsonl {
                export_jsonl(result)
            } else {
                export_json(result, pretty)
            };

            if let Some(path) = option_as_str(export, "path")? {
                std::fs::write(path, &json)
                    .map_err(|e| NopalError::QueryExecutionError(format!("Failed to write JSON export to '{}': {}", path, e)))?;
            }

            ("JSON".to_string(), json)
        }
        ExportFormat::Arrow => {
            check_options(export, "arrow", ARROW_OPTIONS)?;
            // Sin `path` no hay dónde poner el binario: `data` es texto. Hasta
            // 0.9.3 se devolvía un resumen ("Arrow RecordBatch: …") como si
            // fuera la exportación.
            let path = option_as_str(export, "path")?.ok_or_else(|| {
                NopalError::QueryExecutionError(
                    "export arrow needs a file: use `export arrow with path=\"file.arrow\"` \
                     (inline export is text only: use `export csv` or `export json`)"
                        .to_string(),
                )
            })?;
            write_binary(result, BinaryFormat::Arrow, path)?;
            ("Arrow".to_string(), path.to_string())
        }
        ExportFormat::Parquet(path) => {
            check_options(export, "parquet", PARQUET_OPTIONS)?;
            write_binary(result, BinaryFormat::Parquet, path)?;
            ("Parquet".to_string(), path.clone())
        }
    };

    Ok(NqlResult::Export {
        format: format_name,
        data,
        rows_exported,
    })
}

#[derive(Clone, Copy)]
enum BinaryFormat {
    Arrow,
    Parquet,
}

impl BinaryFormat {
    fn name(self) -> &'static str {
        match self {
            BinaryFormat::Arrow => "Arrow",
            BinaryFormat::Parquet => "Parquet",
        }
    }
}

/// Escribe el resultado como Arrow IPC o Parquet con los escritores de
/// `arrow_export` (el de Parquet es el mismo de `Graph::export_parquet`).
#[cfg(feature = "analytics")]
fn write_binary(result: &QueryResult, format: BinaryFormat, path: &str) -> Result<()> {
    let rows: Vec<_> = result.rows.iter().map(|row| &row.values).collect();
    let batch = crate::arrow_export::table_to_arrow(&result.columns, &rows)?;
    let written = match format {
        BinaryFormat::Arrow => crate::arrow_export::write_arrow_ipc(&batch, path),
        BinaryFormat::Parquet => crate::arrow_export::write_parquet(&batch, path),
    };
    written.map_err(|e| {
        NopalError::QueryExecutionError(format!(
            "Failed to write {} export to '{}': {}",
            format.name(),
            path,
            e
        ))
    })
}

/// Sin `analytics` no están las dependencias `arrow`/`parquet`: error con
/// nombre en vez de responder "Exported N rows" sin escribir nada.
#[cfg(not(feature = "analytics"))]
fn write_binary(_result: &QueryResult, format: BinaryFormat, _path: &str) -> Result<()> {
    Err(NopalError::QueryExecutionError(format!(
        "export {} requires the `analytics` feature, which this build does not include; \
         use `export csv` or `export json`",
        format.name().to_lowercase()
    )))
}

/// Rechaza las opciones que el formato no conoce, con la lista de las válidas.
fn check_options(export: &ExportClause, format: &str, valid: &[&str]) -> Result<()> {
    let mut unknown: Vec<&str> = export
        .options
        .keys()
        .map(String::as_str)
        .filter(|key| !valid.contains(key))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    unknown.sort_unstable();
    let valid_list = if valid.is_empty() { "none".to_string() } else { valid.join(", ") };
    let hint = if format == "csv" && unknown.contains(&"delimiter") {
        " (the delimiter option is called `separator`)"
    } else if format == "parquet" && unknown.contains(&"path") {
        " (the path goes in the syntax: `export parquet \"file.parquet\"`)"
    } else {
        ""
    };
    Err(NopalError::QueryExecutionError(format!(
        "unknown export {} option{}: {}; valid options: {}{}",
        format,
        if unknown.len() == 1 { "" } else { "s" },
        unknown.join(", "),
        valid_list,
        hint
    )))
}

/// Export QueryResult to CSV string
fn export_csv(result: &QueryResult, separator: &str, include_header: bool) -> String {
    let mut output = String::new();

    // Header
    if include_header {
        output.push_str(&result.columns.join(separator));
        output.push('\n');
    }

    // Rows
    for row in &result.rows {
        let values: Vec<String> = result.columns.iter()
            .map(|col| {
                match row.values.get(col) {
                    Some(val) => csv_escape(&property_to_string(val), separator),
                    None => String::new(),
                }
            })
            .collect();
        output.push_str(&values.join(separator));
        output.push('\n');
    }

    output
}

/// Export QueryResult to JSON string
fn export_json(result: &QueryResult, pretty: bool) -> String {
    let mut rows_json = Vec::new();

    for row in &result.rows {
        let mut obj_parts = Vec::new();
        for col in &result.columns {
            let val = match row.values.get(col) {
                Some(v) => property_to_json(v),
                None => "null".to_string(),
            };
            obj_parts.push(format!("\"{}\":{}", json_escape(col), val));
        }
        rows_json.push(format!("{{{}}}", obj_parts.join(",")));
    }

    if pretty {
        // Simple pretty print
        let mut output = String::from("[\n");
        for (i, row) in rows_json.iter().enumerate() {
            output.push_str("  ");
            output.push_str(row);
            if i < rows_json.len() - 1 {
                output.push(',');
            }
            output.push('\n');
        }
        output.push(']');
        output
    } else {
        format!("[{}]", rows_json.join(","))
    }
}

/// Export QueryResult as JSONL (one JSON object per line)
fn export_jsonl(result: &QueryResult) -> String {
    let mut lines = Vec::new();

    for row in &result.rows {
        let mut obj_parts = Vec::new();
        for col in &result.columns {
            let val = match row.values.get(col) {
                Some(v) => property_to_json(v),
                None => "null".to_string(),
            };
            obj_parts.push(format!("\"{}\":{}", json_escape(col), val));
        }
        lines.push(format!("{{{}}}", obj_parts.join(",")));
    }

    lines.join("\n")
}

// ═══════════════════════════════════════════════════════════
// PUBLIC API — called by QueryResult methods
// ═══════════════════════════════════════════════════════════

/// Convert QueryResult to CSV (public, for Rust API)
pub fn query_result_to_csv(result: &QueryResult, separator: &str, include_header: bool) -> String {
    export_csv(result, separator, include_header)
}

/// Convert QueryResult to JSON (public, for Rust API)
pub fn query_result_to_json(result: &QueryResult, pretty: bool) -> String {
    export_json(result, pretty)
}

// ═══════════════════════════════════════════════════════════
// HELPERS
// ═══════════════════════════════════════════════════════════

// ⚠️ Formato user-facing del EXPORT, congelado por goldens en
// tests/export_test.rs. NO delegar en Display/to_display_string ni en el
// puente serde_json (escapes y semántica de NaN propios).
fn property_to_string(val: &PropertyValue) -> String {
    match val {
        PropertyValue::String(s) => s.clone(),
        PropertyValue::Int(i) => i.to_string(),
        PropertyValue::Float(f) => f.to_string(),
        PropertyValue::Bool(b) => b.to_string(),
        PropertyValue::Null => String::new(),
        PropertyValue::Bytes(b) => format!("<{} bytes>", b.len()),
        PropertyValue::List(_) | PropertyValue::Object(_) => property_to_json(val),
    }
}

fn property_to_json(val: &PropertyValue) -> String {
    match val {
        PropertyValue::String(s) => format!("\"{}\"", json_escape(s)),
        PropertyValue::Int(i) => i.to_string(),
        PropertyValue::Float(f) => {
            if f.is_nan() || f.is_infinite() { "null".to_string() }
            else { f.to_string() }
        }
        PropertyValue::Bool(b) => b.to_string(),
        PropertyValue::Null => "null".to_string(),
        PropertyValue::Bytes(b) => format!("\"<{} bytes>\"", b.len()),
        PropertyValue::List(values) => {
            let items = values.iter().map(property_to_json).collect::<Vec<_>>().join(",");
            format!("[{}]", items)
        }
        PropertyValue::Object(fields) => {
            let items = fields.iter()
                .map(|(key, value)| format!("\"{}\":{}", json_escape(key), property_to_json(value)))
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{}}}", items)
        }
    }
}

fn csv_escape(value: &str, separator: &str) -> String {
    if value.contains(separator) || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Opción de texto. Presente con otro tipo (`path=3`) es error: si no, la
/// exportación respondería éxito sin escribir el archivo.
fn option_as_str<'a>(export: &'a ExportClause, key: &str) -> Result<Option<&'a str>> {
    match export.options.get(key) {
        None => Ok(None),
        Some(PropertyValue::String(s)) => Ok(Some(s.as_str())),
        Some(other) => Err(NopalError::QueryExecutionError(format!(
            "export option '{}' must be a string, got {:?}",
            key, other
        ))),
    }
}

fn option_as_bool(export: &ExportClause, key: &str, default: bool) -> bool {
    export.options.get(key).map(|v| {
        match v {
            PropertyValue::Bool(b) => *b,
            PropertyValue::String(s) => s != "false",
            _ => default,
        }
    }).unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::nql::executor::result::Row;

    fn make_result() -> QueryResult {
        let mut result = QueryResult::new(vec!["name".into(), "age".into()]);
        let mut r1 = Row::new();
        r1.set("name", PropertyValue::String("Alice".into()));
        r1.set("age", PropertyValue::Int(30));
        result.add_row(r1);
        let mut r2 = Row::new();
        r2.set("name", PropertyValue::String("Bob".into()));
        r2.set("age", PropertyValue::Int(25));
        result.add_row(r2);
        result
    }

    #[test]
    fn test_csv_export() {
        let result = make_result();
        let csv = export_csv(&result, ",", true);
        assert!(csv.starts_with("name,age\n"));
        assert!(csv.contains("Alice,30"));
        assert!(csv.contains("Bob,25"));
    }

    #[test]
    fn test_csv_custom_separator() {
        let result = make_result();
        let csv = export_csv(&result, "|", true);
        assert!(csv.starts_with("name|age\n"));
        assert!(csv.contains("Alice|30"));
    }

    #[test]
    fn test_csv_no_header() {
        let result = make_result();
        let csv = export_csv(&result, ",", false);
        assert!(!csv.starts_with("name,age"));
        assert!(csv.contains("Alice,30"));
    }

    #[test]
    fn test_json_export() {
        let result = make_result();
        let json = export_json(&result, false);
        assert!(json.starts_with('['));
        assert!(json.ends_with(']'));
        assert!(json.contains("\"name\":\"Alice\""));
        assert!(json.contains("\"age\":30"));
    }

    #[test]
    fn test_json_pretty() {
        let result = make_result();
        let json = export_json(&result, true);
        assert!(json.contains('\n'));
        assert!(json.contains("  {"));
    }

    #[test]
    fn test_csv_escape_comma_in_value() {
        let mut result = QueryResult::new(vec!["desc".into()]);
        let mut row = Row::new();
        row.set("desc", PropertyValue::String("hello, world".into()));
        result.add_row(row);
        let csv = export_csv(&result, ",", false);
        assert!(csv.contains("\"hello, world\""));
    }
}
