use nopaldb::{Edge, Graph, Node, NqlResult, PropertyValue, Result};

fn field<'a>(fields: &'a [(String, PropertyValue)], key: &str) -> Option<&'a PropertyValue> {
    fields
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}

#[tokio::test]
async fn test_path_metadata_projection_for_fixed_path() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let start = tx.add_node(Node::new("Start")).await?;
    let middle = tx.add_node(Node::new("Middle")).await?;
    let end = tx.add_node(Node::new("End")).await?;
    tx.add_edge(Edge::new(start, middle, "LINK"))?;
    tx.add_edge(Edge::new(middle, end, "LINK"))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find path.depth, path.nodes, path.edges
        from (s:Start)-[:LINK]->(m:Middle)-[:LINK]->(e:End)
    "#,
        )
        .await?;

    assert_eq!(result.len(), 1);
    let row = &result.rows()[0];

    assert_eq!(row.get("path.depth"), Some(&PropertyValue::Int(2)));

    let nodes = row.get("path.nodes").expect("path.nodes must exist");
    let nodes = nodes.as_list().expect("path.nodes must be a list");
    assert_eq!(nodes.len(), 3);
    assert!(matches!(nodes[0], PropertyValue::Object(_)));
    if let PropertyValue::Object(fields) = &nodes[0] {
        assert_eq!(
            field(fields, "label"),
            Some(&PropertyValue::String("Start".to_string()))
        );
        assert!(matches!(
            field(fields, "id"),
            Some(PropertyValue::String(_))
        ));
    }
    if let PropertyValue::Object(fields) = &nodes[2] {
        assert_eq!(
            field(fields, "label"),
            Some(&PropertyValue::String("End".to_string()))
        );
    }

    let edges = row.get("path.edges").expect("path.edges must exist");
    let edges = edges.as_list().expect("path.edges must be a list");
    assert_eq!(edges.len(), 2);
    if let PropertyValue::Object(fields) = &edges[0] {
        assert_eq!(
            field(fields, "type"),
            Some(&PropertyValue::String("LINK".to_string()))
        );
        assert!(matches!(
            field(fields, "source"),
            Some(PropertyValue::String(_))
        ));
        assert!(matches!(
            field(fields, "target"),
            Some(PropertyValue::String(_))
        ));
    }

    Ok(())
}

#[tokio::test]
async fn test_path_depth_where_and_order_by() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let alice = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Alice".into())))
        .await?;
    let bob = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Bob".into())))
        .await?;
    let carol = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Carol".into())))
        .await?;
    tx.add_edge(Edge::new(alice, bob, "KNOWS"))?;
    tx.add_edge(Edge::new(bob, carol, "KNOWS"))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find b.name, path.depth
        from (a:Person {name: "Alice"})-[:KNOWS]->{1,2}(b:Person)
        where path.depth >= 1
        order by path.depth desc
    "#,
        )
        .await?;

    assert_eq!(result.len(), 2);
    assert_eq!(
        result.rows()[0].get("path.depth"),
        Some(&PropertyValue::Int(2))
    );
    assert_eq!(
        result.rows()[0].get("b.name"),
        Some(&PropertyValue::String("Carol".to_string()))
    );
    assert_eq!(
        result.rows()[1].get("path.depth"),
        Some(&PropertyValue::Int(1))
    );
    assert_eq!(
        result.rows()[1].get("b.name"),
        Some(&PropertyValue::String("Bob".to_string()))
    );

    Ok(())
}

#[tokio::test]
async fn test_path_nodes_and_edges_rejected_outside_find() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx.add_node(Node::new("Person")).await?;
    let b = tx.add_node(Node::new("Person")).await?;
    tx.add_edge(Edge::new(a, b, "KNOWS"))?;
    tx.commit().await?;

    let where_err = graph
        .execute_nql(
            r#"
        find b.id
        from (a:Person)-[:KNOWS]->(b:Person)
        where path.nodes = null
    "#,
        )
        .await
        .unwrap_err();
    assert!(format!("{}", where_err).contains("only supported in FIND projections"));

    let order_err = graph
        .execute_nql(
            r#"
        find b.id
        from (a:Person)-[:KNOWS]->(b:Person)
        order by path.edges
    "#,
        )
        .await
        .unwrap_err();
    assert!(format!("{}", order_err).contains("not supported in ORDER BY"));

    Ok(())
}

#[tokio::test]
async fn test_path_metadata_requires_single_linear_pattern() -> Result<()> {
    let graph = Graph::in_memory().await?;
    let err = graph
        .execute_nql(
            r#"
        find path.depth
        from (n:Person)
    "#,
        )
        .await
        .unwrap_err();

    assert!(format!("{}", err).contains("single linear pattern"));
    Ok(())
}

#[tokio::test]
async fn test_profile_returns_structured_result_for_path_query() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let alice = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Alice".into())))
        .await?;
    let bob = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Bob".into())))
        .await?;
    let carol = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Carol".into())))
        .await?;
    tx.add_edge(Edge::new(alice, bob, "KNOWS"))?;
    tx.add_edge(Edge::new(bob, carol, "KNOWS"))?;
    tx.commit().await?;

    let result = graph
        .execute_statement(
            r#"
        profile find b.name, path.depth
        from (a:Person {name: "Alice"})-[:KNOWS]->{1,2}(b:Person)
    "#,
        )
        .await?;

    match result {
        NqlResult::Profile(profile) => {
            assert_eq!(profile.statement_type, "query");
            assert_eq!(profile.rows_returned, 2);
            assert!(profile.execution_ms >= 0.0);
            assert!(profile.path_query);
            assert_eq!(
                profile.columns,
                vec!["b.name".to_string(), "path.depth".to_string()]
            );

            let metrics = profile.path_metrics.expect("path metrics must exist");
            let fields = metrics.as_object().expect("path metrics must be an object");
            assert!(field(fields, "bindings_examined").is_some());
            assert!(field(fields, "bindings_emitted").is_some());
            assert!(field(fields, "frontier_states_visited").is_some());
            assert!(field(fields, "cycle_prunes").is_some());
            assert!(field(fields, "max_depth_observed").is_some());
        }
        other => panic!("Expected NqlResult::Profile, got {:?}", other),
    }

    Ok(())
}

#[tokio::test]
async fn test_profile_non_path_query_has_no_path_metrics() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    tx.add_node(Node::new("Person").with_property("name", PropertyValue::String("Alice".into())))
        .await?;
    tx.commit().await?;

    let result = graph
        .execute_statement(
            r#"
        profile find n.name
        from (n:Person)
    "#,
        )
        .await?;

    match result {
        NqlResult::Profile(profile) => {
            assert!(!profile.path_query);
            assert!(profile.path_metrics.is_none());
            assert_eq!(profile.rows_returned, 1);
        }
        other => panic!("Expected NqlResult::Profile, got {:?}", other),
    }

    Ok(())
}

#[tokio::test]
async fn test_execute_nql_rejects_profile_statement() -> Result<()> {
    let graph = Graph::in_memory().await?;
    let err = graph
        .execute_nql("profile find n.id from (n:Person)")
        .await
        .unwrap_err();
    assert!(format!("{}", err).contains("execute_statement"));
    Ok(())
}

#[test]
fn test_property_value_messagepack_roundtrip_for_structured_values() {
    let value = PropertyValue::Object(vec![
        ("depth".to_string(), PropertyValue::Int(2)),
        (
            "nodes".to_string(),
            PropertyValue::List(vec![
                PropertyValue::Object(vec![
                    ("id".to_string(), PropertyValue::String("n1".to_string())),
                    (
                        "label".to_string(),
                        PropertyValue::String("Start".to_string()),
                    ),
                ]),
                PropertyValue::Object(vec![
                    ("id".to_string(), PropertyValue::String("n2".to_string())),
                    (
                        "label".to_string(),
                        PropertyValue::String("End".to_string()),
                    ),
                ]),
            ]),
        ),
    ]);

    let bytes = rmp_serde::to_vec(&value).expect("MessagePack serialization must work");
    let decoded: PropertyValue =
        rmp_serde::from_slice(&bytes).expect("MessagePack deserialization must work");
    assert_eq!(decoded, value);
}
