// tests/nql_path_reducers_test.rs
//
// Integration tests for NQL Path Queries F3: path_sum, path_min, path_max, path_avg
// over edge properties along quantified and fixed patterns.

use nopaldb::{Edge, Graph, Node, PropertyValue, Result};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn int(v: i64) -> PropertyValue {
    PropertyValue::Int(v)
}
fn float(v: f64) -> PropertyValue {
    PropertyValue::Float(v)
}

fn get_col(
    result: &nopaldb::query::nql::QueryResult,
    row: usize,
    col: &str,
) -> Option<PropertyValue> {
    result.rows().get(row)?.get(col).cloned()
}

// ---------------------------------------------------------------------------
// Test 1: path_sum sobre path fijo de 2 hops
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_path_sum_fixed_2_hops() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("Account").with_property("name", PropertyValue::String("A".into())))
        .await?;
    let b = tx
        .add_node(Node::new("Account").with_property("name", PropertyValue::String("B".into())))
        .await?;
    let c = tx
        .add_node(Node::new("Account").with_property("name", PropertyValue::String("C".into())))
        .await?;
    tx.add_edge(Edge::new(a, b, "TX").with_property("amount", int(300)))?;
    tx.add_edge(Edge::new(b, c, "TX").with_property("amount", int(200)))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find c.name, path_sum("amount") as total
        from (a:Account {name: "A"}) -[:TX]-> (b:Account) -[:TX]-> (c:Account)
    "#,
        )
        .await?;

    assert_eq!(result.len(), 1);
    assert_eq!(get_col(&result, 0, "total"), Some(int(500)));

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 2: path_sum sobre path cuantificado {1,3}
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_path_sum_quantified() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("Node").with_property("name", PropertyValue::String("A".into())))
        .await?;
    let b = tx
        .add_node(Node::new("Node").with_property("name", PropertyValue::String("B".into())))
        .await?;
    let c = tx
        .add_node(Node::new("Node").with_property("name", PropertyValue::String("C".into())))
        .await?;
    tx.add_edge(Edge::new(a, b, "LINK").with_property("weight", int(10)))?;
    tx.add_edge(Edge::new(b, c, "LINK").with_property("weight", int(20)))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find n.name, path_sum("weight") as total
        from (a:Node {name: "A"}) -[:LINK]->{1,2} (n:Node)
        order by n.name
    "#,
        )
        .await?;

    // depth 1: B → weight=10; depth 2: C → weight=10+20=30
    assert_eq!(result.len(), 2);

    let totals: Vec<PropertyValue> = result
        .rows()
        .iter()
        .filter_map(|r| r.get("total").cloned())
        .collect();

    assert!(totals.contains(&int(10)), "B should have total 10");
    assert!(totals.contains(&int(30)), "C should have total 30");

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 3: path_min sobre todas las aristas del binding
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_path_min() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("R").with_property("name", PropertyValue::String("A".into())))
        .await?;
    let b = tx
        .add_node(Node::new("R").with_property("name", PropertyValue::String("B".into())))
        .await?;
    let c = tx
        .add_node(Node::new("R").with_property("name", PropertyValue::String("C".into())))
        .await?;
    tx.add_edge(Edge::new(a, b, "ROAD").with_property("capacity", int(50)))?;
    tx.add_edge(Edge::new(b, c, "ROAD").with_property("capacity", int(30)))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find c.name, path_min("capacity") as bottleneck
        from (a:R {name: "A"}) -[:ROAD]-> (b:R) -[:ROAD]-> (c:R)
    "#,
        )
        .await?;

    assert_eq!(result.len(), 1);
    assert_eq!(get_col(&result, 0, "bottleneck"), Some(int(30)));

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 4: path_max
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_path_max() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("N").with_property("name", PropertyValue::String("A".into())))
        .await?;
    let b = tx
        .add_node(Node::new("N").with_property("name", PropertyValue::String("B".into())))
        .await?;
    let c = tx
        .add_node(Node::new("N").with_property("name", PropertyValue::String("C".into())))
        .await?;
    tx.add_edge(Edge::new(a, b, "E").with_property("speed", int(60)))?;
    tx.add_edge(Edge::new(b, c, "E").with_property("speed", int(90)))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find c.name, path_max("speed") as peak
        from (a:N {name: "A"}) -[:E]-> (b:N) -[:E]-> (c:N)
    "#,
        )
        .await?;

    assert_eq!(result.len(), 1);
    assert_eq!(get_col(&result, 0, "peak"), Some(int(90)));

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 5: path_avg devuelve Float
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_path_avg() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("S").with_property("name", PropertyValue::String("A".into())))
        .await?;
    let b = tx
        .add_node(Node::new("S").with_property("name", PropertyValue::String("B".into())))
        .await?;
    let c = tx
        .add_node(Node::new("S").with_property("name", PropertyValue::String("C".into())))
        .await?;
    tx.add_edge(Edge::new(a, b, "HOP").with_property("latency", int(10)))?;
    tx.add_edge(Edge::new(b, c, "HOP").with_property("latency", int(20)))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find c.name, path_avg("latency") as avg_lat
        from (a:S {name: "A"}) -[:HOP]-> (b:S) -[:HOP]-> (c:S)
    "#,
        )
        .await?;

    assert_eq!(result.len(), 1);
    assert_eq!(get_col(&result, 0, "avg_lat"), Some(float(15.0)));

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 6: path_sum en WHERE — filtrar paths por costo total
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_path_sum_in_where() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let hub = tx
        .add_node(Node::new("A").with_property("name", PropertyValue::String("Hub".into())))
        .await?;
    let x = tx
        .add_node(Node::new("A").with_property("name", PropertyValue::String("X".into())))
        .await?;
    let y = tx
        .add_node(Node::new("A").with_property("name", PropertyValue::String("Y".into())))
        .await?;
    let far = tx
        .add_node(Node::new("A").with_property("name", PropertyValue::String("Far".into())))
        .await?;
    // path Hub→X: total 150 000; path Hub→Y: total 50 000
    tx.add_edge(Edge::new(hub, x, "FLOW").with_property("amount", int(150_000)))?;
    tx.add_edge(Edge::new(hub, y, "FLOW").with_property("amount", int(50_000)))?;
    // path Hub→X→Far: total 150 000 + 10 000 = 160 000
    tx.add_edge(Edge::new(x, far, "FLOW").with_property("amount", int(10_000)))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find b.name, path_sum("amount") as total
        from (a:A {name: "Hub"}) -[:FLOW]->{1,2} (b:A)
        where path_sum("amount") > 100000
    "#,
        )
        .await?;

    let names: Vec<String> = result
        .rows()
        .iter()
        .filter_map(|r| r.get("b.name"))
        .filter_map(|v| {
            if let PropertyValue::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect();

    assert!(
        names.iter().any(|n| n == "X"),
        "X (total 150k) should pass filter"
    );
    assert!(
        names.iter().any(|n| n == "Far"),
        "Far (total 160k) should pass filter"
    );
    assert!(
        !names.iter().any(|n| n == "Y"),
        "Y (total 50k) should be filtered out"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 7: path_min en WHERE (bottleneck de capacidad)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_path_min_in_where() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let src = tx
        .add_node(Node::new("R").with_property("name", PropertyValue::String("Src".into())))
        .await?;
    let a = tx
        .add_node(Node::new("R").with_property("name", PropertyValue::String("A".into())))
        .await?;
    let b = tx
        .add_node(Node::new("R").with_property("name", PropertyValue::String("B".into())))
        .await?;
    // path Src→A: min capacity = 80; path Src→B: min capacity = 5 (bottleneck)
    tx.add_edge(Edge::new(src, a, "PIPE").with_property("cap", int(80)))?;
    tx.add_edge(Edge::new(src, b, "PIPE").with_property("cap", int(5)))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find b.name
        from (s:R {name: "Src"}) -[:PIPE]->{1,1} (b:R)
        where path_min("cap") >= 10
    "#,
        )
        .await?;

    let names: Vec<String> = result
        .rows()
        .iter()
        .filter_map(|r| r.get("b.name"))
        .filter_map(|v| {
            if let PropertyValue::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect();

    assert!(names.iter().any(|n| n == "A"), "A (cap 80) should pass");
    assert!(
        !names.iter().any(|n| n == "B"),
        "B (cap 5) should be filtered"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 8: patrón mixto fijo + cuantificado — reducer usa TODAS las aristas
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_reducer_mixed_fixed_and_quantified() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Root -[HAS, amount=100]-> A -[TX, amount=200]-> B -[TX, amount=300]-> C
    let mut tx = graph.begin_transaction().await?;
    let root = tx
        .add_node(Node::new("Root").with_property("name", PropertyValue::String("Root".into())))
        .await?;
    let a = tx
        .add_node(Node::new("Node").with_property("name", PropertyValue::String("A".into())))
        .await?;
    let b = tx
        .add_node(Node::new("Node").with_property("name", PropertyValue::String("B".into())))
        .await?;
    let c = tx
        .add_node(Node::new("Node").with_property("name", PropertyValue::String("C".into())))
        .await?;
    tx.add_edge(Edge::new(root, a, "HAS").with_property("amount", int(100)))?;
    tx.add_edge(Edge::new(a, b, "TX").with_property("amount", int(200)))?;
    tx.add_edge(Edge::new(b, c, "TX").with_property("amount", int(300)))?;
    tx.commit().await?;

    // Fijo HAS + cuantificado TX{1,2} — el reducer debe sumar TODAS las aristas del binding
    let result = graph
        .execute_nql(
            r#"
        find n.name, path_sum("amount") as total
        from (r:Root) -[:HAS]-> (a:Node) -[:TX]->{1,2} (n:Node)
        order by n.name
    "#,
        )
        .await?;

    let mut name_total: Vec<(String, i64)> = result
        .rows()
        .iter()
        .filter_map(|r| {
            let name = if let Some(PropertyValue::String(s)) = r.get("n.name") {
                s.clone()
            } else {
                return None;
            };
            let total = if let Some(PropertyValue::Int(i)) = r.get("total") {
                *i
            } else {
                return None;
            };
            Some((name, total))
        })
        .collect();
    name_total.sort();

    // B via Root→A→B: HAS(100) + TX(200) = 300
    // C via Root→A→B→C: HAS(100) + TX(200) + TX(300) = 600
    assert!(
        name_total.iter().any(|(n, t)| n == "B" && *t == 300),
        "B should have total 300, got: {:?}",
        name_total
    );
    assert!(
        name_total.iter().any(|(n, t)| n == "C" && *t == 600),
        "C should have total 600, got: {:?}",
        name_total
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 9: propiedad faltante → error estricto
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_missing_property_fails() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("N").with_property("name", PropertyValue::String("A".into())))
        .await?;
    let b = tx
        .add_node(Node::new("N").with_property("name", PropertyValue::String("B".into())))
        .await?;
    // Arista SIN propiedad "amount"
    tx.add_edge(Edge::new(a, b, "E"))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find b.name, path_sum("amount") as total
        from (a:N {name: "A"}) -[:E]-> (b:N)
    "#,
        )
        .await;

    assert!(result.is_err(), "Missing property must return error");
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("missing") || err.contains("amount"),
        "Error must mention missing/amount, got: {}",
        err
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 10: propiedad con tipo incorrecto → error estricto
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_non_numeric_property_fails() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("N").with_property("name", PropertyValue::String("A".into())))
        .await?;
    let b = tx
        .add_node(Node::new("N").with_property("name", PropertyValue::String("B".into())))
        .await?;
    // Arista con "amount" como string, no numérico
    tx.add_edge(Edge::new(a, b, "E").with_property("amount", PropertyValue::String("ten".into())))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find b.name, path_sum("amount") as total
        from (a:N {name: "A"}) -[:E]-> (b:N)
    "#,
        )
        .await;

    assert!(result.is_err(), "Non-numeric property must return error");
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("not numeric") || err.contains("amount"),
        "Error must mention not numeric/amount, got: {}",
        err
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 11: aridad inválida → error
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_invalid_arity_fails() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("N").with_property("name", PropertyValue::String("A".into())))
        .await?;
    let b = tx
        .add_node(Node::new("N").with_property("name", PropertyValue::String("B".into())))
        .await?;
    tx.add_edge(Edge::new(a, b, "E").with_property("amount", int(100)))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find b.name, path_sum("amount", "fee") as total
        from (a:N {name: "A"}) -[:E]-> (b:N)
    "#,
        )
        .await;

    assert!(result.is_err(), "Wrong arity must return error");

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 12: reducer en ORDER BY → rechazado por validator
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_reducer_in_order_by_rejected() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("N").with_property("name", PropertyValue::String("A".into())))
        .await?;
    let b = tx
        .add_node(Node::new("N").with_property("name", PropertyValue::String("B".into())))
        .await?;
    tx.add_edge(Edge::new(a, b, "E").with_property("amount", int(100)))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find b.name, path_sum("amount") as total
        from (a:N {name: "A"}) -[:E]-> (b:N)
        order by path_sum("amount")
    "#,
        )
        .await;

    assert!(result.is_err(), "path reducer in ORDER BY must be rejected");
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("ORDER BY") || err.contains("F3"),
        "Error must mention ORDER BY/F3, got: {}",
        err
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 13: reducer y path.depth en la misma query (F2 + F3 combinados)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_reducer_and_path_depth_combined() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("N").with_property("name", PropertyValue::String("A".into())))
        .await?;
    let b = tx
        .add_node(Node::new("N").with_property("name", PropertyValue::String("B".into())))
        .await?;
    let c = tx
        .add_node(Node::new("N").with_property("name", PropertyValue::String("C".into())))
        .await?;
    tx.add_edge(Edge::new(a, b, "E").with_property("weight", int(5)))?;
    tx.add_edge(Edge::new(b, c, "E").with_property("weight", int(7)))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find n.name, path.depth as depth, path_sum("weight") as total
        from (a:N {name: "A"}) -[:E]->{1,2} (n:N)
        order by n.name
    "#,
        )
        .await?;

    assert_eq!(result.len(), 2);

    let b_row = result
        .rows()
        .iter()
        .find(|r| r.get("n.name") == Some(&PropertyValue::String("B".into())));
    let c_row = result
        .rows()
        .iter()
        .find(|r| r.get("n.name") == Some(&PropertyValue::String("C".into())));

    assert!(b_row.is_some());
    assert_eq!(b_row.unwrap().get("depth"), Some(&PropertyValue::Int(1)));
    assert_eq!(b_row.unwrap().get("total"), Some(&PropertyValue::Int(5)));

    assert!(c_row.is_some());
    assert_eq!(c_row.unwrap().get("depth"), Some(&PropertyValue::Int(2)));
    assert_eq!(c_row.unwrap().get("total"), Some(&PropertyValue::Int(12)));

    Ok(())
}
