// tests/all_fixes_test.rs
//
// Tests for: I6 (no panics), C3 (WHERE/LIMIT patterns),
// I4 (ADD edges), I1 (ORDER BY), I2 (GROUP BY multi),
// I3 (variable scoping), I5 (DELETE/UPDATE)

use nopaldb::{Edge, Graph, Node, NqlResult, PropertyValue, Result};

// ═══════════════════════════════════════════════════════════
// I6: Row Index returns Null instead of panicking
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_i6_row_index_no_panic() -> Result<()> {
    use nopaldb::query::nql::executor::result::Row;

    let row = Row::new();
    // This used to panic with expect("Key not found in row")
    let val = &row["nonexistent_key"];
    assert_eq!(*val, PropertyValue::Null);
    Ok(())
}

// ═══════════════════════════════════════════════════════════
// I1: ORDER BY
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_i1_order_by_asc() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    for i in &[30, 10, 50, 20, 40] {
        tx.add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String(format!("P{}", i)))
                .with_property("age", PropertyValue::Int(*i)),
        )
        .await?;
    }
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find p.name, p.age
        from (p:Person)
        order by p.age asc
    "#,
        )
        .await?;

    assert_eq!(result.len(), 5);
    // Verify ordering
    let ages: Vec<Option<i64>> = result.rows().iter().map(|r| r.get_int("p.age")).collect();

    for i in 1..ages.len() {
        if let (Some(prev), Some(curr)) = (ages[i - 1], ages[i]) {
            assert!(
                prev <= curr,
                "Expected ascending order: {} <= {}",
                prev,
                curr
            );
        }
    }

    Ok(())
}

#[tokio::test]
async fn test_i1_order_by_desc() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    for i in &[30, 10, 50, 20, 40] {
        tx.add_node(Node::new("Item").with_property("score", PropertyValue::Int(*i)))
            .await?;
    }
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find i.score
        from (i:Item)
        order by i.score desc
    "#,
        )
        .await?;

    let scores: Vec<Option<i64>> = result.rows().iter().map(|r| r.get_int("i.score")).collect();

    for i in 1..scores.len() {
        if let (Some(prev), Some(curr)) = (scores[i - 1], scores[i]) {
            assert!(
                prev >= curr,
                "Expected descending order: {} >= {}",
                prev,
                curr
            );
        }
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// C3: WHERE and LIMIT in pattern queries
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_c3_pattern_with_limit() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Create 5 relationships
    let mut tx = graph.begin_transaction().await?;
    let alice = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Alice".into())))
        .await?;
    tx.commit().await?;

    for i in 0..5 {
        let friend = graph
            .add_node(
                Node::new("Person")
                    .with_property("name", PropertyValue::String(format!("Friend{}", i))),
            )
            .await?;
        graph.add_edge(Edge::new(alice, friend, "KNOWS")).await?;
    }

    let result = graph
        .execute_nql(
            r#"
        find a.name, b.name
        from (a:Person)-[r:KNOWS]->(b:Person)
        limit 3
    "#,
        )
        .await?;

    assert_eq!(
        result.len(),
        3,
        "LIMIT 3 should return exactly 3 rows, got {}",
        result.len()
    );

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// I4: ADD with relationships
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_i4_add_with_relationship() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let result = graph
        .execute_statement(r#"add (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#)
        .await?;

    match result {
        NqlResult::Write(w) => {
            assert_eq!(w.nodes_created, 2, "Should create 2 nodes");
            assert_eq!(w.edges_created, 1, "Should create 1 edge");
        }
        other => panic!("Expected Write result, got: {:?}", other),
    }

    // Verify the relationship exists
    let edges = graph.get_all_edges().await?;
    assert_eq!(edges.len(), 1, "Should have 1 edge");
    assert_eq!(edges[0].edge_type, "KNOWS");

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// I5: DELETE
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_i5_delete_with_where() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Create nodes
    let mut tx = graph.begin_transaction().await?;
    for name in &["Alice", "Bob", "Charlie"] {
        tx.add_node(
            Node::new("Person").with_property("name", PropertyValue::String(name.to_string())),
        )
        .await?;
    }
    tx.commit().await?;

    assert_eq!(graph.get_nodes_by_label("Person").await?.len(), 3);

    // Delete Bob
    let result = graph
        .execute_statement(r#"delete (p:Person) where p.name = "Bob""#)
        .await?;

    match result {
        NqlResult::Write(w) => {
            assert_eq!(w.nodes_deleted, 1, "Should delete 1 node");
        }
        other => panic!("Expected Write result, got: {:?}", other),
    }

    let remaining = graph.get_nodes_by_label("Person").await?;
    assert_eq!(remaining.len(), 2, "Should have 2 remaining nodes");

    // Verify Bob is gone
    let names: Vec<String> = remaining
        .iter()
        .filter_map(|n| n.properties.get("name"))
        .filter_map(|v| {
            if let PropertyValue::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect();
    assert!(!names.contains(&"Bob".to_string()), "Bob should be deleted");

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// I5: UPDATE
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_i5_update_with_where() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Create nodes
    let mut tx = graph.begin_transaction().await?;
    tx.add_node(
        Node::new("Person")
            .with_property("name", PropertyValue::String("Alice".into()))
            .with_property("status", PropertyValue::String("active".into())),
    )
    .await?;
    tx.add_node(
        Node::new("Person")
            .with_property("name", PropertyValue::String("Bob".into()))
            .with_property("status", PropertyValue::String("active".into())),
    )
    .await?;
    tx.commit().await?;

    // Update Alice's status
    let result = graph
        .execute_statement(r#"update (p:Person) set p.status = "inactive" where p.name = "Alice""#)
        .await?;

    match result {
        NqlResult::Write(w) => {
            assert_eq!(w.nodes_updated, 1, "Should update 1 node");
            assert_eq!(w.properties_changed, 1, "Should change 1 property");
        }
        other => panic!("Expected Write result, got: {:?}", other),
    }

    // Verify Alice was updated
    let nodes = graph.get_nodes_by_label("Person").await?;
    for node in &nodes {
        if let Some(PropertyValue::String(name)) = node.properties.get("name") {
            if name == "Alice" {
                let status = node.properties.get("status");
                assert_eq!(
                    status,
                    Some(&PropertyValue::String("inactive".into())),
                    "Alice's status should be 'inactive'"
                );
            }
            if name == "Bob" {
                let status = node.properties.get("status");
                assert_eq!(
                    status,
                    Some(&PropertyValue::String("active".into())),
                    "Bob's status should still be 'active'"
                );
            }
        }
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// I3: AND/OR in WHERE conditions
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_i3_and_or_conditions() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    for (name, age) in &[("Alice", 25), ("Bob", 35), ("Charlie", 45), ("Diana", 30)] {
        tx.add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String(name.to_string()))
                .with_property("age", PropertyValue::Int(*age)),
        )
        .await?;
    }
    tx.commit().await?;

    // Test AND: age > 25 AND age < 40
    let result = graph
        .execute_nql(
            r#"
        find p.name, p.age
        from (p:Person)
        where p.age > 25 and p.age < 40
    "#,
        )
        .await?;

    // Should match Bob (35) and Diana (30)
    assert_eq!(
        result.len(),
        2,
        "AND filter should match 2 people, got {}",
        result.len()
    );

    Ok(())
}
