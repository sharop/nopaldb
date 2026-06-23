use nopaldb::{Graph, NqlResult, PropertyValue, Result};

#[tokio::test]
async fn test_add_relationship_persists_edge_properties() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let result = graph
        .execute_statement(
            r#"add (a:Person {name: "Alice"})-[:KNOWS {since: 2020, strength: "high"}]->(b:Person {name: "Bob"})"#,
        )
        .await?;

    match result {
        NqlResult::Write(w) => {
            assert_eq!(w.nodes_created, 2);
            assert_eq!(w.edges_created, 1);
        }
        other => panic!("Expected Write result, got {:?}", other),
    }

    let edges = graph.get_all_edges().await?;
    assert_eq!(edges.len(), 1);
    assert_eq!(
        edges[0].properties.get("since"),
        Some(&PropertyValue::Int(2020))
    );
    assert_eq!(
        edges[0].properties.get("strength"),
        Some(&PropertyValue::String("high".into()))
    );

    Ok(())
}

#[tokio::test]
async fn test_update_relationship_properties_via_nql() -> Result<()> {
    let graph = Graph::in_memory().await?;

    graph
        .execute_statement(
            r#"add (a:Person {name: "Alice"})-[r:KNOWS {since: 2020, strength: "medium"}]->(b:Person {name: "Bob"})"#,
        )
        .await?;

    let result = graph
        .execute_statement(
            r#"update (a:Person)-[r:KNOWS]->(b:Person) set r.since = 2024, r.strength = "high" where a.name = "Alice" and b.name = "Bob""#,
        )
        .await?;

    match result {
        NqlResult::Write(w) => {
            assert_eq!(w.nodes_updated, 0);
            assert_eq!(w.edges_updated, 1);
            assert_eq!(w.properties_changed, 2);
        }
        other => panic!("Expected Write result, got {:?}", other),
    }

    let edges = graph.get_all_edges().await?;
    assert_eq!(edges.len(), 1);
    assert_eq!(
        edges[0].properties.get("since"),
        Some(&PropertyValue::Int(2024))
    );
    assert_eq!(
        edges[0].properties.get("strength"),
        Some(&PropertyValue::String("high".into()))
    );

    Ok(())
}

#[tokio::test]
async fn test_delete_relationship_only_preserves_nodes() -> Result<()> {
    let graph = Graph::in_memory().await?;

    graph
        .execute_statement(
            r#"add (a:Person {name: "Alice"})-[:KNOWS {since: 2020}]->(b:Person {name: "Bob"})"#,
        )
        .await?;

    let result = graph
        .execute_statement(
            r#"delete (a:Person)-[:KNOWS]->(b:Person) where a.name = "Alice" and b.name = "Bob""#,
        )
        .await?;

    match result {
        NqlResult::Write(w) => {
            assert_eq!(w.nodes_deleted, 0);
            assert_eq!(w.edges_deleted, 1);
        }
        other => panic!("Expected Write result, got {:?}", other),
    }

    assert_eq!(graph.get_nodes_by_label("Person").await?.len(), 2);
    assert_eq!(graph.get_all_edges().await?.len(), 0);

    Ok(())
}
