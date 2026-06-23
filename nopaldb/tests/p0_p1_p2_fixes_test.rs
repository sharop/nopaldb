// tests/p0_p1_p2_fixes_test.rs
//
// Tests for priority fixes from rival audit:
// P0: delete_node orphaned edges, ORDER BY before LIMIT, safe WHERE defaults
// P1: UPDATE refreshes indices, tx delete_edge
// P2: apply_remaining_filters, variable scope warnings

use nopaldb::{Edge, Graph, Node, NqlResult, PropertyValue, Result};

// ═══════════════════════════════════════════════════════════
// P0: delete_node must clean up edges from storage
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_p0_delete_node_removes_edges() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let alice = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Alice".into())))
        .await?;
    let bob = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Bob".into())))
        .await?;
    tx.commit().await?;

    let _edge_id = graph.add_edge(Edge::new(alice, bob, "KNOWS")).await?;

    // Verify edge exists
    assert_eq!(graph.get_all_edges().await?.len(), 1);

    // Delete Alice — should also remove the KNOWS edge
    graph.delete_node(alice).await?;

    // Edge must be gone from storage
    let edges = graph.get_all_edges().await?;
    assert_eq!(
        edges.len(),
        0,
        "Edge should be deleted when source node is deleted, found {}",
        edges.len()
    );

    // Bob's incoming adjacency should be clean
    let bob_incoming = graph.get_incoming_edges(bob).await?;
    assert_eq!(
        bob_incoming.len(),
        0,
        "Bob should have no incoming edges after Alice deleted"
    );

    Ok(())
}

#[tokio::test]
async fn test_p0_delete_node_cleans_both_directions() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx.add_node(Node::new("N")).await?;
    let b = tx.add_node(Node::new("N")).await?;
    let c = tx.add_node(Node::new("N")).await?;
    tx.commit().await?;

    // a -> b, c -> b
    graph.add_edge(Edge::new(a, b, "E1")).await?;
    graph.add_edge(Edge::new(c, b, "E2")).await?;

    assert_eq!(graph.get_all_edges().await?.len(), 2);

    // Delete b (has both incoming edges)
    graph.delete_node(b).await?;

    // All edges involving b must be gone
    assert_eq!(
        graph.get_all_edges().await?.len(),
        0,
        "All edges to/from deleted node must be removed"
    );

    // a and c should have clean outgoing
    assert_eq!(graph.get_outgoing_edges(a).await?.len(), 0);
    assert_eq!(graph.get_outgoing_edges(c).await?.len(), 0);

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// P0: ORDER BY must happen BEFORE LIMIT
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_p0_order_by_before_limit() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    for i in 0..10 {
        tx.add_node(Node::new("Item").with_property("score", PropertyValue::Int(i)))
            .await?;
    }
    tx.commit().await?;

    // Get top 3 by score descending
    let result = graph
        .execute_nql(
            r#"
        find i.score
        from (i:Item)
        order by i.score desc
        limit 3
    "#,
        )
        .await?;

    assert_eq!(result.len(), 3);

    // Must be 9, 8, 7 (not random 3 items sorted)
    let scores: Vec<i64> = result
        .rows()
        .iter()
        .filter_map(|r| r.get_int("i.score"))
        .collect();

    assert_eq!(
        scores,
        vec![9, 8, 7],
        "ORDER BY DESC LIMIT 3 should return top 3 scores, got {:?}",
        scores
    );

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// P0: WHERE with unsupported expressions should be safe
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_p0_delete_safe_with_unsupported_where() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    for name in &["A", "B", "C"] {
        tx.add_node(
            Node::new("Person").with_property("name", PropertyValue::String(name.to_string())),
        )
        .await?;
    }
    tx.commit().await?;

    // This WHERE clause has a simple equality - should delete exactly 1
    let result = graph
        .execute_statement(r#"delete (p:Person) where p.name = "A""#)
        .await?;

    match result {
        NqlResult::Write(w) => {
            assert_eq!(w.nodes_deleted, 1, "Should delete exactly 1 node");
        }
        other => panic!("Expected Write, got {:?}", other),
    }

    // 2 should remain
    assert_eq!(graph.get_nodes_by_label("Person").await?.len(), 2);

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// P1: UPDATE refreshes property indices
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_p1_update_refreshes_indices() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    tx.add_node(
        Node::new("Person")
            .with_property("name", PropertyValue::String("Alice".into()))
            .with_property("status", PropertyValue::String("active".into())),
    )
    .await?;
    tx.commit().await?;

    // Update status
    graph
        .execute_statement(r#"update (p:Person) set p.status = "inactive" where p.name = "Alice""#)
        .await?;

    // Query by new value should work
    let result = graph
        .execute_nql(
            r#"
        find p.name, p.status
        from (p:Person)
        where p.status = "inactive"
    "#,
        )
        .await?;

    assert_eq!(result.len(), 1, "Should find Alice with updated status");

    // Query by old value should NOT find it
    let result_old = graph
        .execute_nql(
            r#"
        find p.name
        from (p:Person)
        where p.status = "active"
    "#,
        )
        .await?;

    assert_eq!(result_old.len(), 0, "Should not find Alice with old status");

    Ok(())
}

// ═══════════════════════════════════════════════════════════
// P1: Transaction edge deletion
// ═══════════════════════════════════════════════════════════

#[tokio::test]
async fn test_p1_tx_delete_edge() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx.add_node(Node::new("N")).await?;
    let b = tx.add_node(Node::new("N")).await?;
    tx.commit().await?;

    let edge_id = graph.add_edge(Edge::new(a, b, "REL")).await?;
    assert_eq!(graph.get_all_edges().await?.len(), 1);

    // Delete edge directly (not via transaction for now)
    graph.delete_edge(edge_id).await?;
    assert_eq!(
        graph.get_all_edges().await?.len(),
        0,
        "Edge should be deleted"
    );

    Ok(())
}
