// tests/p0_transactional_test.rs
//
// P0-A: WriteExecutor uses Transaction for atomicity
// P0-B: DELETE/UPDATE support relationship patterns

use nopaldb::{Graph, Node, Edge, PropertyValue, NqlResult, Result};

// ═══════════════════════════════════════════════════════════
// P0-A: ADD is transactional — nodes buffered until commit
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_p0a_add_is_transactional() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // ADD via execute_statement (which opens tx, executes, commits)
    let result = graph.execute_statement(
        r#"add (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#
    ).await?;

    match &result {
        NqlResult::Write(w) => {
            assert_eq!(w.nodes_created, 2);
            assert_eq!(w.edges_created, 1);
        }
        other => panic!("Expected Write, got {:?}", other),
    }

    // Verify data persisted after commit
    let persons = graph.get_nodes_by_label("Person").await?;
    assert_eq!(persons.len(), 2, "2 Person nodes after commit");

    let edges = graph.get_all_edges().await?;
    assert_eq!(edges.len(), 1, "1 edge after commit");
    assert_eq!(edges[0].edge_type, "KNOWS");

    Ok(())
}

#[tokio::test]
async fn test_p0a_add_simple_node_transactional() -> Result<()> {
    let graph = Graph::in_memory().await?;

    graph.execute_statement(
        r#"add (x:Test {val: 42})"#
    ).await?;

    let nodes = graph.get_nodes_by_label("Test").await?;
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].properties.get("val"), Some(&PropertyValue::Int(42)));

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// P0-A: DELETE is transactional
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_p0a_delete_is_transactional() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Setup
    let mut tx = graph.begin_transaction().await?;
    for name in &["Alice", "Bob", "Charlie"] {
        tx.add_node(
            Node::new("Person").with_property("name", PropertyValue::String(name.to_string()))
        ).await?;
    }
    tx.commit().await?;
    assert_eq!(graph.get_nodes_by_label("Person").await?.len(), 3);

    // Delete via NQL
    graph.execute_statement(
        r#"delete (p:Person) where p.name = "Bob""#
    ).await?;

    let remaining = graph.get_nodes_by_label("Person").await?;
    assert_eq!(remaining.len(), 2, "Should have 2 after deleting Bob");

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// P0-B: DELETE with relationship pattern
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_p0b_delete_with_relationship_pattern() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Create: Alice -KNOWS-> Bob, Alice -KNOWS-> Charlie, Dave (unconnected)
    let mut tx = graph.begin_transaction().await?;
    let alice = tx.add_node(Node::new("Person").with_property("name", PropertyValue::String("Alice".into()))).await?;
    let bob = tx.add_node(Node::new("Person").with_property("name", PropertyValue::String("Bob".into()))).await?;
    let charlie = tx.add_node(Node::new("Person").with_property("name", PropertyValue::String("Charlie".into()))).await?;
    let _dave = tx.add_node(Node::new("Person").with_property("name", PropertyValue::String("Dave".into()))).await?;
    tx.commit().await?;

    graph.add_edge(Edge::new(alice, bob, "KNOWS")).await?;
    graph.add_edge(Edge::new(alice, charlie, "KNOWS")).await?;

    assert_eq!(graph.get_nodes_by_label("Person").await?.len(), 4);
    assert_eq!(graph.get_all_edges().await?.len(), 2);

    // Delete only the relationship where target is Bob
    let result = graph.execute_statement(
        r#"delete (a:Person)-[:KNOWS]->(b:Person) where b.name = "Bob""#
    ).await?;

    match &result {
        NqlResult::Write(w) => {
            assert!(w.edges_deleted >= 1, "Should delete at least 1 edge");
            assert_eq!(w.nodes_deleted, 0, "Relationship delete should preserve matched nodes");
        }
        other => panic!("Expected Write, got {:?}", other),
    }

    // Bob should still exist
    let remaining = graph.get_nodes_by_label("Person").await?;
    let names: Vec<String> = remaining.iter()
        .filter_map(|n| n.properties.get("name"))
        .filter_map(|v| if let PropertyValue::String(s) = v { Some(s.clone()) } else { None })
        .collect();
    assert!(names.contains(&"Bob".to_string()), "Bob should still exist");

    // Dave should still exist (not part of any KNOWS relationship)
    assert!(names.contains(&"Dave".to_string()), "Dave should still exist");

    let edges = graph.get_all_edges().await?;
    assert_eq!(edges.len(), 1, "Only the matching relationship should be deleted");

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// P0-B: UPDATE with relationship pattern
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_p0b_update_with_relationship_pattern() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Create: Alice -MANAGES-> Bob, Alice -MANAGES-> Charlie
    let mut tx = graph.begin_transaction().await?;
    let alice = tx.add_node(
        Node::new("Person")
            .with_property("name", PropertyValue::String("Alice".into()))
            .with_property("role", PropertyValue::String("manager".into()))
    ).await?;
    let bob = tx.add_node(
        Node::new("Person")
            .with_property("name", PropertyValue::String("Bob".into()))
            .with_property("role", PropertyValue::String("engineer".into()))
    ).await?;
    let charlie = tx.add_node(
        Node::new("Person")
            .with_property("name", PropertyValue::String("Charlie".into()))
            .with_property("role", PropertyValue::String("engineer".into()))
    ).await?;
    tx.commit().await?;

    graph.add_edge(Edge::new(alice, bob, "MANAGES")).await?;
    graph.add_edge(Edge::new(alice, charlie, "MANAGES")).await?;

    // Update the target nodes (managed employees) to senior
    let result = graph.execute_statement(
        r#"update (a:Person)-[:MANAGES]->(b:Person) set b.role = "senior""#
    ).await?;

    match &result {
        NqlResult::Write(w) => {
            assert_eq!(w.nodes_updated, 2, "Should update Bob and Charlie");
        }
        other => panic!("Expected Write, got {:?}", other),
    }

    // Verify Bob and Charlie are now senior
    let nodes = graph.get_nodes_by_label("Person").await?;
    for node in &nodes {
        if let Some(PropertyValue::String(name)) = node.properties.get("name") {
            let role = node.properties.get("role");
            if name == "Alice" {
                assert_eq!(role, Some(&PropertyValue::String("manager".into())),
                           "Alice should still be manager");
            } else {
                assert_eq!(role, Some(&PropertyValue::String("senior".into())),
                           "{} should be senior now", name);
            }
        }
    }

    Ok(())
}
