// tests/nql_executor_test.rs

use nopaldb::{Edge, Graph, Node, PropertyValue, Result};
use std::fs;

#[tokio::test]
async fn test_simple_nql_query() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Insert test data
    let mut tx = graph.begin_transaction().await?;

    for i in 0..10 {
        let node = Node::new("Person")
            .with_property("name", PropertyValue::String(format!("Person{}", i)))
            .with_property("age", PropertyValue::Int(20 + i));

        tx.add_node(node).await?;
    }

    tx.commit().await?;

    println!("✅ Inserted 10 test nodes\n");

    // Execute NQL query
    let result = graph.execute_nql(r#"
        find p.name, p.age
        from (p:Person)
        where p.age > 25
        limit 3
    "#).await?;

    println!("📊 Query result:");
    println!("   Rows: {}", result.len());
    println!("   Columns: {:?}\n", result.columns);

    for (i, row) in result.rows().iter().enumerate() {
        println!("   Row {}: name={:?}, age={:?}",
                 i,
                 row.get("p.name"),
                 row.get("p.age"));
    }

    assert!(result.len() <= 3); // LIMIT 3
    assert!(!result.is_empty()); // Should have results

    // Verify ages are > 25
    for row in result.rows() {
        if let Some(PropertyValue::Int(age)) = row.get("p.age") {
            assert!(*age > 25, "Age should be > 25, got {}", age);
        }
    }

    println!("\n✅ NQL query executed successfully!");

    Ok(())
}

#[tokio::test]
async fn test_wildcard_query() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Insert test data
    let mut tx = graph.begin_transaction().await?;

    let node = Node::new("Test")
        .with_property("foo", PropertyValue::String("bar".into()))
        .with_property("count", PropertyValue::Int(42));

    tx.add_node(node).await?;
    tx.commit().await?;

    println!("✅ Inserted test node\n");

    // Execute wildcard query
    let result = graph.execute_nql(r#"
        find *
        from (n:Test)
    "#).await?;

    println!("📊 Wildcard query result:");
    println!("   Rows: {}", result.len());
    println!("   Columns: {:?}\n", result.columns);

    for row in result.rows() {
        println!("   Row data:");
        if let Some(val) = row.get("n.foo") {
            println!("      n.foo = {:?}", val);
        }
        if let Some(val) = row.get("n.count") {
            println!("      n.count = {:?}", val);
        }
    }

    assert_eq!(result.len(), 1);

    println!("\n✅ Wildcard query executed successfully!");

    Ok(())
}

#[tokio::test]
async fn test_no_filter_query() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Insert test data
    let mut tx = graph.begin_transaction().await?;

    for i in 0..5 {
        let node = Node::new("Item")
            .with_property("id", PropertyValue::Int(i));

        tx.add_node(node).await?;
    }

    tx.commit().await?;

    println!("✅ Inserted 5 test nodes\n");

    // Query without WHERE
    let result = graph.execute_nql(r#"
        find i.id
        from (i:Item)
    "#).await?;

    println!("📊 Query result: {} rows", result.len());

    assert_eq!(result.len(), 5);

    println!("\n✅ No-filter query executed successfully!");

    Ok(())
}

#[tokio::test]
async fn test_limit_offset() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Insert test data
    let mut tx = graph.begin_transaction().await?;

    for i in 0..20 {
        let node = Node::new("Number")
            .with_property("value", PropertyValue::Int(i));

        tx.add_node(node).await?;
    }

    tx.commit().await?;

    println!("✅ Inserted 20 test nodes\n");

    // Query with LIMIT and OFFSET
    let result = graph.execute_nql(r#"
        find n.value
        from (n:Number)
        limit 5 offset 10
    "#).await?;

    println!("📊 Query result: {} rows", result.len());
    println!("   Expected: 5 rows (LIMIT 5)");
    println!("   Starting from offset 10");

    assert_eq!(result.len(), 5, "Should return exactly 5 rows");

    println!("\n✅ LIMIT/OFFSET query executed successfully!");

    Ok(())
}

#[tokio::test]
async fn test_multiple_conditions() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Insert test data
    let mut tx = graph.begin_transaction().await?;

    for i in 0..20 {
        let node = Node::new("Product")
            .with_property("price", PropertyValue::Int(i * 10))
            .with_property("stock", PropertyValue::Int(100 - i * 5));

        tx.add_node(node).await?;
    }

    tx.commit().await?;

    println!("✅ Inserted 20 test nodes\n");

    // Query with comparison
    let result = graph.execute_nql(r#"
        find p.price, p.stock
        from (p:Product)
        where p.price >= 50
    "#).await?;

    println!("📊 Query result: {} rows", result.len());

    // Verify all prices >= 50
    for row in result.rows() {
        if let Some(PropertyValue::Int(price)) = row.get("p.price") {
            assert!(*price >= 50, "Price should be >= 50, got {}", price);
            println!("   ✓ Price: {}", price);
        }
    }

    println!("\n✅ Multiple conditions query executed successfully!");

    Ok(())
}

#[tokio::test]
async fn test_different_labels() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Insert test data with different labels
    let mut tx = graph.begin_transaction().await?;

    for i in 0..5 {
        let person = Node::new("Person")
            .with_property("name", PropertyValue::String(format!("Person{}", i)));
        tx.add_node(person).await?;
    }

    for i in 0..3 {
        let company = Node::new("Company")
            .with_property("name", PropertyValue::String(format!("Company{}", i)));
        tx.add_node(company).await?;
    }

    tx.commit().await?;

    println!("✅ Inserted 5 Person + 3 Company nodes\n");

    // Query only Persons
    let result = graph.execute_nql(r#"
        find p.name
        from (p:Person)
    "#).await?;

    println!("📊 Query result (Person): {} rows", result.len());
    assert_eq!(result.len(), 5, "Should return only Person nodes");

    // Query only Companies
    let result2 = graph.execute_nql(r#"
        find c.name
        from (c:Company)
    "#).await?;

    println!("📊 Query result (Company): {} rows", result2.len());
    assert_eq!(result2.len(), 3, "Should return only Company nodes");

    println!("\n✅ Label filtering executed successfully!");

    Ok(())
}

#[tokio::test]
async fn test_empty_result() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Insert test data
    let mut tx = graph.begin_transaction().await?;

    let node = Node::new("Test")
        .with_property("value", PropertyValue::Int(10));

    tx.add_node(node).await?;
    tx.commit().await?;

    println!("✅ Inserted test node\n");

    // Query that returns no results
    let result = graph.execute_nql(r#"
        find t.value
        from (t:Test)
        where t.value > 100
    "#).await?;

    println!("📊 Query result: {} rows", result.len());
    assert_eq!(result.len(), 0, "Should return empty result");

    println!("\n✅ Empty result query executed successfully!");

    Ok(())
}

#[tokio::test]
async fn test_string_property() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Insert test data
    let mut tx = graph.begin_transaction().await?;

    let node1 = Node::new("User")
        .with_property("username", PropertyValue::String("alice".into()))
        .with_property("active", PropertyValue::Bool(true));

    let node2 = Node::new("User")
        .with_property("username", PropertyValue::String("bob".into()))
        .with_property("active", PropertyValue::Bool(false));

    tx.add_node(node1).await?;
    tx.add_node(node2).await?;
    tx.commit().await?;

    println!("✅ Inserted 2 user nodes\n");

    // Query users
    let result = graph.execute_nql(r#"
        find u.username
        from (u:User)
    "#).await?;

    println!("📊 Query result: {} rows", result.len());

    for row in result.rows() {
        if let Some(PropertyValue::String(username)) = row.get("u.username") {
            println!("   User: {}", username);
        }
    }

    assert_eq!(result.len(), 2);

    println!("\n✅ String property query executed successfully!");

    Ok(())
}

#[tokio::test]
async fn test_nonexistent_label() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Insert test data
    let mut tx = graph.begin_transaction().await?;

    let node = Node::new("Real")
        .with_property("value", PropertyValue::Int(42));

    tx.add_node(node).await?;
    tx.commit().await?;

    println!("✅ Inserted node with label 'Real'\n");

    // Query non-existent label
    let result = graph.execute_nql(r#"
        find f.value
        from (f:Fake)
    "#).await?;

    println!("📊 Query result: {} rows", result.len());
    assert_eq!(result.len(), 0, "Should return empty for non-existent label");

    println!("\n✅ Non-existent label query executed successfully!");

    Ok(())
}

#[tokio::test]
async fn test_order_by_non_projected_column() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;

    let alice = Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()))
        .with_property("age", PropertyValue::Int(30));
    let bob = Node::new("Person")
        .with_property("name", PropertyValue::String("Bob".into()))
        .with_property("age", PropertyValue::Int(20));
    let carol = Node::new("Person")
        .with_property("name", PropertyValue::String("Carol".into()))
        .with_property("age", PropertyValue::Int(25));

    tx.add_node(alice).await?;
    tx.add_node(bob).await?;
    tx.add_node(carol).await?;
    tx.commit().await?;

    let result = graph.execute_nql(r#"
        find p.name
        from (p:Person)
        order by p.age
    "#).await?;

    let names: Vec<String> = result.rows().iter()
        .filter_map(|row| {
            if let Some(PropertyValue::String(name)) = row.get("p.name") {
                Some(name.clone())
            } else {
                None
            }
        })
        .collect();

    assert_eq!(names, vec!["Bob", "Carol", "Alice"]);

    Ok(())
}

#[tokio::test]
async fn test_export_csv_to_path() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let node = Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()))
        .with_property("age", PropertyValue::Int(30));
    tx.add_node(node).await?;
    tx.commit().await?;

    let path = std::env::temp_dir()
        .join(format!("nql_export_test_{}.csv", uuid::Uuid::new_v4()));
    let path_str = path.to_string_lossy();

    let query = format!(r#"
        find p.name, p.age
        from (p:Person)
        export csv with path="{}", header=true
    "#, path_str);

    let result = graph.execute_nql(&query).await?;

    // Expect summary result
    assert_eq!(result.columns, vec!["format", "exported_to", "rows"]);
    assert_eq!(result.len(), 1);

    let contents = fs::read_to_string(&path).expect("CSV file should exist");
    let lines: Vec<&str> = contents.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "p.name,p.age");

    Ok(())
}

#[tokio::test]
async fn test_export_jsonl_inline() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let node = Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()))
        .with_property("age", PropertyValue::Int(30));
    tx.add_node(node).await?;
    tx.commit().await?;

    let result = graph.execute_nql(r#"
        find p.name, p.age
        from (p:Person)
        export json with jsonl=true
    "#).await?;

    assert_eq!(result.columns, vec!["format", "data"]);
    assert_eq!(result.len(), 1);

    let data = match result.rows()[0].get("data") {
        Some(PropertyValue::String(s)) => s.clone(),
        _ => panic!("expected data string"),
    };

    let lines: Vec<&str> = data.lines().collect();
    assert_eq!(lines.len(), 1);

    let v: serde_json::Value = serde_json::from_str(lines[0]).expect("valid JSON");
    assert_eq!(v["p.name"], "Alice");
    assert_eq!(v["p.age"], 30);

    Ok(())
}

#[tokio::test]
async fn test_export_prefix_not_supported() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let node = Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()));
    tx.add_node(node).await?;
    tx.commit().await?;

    let result = graph.execute_nql(r#"
        export json with jsonl=true
        find p.name
        from (p:Person)
    "#).await;

    assert!(result.is_err(), "prefix export should be rejected");

    Ok(())
}

#[tokio::test]
async fn test_single_quotes_and_block_comments() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let node = Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()))
        .with_property("city", PropertyValue::String("CDMX".into()));
    tx.add_node(node).await?;
    tx.commit().await?;

    let result = graph.execute_nql(r#"
        /* Block comment should be ignored */
        find p.name
        from (p:Person)
        where p.city = 'CDMX' // inline comment
    "#).await?;

    assert_eq!(result.len(), 1);

    let name = match result.rows()[0].get("p.name") {
        Some(PropertyValue::String(s)) => s.clone(),
        _ => "".to_string(),
    };
    assert_eq!(name, "Alice");

    Ok(())
}

#[tokio::test]
async fn test_pattern_group_by_count() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;

    let alice = tx.add_node(
        Node::new("Officer")
            .with_property("name", PropertyValue::String("Alice".into()))
    ).await?;
    let bob = tx.add_node(
        Node::new("Officer")
            .with_property("name", PropertyValue::String("Bob".into()))
    ).await?;

    let e1 = tx.add_node(
        Node::new("Entity")
            .with_property("name", PropertyValue::String("E1".into()))
    ).await?;
    let e2 = tx.add_node(
        Node::new("Entity")
            .with_property("name", PropertyValue::String("E2".into()))
    ).await?;
    let e3 = tx.add_node(
        Node::new("Entity")
            .with_property("name", PropertyValue::String("E3".into()))
    ).await?;

    tx.add_edge(Edge::new(alice, e1, "OFFICER_OF"))?;
    tx.add_edge(Edge::new(alice, e2, "OFFICER_OF"))?;
    tx.add_edge(Edge::new(bob, e3, "OFFICER_OF"))?;
    tx.commit().await?;

    let result = graph.execute_nql(r#"
        find o.name, count(*) as num_entities
        from (o:Officer)-[:OFFICER_OF]->(e:Entity)
        group by o.name
        order by o.name asc
    "#).await?;

    assert_eq!(result.len(), 2);

    let rows = result.rows();
    let alice_count = rows.iter().find_map(|r| {
        match (r.get("o.name"), r.get("num_entities")) {
            (Some(PropertyValue::String(name)), Some(PropertyValue::Int(n))) if name == "Alice" => Some(*n),
            _ => None,
        }
    }).unwrap_or_default();
    let bob_count = rows.iter().find_map(|r| {
        match (r.get("o.name"), r.get("num_entities")) {
            (Some(PropertyValue::String(name)), Some(PropertyValue::Int(n))) if name == "Bob" => Some(*n),
            _ => None,
        }
    }).unwrap_or_default();

    assert_eq!(alice_count, 2);
    assert_eq!(bob_count, 1);

    Ok(())
}

#[tokio::test]
async fn test_pattern_group_by_having_count() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;

    let alice = tx.add_node(
        Node::new("Officer")
            .with_property("name", PropertyValue::String("Alice".into()))
    ).await?;
    let bob = tx.add_node(
        Node::new("Officer")
            .with_property("name", PropertyValue::String("Bob".into()))
    ).await?;

    let e1 = tx.add_node(Node::new("Entity")).await?;
    let e2 = tx.add_node(Node::new("Entity")).await?;
    let e3 = tx.add_node(Node::new("Entity")).await?;

    tx.add_edge(Edge::new(alice, e1, "OFFICER_OF"))?;
    tx.add_edge(Edge::new(alice, e2, "OFFICER_OF"))?;
    tx.add_edge(Edge::new(bob, e3, "OFFICER_OF"))?;
    tx.commit().await?;

    let result = graph.execute_nql(r#"
        find o.name, count(*) as num_entities
        from (o:Officer)-[:OFFICER_OF]->(e:Entity)
        group by o.name
        having count(*) > 1
    "#).await?;

    assert_eq!(result.len(), 1);
    assert_eq!(
        result.rows()[0].get("o.name"),
        Some(&PropertyValue::String("Alice".into()))
    );
    assert_eq!(
        result.rows()[0].get("num_entities"),
        Some(&PropertyValue::Int(2))
    );

    Ok(())
}
