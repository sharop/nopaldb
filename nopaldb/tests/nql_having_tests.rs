// tests/nql_having_tests.rs
//
// Tests for HAVING clause parsing

use nopaldb::query::nql::parser::ast::Statement;
use nopaldb::query::nql::parser::parse;

#[test]
fn test_having_with_count() {
    let query = r#"
        find p.city, count(*) as total
        from (p:Person)
        group by p.city
        having count(*) > 1000
    "#;

    let result = parse(query);
    assert!(
        result.is_ok(),
        "Failed to parse HAVING with count(*): {:?}",
        result.err()
    );

    if let Statement::Query(parsed) = result.unwrap() {
        assert!(parsed.having.is_some(), "HAVING clause not parsed");
        assert!(parsed.group_by.is_some(), "GROUP BY clause not parsed");
    } else {
        panic!("Expected Query statement");
    }
}

#[test]
fn test_having_without_group_by_should_fail_validation() {
    // Note: This test won't fail at parse time, but at validation time
    // The parser allows it, but the validator rejects it
    let query = r#"
        find p.name
        from (p:Person)
        having count(*) > 100
    "#;

    let result = parse(query);
    assert!(
        result.is_ok(),
        "Parser should accept HAVING without GROUP BY"
    );

    // TODO: Test validation separately
    // use nopaldb::query::nql::validator::SemanticValidator;
    // let validator = SemanticValidator::new();
    // let validation_result = validator.validate(&result.unwrap());
    // assert!(validation_result.is_err());
}

#[test]
fn test_having_with_multiple_conditions() {
    let query = r#"
        find p.city, count(*) as total, avg(p.age) as avg_age
        from (p:Person)
        group by p.city
        having count(*) > 100 and avg(p.age) < 30
    "#;

    let result = parse(query);
    assert!(
        result.is_ok(),
        "Failed to parse HAVING with multiple conditions: {:?}",
        result.err()
    );

    if let Statement::Query(parsed) = result.unwrap() {
        assert!(parsed.having.is_some());
    } else {
        panic!("Expected Query statement");
    }
}

#[test]
#[ignore]
fn test_query_without_having() {
    let query = r#"
        find p.city, count(*) as total
        from (p:Person)
        group by p.city
        order by p.city desc
        limit 10
    "#;

    let result = parse(query);
    assert!(
        result.is_ok(),
        "Failed to parse query with ORDER BY: {:?}",
        result.err()
    );

    if let Statement::Query(parsed) = result.unwrap() {
        assert!(
            parsed.having.is_none(),
            "HAVING should be None when not specified"
        );
    } else {
        panic!("Expected Query statement");
    }
}
