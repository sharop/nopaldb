// tests/streaming_executor.rs
//
// Integration tests for the streaming executor (Volcano Model Phase 2)

use nopaldb::Graph;
use nopaldb::types::{Edge, Node, PropertyValue};
use std::collections::HashMap;

#[tokio::test]
async fn test_streaming_executor_scan_with_label() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let graph = Graph::open(temp_dir.path().to_str().unwrap()).await?;

    // Add data using NQL `add` statement
    graph
        .execute_nql(r#"add (n:Person {name: "Alice", age: 30})"#)
        .await?;
    graph
        .execute_nql(r#"add (n:Person {name: "Bob", age: 25})"#)
        .await?;
    graph.execute_nql(r#"add (n:Robot {name: "R2D2"})"#).await?;

    // Scan with label filter — exercises ScanNodesStream
    let result = graph
        .execute_nql(
            r#"
        find n.name
        from (n:Person)
    "#,
        )
        .await?;
    assert_eq!(
        result.rows.len(),
        2,
        "Expected 2 Person nodes, got {}",
        result.rows.len()
    );

    let mut names: Vec<String> = result
        .rows
        .iter()
        .filter_map(|r| r.get("n.name"))
        .filter_map(|v| {
            if let PropertyValue::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect();
    names.sort();
    assert_eq!(names, vec!["Alice", "Bob"]);

    Ok(())
}

#[tokio::test]
async fn test_streaming_executor_filter() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let graph = Graph::open(temp_dir.path().to_str().unwrap()).await?;

    graph
        .execute_nql(r#"add (n:Person {name: "Alice", age: 30})"#)
        .await?;
    graph
        .execute_nql(r#"add (n:Person {name: "Bob", age: 25})"#)
        .await?;

    // WHERE filter — exercises the filter collection step (backed by ScanNodesStream)
    let result = graph
        .execute_nql(
            r#"
        find n.name
        from (n:Person)
        where n.age > 28
    "#,
        )
        .await?;
    assert_eq!(
        result.rows.len(),
        1,
        "Expected 1 node where age > 28, got {}",
        result.rows.len()
    );

    assert_eq!(
        result.rows[0].get("n.name"),
        Some(&PropertyValue::String("Alice".to_string()))
    );

    Ok(())
}

#[tokio::test]
async fn test_streaming_executor_scan_all() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let graph = Graph::open(temp_dir.path().to_str().unwrap()).await?;

    graph
        .execute_nql(r#"add (n:Person {name: "Alice"})"#)
        .await?;
    graph.execute_nql(r#"add (n:Robot {name: "R2D2"})"#).await?;

    // Scan without label — returns all nodes
    let result = graph
        .execute_nql(
            r#"
        find n.name
        from (n)
    "#,
        )
        .await?;
    assert_eq!(
        result.rows.len(),
        2,
        "Expected all 2 nodes, got {}",
        result.rows.len()
    );

    Ok(())
}

#[tokio::test]
async fn test_streaming_executor_order_by_hidden_column() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let graph = Graph::open(temp_dir.path().to_str().unwrap()).await?;

    graph
        .execute_nql(r#"add (n:Person {name: "Alice", age: 30})"#)
        .await?;
    graph
        .execute_nql(r#"add (n:Person {name: "Bob", age: 25})"#)
        .await?;
    graph
        .execute_nql(r#"add (n:Person {name: "Charlie", age: 35})"#)
        .await?;

    // ORDER BY age (hidden) DESC
    // Expected order: Charlie (35), Alice (30), Bob (25)
    let result = graph
        .execute_nql(
            r#"
        find n.name
        from (n:Person)
        order by n.age desc
    "#,
        )
        .await?;

    assert_eq!(result.rows.len(), 3);
    assert_eq!(
        result.rows[0].get("n.name"),
        Some(&PropertyValue::String("Charlie".to_string()))
    );
    assert_eq!(
        result.rows[1].get("n.name"),
        Some(&PropertyValue::String("Alice".to_string()))
    );
    assert_eq!(
        result.rows[2].get("n.name"),
        Some(&PropertyValue::String(" Bob".trim().to_string()))
    ); // trimming just in case

    // Verify n.age IS NOT in the final rows (it should have been stripped)
    assert!(
        result.rows[0].get("n.age").is_none(),
        "Extra column n.age should have been stripped"
    );

    Ok(())
}

#[tokio::test]
async fn test_streaming_executor_relationship_match() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let graph = nopaldb::Graph::open(temp_dir.path()).await?;

    let mut alice_props = HashMap::new();
    alice_props.insert(
        "name".to_string(),
        PropertyValue::String("Alice".to_string()),
    );
    alice_props.insert("age".to_string(), PropertyValue::Int(30));
    let alice_id = graph
        .add_node(Node::new("Person").with_properties(alice_props))
        .await?;

    let mut bob_props = HashMap::new();
    bob_props.insert("name".to_string(), PropertyValue::String("Bob".to_string()));
    bob_props.insert("age".to_string(), PropertyValue::Int(25));
    let bob_id = graph
        .add_node(Node::new("Person").with_properties(bob_props))
        .await?;

    let mut edge_props = HashMap::new();
    edge_props.insert("since".to_string(), PropertyValue::Int(2020));
    graph
        .add_edge(Edge::new(alice_id, bob_id, "KNOWS").with_properties(edge_props))
        .await?;

    // Query: (a)-[r]->(b) where a.name = "Alice" find a.name
    let result = graph
        .execute_nql(
            r#"
        find a.name
        from (a:Person)-[r:KNOWS]->(b:Person)
        where a.name = "Alice"
    "#,
        )
        .await?;

    assert_eq!(result.rows.len(), 1);
    assert_eq!(
        result.rows[0].get("a.name").unwrap().as_str().unwrap(),
        "Alice"
    );

    Ok(())
}
