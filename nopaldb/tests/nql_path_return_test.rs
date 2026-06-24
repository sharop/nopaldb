// tests/nql_path_return_test.rs
//
// Integration tests for NQL Path Queries F4-C:
//   return "..." clause, path.result, path.state, path.start, path.end

use nopaldb::{Edge, Graph, Node, PropertyValue, Result};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn int(v: i64) -> PropertyValue {
    PropertyValue::Int(v)
}
#[allow(dead_code)]
fn float(v: f64) -> PropertyValue {
    PropertyValue::Float(v)
}
fn bool_val(v: bool) -> PropertyValue {
    PropertyValue::Bool(v)
}
fn str_val(s: &str) -> PropertyValue {
    PropertyValue::String(s.to_string())
}

fn get_col(
    result: &nopaldb::query::nql::QueryResult,
    row: usize,
    col: &str,
) -> Option<PropertyValue> {
    result.rows().get(row)?.get(col).cloned()
}

fn get_object_field(val: &PropertyValue, field: &str) -> Option<PropertyValue> {
    if let PropertyValue::Object(entries) = val {
        entries
            .iter()
            .find(|(k, _)| k == field)
            .map(|(_, v)| v.clone())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Test 1: path.result projectable in FIND
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_path_result_in_find() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("Account").with_property("name", str_val("A")))
        .await?;
    let b = tx
        .add_node(Node::new("Account").with_property("name", str_val("B")))
        .await?;
    let c = tx
        .add_node(Node::new("Account").with_property("name", str_val("C")))
        .await?;
    tx.add_edge(Edge::new(a, b, "TX").with_property("amount", int(100)))?;
    tx.add_edge(Edge::new(b, c, "TX").with_property("amount", int(50)))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find c.name, path.result as score
        from (a:Account {name: "A"})-[:TX]->{1,2}(c:Account)
        init "sum = 0"
        gather "sum = sum + edge.amount"
        return "sum"
    "#,
        )
        .await?;

    // Path A->B has sum=100; path A->B->C has sum=150
    let mut scores: Vec<i64> = result
        .rows()
        .iter()
        .filter_map(|r| r.get("score"))
        .filter_map(|v| {
            if let PropertyValue::Int(n) = v {
                Some(*n)
            } else {
                None
            }
        })
        .collect();
    scores.sort_unstable();

    assert!(
        scores.contains(&100),
        "score 100 (A->B) expected, got {:?}",
        scores
    );
    assert!(
        scores.contains(&150),
        "score 150 (A->B->C) expected, got {:?}",
        scores
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 2: path.result in WHERE filters correctly
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_path_result_in_where() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("Account").with_property("name", str_val("A")))
        .await?;
    let b = tx
        .add_node(Node::new("Account").with_property("name", str_val("B")))
        .await?;
    let c = tx
        .add_node(Node::new("Account").with_property("name", str_val("C")))
        .await?;
    tx.add_edge(Edge::new(a, b, "TX").with_property("amount", int(200)))?;
    tx.add_edge(Edge::new(b, c, "TX").with_property("amount", int(50)))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find c.name, path.result as total
        from (a:Account {name: "A"})-[:TX]->{1,2}(c:Account)
        where path.result > 100
        init "sum = 0"
        gather "sum = sum + edge.amount"
        return "sum"
    "#,
        )
        .await?;

    // Only paths with sum > 100 pass: A->B (sum=200) and A->B->C (sum=250)
    assert!(!result.is_empty(), "expected at least one result");
    for row in result.rows() {
        let total = row.get("total").expect("total column must exist");
        match total {
            PropertyValue::Int(n) => assert!(*n > 100, "filter should remove sums <= 100"),
            PropertyValue::Float(f) => assert!(*f > 100.0),
            _ => panic!("unexpected type for total"),
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 3: path.state projectable in FIND contains VM variables as Object
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_path_state_in_find() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("Account").with_property("name", str_val("A")))
        .await?;
    let b = tx
        .add_node(Node::new("Account").with_property("name", str_val("B")))
        .await?;
    tx.add_edge(Edge::new(a, b, "TX").with_property("amount", int(42)))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find b.name, path.state as final_state
        from (a:Account {name: "A"})-[:TX]->(b:Account)
        init "total = 0"
        gather "total = total + edge.amount"
        return "total"
    "#,
        )
        .await?;

    assert_eq!(result.len(), 1);
    let state_val = get_col(&result, 0, "final_state").expect("final_state column must exist");

    // state should be an Object containing "total"
    let total_from_state = get_object_field(&state_val, "total");
    assert_eq!(total_from_state, Some(int(42)), "state.total should be 42");

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 4: path.start and path.end projectable in FIND
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_path_start_end_in_find() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("Wallet").with_property("name", str_val("Alice")))
        .await?;
    let b = tx
        .add_node(Node::new("Wallet").with_property("name", str_val("Bob")))
        .await?;
    tx.add_edge(Edge::new(a, b, "SEND").with_property("amount", int(1)))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find path.start as s, path.end as e
        from (a:Wallet {name: "Alice"})-[:SEND]->(b:Wallet)
        init "x = 0"
        gather "x = 1"
        return "x"
    "#,
        )
        .await?;

    assert_eq!(result.len(), 1);

    let start_val = get_col(&result, 0, "s").expect("s column must exist");
    let end_val = get_col(&result, 0, "e").expect("e column must exist");

    // start should be an Object with label "Wallet"
    let start_label = get_object_field(&start_val, "label");
    assert_eq!(start_label, Some(str_val("Wallet")));

    // end should be an Object with label "Wallet"
    let end_label = get_object_field(&end_val, "label");
    assert_eq!(end_label, Some(str_val("Wallet")));

    // start and end should have id fields
    assert!(get_object_field(&start_val, "id").is_some());
    assert!(get_object_field(&end_val, "id").is_some());

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 5: return evaluated once per path — different paths produce different results
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_return_evaluated_once_per_path() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let hub = tx
        .add_node(Node::new("Hub").with_property("name", str_val("HUB")))
        .await?;
    let p1 = tx
        .add_node(Node::new("Peer").with_property("name", str_val("P1")))
        .await?;
    let p2 = tx
        .add_node(Node::new("Peer").with_property("name", str_val("P2")))
        .await?;
    tx.add_edge(Edge::new(hub, p1, "LINK").with_property("weight", int(10)))?;
    tx.add_edge(Edge::new(hub, p2, "LINK").with_property("weight", int(30)))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find p.name, path.result as w
        from (h:Hub {name: "HUB"})-[:LINK]->(p:Peer)
        init "total = 0"
        gather "total = total + edge.weight"
        return "total"
    "#,
        )
        .await?;

    assert_eq!(result.len(), 2, "should have 2 paths");
    let mut weights: Vec<i64> = result
        .rows()
        .iter()
        .filter_map(|r| r.get("w"))
        .filter_map(|v| {
            if let PropertyValue::Int(n) = v {
                Some(*n)
            } else {
                None
            }
        })
        .collect();
    weights.sort_unstable();
    assert_eq!(
        weights,
        vec![10, 30],
        "each path should have its own result"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 6: return produces boolean result, filterable in WHERE
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_return_boolean_result() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("Node").with_property("name", str_val("A")))
        .await?;
    let b = tx
        .add_node(Node::new("Node").with_property("name", str_val("B")))
        .await?;
    let c = tx
        .add_node(Node::new("Node").with_property("name", str_val("C")))
        .await?;
    tx.add_edge(Edge::new(a, b, "EDGE").with_property("ok", bool_val(true)))?;
    tx.add_edge(Edge::new(a, c, "EDGE").with_property("ok", bool_val(false)))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find n.name
        from (a:Node {name: "A"})-[:EDGE]->(n:Node)
        where path.result = true
        init "safe = true"
        gather "safe = safe and edge.ok = true"
        return "safe"
    "#,
        )
        .await?;

    assert_eq!(result.len(), 1, "only one path has safe=true");
    assert_eq!(get_col(&result, 0, "n.name"), Some(str_val("B")));

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 7: return numeric result with path.depth multiplication
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_return_numeric_with_depth() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("N").with_property("name", str_val("A")))
        .await?;
    let b = tx
        .add_node(Node::new("N").with_property("name", str_val("B")))
        .await?;
    tx.add_edge(Edge::new(a, b, "E").with_property("v", int(10)))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find b.name, path.result as score
        from (a:N {name: "A"})-[:E]->(b:N)
        init "sum = 0"
        gather "sum = sum + edge.v"
        return "sum * path.depth"
    "#,
        )
        .await?;

    assert_eq!(result.len(), 1);
    // sum=10, depth=1 → result=10
    assert_eq!(get_col(&result, 0, "score"), Some(int(10)));

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 8: path.result without RETURN fails with SemanticError
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_path_result_without_return_fails() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("N").with_property("name", str_val("A")))
        .await?;
    let b = tx
        .add_node(Node::new("N").with_property("name", str_val("B")))
        .await?;
    tx.add_edge(Edge::new(a, b, "E").with_property("v", int(1)))?;
    tx.commit().await?;

    let err = graph
        .execute_nql(
            r#"
        find b.name, path.result as r
        from (a:N {name: "A"})-[:E]->(b:N)
    "#,
        )
        .await;

    assert!(err.is_err());
    let msg = err.unwrap_err().to_string();
    assert!(
        msg.contains("path.result requires a RETURN"),
        "unexpected: {}",
        msg
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 9: path.state in WHERE is rejected with SemanticError
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_path_state_in_where_fails() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("N").with_property("name", str_val("A")))
        .await?;
    let b = tx
        .add_node(Node::new("N").with_property("name", str_val("B")))
        .await?;
    tx.add_edge(Edge::new(a, b, "E").with_property("v", int(1)))?;
    tx.commit().await?;

    let err = graph
        .execute_nql(
            r#"
        find b.name
        from (a:N {name: "A"})-[:E]->(b:N)
        where path.state > 0
        init "x = 1"
        gather "x = x + 1"
        return "x"
    "#,
        )
        .await;

    assert!(err.is_err());
    let msg = err.unwrap_err().to_string();
    assert!(
        msg.contains("path.state is not allowed in WHERE"),
        "unexpected: {}",
        msg
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 10: path.start in WHERE is rejected with SemanticError
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_path_start_in_where_fails() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("N").with_property("name", str_val("A")))
        .await?;
    let b = tx
        .add_node(Node::new("N").with_property("name", str_val("B")))
        .await?;
    tx.add_edge(Edge::new(a, b, "E").with_property("v", int(1)))?;
    tx.commit().await?;

    let err = graph
        .execute_nql(
            r#"
        find b.name
        from (a:N {name: "A"})-[:E]->(b:N)
        where path.start = 0
        init "x = 1"
        gather "x = 2"
        return "x"
    "#,
        )
        .await;

    assert!(err.is_err());
    let msg = err.unwrap_err().to_string();
    assert!(
        msg.contains("path.start is not allowed in WHERE"),
        "unexpected: {}",
        msg
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 11: return that produces Object fails with QueryExecutionError
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_return_object_fails() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("N").with_property("name", str_val("A")))
        .await?;
    let b = tx
        .add_node(Node::new("N").with_property("name", str_val("B")))
        .await?;
    tx.add_edge(Edge::new(a, b, "E").with_property("v", int(1)))?;
    tx.commit().await?;

    // path.state is an Object — returning it should fail
    let err = graph
        .execute_nql(
            r#"
        find b.name
        from (a:N {name: "A"})-[:E]->(b:N)
        init "x = 0"
        gather "x = edge.v"
        return "path.state"
    "#,
        )
        .await;

    assert!(err.is_err());
    let msg = err.unwrap_err().to_string();
    assert!(
        msg.contains("scalar"),
        "expected scalar error, got: {}",
        msg
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 12: return with ORDER BY fails with SemanticError
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_return_with_order_by_fails() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("N").with_property("name", str_val("A")))
        .await?;
    let b = tx
        .add_node(Node::new("N").with_property("name", str_val("B")))
        .await?;
    tx.add_edge(Edge::new(a, b, "E").with_property("v", int(1)))?;
    tx.commit().await?;

    let err = graph
        .execute_nql(
            r#"
        find b.name, path.result as r
        from (a:N {name: "A"})-[:E]->(b:N)
        init "x = 0"
        gather "x = edge.v"
        return "x"
        order by b.name
    "#,
        )
        .await;

    assert!(err.is_err());
    let msg = err.unwrap_err().to_string();
    assert!(
        msg.contains("RETURN is not supported with ORDER BY"),
        "unexpected: {}",
        msg
    );

    Ok(())
}
