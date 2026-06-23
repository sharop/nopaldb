// tests/p2_order_by_non_projected_test.rs
//
// P2: ORDER BY should work on columns not in FIND projection

use nopaldb::{Graph, Node, PropertyValue, Result};

#[tokio::test]
async fn test_order_by_non_projected_column() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    for (name, age) in &[("Charlie", 45), ("Alice", 25), ("Bob", 35), ("Diana", 30)] {
        tx.add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String(name.to_string()))
                .with_property("age", PropertyValue::Int(*age)),
        )
        .await?;
    }
    tx.commit().await?;

    // Project only name, but ORDER BY age (not projected)
    let result = graph
        .execute_nql(
            r#"
        find p.name
        from (p:Person)
        order by p.age asc
    "#,
        )
        .await?;

    assert_eq!(result.len(), 4);

    // Verify names are ordered by age ascending: Alice(25), Diana(30), Bob(35), Charlie(45)
    let names: Vec<String> = result
        .rows()
        .iter()
        .filter_map(|r| r.get_string("p.name"))
        .collect();

    assert_eq!(
        names,
        vec!["Alice", "Diana", "Bob", "Charlie"],
        "Should be ordered by age ascending, got {:?}",
        names
    );

    // Verify age column is NOT in the result (was stripped)
    assert!(
        !result.columns.contains(&"p.age".to_string()),
        "p.age should not be in result columns: {:?}",
        result.columns
    );

    Ok(())
}

#[tokio::test]
async fn test_order_by_non_projected_desc() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    for (name, score) in &[("A", 10), ("B", 50), ("C", 30), ("D", 20)] {
        tx.add_node(
            Node::new("Item")
                .with_property("name", PropertyValue::String(name.to_string()))
                .with_property("score", PropertyValue::Int(*score)),
        )
        .await?;
    }
    tx.commit().await?;

    // Only project name, order by score desc
    let result = graph
        .execute_nql(
            r#"
        find i.name
        from (i:Item)
        order by i.score desc
    "#,
        )
        .await?;

    let names: Vec<String> = result
        .rows()
        .iter()
        .filter_map(|r| r.get_string("i.name"))
        .collect();

    assert_eq!(
        names,
        vec!["B", "C", "D", "A"],
        "Should be ordered by score descending, got {:?}",
        names
    );

    // score should not leak into result
    assert!(!result.columns.contains(&"i.score".to_string()));

    Ok(())
}

#[tokio::test]
async fn test_order_by_projected_column_still_works() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    for (name, age) in &[("Charlie", 45), ("Alice", 25), ("Bob", 35)] {
        tx.add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String(name.to_string()))
                .with_property("age", PropertyValue::Int(*age)),
        )
        .await?;
    }
    tx.commit().await?;

    // ORDER BY a column that IS projected — should still work
    let result = graph
        .execute_nql(
            r#"
        find p.name, p.age
        from (p:Person)
        order by p.age asc
    "#,
        )
        .await?;

    let names: Vec<String> = result
        .rows()
        .iter()
        .filter_map(|r| r.get_string("p.name"))
        .collect();

    assert_eq!(names, vec!["Alice", "Bob", "Charlie"]);

    // age should remain in result since it was projected
    assert!(result.columns.contains(&"p.age".to_string()));

    Ok(())
}

#[tokio::test]
async fn test_order_by_non_projected_with_limit() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    for i in 0..10 {
        tx.add_node(
            Node::new("Item")
                .with_property("name", PropertyValue::String(format!("item_{}", i)))
                .with_property("rank", PropertyValue::Int(i)),
        )
        .await?;
    }
    tx.commit().await?;

    // Top 3 by rank desc, but only show name
    let result = graph
        .execute_nql(
            r#"
        find i.name
        from (i:Item)
        order by i.rank desc
        limit 3
    "#,
        )
        .await?;

    assert_eq!(result.len(), 3);

    let names: Vec<String> = result
        .rows()
        .iter()
        .filter_map(|r| r.get_string("i.name"))
        .collect();

    assert_eq!(
        names,
        vec!["item_9", "item_8", "item_7"],
        "Should get top 3 items by rank desc, got {:?}",
        names
    );

    // rank should not be in result
    assert!(!result.columns.contains(&"i.rank".to_string()));

    Ok(())
}
