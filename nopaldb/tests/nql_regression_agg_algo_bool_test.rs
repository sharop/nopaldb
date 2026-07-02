use nopaldb::query::nql::parse;
use nopaldb::query::nql::parser::ast::{Expression, Statement};
use nopaldb::{Edge, Graph, Node, PropertyValue, Result, NqlResult};

async fn setup_graph() -> Result<Graph> {
    let graph = Graph::in_memory().await?;
    let mut tx = graph.begin_transaction().await?;

    let mut users = Vec::new();
    for i in 0..24 {
        let user = Node::new("User")
            .with_property("username", PropertyValue::String(format!("user_{:03}", i)))
            .with_property("team", PropertyValue::String(format!("team_{}", i % 4)))
            .with_property("score", PropertyValue::Int(70 + (i % 31) as i64))
            .with_property("active", PropertyValue::Bool(i % 3 != 0));
        users.push(user.id);
        tx.add_node(user).await?;
    }

    // Add a simple ring so centrality algorithms have topology.
    for i in 0..users.len() {
        let source = users[i];
        let target = users[(i + 1) % users.len()];
        tx.add_edge(Edge::new(source, target, "KNOWS"))?;
    }

    tx.commit().await?;
    Ok(graph)
}

#[tokio::test]
async fn test_group_by_aggregations_are_materialized() -> Result<()> {
    let graph = setup_graph().await?;
    let result = graph.execute_nql(
        "find u.team, count(u) as total, avg(u.score) as avg_score from (u:User) group by u.team order by u.team asc"
    ).await?;

    assert_eq!(result.len(), 4, "expected one row per team");

    for row in result.rows() {
        assert!(matches!(row.get("u.team"), Some(PropertyValue::String(_))));
        assert!(matches!(row.get("total"), Some(PropertyValue::Int(_))));
        assert!(matches!(row.get("avg_score"), Some(PropertyValue::Float(_))));
    }

    Ok(())
}

#[tokio::test]
async fn test_algorithms_are_materialized_not_null() -> Result<()> {
    let graph = setup_graph().await?;
    let result = graph.execute_nql(
        "find degree(u) as deg, pagerank(u) as pr, betweenness(u) as bc, clustering(u) as cc from (u:User)"
    ).await?;

    assert_eq!(result.len(), 1, "non-grouped algorithm query should return one aggregated row");
    let row = &result.rows()[0];

    assert!(matches!(row.get("deg"), Some(PropertyValue::Float(_))));
    assert!(matches!(row.get("pr"), Some(PropertyValue::Float(_))));
    assert!(matches!(row.get("bc"), Some(PropertyValue::Float(_))));
    assert!(matches!(row.get("cc"), Some(PropertyValue::Float(_))));

    Ok(())
}

#[tokio::test]
async fn test_community_returns_float_aggregation() -> Result<()> {
    let graph = setup_graph().await?;
    let result = graph.execute_nql(
        "find community(u) as comm from (u:User)"
    ).await?;

    assert_eq!(result.len(), 1, "community() debe devolver un row agregado");
    let row = &result.rows()[0];
    assert!(
        matches!(row.get("comm"), Some(PropertyValue::Float(_))),
        "community() debe devolver Float"
    );

    Ok(())
}

#[tokio::test]
async fn test_community_fast_returns_float_aggregation() -> Result<()> {
    let graph = setup_graph().await?;
    let result = graph.execute_nql(
        "find community_fast(u) as comm_fast from (u:User)"
    ).await?;

    assert_eq!(result.len(), 1, "community_fast() debe devolver un row agregado");
    let row = &result.rows()[0];
    assert!(
        matches!(row.get("comm_fast"), Some(PropertyValue::Float(_))),
        "community_fast() debe devolver Float"
    );

    Ok(())
}

#[tokio::test]
async fn test_shortest_path_returns_distance() -> Result<()> {
    // Crear grafo pequeño con 3 nodos conectados: A → B → C
    let graph = Graph::in_memory().await?;
    let mut tx = graph.begin_transaction().await?;

    let a = Node::new("Step").with_property("name", PropertyValue::String("A".into()));
    let b = Node::new("Step").with_property("name", PropertyValue::String("B".into()));
    let c = Node::new("Step").with_property("name", PropertyValue::String("C".into()));

    let id_a = a.id;
    let id_b = b.id;
    let id_c = c.id;

    tx.add_node(a).await?;
    tx.add_node(b).await?;
    tx.add_node(c).await?;
    tx.add_edge(Edge::new(id_a, id_b, "NEXT"))?;
    tx.add_edge(Edge::new(id_b, id_c, "NEXT"))?;
    tx.commit().await?;

    // shortestPath de A a C debe devolver distancia 2.0
    let query = format!(
        r#"find shortestPath("{}", "{}") as dist from (n:Step) limit 1"#,
        id_a, id_c
    );
    let result = graph.execute_nql(&query).await?;

    assert_eq!(result.len(), 1);
    let row = &result.rows()[0];
    assert!(
        matches!(row.get("dist"), Some(PropertyValue::Float(d)) if *d == 2.0),
        "shortestPath(A, C) debe ser 2.0, got: {:?}", row.get("dist")
    );

    Ok(())
}

#[tokio::test]
async fn test_shortest_path_returns_minus_one_when_no_path() -> Result<()> {
    // Dos nodos sin arista entre ellos
    let graph = Graph::in_memory().await?;
    let mut tx = graph.begin_transaction().await?;

    let a = Node::new("Island");
    let b = Node::new("Island");
    let id_a = a.id;
    let id_b = b.id;

    tx.add_node(a).await?;
    tx.add_node(b).await?;
    tx.commit().await?;

    let query = format!(
        r#"find shortestPath("{}", "{}") as dist from (n:Island) limit 1"#,
        id_a, id_b
    );
    let result = graph.execute_nql(&query).await?;

    assert_eq!(result.len(), 1);
    let row = &result.rows()[0];
    assert!(
        matches!(row.get("dist"), Some(PropertyValue::Float(d)) if *d == -1.0),
        "shortestPath con nodos desconectados debe devolver -1.0, got: {:?}", row.get("dist")
    );

    Ok(())
}

#[tokio::test]
async fn test_boolean_update_and_where_roundtrip() -> Result<()> {
    let graph = setup_graph().await?;

    let update_result = graph.execute_statement(
        "update (u:User) set u.ndbstudio_checked = true"
    ).await?;
    match update_result {
        NqlResult::Write(w) => {
            assert!(w.nodes_updated > 0, "expected UPDATE to modify rows");
            assert!(w.properties_changed > 0, "expected UPDATE to modify at least one property");
        }
        other => panic!("expected write result, got: {:?}", other),
    }

    let result = graph.execute_nql(
        "find u.username, u.ndbstudio_checked from (u:User) where u.ndbstudio_checked = true limit 10"
    ).await?;

    assert!(!result.is_empty(), "expected boolean equality filter to match updated rows");
    for row in result.rows() {
        assert!(matches!(
            row.get("u.ndbstudio_checked"),
            Some(PropertyValue::Bool(true))
        ));
    }

    Ok(())
}

#[test]
fn test_parser_treats_true_as_boolean_literal() {
    let stmt = parse("update (u:User) set u.flag = true where u.active = true")
        .expect("query should parse");

    match stmt {
        Statement::Update(update) => {
            assert!(matches!(
                update.assignments[0].value,
                Expression::Literal(PropertyValue::Bool(true))
            ));

            let filter = update.filter.expect("where clause is expected");
            match filter.condition {
                Expression::BinaryOp { right, .. } => {
                    assert!(matches!(
                        *right,
                        Expression::Literal(PropertyValue::Bool(true))
                    ));
                }
                _ => panic!("expected binary condition in where"),
            }
        }
        _ => panic!("expected update statement"),
    }
}

#[test]
fn test_parser_treats_true_as_boolean_literal_in_update_queries_used_by_qa() {
    let stmt = parse("update (u:User) set u.ndbstudio_checked = true where u.score > 95 limit 5")
        .expect("query should parse");

    match stmt {
        Statement::Update(update) => {
            assert!(matches!(
                update.assignments[0].value,
                Expression::Literal(PropertyValue::Bool(true))
            ));
        }
        _ => panic!("expected update statement"),
    }
}

#[tokio::test]
async fn test_bare_variable_projection_returns_ids_not_null() {
    // Regresion: `find n from (n) limit 25` devolvía null en todas las filas.
    // La causa era que el parser convertía `n` en Expression::Property { variable: "n", property: "" },
    // generando la columna "n." — y ProjectNodesStream no manejaba prop vacío.
    // El valor ya se corrigió y el header público debe exponerse como "n".
    let graph = setup_graph().await.expect("graph setup");
    let result = graph.execute_nql("find n from (n) limit 25").await
        .expect("query must not fail");

    assert!(!result.rows().is_empty(), "debe haber filas");
    assert_eq!(result.columns, vec!["n".to_string()], "header público debe ser 'n'");
    for row in result.rows() {
        let val = row.values.get("n");
        assert!(val.is_some(), "la columna 'n' no debe ser null: {:?}", row);
        if let Some(PropertyValue::String(id)) = val {
            assert!(!id.is_empty(), "el id no debe ser vacío");
        }
    }
}
