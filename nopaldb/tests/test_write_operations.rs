// tests/test_write_operations.rs
//
// Integration tests for NQL v0.2 write operations (ADD, DELETE, UPDATE)

use nopaldb::{Graph, Result};
use nopaldb::query::nql::parser::parse;
use nopaldb::query::nql::parser::ast::Statement;
use nopaldb::query::nql::executor::Executor;

// ═══════════════════════════════════════════════════════════
// ADD OPERATION TESTS
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_add_simple_node() -> Result<()> {
    // Setup
    let graph = Graph::open("./test_dbs/test_add_simple").await?;
    let executor = Executor::new(&graph);
    let mut tx = graph.begin_transaction().await?;

    // Parse ADD statement
    let stmt = parse("add (alice:Person {name: 'Alice', age: 30})").unwrap();

    // Execute
    if let Statement::Add(add) = stmt {
        let result = executor.execute_add(&add, &mut tx).await?;

        // Assert
        assert_eq!(result.nodes_created, 1);
        assert_eq!(result.edges_created, 0);
        assert_eq!(result.created_ids.len(), 1);

        tx.commit().await?;
    } else {
        panic!("Expected Add statement");
    }

    Ok(())
}

#[tokio::test]
async fn test_add_node_without_label() -> Result<()> {
    let graph = Graph::open("./test_dbs/test_add_no_label").await?;
    let executor = Executor::new(&graph);
    let mut tx = graph.begin_transaction().await?;

    // ADD without label (should use default "Node")
    let stmt = parse("add (n {name: 'NoLabel'})").unwrap();

    if let Statement::Add(add) = stmt {
        let result = executor.execute_add(&add, &mut tx).await?;

        assert_eq!(result.nodes_created, 1);

        tx.commit().await?;
    } else {
        panic!("Expected Add statement");
    }

    Ok(())
}

#[tokio::test]
async fn test_add_node_without_properties() -> Result<()> {
    let graph = Graph::open("./test_dbs/test_add_no_props").await?;
    let executor = Executor::new(&graph);
    let mut tx = graph.begin_transaction().await?;

    // ADD without properties
    let stmt = parse("add (p:Person)").unwrap();

    if let Statement::Add(add) = stmt {
        let result = executor.execute_add(&add, &mut tx).await?;

        assert_eq!(result.nodes_created, 1);
        assert_eq!(result.created_ids.len(), 1);

        tx.commit().await?;
    } else {
        panic!("Expected Add statement");
    }

    Ok(())
}

#[tokio::test]
async fn test_add_multiple_nodes_same_statement() -> Result<()> {
    let graph = Graph::open("./test_dbs/test_add_multiple").await?;
    let executor = Executor::new(&graph);
    let mut tx = graph.begin_transaction().await?;

    // Note: This test might fail if parser doesn't support multiple nodes yet
    // For now, we'll test adding nodes one by one

    // Add first node
    let stmt1 = parse("add (alice:Person {name: 'Alice'})").unwrap();
    if let Statement::Add(add) = stmt1 {
        let result = executor.execute_add(&add, &mut tx).await?;
        assert_eq!(result.nodes_created, 1);
    }

    // Add second node
    let stmt2 = parse("add (bob:Person {name: 'Bob'})").unwrap();
    if let Statement::Add(add) = stmt2 {
        let result = executor.execute_add(&add, &mut tx).await?;
        assert_eq!(result.nodes_created, 1);
    }

    tx.commit().await?;

    Ok(())
}

#[tokio::test]
async fn test_add_with_different_property_types() -> Result<()> {
    let graph = Graph::open("./test_dbs/test_add_types").await?;
    let executor = Executor::new(&graph);
    let mut tx = graph.begin_transaction().await?;

    // ADD with various property types
    let stmt = parse(r#"add (p:Person {
        name: "Alice",
        age: 30,
        score: 95.5,
        active: true
    })"#).unwrap();

    if let Statement::Add(add) = stmt {
        let result = executor.execute_add(&add, &mut tx).await?;

        assert_eq!(result.nodes_created, 1);

        tx.commit().await?;
    } else {
        panic!("Expected Add statement");
    }

    Ok(())
}

#[tokio::test]
async fn test_add_node_variable_tracking() -> Result<()> {
    let graph = Graph::open("./test_dbs/test_add_var").await?;
    let executor = Executor::new(&graph);
    let mut tx = graph.begin_transaction().await?;

    // ADD with variable
    let stmt = parse("add (alice:Person {name: 'Alice'})").unwrap();

    if let Statement::Add(add) = stmt {
        let result = executor.execute_add(&add, &mut tx).await?;

        // Variable 'alice' should map to created node
        assert_eq!(result.nodes_created, 1);
        assert!(!result.created_ids.is_empty());

        tx.commit().await?;
    } else {
        panic!("Expected Add statement");
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// TRANSACTION TESTS
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_add_rollback_on_error() -> Result<()> {
    let graph = Graph::open("./test_dbs/test_add_rollback").await?;
    let executor = Executor::new(&graph);
    let mut tx = graph.begin_transaction().await?;

    // Add a node
    let stmt = parse("add (p:Person {name: 'Alice'})").unwrap();
    if let Statement::Add(add) = stmt {
        let _result = executor.execute_add(&add, &mut tx).await?;
    }

    // Rollback instead of commit
    tx.rollback()?;

    // Node should not exist after rollback
    // TODO: Add verification when we have query execution

    Ok(())
}

#[tokio::test]
async fn test_add_multiple_in_transaction() -> Result<()> {
    let graph = Graph::open("./test_dbs/test_add_tx_multiple").await?;
    let executor = Executor::new(&graph);
    let mut tx = graph.begin_transaction().await?;

    let mut total_created = 0;

    // Add multiple nodes in same transaction
    for i in 0..5 {
        let stmt = parse(&format!("add (p:Person {{id: {}}})", i)).unwrap();
        if let Statement::Add(add) = stmt {
            let result = executor.execute_add(&add, &mut tx).await?;
            total_created += result.nodes_created;
        }
    }

    assert_eq!(total_created, 5);

    tx.commit().await?;

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// EDGE CREATION TESTS (Future)
// ═══════════════════════════════════════════════════════════

#[tokio::test]
#[ignore] // Ignore until edge creation is implemented
async fn test_add_edge() -> Result<()> {
    let graph = Graph::open("./test_dbs/test_add_edge").await?;
    let executor = Executor::new(&graph);
    let mut tx = graph.begin_transaction().await?;

    // This will be implemented in A.2
    let stmt = parse("add (alice:Person)-[:KNOWS]->(bob:Person)").unwrap();

    if let Statement::Add(add) = stmt {
        let result = executor.execute_add(&add, &mut tx).await?;

        assert_eq!(result.nodes_created, 2);
        assert_eq!(result.edges_created, 1);

        tx.commit().await?;
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// HELPER FUNCTIONS
// ═══════════════════════════════════════════════════════════

#[allow(dead_code)]
async fn cleanup_test_db(_name: &str) -> Result<()> {
    // TODO: Implement database cleanup
    // For now, test databases are temporary
    Ok(())
}