// tests/nql_algorithm_filter_test.rs
//
// Tests for issue #67 — algorithm functions (degree, pagerank, community)
// must work as filter predicates in WHERE/HAVING and as projections in
// pattern queries with joins.
//
// Bugs covered:
//   1. degree() rejected in WHERE (validator treated algos as aggregations)
//   2. degree() returned NULL when used in projection with pattern join
//   3. degree() rejected in HAVING with GROUP BY (validator parsed `e.` as
//      incomplete column reference)

#![cfg(feature = "algorithms")]

use nopaldb::{Edge, Graph, Node, PropertyValue, Result};

/// Build a small graph: 3 ShellCompanies, 1 Jurisdiction (Harbor Cay).
/// ShellCompany #0 has degree 5 (5 edges in/out total).
/// ShellCompany #1 has degree 3.
/// ShellCompany #2 has degree 2.
async fn setup_graph() -> Result<Graph> {
    let g = Graph::in_memory().await?;
    let mut tx = g.begin_transaction().await?;

    let harbor = tx.add_node(
        Node::new("Jurisdiction").with_property("name", PropertyValue::String("Harbor Cay".into()))
    ).await?;

    let s0 = tx.add_node(
        Node::new("ShellCompany")
            .with_property("name", PropertyValue::String("Acme".into()))
            .with_property("industry", PropertyValue::String("trading".into()))
    ).await?;
    let s1 = tx.add_node(
        Node::new("ShellCompany")
            .with_property("name", PropertyValue::String("Beta".into()))
            .with_property("industry", PropertyValue::String("real estate".into()))
    ).await?;
    let s2 = tx.add_node(
        Node::new("ShellCompany")
            .with_property("name", PropertyValue::String("Gamma".into()))
            .with_property("industry", PropertyValue::String("logistics".into()))
    ).await?;

    // Each ShellCompany registers in Harbor Cay (1 edge each).
    tx.add_edge(Edge::new(s0, harbor, "REGISTERED_IN"))?;
    tx.add_edge(Edge::new(s1, harbor, "REGISTERED_IN"))?;
    tx.add_edge(Edge::new(s2, harbor, "REGISTERED_IN"))?;

    // Add additional edges to vary degrees.
    // s0 will have degree 5 (1 REGISTERED_IN + 4 extras).
    // s1 will have degree 3 (1 + 2 extras).
    // s2 will have degree 2 (1 + 1 extra).
    let alice = tx.add_node(Node::new("Person").with_property("name", PropertyValue::String("Alice".into()))).await?;
    let bob = tx.add_node(Node::new("Person").with_property("name", PropertyValue::String("Bob".into()))).await?;
    let carol = tx.add_node(Node::new("Person").with_property("name", PropertyValue::String("Carol".into()))).await?;

    // s0 ← 4 extras
    tx.add_edge(Edge::new(alice, s0, "OWNS"))?;
    tx.add_edge(Edge::new(bob, s0, "OWNS"))?;
    tx.add_edge(Edge::new(carol, s0, "OWNS"))?;
    tx.add_edge(Edge::new(s0, alice, "PAYS"))?;
    // s1 ← 2 extras
    tx.add_edge(Edge::new(alice, s1, "OWNS"))?;
    tx.add_edge(Edge::new(bob, s1, "OWNS"))?;
    // s2 ← 1 extra
    tx.add_edge(Edge::new(alice, s2, "OWNS"))?;

    tx.commit().await?;
    Ok(g)
}

/// Bug 2 fix: `degree(e)` in projection with a pattern join should return
/// non-null values, one row per match.
#[tokio::test]
async fn test_degree_in_projection_with_pattern_join() -> Result<()> {
    let g = setup_graph().await?;

    let result = g.execute_nql(r#"
        find e.name, e.industry, degree(e) as conexiones
        from (e:ShellCompany) -[:REGISTERED_IN]-> (j:Jurisdiction)
        where j.name = "Harbor Cay"
        order by conexiones desc
    "#).await?;

    println!("Rows: {}", result.len());
    for (i, row) in result.rows().iter().enumerate() {
        println!("  [{}] name={:?} industry={:?} conexiones={:?}",
            i, row.get("e.name"), row.get("e.industry"), row.get("conexiones"));
    }

    // Should return 3 rows (one per ShellCompany registered in Harbor Cay).
    assert_eq!(result.len(), 3, "Expected 3 rows, one per ShellCompany");

    // None of the conexiones values should be NULL.
    for row in result.rows() {
        let conexiones = row.get("conexiones");
        assert!(conexiones.is_some(), "conexiones column missing");
        assert!(
            !matches!(conexiones.unwrap(), PropertyValue::Null),
            "Bug 2: degree(e) returned NULL — conexiones={:?}", conexiones
        );
    }

    // Verify ordering: highest degree first (Acme should be top with degree 5).
    let first = &result.rows()[0];
    assert!(matches!(first.get("e.name"), Some(PropertyValue::String(s)) if s == "Acme"),
        "Expected Acme first by degree, got {:?}", first.get("e.name"));

    Ok(())
}

/// Bug 1 fix: `degree(e)` should be permitted in WHERE clauses.
#[tokio::test]
async fn test_degree_in_where_filter() -> Result<()> {
    let g = setup_graph().await?;

    let result = g.execute_nql(r#"
        find e.name, degree(e) as d
        from (e:ShellCompany) -[:REGISTERED_IN]-> (j:Jurisdiction)
        where j.name = "Harbor Cay" and degree(e) > 3
        order by d desc
    "#).await?;

    println!("WHERE-filtered rows: {}", result.len());
    for row in result.rows() {
        println!("  name={:?} d={:?}", row.get("e.name"), row.get("d"));
    }

    // Only Acme has degree > 3 (degree 5). Beta has 3, Gamma has 2.
    assert_eq!(result.len(), 1, "Expected only Acme (degree 5)");
    let name = result.rows()[0].get("e.name");
    assert!(matches!(name, Some(PropertyValue::String(s)) if s == "Acme"),
        "Expected Acme, got {:?}", name);
    Ok(())
}

/// Bug 1 (single-node): `degree(e)` filtering in WHERE without a pattern join.
/// The pattern path was fixed initially but single-node queries follow a
/// different executor branch that also needs the algorithm-aware filter.
#[tokio::test]
async fn test_degree_in_where_single_node_query() -> Result<()> {
    let g = setup_graph().await?;

    let result = g.execute_nql(r#"
        find e.name from (e:ShellCompany)
        where degree(e) > 3
    "#).await?;

    println!("single-node WHERE rows: {}", result.len());
    for row in result.rows() {
        println!("  name={:?}", row.get("e.name"));
    }
    // Acme has degree 5; the only one > 3.
    assert_eq!(result.len(), 1, "Expected 1 row (Acme, degree 5)");
    Ok(())
}

/// Bug 3 fix: `degree(e)` should be permitted in HAVING when GROUP BY exists.
#[tokio::test]
async fn test_degree_in_having_with_group_by() -> Result<()> {
    let g = setup_graph().await?;

    let result = g.execute_nql(r#"
        find e.name, e.industry, degree(e) as conexiones
        from (e:ShellCompany) -[:REGISTERED_IN]-> (j:Jurisdiction)
        where j.name = "Harbor Cay"
        group by e.name, e.industry
        having degree(e) > 3
        order by conexiones desc
    "#).await?;

    println!("HAVING-filtered rows: {}", result.len());
    for row in result.rows() {
        println!("  name={:?} conexiones={:?}", row.get("e.name"), row.get("conexiones"));
    }
    assert_eq!(result.len(), 1, "Expected only Acme");
    Ok(())
}

/// Validator no longer rejects `degree()` in WHERE — verify via execute_nql.
#[tokio::test]
async fn test_validator_accepts_degree_in_where() -> Result<()> {
    let g = setup_graph().await?;
    let res = g.execute_nql(r#"
        find e.name from (e:ShellCompany)
        where degree(e) > 0
    "#).await;
    assert!(res.is_ok(),
        "Validator should accept degree() in WHERE. Got: {:?}", res.err());
    Ok(())
}

/// Validator no longer rejects `degree()` in HAVING-with-GROUP-BY.
#[tokio::test]
async fn test_validator_accepts_degree_in_having() -> Result<()> {
    let g = setup_graph().await?;
    let res = g.execute_nql(r#"
        find e.name, degree(e) as d
        from (e:ShellCompany)
        group by e.name
        having degree(e) > 0
    "#).await;
    assert!(res.is_ok(),
        "Validator should accept degree() in HAVING with GROUP BY. Got: {:?}", res.err());
    Ok(())
}

/// True aggregations (count) STILL rejected in WHERE — regression guard.
#[tokio::test]
async fn test_validator_still_rejects_count_in_where() -> Result<()> {
    let g = setup_graph().await?;
    let res = g.execute_nql(r#"
        find p.name from (p:Person)
        where count(*) > 1
    "#).await;
    assert!(res.is_err(),
        "Validator should still reject count() in WHERE.");
    let msg = res.err().unwrap().to_string();
    assert!(msg.contains("Aggregation functions not allowed in WHERE")
            || msg.contains("aggregation"),
        "Expected aggregation-blocked error, got: {}", msg);
    Ok(())
}
