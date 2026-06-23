// tests/nql_statement_test.rs
//
// Test C1: Verify that execute_statement handles all NQL statement types
// Bug: Graph.execute_nql only accepted FIND queries, rejecting
// ADD/DELETE/UPDATE/CREATE INDEX/DROP INDEX/EXPLAIN.

use nopaldb::{Graph, Node, NqlResult, PropertyValue, Result};

// ═══════════════════════════════════════════════════════════
// execute_statement: FIND queries
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_execute_statement_find() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Setup data
    let mut tx = graph.begin_transaction().await?;
    let node = Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()))
        .with_property("age", PropertyValue::Int(30));
    tx.add_node(node).await?;
    tx.commit().await?;

    // Execute via execute_statement
    let result = graph
        .execute_statement("find p.name from (p:Person)")
        .await?;

    match result {
        NqlResult::Query(qr) => {
            assert!(!qr.is_empty(), "Should return at least 1 row");
        }
        other => panic!("Expected Query result, got: {:?}", other),
    }

    Ok(())
}

#[tokio::test]
async fn test_execute_statement_find_bare_variable_header_is_clean() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    tx.add_node(Node::new("Person").with_property("name", PropertyValue::String("Alice".into())))
        .await?;
    tx.commit().await?;

    let result = graph
        .execute_statement("find p from (p:Person) limit 1")
        .await?;

    match result {
        NqlResult::Query(qr) => {
            assert_eq!(qr.columns, vec!["p".to_string()]);
            assert_eq!(qr.rows.len(), 1);
            assert!(
                qr.rows[0].get("p").is_some(),
                "bare variable header should be 'p'"
            );
        }
        other => panic!("Expected Query result, got: {:?}", other),
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// execute_statement: ADD
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_execute_statement_add() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let result = graph
        .execute_statement(r#"add (alice:Person {name: "Alice", age: 30})"#)
        .await?;

    match result {
        NqlResult::Write(w) => {
            assert_eq!(w.nodes_created, 1, "Should create 1 node");
        }
        other => panic!("Expected Write result, got: {:?}", other),
    }

    // Verify node was actually created
    let nodes = graph.get_nodes_by_label("Person").await?;
    assert_eq!(nodes.len(), 1, "Should have 1 Person node");

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// execute_statement: CREATE INDEX / DROP INDEX
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_execute_statement_create_index() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Add a node first so index has something to work with
    let mut tx = graph.begin_transaction().await?;
    let node = Node::new("Person").with_property("name", PropertyValue::String("Alice".into()));
    tx.add_node(node).await?;
    tx.commit().await?;

    let result = graph
        .execute_statement("create index on Person(name) type hash")
        .await?;

    match result {
        NqlResult::Index(msg) => {
            assert!(
                msg.contains("Index created"),
                "Should confirm index creation: {}",
                msg
            );
        }
        other => panic!("Expected Index result, got: {:?}", other),
    }

    Ok(())
}

#[tokio::test]
async fn test_execute_statement_drop_index() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Create then drop
    let _ = graph
        .execute_statement("create index on Person(name) type hash")
        .await;
    let result = graph.execute_statement("drop index Person_name").await?;

    match result {
        NqlResult::Index(msg) => {
            assert!(
                msg.contains("Index dropped"),
                "Should confirm index drop: {}",
                msg
            );
        }
        other => panic!("Expected Index result, got: {:?}", other),
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// execute_statement: EXPLAIN
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_execute_statement_explain() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let result = graph
        .execute_statement("explain find p.name from (p:Person)")
        .await?;

    match result {
        NqlResult::Explain(plan) => {
            assert!(!plan.is_empty(), "Explain plan should not be empty");
        }
        other => panic!("Expected Explain result, got: {:?}", other),
    }

    Ok(())
}

#[tokio::test]
async fn test_execute_statement_explain_reports_community_cost_notes() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let result = graph
        .execute_statement("explain find community(p) as c from (p:Person) limit 1")
        .await?;

    match result {
        NqlResult::Explain(plan) => {
            assert!(
                plan.contains("community() requires global community partition computation"),
                "EXPLAIN should include global cost note for community(), got: {}",
                plan
            );
            assert!(
                plan.contains("LIMIT applies after aggregation"),
                "EXPLAIN should state LIMIT behavior for community(), got: {}",
                plan
            );
        }
        other => panic!("Expected Explain result, got: {:?}", other),
    }

    Ok(())
}

#[cfg(not(feature = "algorithms"))]
#[tokio::test]
async fn test_algorithm_query_without_feature_returns_clear_error() {
    let graph = Graph::in_memory().await.expect("graph should open");

    let err = graph
        .execute_nql("find community(p) as c from (p:Person)")
        .await
        .expect_err("algorithm query should fail without feature");

    assert!(
        err.to_string().contains("requires feature `algorithms`"),
        "unexpected error: {err}"
    );
}

// ═══════════════════════════════════════════════════════════
// execute_nql backward compatibility: non-query statements
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_execute_nql_backward_compat_with_add() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Before C1 fix, this would fail with "Expected Query statement"
    let result = graph
        .execute_nql(r#"add (bob:Person {name: "Bob"})"#)
        .await?;

    // Should return a QueryResult with summary
    assert_eq!(result.len(), 1, "Should return 1 row with summary");
    let summary = result.rows()[0].get("result");
    assert!(summary.is_some(), "Should have a 'result' column");

    Ok(())
}

#[tokio::test]
async fn test_execute_nql_backward_compat_with_create_index() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Before C1 fix, this would fail with "Expected Query statement"
    let result = graph
        .execute_nql("create index on Person(name) type hash")
        .await?;

    assert_eq!(result.len(), 1);

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// execute_statement: NqlResult.into_query()
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_nql_result_into_query() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Setup
    let mut tx = graph.begin_transaction().await?;
    tx.add_node(Node::new("Fruit").with_property("name", PropertyValue::String("Nopal".into())))
        .await?;
    tx.commit().await?;

    // into_query on a Query result should succeed
    let result = graph
        .execute_statement("find f.name from (f:Fruit)")
        .await?;
    let qr = result.into_query()?;
    assert!(!qr.is_empty());

    // into_query on a non-Query result should fail
    let result = graph.execute_statement("add (x:Test {v: 1})").await?;
    assert!(result.into_query().is_err());

    Ok(())
}
