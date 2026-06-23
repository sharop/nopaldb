// tests/nql_path_quantifier_test.rs
//
// Integration tests for NQL Path Queries F1: variable-depth quantified traversal.
//
// NQL quantifier syntax (matches current grammar):
//   -[:TYPE]->{n}      exact n hops (arrow_left ~ spec ~ arrow_right ~ quantifier)
//   -[:TYPE]->{n,m}    n to m hops
//   -[:TYPE]->{n,}     unbounded (rejected in F1)

use nopaldb::{Edge, Graph, Node, PropertyValue, Result};

// ---------------------------------------------------------------------------
// Test 1: depth exacto {2} — solo retorna el nodo a exactamente 2 hops
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_exact_depth_2() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Cadena: Alice -[KNOWS]-> Bob -[KNOWS]-> Charlie -[KNOWS]-> Diana
    let mut tx = graph.begin_transaction().await?;
    let alice = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Alice".into())))
        .await?;
    let bob = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Bob".into())))
        .await?;
    let charlie = tx
        .add_node(
            Node::new("Person").with_property("name", PropertyValue::String("Charlie".into())),
        )
        .await?;
    let diana = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Diana".into())))
        .await?;
    tx.add_edge(Edge::new(alice, bob, "KNOWS"))?;
    tx.add_edge(Edge::new(bob, charlie, "KNOWS"))?;
    tx.add_edge(Edge::new(charlie, diana, "KNOWS"))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find b.name
        from (a:Person {name: "Alice"}) -[:KNOWS]->{2} (b:Person)
    "#,
        )
        .await?;

    let names: Vec<_> = result
        .rows()
        .iter()
        .filter_map(|r| r.get("b.name"))
        .collect();

    assert_eq!(
        names.len(),
        1,
        "Expected exactly 1 result at depth 2, got: {:?}",
        names
    );
    assert!(
        matches!(names[0], PropertyValue::String(s) if s == "Charlie"),
        "Expected Charlie at depth 2, got: {:?}",
        names[0]
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 2: rango {1,3} — retorna nodos a 1, 2 y 3 hops
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_range_depth_1_to_3() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let alice = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Alice".into())))
        .await?;
    let bob = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Bob".into())))
        .await?;
    let charlie = tx
        .add_node(
            Node::new("Person").with_property("name", PropertyValue::String("Charlie".into())),
        )
        .await?;
    let diana = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Diana".into())))
        .await?;
    tx.add_edge(Edge::new(alice, bob, "KNOWS"))?;
    tx.add_edge(Edge::new(bob, charlie, "KNOWS"))?;
    tx.add_edge(Edge::new(charlie, diana, "KNOWS"))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find b.name
        from (a:Person {name: "Alice"}) -[:KNOWS]->{1,3} (b:Person)
    "#,
        )
        .await?;

    let mut names: Vec<String> = result
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
    names.sort();

    assert_eq!(
        names.len(),
        3,
        "Expected Bob, Charlie, Diana — got: {:?}",
        names
    );
    assert!(names.iter().any(|n| n == "Bob"));
    assert!(names.iter().any(|n| n == "Charlie"));
    assert!(names.iter().any(|n| n == "Diana"));

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 3: seguridad de ciclos — la query termina y no repite nodos en un camino
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_cycle_safety() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Ciclo: Alfa -[KNOWS]-> Beta -[KNOWS]-> Alfa
    let mut tx = graph.begin_transaction().await?;
    let alfa = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Alfa".into())))
        .await?;
    let beta = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Beta".into())))
        .await?;
    tx.add_edge(Edge::new(alfa, beta, "KNOWS"))?;
    tx.add_edge(Edge::new(beta, alfa, "KNOWS"))?;
    tx.commit().await?;

    // Con max=5, la BFS debe terminar (simple-path impide bucles)
    let result = graph
        .execute_nql(
            r#"
        find b.name
        from (a:Person {name: "Alfa"}) -[:KNOWS]->{1,5} (b:Person)
    "#,
        )
        .await?;

    // La prueba principal: la query termina sin timeout y produce un número finito de filas
    assert!(
        result.len() <= 10,
        "Cycle safety: too many rows (possible loop), got: {}",
        result.len()
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 4: filtro de propiedades en arista aplicado en cada hop
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_edge_property_filter_quantified() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a1 = tx
        .add_node(Node::new("Account").with_property("name", PropertyValue::String("A1".into())))
        .await?;
    let a2 = tx
        .add_node(Node::new("Account").with_property("name", PropertyValue::String("A2".into())))
        .await?;
    let a3 = tx
        .add_node(Node::new("Account").with_property("name", PropertyValue::String("A3".into())))
        .await?;
    // Primera arista: risk = "high"
    tx.add_edge(
        Edge::new(a1, a2, "TRANSFER").with_property("risk", PropertyValue::String("high".into())),
    )?;
    // Segunda arista: risk = "low" (debe ser filtrada)
    tx.add_edge(
        Edge::new(a2, a3, "TRANSFER").with_property("risk", PropertyValue::String("low".into())),
    )?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find b.name
        from (a:Account {name: "A1"}) -[:TRANSFER {risk: "high"}]->{1,2} (b:Account)
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
        names.iter().any(|n| n == "A2"),
        "Expected A2 via high-risk edge, got: {:?}",
        names
    );
    assert!(
        !names.iter().any(|n| n == "A3"),
        "A3 must NOT appear (edge has low risk), got: {:?}",
        names
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 5: patrón terminal solo en el nodo final — nodos intermedios ignorados
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_terminal_pattern_only_on_final() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let s = tx
        .add_node(Node::new("Start").with_property("name", PropertyValue::String("S".into())))
        .await?;
    let m = tx
        .add_node(Node::new("Middle").with_property("name", PropertyValue::String("M".into())))
        .await?;
    let e = tx
        .add_node(Node::new("End").with_property("name", PropertyValue::String("E".into())))
        .await?;
    tx.add_edge(Edge::new(s, m, "LINK"))?;
    tx.add_edge(Edge::new(m, e, "LINK"))?;
    tx.commit().await?;

    // El nodo intermedio tiene label Middle — el patrón terminal exige End
    let result = graph
        .execute_nql(
            r#"
        find e.name
        from (s:Start) -[:LINK]->{2} (e:End)
    "#,
        )
        .await?;

    let names: Vec<String> = result
        .rows()
        .iter()
        .filter_map(|r| r.get("e.name"))
        .filter_map(|v| {
            if let PropertyValue::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect();

    assert_eq!(
        names.len(),
        1,
        "Expected End node at depth 2, got: {:?}",
        names
    );
    assert_eq!(names[0], "E");

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 6: cuantificador unbounded {n,} → error explícito
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_unbounded_quantifier_rejected() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("A".into())))
        .await?;
    let b = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("B".into())))
        .await?;
    tx.add_edge(Edge::new(a, b, "KNOWS"))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find b.name
        from (a:Person) -[:KNOWS]->{1,} (b:Person)
    "#,
        )
        .await;

    assert!(result.is_err(), "Unbounded quantifier must return an error");
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("F1") || err.contains("upper bound") || err.contains("Unbounded"),
        "Error must mention F1/upper bound, got: {}",
        err
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 7: variable en relación cuantificada → error explícito
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_quantified_relationship_variable_rejected() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("A".into())))
        .await?;
    let b = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("B".into())))
        .await?;
    tx.add_edge(Edge::new(a, b, "KNOWS"))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find b.name
        from (a:Person) -[r:KNOWS]->{2} (b:Person)
    "#,
        )
        .await;

    assert!(
        result.is_err(),
        "Quantified relationship variable must return an error"
    );
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("variable") || err.contains("F1"),
        "Error must mention variable/F1, got: {}",
        err
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 8: patrón mixto — hop fijo + hop cuantificado en cadena
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_mixed_fixed_and_quantified_hops() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Root -[HAS]-> Alice -[KNOWS]-> Bob -[KNOWS]-> Charlie
    let mut tx = graph.begin_transaction().await?;
    let root = tx
        .add_node(Node::new("Root").with_property("name", PropertyValue::String("Root".into())))
        .await?;
    let alice = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Alice".into())))
        .await?;
    let bob = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Bob".into())))
        .await?;
    let charlie = tx
        .add_node(
            Node::new("Person").with_property("name", PropertyValue::String("Charlie".into())),
        )
        .await?;
    tx.add_edge(Edge::new(root, alice, "HAS"))?;
    tx.add_edge(Edge::new(alice, bob, "KNOWS"))?;
    tx.add_edge(Edge::new(bob, charlie, "KNOWS"))?;
    tx.commit().await?;

    let result = graph
        .execute_nql(
            r#"
        find p.name
        from (r:Root) -[:HAS]-> (a:Person) -[:KNOWS]->{1,2} (p:Person)
    "#,
        )
        .await?;

    let mut names: Vec<String> = result
        .rows()
        .iter()
        .filter_map(|r| r.get("p.name"))
        .filter_map(|v| {
            if let PropertyValue::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect();
    names.sort();

    assert!(
        names.iter().any(|n| n == "Bob"),
        "Expected Bob in mixed pattern, got: {:?}",
        names
    );
    assert!(
        names.iter().any(|n| n == "Charlie"),
        "Expected Charlie in mixed pattern, got: {:?}",
        names
    );
    assert!(
        !names.iter().any(|n| n == "Alice"),
        "Alice is anchor, not result, got: {:?}",
        names
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test 9: cuantificador con min=0 → error explícito (zero-hop no soportado en F1)
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_zero_min_quantifier_rejected() -> Result<()> {
    let graph = Graph::in_memory().await?;

    let mut tx = graph.begin_transaction().await?;
    let a = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("A".into())))
        .await?;
    let b = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("B".into())))
        .await?;
    tx.add_edge(Edge::new(a, b, "KNOWS"))?;
    tx.commit().await?;

    // {0} — exact zero hops
    let r1 = graph
        .execute_nql(
            r#"
        find b.name
        from (a:Person) -[:KNOWS]->{0} (b:Person)
    "#,
        )
        .await;

    assert!(r1.is_err(), "{{0}} must return an error");
    let err1 = format!("{}", r1.unwrap_err());
    assert!(
        err1.contains("Zero-hop") || err1.contains("min") || err1.contains("F1"),
        "Error must mention zero-hop/min/F1, got: {}",
        err1
    );

    // {0,3} — range starting at zero
    let r2 = graph
        .execute_nql(
            r#"
        find b.name
        from (a:Person) -[:KNOWS]->{0,3} (b:Person)
    "#,
        )
        .await;

    assert!(r2.is_err(), "{{0,3}} must return an error");
    let err2 = format!("{}", r2.unwrap_err());
    assert!(
        err2.contains("Zero-hop") || err2.contains("min") || err2.contains("F1"),
        "Error must mention zero-hop/min/F1, got: {}",
        err2
    );

    Ok(())
}
