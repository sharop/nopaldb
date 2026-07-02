// tests/nql_parser_test.rs

use nopaldb::{parse_query, Result};

#[test]
fn test_simple_query() -> Result<()> {
    let query = r#"
        find p.name, p.age
        from (p:Person)
        where p.age > 25
        limit 10
    "#;

    let ast = parse_query(query)?;

    println!("✅ Simple query parsed:");
    println!("   Projections: {}", ast.find.projections.len());
    println!("   Patterns: {}", ast.from.patterns.len());
    println!("   Has filter: {}", ast.filter.is_some());
    println!("   Has limit: {}", ast.limit.is_some());

    assert_eq!(ast.find.projections.len(), 2);
    assert_eq!(ast.from.patterns.len(), 1);
    assert!(ast.filter.is_some());
    assert!(ast.limit.is_some());

    if let Some(limit) = ast.limit {
        assert_eq!(limit.limit, 10);
    }

    Ok(())
}

#[test]
fn test_pattern_matching() -> Result<()> {
    let query = r#"
        find a.name, b.name
        from (a:Person)-[:KNOWS]->(b:Person)
    "#;

    let ast = parse_query(query)?;

    println!("✅ Pattern matching query parsed:");
    println!("   Pattern elements: {}", ast.from.patterns[0].elements.len());

    assert_eq!(ast.from.patterns.len(), 1);
    let pattern = &ast.from.patterns[0];
    assert_eq!(pattern.elements.len(), 3); // node, rel, node

    Ok(())
}

#[test]
fn test_time_travel_query() -> Result<()> {
    let query = r#"
        find n.name, n.age
        from (n:Person)
        at timestamp 1234567890
    "#;

    let ast = parse_query(query)?;

    println!("✅ Time-travel query parsed:");
    if let Some(tt) = &ast.time_travel {
        println!("   Timestamp: {}", tt.timestamp);
    }

    assert!(ast.time_travel.is_some());
    assert_eq!(ast.time_travel.unwrap().timestamp, 1234567890);

    Ok(())
}

#[test]
fn test_wildcard_query() -> Result<()> {
    let query = r#"
        find *
        from (n:Person)
    "#;

    let ast = parse_query(query)?;

    println!("✅ Wildcard query parsed:");
    println!("   Projections: {}", ast.find.projections.len());

    assert_eq!(ast.find.projections.len(), 1);

    Ok(())
}

#[test]
fn test_multiple_patterns() -> Result<()> {
    let query = r#"
        find p.name, c.name
        from (p:Person), (c:Company)
        where p.company_id = c.id
    "#;

    let ast = parse_query(query)?;

    println!("✅ Multiple patterns query parsed:");
    println!("   Number of patterns: {}", ast.from.patterns.len());

    assert_eq!(ast.from.patterns.len(), 2);

    Ok(())
}

#[test]
fn test_complex_where() -> Result<()> {
    let query = r#"
        find p.name
        from (p:Person)
        where p.age > 25
    "#;

    let ast = parse_query(query)?;

    println!("✅ Complex WHERE query parsed:");
    println!("   Has filter: {}", ast.filter.is_some());

    assert!(ast.filter.is_some());

    Ok(())
}

#[test]
fn test_limit_offset() -> Result<()> {
    let query = r#"
        find p.name
        from (p:Person)
        limit 20 offset 10
    "#;

    let ast = parse_query(query)?;

    println!("✅ LIMIT OFFSET query parsed:");
    if let Some(limit) = &ast.limit {
        println!("   Limit: {}", limit.limit);
        println!("   Offset: {:?}", limit.offset);
    }

    assert!(ast.limit.is_some());
    let limit = ast.limit.unwrap();
    assert_eq!(limit.limit, 20);
    assert_eq!(limit.offset, Some(10));

    Ok(())
}

#[test]
fn test_bidirectional_relationship() -> Result<()> {
    let query = r#"
        find a.name, b.name
        from (a:Person)<-[:FRIENDS]->(b:Person)
    "#;

    let ast = parse_query(query)?;

    println!("✅ Bidirectional relationship parsed");

    assert_eq!(ast.from.patterns.len(), 1);

    Ok(())
}

#[test]
fn test_node_with_label_only() -> Result<()> {
    let query = r#"
        find *
        from (:Person)
    "#;

    let ast = parse_query(query)?;

    println!("✅ Node with label only (no variable) parsed");

    assert_eq!(ast.from.patterns.len(), 1);

    Ok(())
}

#[test]
fn test_relationship_without_type() -> Result<()> {
    let query = r#"
        find a.name, b.name
        from (a:Person)-[]->(b)
    "#;

    let ast = parse_query(query)?;

    println!("✅ Relationship without type parsed");

    assert_eq!(ast.from.patterns.len(), 1);

    Ok(())
}

#[test]
fn test_invalid_query_missing_from() {
    let query = r#"
        find p.name
        where p.age > 25
    "#;

    let result = parse_query(query);

    println!("✅ Correctly rejected query missing FROM clause");
    assert!(result.is_err());
}

#[test]
fn test_invalid_query_missing_find() {
    let query = r#"
        from (p:Person)
        where p.age > 25
    "#;

    let result = parse_query(query);

    println!("✅ Correctly rejected query missing FIND clause");
    assert!(result.is_err());
}