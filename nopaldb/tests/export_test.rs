// tests/export_test.rs
//
// P1: EXPORT clause integration tests (CSV, JSON, Arrow)

use nopaldb::{Graph, Node, PropertyValue, NqlResult, Result};

async fn setup_graph() -> Result<Graph> {
    let graph = Graph::in_memory().await?;
    let mut tx = graph.begin_transaction().await?;
    for (name, age) in &[("Alice", 30), ("Bob", 25), ("Charlie", 35)] {
        tx.add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String(name.to_string()))
                .with_property("age", PropertyValue::Int(*age))
        ).await?;
    }
    tx.commit().await?;
    Ok(graph)
}

// ═══════════════════════════════════════════════════════════
// CSV EXPORT
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_export_csv() -> Result<()> {
    let graph = setup_graph().await?;

    let result = graph.execute_statement(
        "find p.name, p.age from (p:Person) export csv"
    ).await?;

    match &result {
        NqlResult::Export { format, data, rows_exported } => {
            assert_eq!(format, "CSV");
            assert_eq!(*rows_exported, 3);

            // Should have header
            let lines: Vec<&str> = data.trim().lines().collect();
            assert_eq!(lines[0], "p.name,p.age");
            assert_eq!(lines.len(), 4); // header + 3 rows

            // Should contain all names
            assert!(data.contains("Alice"));
            assert!(data.contains("Bob"));
            assert!(data.contains("Charlie"));
        }
        other => panic!("Expected Export, got: {}", other.summary()),
    }

    Ok(())
}

#[tokio::test]
async fn test_export_csv_with_filter() -> Result<()> {
    let graph = setup_graph().await?;

    let result = graph.execute_statement(
        r#"find p.name from (p:Person) where p.age > 26 export csv"#
    ).await?;

    match &result {
        NqlResult::Export { rows_exported, data, .. } => {
            assert_eq!(*rows_exported, 2); // Alice(30) and Charlie(35)
            assert!(data.contains("Alice"));
            assert!(data.contains("Charlie"));
            assert!(!data.contains("Bob"));
        }
        other => panic!("Expected Export, got: {}", other.summary()),
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// JSON EXPORT
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_export_json() -> Result<()> {
    let graph = setup_graph().await?;

    let result = graph.execute_statement(
        "find p.name, p.age from (p:Person) export json"
    ).await?;

    match &result {
        NqlResult::Export { format, data, rows_exported } => {
            assert_eq!(format, "JSON");
            assert_eq!(*rows_exported, 3);

            // Should be valid JSON array
            assert!(data.starts_with('['));
            assert!(data.ends_with(']'));
            assert!(data.contains("\"p.name\":\"Alice\""));
            assert!(data.contains("\"p.age\":30"));
        }
        other => panic!("Expected Export, got: {}", other.summary()),
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// ARROW EXPORT
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_export_arrow() -> Result<()> {
    let graph = setup_graph().await?;

    let result = graph.execute_statement(
        "find p.name, p.age from (p:Person) export arrow"
    ).await?;

    match &result {
        NqlResult::Export { format, rows_exported, .. } => {
            assert_eq!(format, "Arrow");
            assert_eq!(*rows_exported, 3);
        }
        other => panic!("Expected Export, got: {}", other.summary()),
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// NO EXPORT — should return Query result as usual
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_no_export_returns_query_result() -> Result<()> {
    let graph = setup_graph().await?;

    // Use execute_nql (not execute_statement) for queries without export
    let result = graph.execute_nql(
        "find p.name from (p:Person)"
    ).await?;

    assert_eq!(result.len(), 3);

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// RUST API — direct export methods on QueryResult
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_api_to_csv() -> Result<()> {
    let graph = setup_graph().await?;

    let result = graph.execute_nql(
        "find p.name, p.age from (p:Person) order by p.age asc"
    ).await?;

    let csv = result.to_csv();
    let lines: Vec<&str> = csv.trim().lines().collect();

    // Header + 3 rows
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0], "p.name,p.age");
    assert!(csv.contains("Alice"));
    assert!(csv.contains("Bob"));
    assert!(csv.contains("Charlie"));

    Ok(())
}

#[tokio::test]
async fn test_api_to_csv_custom_separator() -> Result<()> {
    let graph = setup_graph().await?;

    let result = graph.execute_nql(
        "find p.name, p.age from (p:Person)"
    ).await?;

    let tsv = result.to_csv_custom("\t", true);
    assert!(tsv.starts_with("p.name\tp.age\n"));

    let no_header = result.to_csv_custom(",", false);
    assert!(!no_header.starts_with("p.name"));

    Ok(())
}

#[tokio::test]
async fn test_api_to_json() -> Result<()> {
    let graph = setup_graph().await?;

    let result = graph.execute_nql(
        "find p.name, p.age from (p:Person)"
    ).await?;

    let json = result.to_json();
    assert!(json.starts_with('['));
    assert!(json.ends_with(']'));
    assert!(json.contains("\"p.name\":\"Alice\""));
    assert!(json.contains("\"p.age\":30"));

    Ok(())
}

#[tokio::test]
async fn test_api_to_json_pretty() -> Result<()> {
    let graph = setup_graph().await?;

    let result = graph.execute_nql(
        "find p.name from (p:Person)"
    ).await?;

    let json = result.to_json_pretty();
    assert!(json.contains('\n'));
    assert!(json.contains("  {"));

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// EXPORT with ORDER BY + LIMIT
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_export_csv_with_order_and_limit() -> Result<()> {
    let graph = setup_graph().await?;

    let result = graph.execute_statement(
        "find p.name, p.age from (p:Person) order by p.age desc limit 2 export csv"
    ).await?;

    match &result {
        NqlResult::Export { data, rows_exported, .. } => {
            assert_eq!(*rows_exported, 2);
            let lines: Vec<&str> = data.trim().lines().collect();
            assert_eq!(lines.len(), 3); // header + 2 rows

            // First data line should be Charlie (35), second Alice (30)
            assert!(lines[1].contains("Charlie"));
            assert!(lines[2].contains("Alice"));
        }
        other => panic!("Expected Export, got: {}", other.summary()),
    }

    Ok(())
}