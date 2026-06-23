// tests/nql_pattern_matching_test.rs

use nopaldb::{Edge, Graph, Node, PropertyValue, Result};

#[tokio::test]
async fn test_simple_pattern_matching() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Create test data: Alice -> KNOWS -> Bob
    let mut tx = graph.begin_transaction().await?;

    let alice = Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()))
        .with_property("age", PropertyValue::Int(30));

    let bob = Node::new("Person")
        .with_property("name", PropertyValue::String("Bob".into()))
        .with_property("age", PropertyValue::Int(25));

    let alice_id = tx.add_node(alice).await?;
    let bob_id = tx.add_node(bob).await?;

    let edge = Edge::new(alice_id, bob_id, "KNOWS");
    tx.add_edge(edge)?; // ← SIN .await

    tx.commit().await?;

    println!("✅ Created test graph: Alice -> KNOWS -> Bob\n");

    // Execute pattern matching query
    let result = graph
        .execute_nql(
            r#"
        find a.name, b.name
        from (a:Person) -> [:KNOWS] -> (b:Person)
    "#,
        )
        .await?;

    println!("📊 Query result:");
    println!("   Rows: {}", result.len());
    println!("   Columns: {:?}\n", result.columns);

    for (i, row) in result.rows().iter().enumerate() {
        println!(
            "   Row {}: a.name={:?}, b.name={:?}",
            i,
            row.get("a.name"),
            row.get("b.name")
        );
    }

    assert_eq!(result.len(), 1, "Should find one match");

    // Verify the match
    let row = &result.rows()[0];
    assert_eq!(
        row.get("a.name"),
        Some(&PropertyValue::String("Alice".into()))
    );
    assert_eq!(
        row.get("b.name"),
        Some(&PropertyValue::String("Bob".into()))
    );

    println!("\n✅ Pattern matching test passed!");

    Ok(())
}

#[tokio::test]
async fn test_pattern_with_multiple_relationships() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Create: Alice -> KNOWS -> Bob, Alice -> KNOWS -> Charlie
    let mut tx = graph.begin_transaction().await?;

    let alice = Node::new("Person").with_property("name", PropertyValue::String("Alice".into()));
    let bob = Node::new("Person").with_property("name", PropertyValue::String("Bob".into()));
    let charlie =
        Node::new("Person").with_property("name", PropertyValue::String("Charlie".into()));

    let alice_id = tx.add_node(alice).await?;
    let bob_id = tx.add_node(bob).await?;
    let charlie_id = tx.add_node(charlie).await?;

    tx.add_edge(Edge::new(alice_id, bob_id, "KNOWS"))?; // ← SIN .await
    tx.add_edge(Edge::new(alice_id, charlie_id, "KNOWS"))?; // ← SIN .await

    tx.commit().await?;

    println!("✅ Created graph: Alice -> KNOWS -> Bob, Charlie\n");

    // Query should return 2 matches
    let result = graph
        .execute_nql(
            r#"
        find a.name, friend.name
        from (a:Person) -> [:KNOWS] -> (friend:Person)
    "#,
        )
        .await?;

    println!("📊 Found {} relationships", result.len());

    for row in result.rows() {
        println!(
            "   {} knows {}",
            match row.get("a.name") {
                Some(PropertyValue::String(s)) => s,
                _ => "?",
            },
            match row.get("friend.name") {
                Some(PropertyValue::String(s)) => s,
                _ => "?",
            }
        );
    }

    assert_eq!(result.len(), 2, "Should find two KNOWS relationships");

    println!("\n✅ Multiple relationships test passed!");

    Ok(())
}

#[tokio::test]
async fn test_pattern_with_edge_type_filter() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Create mixed relationships
    let mut tx = graph.begin_transaction().await?;

    let alice = Node::new("Person").with_property("name", PropertyValue::String("Alice".into()));
    let bob = Node::new("Person").with_property("name", PropertyValue::String("Bob".into()));
    let acme =
        Node::new("Company").with_property("name", PropertyValue::String("Acme Corp".into()));

    let alice_id = tx.add_node(alice).await?;
    let bob_id = tx.add_node(bob).await?;
    let acme_id = tx.add_node(acme).await?;

    tx.add_edge(Edge::new(alice_id, bob_id, "KNOWS"))?; // ← SIN .await
    tx.add_edge(Edge::new(alice_id, acme_id, "WORKS_AT"))?; // ← SIN .await

    tx.commit().await?;

    println!("✅ Created mixed graph\n");

    // Query only KNOWS relationships
    let result = graph
        .execute_nql(
            r#"
        find a.name, b.name
        from (a:Person) -> [:KNOWS] -> (b:Person)
    "#,
        )
        .await?;

    println!("📊 KNOWS relationships: {}", result.len());
    assert_eq!(result.len(), 1, "Should find only KNOWS relationship");

    // Query WORKS_AT relationships
    let result2 = graph
        .execute_nql(
            r#"
        find p.name, c.name
        from (p:Person) -> [:WORKS_AT] -> (c:Company)
    "#,
        )
        .await?;

    println!("📊 WORKS_AT relationships: {}", result2.len());
    assert_eq!(result2.len(), 1, "Should find only WORKS_AT relationship");

    println!("\n✅ Edge type filter test passed!");

    Ok(())
}

#[tokio::test]
async fn test_pattern_no_matches() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Create nodes but no relationships
    let mut tx = graph.begin_transaction().await?;

    let alice = Node::new("Person").with_property("name", PropertyValue::String("Alice".into()));

    tx.add_node(alice).await?;
    tx.commit().await?;

    println!("✅ Created lone node\n");

    // Query for non-existent relationship
    let result = graph
        .execute_nql(
            r#"
        find a.name, b.name
        from (a:Person) -> [:KNOWS] -> (b:Person)
    "#,
        )
        .await?;

    println!("📊 Matches found: {}", result.len());
    assert_eq!(result.len(), 0, "Should find no matches");

    println!("\n✅ No matches test passed!");

    Ok(())
}

#[tokio::test]
async fn test_pattern_projection_all_for_target_and_relationship() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;

    let alice = Node::new("Person").with_property("name", PropertyValue::String("Alice".into()));
    let bob = Node::new("Person")
        .with_property("name", PropertyValue::String("Bob".into()))
        .with_property("country", PropertyValue::String("MX".into()));

    let alice_id = tx.add_node(alice).await?;
    let bob_id = tx.add_node(bob).await?;

    let edge =
        Edge::new(alice_id, bob_id, "KNOWS").with_property("since", PropertyValue::Int(2020));
    tx.add_edge(edge)?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find all(b), all(r)
        from (a:Person)-[r:KNOWS]->(b:Person)
        where a.name = "Alice"
    "#,
        )
        .await?;

    assert_eq!(result.len(), 1);
    let row = &result.rows()[0];

    assert_eq!(
        row.get("b.name"),
        Some(&PropertyValue::String("Bob".into()))
    );
    assert_eq!(
        row.get("b.country"),
        Some(&PropertyValue::String("MX".into()))
    );
    assert!(row.get("b.id").is_some());
    assert_eq!(
        row.get("b.label"),
        Some(&PropertyValue::String("Person".into()))
    );

    assert_eq!(row.get("r.since"), Some(&PropertyValue::Int(2020)));
    assert_eq!(
        row.get("r.type"),
        Some(&PropertyValue::String("KNOWS".into()))
    );
    assert!(row.get("r.id").is_some());

    Ok(())
}

#[tokio::test]
async fn test_pattern_matching_projection_aliases_return_values() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;

    let alice = Node::new("Family")
        .with_property("name", PropertyValue::String("Medici".into()))
        .with_property("faction", PropertyValue::String("Bank".into()))
        .with_property("wealth_rank", PropertyValue::Int(1));
    let bob = Node::new("Family")
        .with_property("name", PropertyValue::String("Pazzi".into()))
        .with_property("faction", PropertyValue::String("Trade".into()))
        .with_property("wealth_rank", PropertyValue::Int(2));

    let alice_id = tx.add_node(alice).await?;
    let bob_id = tx.add_node(bob).await?;
    tx.add_edge(Edge::new(alice_id, bob_id, "MARRIAGE"))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find a.name as source,
             a.faction as source_faction,
             a.wealth_rank as source_wealth,
             b.name as target,
             b.faction as target_faction,
             b.wealth_rank as target_wealth
        from (a:Family)-[e:MARRIAGE]->(b:Family)
        where a.faction != b.faction
        order by a.wealth_rank asc, b.wealth_rank asc
        limit 30
    "#,
        )
        .await?;

    assert_eq!(result.len(), 1);
    assert_eq!(
        result.columns,
        vec![
            "source".to_string(),
            "source_faction".to_string(),
            "source_wealth".to_string(),
            "target".to_string(),
            "target_faction".to_string(),
            "target_wealth".to_string(),
        ]
    );

    let row = &result.rows()[0];
    assert_eq!(
        row.get("source"),
        Some(&PropertyValue::String("Medici".into()))
    );
    assert_eq!(
        row.get("source_faction"),
        Some(&PropertyValue::String("Bank".into()))
    );
    assert_eq!(row.get("source_wealth"), Some(&PropertyValue::Int(1)));
    assert_eq!(
        row.get("target"),
        Some(&PropertyValue::String("Pazzi".into()))
    );
    assert_eq!(
        row.get("target_faction"),
        Some(&PropertyValue::String("Trade".into()))
    );
    assert_eq!(row.get("target_wealth"), Some(&PropertyValue::Int(2)));

    Ok(())
}

#[tokio::test]
async fn test_linear_multihop_pattern_returns_all_bound_nodes() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;

    let albizzi =
        Node::new("Family").with_property("name", PropertyValue::String("Albizzi".into()));
    let bridge = Node::new("Family").with_property("name", PropertyValue::String("Ridolfi".into()));
    let medici = Node::new("Family").with_property("name", PropertyValue::String("Medici".into()));

    let albizzi_id = tx.add_node(albizzi).await?;
    let bridge_id = tx.add_node(bridge).await?;
    let medici_id = tx.add_node(medici).await?;

    tx.add_edge(Edge::new(albizzi_id, bridge_id, "MARRIAGE"))?;
    tx.add_edge(Edge::new(bridge_id, medici_id, "MARRIAGE"))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find a.name, b.name, c.name
        from (a:Family)-[:MARRIAGE]->(b:Family)-[:MARRIAGE]->(c:Family)
    "#,
        )
        .await?;

    assert_eq!(result.len(), 1);
    let row = &result.rows()[0];
    assert_eq!(
        row.get("a.name"),
        Some(&PropertyValue::String("Albizzi".into()))
    );
    assert_eq!(
        row.get("b.name"),
        Some(&PropertyValue::String("Ridolfi".into()))
    );
    assert_eq!(
        row.get("c.name"),
        Some(&PropertyValue::String("Medici".into()))
    );

    Ok(())
}

#[tokio::test]
async fn test_linear_multihop_pattern_supports_where_order_by_and_aliases() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;

    let albizzi = Node::new("Family")
        .with_property("name", PropertyValue::String("Albizzi".into()))
        .with_property("faction", PropertyValue::String("Albizzi".into()));
    let medici = Node::new("Family")
        .with_property("name", PropertyValue::String("Medici".into()))
        .with_property("faction", PropertyValue::String("Medici".into()));
    let ridolfi = Node::new("Family")
        .with_property("name", PropertyValue::String("Ridolfi".into()))
        .with_property("faction", PropertyValue::String("Neutral".into()));
    let salviati = Node::new("Family")
        .with_property("name", PropertyValue::String("Salviati".into()))
        .with_property("faction", PropertyValue::String("Neutral".into()));

    let albizzi_id = tx.add_node(albizzi).await?;
    let medici_id = tx.add_node(medici).await?;
    let ridolfi_id = tx.add_node(ridolfi).await?;
    let salviati_id = tx.add_node(salviati).await?;

    tx.add_edge(Edge::new(albizzi_id, ridolfi_id, "MARRIAGE"))?;
    tx.add_edge(Edge::new(ridolfi_id, medici_id, "MARRIAGE"))?;
    tx.add_edge(Edge::new(albizzi_id, salviati_id, "MARRIAGE"))?;
    tx.add_edge(Edge::new(salviati_id, medici_id, "MARRIAGE"))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find x.name as bridge,
             a.name as albizzi_family,
             m.name as medici_family
        from (a:Family)-[:MARRIAGE]->(x:Family)-[:MARRIAGE]->(m:Family)
        where a.faction = "Albizzi"
          and m.faction = "Medici"
          and x.name != a.name
          and x.name != m.name
        order by bridge
    "#,
        )
        .await?;

    assert_eq!(result.len(), 2);
    assert_eq!(
        result.columns,
        vec![
            "bridge".to_string(),
            "albizzi_family".to_string(),
            "medici_family".to_string(),
        ]
    );

    assert_eq!(
        result.rows()[0].get("bridge"),
        Some(&PropertyValue::String("Ridolfi".into()))
    );
    assert_eq!(
        result.rows()[1].get("bridge"),
        Some(&PropertyValue::String("Salviati".into()))
    );

    for row in result.rows() {
        assert_eq!(
            row.get("albizzi_family"),
            Some(&PropertyValue::String("Albizzi".into()))
        );
        assert_eq!(
            row.get("medici_family"),
            Some(&PropertyValue::String("Medici".into()))
        );
    }

    Ok(())
}

#[tokio::test]
async fn test_multi_pattern_query_joins_shared_variables() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;

    let albizzi =
        Node::new("Family").with_property("name", PropertyValue::String("Albizzi".into()));
    let medici = Node::new("Family").with_property("name", PropertyValue::String("Medici".into()));
    let ridolfi =
        Node::new("Family").with_property("name", PropertyValue::String("Ridolfi".into()));
    let pazzi = Node::new("Family").with_property("name", PropertyValue::String("Pazzi".into()));
    let salviati =
        Node::new("Family").with_property("name", PropertyValue::String("Salviati".into()));

    let albizzi_id = tx.add_node(albizzi).await?;
    let medici_id = tx.add_node(medici).await?;
    let ridolfi_id = tx.add_node(ridolfi).await?;
    let pazzi_id = tx.add_node(pazzi).await?;
    let salviati_id = tx.add_node(salviati).await?;

    tx.add_edge(Edge::new(albizzi_id, medici_id, "MARRIAGE"))?;
    tx.add_edge(Edge::new(medici_id, ridolfi_id, "MARRIAGE"))?;
    tx.add_edge(Edge::new(albizzi_id, ridolfi_id, "MARRIAGE"))?;
    tx.add_edge(Edge::new(albizzi_id, pazzi_id, "MARRIAGE"))?;
    tx.add_edge(Edge::new(pazzi_id, salviati_id, "MARRIAGE"))?;
    tx.add_edge(Edge::new(albizzi_id, salviati_id, "MARRIAGE"))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find a.name as a, b.name as b, c.name as c
        from (a:Family)-[:MARRIAGE]->(b:Family),
             (b:Family)-[:MARRIAGE]->(c:Family),
             (a:Family)-[:MARRIAGE]->(c:Family)
        where a.name < b.name and b.name < c.name
        order by a, b, c
        limit 50
    "#,
        )
        .await?;

    assert_eq!(result.len(), 2);
    let mut tuples: Vec<(String, String, String)> = result
        .rows()
        .iter()
        .map(|row| {
            (
                row.get("a")
                    .and_then(PropertyValue::as_str)
                    .unwrap_or_default()
                    .to_string(),
                row.get("b")
                    .and_then(PropertyValue::as_str)
                    .unwrap_or_default()
                    .to_string(),
                row.get("c")
                    .and_then(PropertyValue::as_str)
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .collect();
    tuples.sort();

    assert_eq!(
        tuples,
        vec![
            (
                "Albizzi".to_string(),
                "Medici".to_string(),
                "Ridolfi".to_string()
            ),
            (
                "Albizzi".to_string(),
                "Pazzi".to_string(),
                "Salviati".to_string()
            ),
        ]
    );

    Ok(())
}

#[tokio::test]
async fn test_distinct_on_multihop_pattern_deduplicates_projected_rows() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;

    let medici = Node::new("Family").with_property("name", PropertyValue::String("Medici".into()));
    let ridolfi =
        Node::new("Family").with_property("name", PropertyValue::String("Ridolfi".into()));
    let salviati =
        Node::new("Family").with_property("name", PropertyValue::String("Salviati".into()));
    let pazzi = Node::new("Family")
        .with_property("name", PropertyValue::String("Pazzi".into()))
        .with_property("faction", PropertyValue::String("Trade".into()))
        .with_property("wealth_rank", PropertyValue::Int(2));

    let medici_id = tx.add_node(medici).await?;
    let ridolfi_id = tx.add_node(ridolfi).await?;
    let salviati_id = tx.add_node(salviati).await?;
    let pazzi_id = tx.add_node(pazzi).await?;

    tx.add_edge(Edge::new(medici_id, ridolfi_id, "MARRIAGE"))?;
    tx.add_edge(Edge::new(ridolfi_id, pazzi_id, "MARRIAGE"))?;
    tx.add_edge(Edge::new(medici_id, salviati_id, "MARRIAGE"))?;
    tx.add_edge(Edge::new(salviati_id, pazzi_id, "MARRIAGE"))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find distinct b.name, b.faction, b.wealth_rank
        from (m:Family {name: "Medici"})-[:MARRIAGE]->(:Family)-[:MARRIAGE]->(b:Family)
        where b.name != "Medici"
        order by b.name
    "#,
        )
        .await?;

    assert_eq!(result.len(), 1);
    let row = &result.rows()[0];
    assert_eq!(
        row.get("b.name"),
        Some(&PropertyValue::String("Pazzi".into()))
    );
    assert_eq!(
        row.get("b.faction"),
        Some(&PropertyValue::String("Trade".into()))
    );
    assert_eq!(row.get("b.wealth_rank"), Some(&PropertyValue::Int(2)));

    Ok(())
}
