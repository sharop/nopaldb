use nopaldb::{Edge, Graph, Node, PropertyValue, Result};

fn int(v: i64) -> PropertyValue {
    PropertyValue::Int(v)
}

#[tokio::test]
async fn test_path_vm_projection_and_where_filter() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx.add_node(Node::new("Account").with_property("name", PropertyValue::String("A".into()))).await?;
    let b = tx.add_node(Node::new("Account").with_property("name", PropertyValue::String("B".into()))).await?;
    let c = tx.add_node(Node::new("Account").with_property("name", PropertyValue::String("C".into()))).await?;
    tx.add_edge(Edge::new(a, b, "TRANSFER").with_property("amount", int(70)))?;
    tx.add_edge(Edge::new(b, c, "TRANSFER").with_property("amount", int(50)))?;
    tx.commit().await?;

    let result = graph.execute_nql(r#"
        find n.name, path_eval("sum") as total
        from (a:Account {name: "A"})-[:TRANSFER]->{1,2}(n:Account)
        where path_eval("sum") > 100
        init "sum = 0"
        gather "sum = sum + edge.amount"
    "#).await?;

    assert_eq!(result.len(), 1);
    assert_eq!(result.rows()[0].get("n.name"), Some(&PropertyValue::String("C".to_string())));
    assert_eq!(result.rows()[0].get("total"), Some(&PropertyValue::Int(120)));

    Ok(())
}

#[tokio::test]
async fn test_path_vm_can_read_target_context() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx.add_node(Node::new("Person").with_property("name", PropertyValue::String("Alice".into()))).await?;
    let b = tx.add_node(Node::new("Person").with_property("name", PropertyValue::String("Bob".into()))).await?;
    let c = tx.add_node(Node::new("Person").with_property("name", PropertyValue::String("Carol".into()))).await?;
    tx.add_edge(Edge::new(a, b, "KNOWS"))?;
    tx.add_edge(Edge::new(b, c, "KNOWS"))?;
    tx.commit().await?;

    let result = graph.execute_nql(r#"
        find n.name, path_eval("last_seen") as last_seen, path_eval("path.depth") as hops
        from (a:Person {name: "Alice"})-[:KNOWS]->{1,2}(n:Person)
        init "last_seen = 'start'"
        gather "last_seen = target.name"
    "#).await?;

    assert_eq!(result.len(), 2);
    assert_eq!(result.rows()[0].get("last_seen"), Some(&PropertyValue::String("Bob".to_string())));
    assert_eq!(result.rows()[0].get("hops"), Some(&PropertyValue::Int(1)));
    assert_eq!(result.rows()[1].get("last_seen"), Some(&PropertyValue::String("Carol".to_string())));
    assert_eq!(result.rows()[1].get("hops"), Some(&PropertyValue::Int(2)));

    Ok(())
}

#[tokio::test]
async fn test_path_vm_requires_initialized_variable() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx.add_node(Node::new("Account").with_property("name", PropertyValue::String("A".into()))).await?;
    let b = tx.add_node(Node::new("Account").with_property("name", PropertyValue::String("B".into()))).await?;
    tx.add_edge(Edge::new(a, b, "TRANSFER").with_property("amount", int(10)))?;
    tx.commit().await?;

    let err = graph.execute_nql(r#"
        find b.name, path_eval("sum") as total
        from (a:Account {name: "A"})-[:TRANSFER]->(b:Account)
        gather "sum = sum + edge.amount"
    "#).await.unwrap_err();

    assert!(format!("{}", err).contains("VM variable 'sum' is not defined"));
    Ok(())
}

#[tokio::test]
async fn test_path_vm_reports_missing_edge_property() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx.add_node(Node::new("Account").with_property("name", PropertyValue::String("A".into()))).await?;
    let b = tx.add_node(Node::new("Account").with_property("name", PropertyValue::String("B".into()))).await?;
    tx.add_edge(Edge::new(a, b, "TRANSFER"))?;
    tx.commit().await?;

    let err = graph.execute_nql(r#"
        find b.name, path_eval("sum") as total
        from (a:Account {name: "A"})-[:TRANSFER]->(b:Account)
        init "sum = 0"
        gather "sum = sum + edge.amount"
    "#).await.unwrap_err();

    assert!(format!("{}", err).contains("VM property 'edge.amount' is missing"));
    Ok(())
}

#[tokio::test]
async fn test_path_vm_rejects_multi_pattern_queries() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let err = graph.execute_nql(r#"
        find a.id, b.id
        from (a:Account)-[:TRANSFER]->(b:Account), (c:Account)-[:TRANSFER]->(d:Account)
        init "sum = 0"
        gather "sum = sum + edge.amount"
    "#).await.unwrap_err();

    let message = format!("{}", err);
    assert!(message.contains("INIT/GATHER") || message.contains("multi-pattern"));
    Ok(())
}

#[tokio::test]
async fn test_path_vm_rejects_order_by_path_eval() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let err = graph.execute_nql(r#"
        find b.name, path_eval("sum") as total
        from (a:Account)-[:TRANSFER]->(b:Account)
        init "sum = 0"
        gather "sum = sum + edge.amount"
        order by path_eval("sum")
    "#).await.unwrap_err();

    assert!(format!("{}", err).contains("path_eval(\"...\") is not supported in ORDER BY"));
    Ok(())
}

#[tokio::test]
async fn test_path_vm_boolean_assignment_from_comparison() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx.add_node(Node::new("Company").with_property("name", PropertyValue::String("A".into()))).await?;
    let b = tx.add_node(Node::new("Company").with_property("name", PropertyValue::String("B".into()))).await?;
    let c = tx.add_node(Node::new("Company").with_property("name", PropertyValue::String("C".into()))).await?;
    tx.add_edge(Edge::new(a, b, "OWNS").with_property("risk_score", int(20)))?;
    tx.add_edge(Edge::new(b, c, "OWNS").with_property("risk_score", int(90)))?;
    tx.commit().await?;

    let result = graph.execute_nql(r#"
        find n.name, path_eval("risky") as risky
        from (a:Company {name: "A"})-[:OWNS]->{1,2}(n:Company)
        init "risky = false"
        gather "risky = edge.risk_score > 80"
    "#).await?;

    assert_eq!(result.len(), 2);
    assert_eq!(result.rows()[0].get("risky"), Some(&PropertyValue::Bool(false)));
    assert_eq!(result.rows()[1].get("risky"), Some(&PropertyValue::Bool(true)));

    Ok(())
}

#[tokio::test]
async fn test_path_vm_boolean_or_accumulates_across_hops() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx.add_node(Node::new("Company").with_property("name", PropertyValue::String("A".into()))).await?;
    let b = tx.add_node(Node::new("Company").with_property("name", PropertyValue::String("B".into()))).await?;
    let c = tx.add_node(Node::new("Company").with_property("name", PropertyValue::String("C".into()))).await?;
    tx.add_edge(Edge::new(a, b, "OWNS").with_property("risk_score", int(20)))?;
    tx.add_edge(Edge::new(b, c, "OWNS").with_property("risk_score", int(90)))?;
    tx.commit().await?;

    let result = graph.execute_nql(r#"
        find n.name, path_eval("risky") as risky
        from (a:Company {name: "A"})-[:OWNS]->{1,2}(n:Company)
        init "risky = false"
        gather "risky = risky or edge.risk_score > 80"
    "#).await?;

    assert_eq!(result.len(), 2);
    assert_eq!(result.rows()[0].get("risky"), Some(&PropertyValue::Bool(false)));
    assert_eq!(result.rows()[1].get("risky"), Some(&PropertyValue::Bool(true)));

    Ok(())
}

#[tokio::test]
async fn test_path_vm_boolean_and_where_filter() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx.add_node(Node::new("Company").with_property("name", PropertyValue::String("A".into()))).await?;
    let b = tx.add_node(Node::new("Company").with_property("name", PropertyValue::String("B".into()))).await?;
    let c = tx.add_node(Node::new("Company").with_property("name", PropertyValue::String("C".into()))).await?;
    tx.add_edge(Edge::new(a, b, "OWNS").with_property("risk_score", int(20)).with_property("amount", int(50)))?;
    tx.add_edge(Edge::new(b, c, "OWNS").with_property("risk_score", int(90)).with_property("amount", int(120)))?;
    tx.commit().await?;

    let result = graph.execute_nql(r#"
        find n.name, path_eval("risky") as risky
        from (a:Company {name: "A"})-[:OWNS]->{1,2}(n:Company)
        where path_eval("risky") = true and path.depth > 1
        init "risky = false"
        gather "risky = risky or edge.risk_score > 80"
    "#).await?;

    assert_eq!(result.len(), 1);
    assert_eq!(result.rows()[0].get("n.name"), Some(&PropertyValue::String("C".to_string())));
    assert_eq!(result.rows()[0].get("risky"), Some(&PropertyValue::Bool(true)));

    Ok(())
}

#[tokio::test]
async fn test_path_vm_unary_not_forms_are_equivalent() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx.add_node(Node::new("Company").with_property("name", PropertyValue::String("A".into()))).await?;
    let b = tx.add_node(Node::new("Company").with_property("name", PropertyValue::String("B".into()))).await?;
    tx.add_edge(Edge::new(a, b, "OWNS").with_property("risk_score", int(90)))?;
    tx.commit().await?;

    let bang = graph.execute_nql(r#"
        find n.name
        from (a:Company {name: "A"})-[:OWNS]->(n:Company)
        where path_eval("!risky")
        init "risky = false"
        gather "risky = edge.risk_score > 80"
    "#).await?;

    let keyword = graph.execute_nql(r#"
        find n.name
        from (a:Company {name: "A"})-[:OWNS]->(n:Company)
        where path_eval("not risky")
        init "risky = false"
        gather "risky = edge.risk_score > 80"
    "#).await?;

    assert_eq!(bang.len(), 0);
    assert_eq!(keyword.len(), 0);

    Ok(())
}

#[tokio::test]
async fn test_path_vm_boolean_or_short_circuits_missing_property() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx.add_node(Node::new("Company").with_property("name", PropertyValue::String("A".into()))).await?;
    let b = tx.add_node(Node::new("Company").with_property("name", PropertyValue::String("B".into()))).await?;
    tx.add_edge(Edge::new(a, b, "OWNS"))?;
    tx.commit().await?;

    let result = graph.execute_nql(r#"
        find n.name
        from (a:Company {name: "A"})-[:OWNS]->(n:Company)
        where path_eval("true or edge.risk_score > 80")
    "#).await?;

    assert_eq!(result.len(), 1);
    Ok(())
}

#[tokio::test]
async fn test_path_vm_boolean_and_short_circuits_missing_property() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx.add_node(Node::new("Company").with_property("name", PropertyValue::String("A".into()))).await?;
    let b = tx.add_node(Node::new("Company").with_property("name", PropertyValue::String("B".into()))).await?;
    tx.add_edge(Edge::new(a, b, "OWNS"))?;
    tx.commit().await?;

    let result = graph.execute_nql(r#"
        find n.name
        from (a:Company {name: "A"})-[:OWNS]->(n:Company)
        where path_eval("false and edge.risk_score > 80")
    "#).await?;

    assert_eq!(result.len(), 0);
    Ok(())
}

#[tokio::test]
async fn test_path_vm_boolean_operators_require_bool_operands() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx.add_node(Node::new("Account").with_property("name", PropertyValue::String("A".into()))).await?;
    let b = tx.add_node(Node::new("Account").with_property("name", PropertyValue::String("B".into()))).await?;
    tx.add_edge(Edge::new(a, b, "TRANSFER").with_property("amount", int(10)))?;
    tx.commit().await?;

    let err = graph.execute_nql(r#"
        find b.name, path_eval("sum and true") as invalid
        from (a:Account {name: "A"})-[:TRANSFER]->(b:Account)
        init "sum = 0"
        gather "sum = sum + edge.amount"
    "#).await.unwrap_err();

    assert!(format!("{}", err).contains("VM operator 'and' requires Bool operands"));
    Ok(())
}
