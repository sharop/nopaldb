// tests/nql_delete_tests.rs
//
// Tests for DELETE statement parsing

use nopaldb::query::nql::parser::ast::Statement;
use nopaldb::query::nql::parser::parse;

#[test]
fn test_delete_with_where() {
    let query = r#"delete (u:User) where u.active = false"#;
    let result = parse(query);

    assert!(result.is_ok(), "Failed to parse DELETE: {:?}", result.err());

    match result.unwrap() {
        Statement::Delete(del) => {
            assert!(del.filter.is_some(), "WHERE clause should be present");
            assert!(del.limit.is_none());
        }
        _ => panic!("Expected Delete statement"),
    }
}

#[test]
fn test_delete_with_limit() {
    let query = r#"delete (u:User) where u.inactive = true limit 100"#;
    let result = parse(query);

    assert!(
        result.is_ok(),
        "Failed to parse DELETE with LIMIT: {:?}",
        result.err()
    );

    if let Statement::Delete(del) = result.unwrap() {
        assert!(del.filter.is_some());
        assert!(del.limit.is_some());
        assert_eq!(del.limit.unwrap().limit, 100);
    }
}

#[test]
fn test_delete_without_where() {
    // This should parse but generate a High danger warning
    let query = r#"delete (n:Node)"#;
    let result = parse(query);

    assert!(
        result.is_ok(),
        "Failed to parse DELETE without WHERE: {:?}",
        result.err()
    );

    if let Statement::Delete(del) = result.unwrap() {
        assert!(del.filter.is_none());
        // Danger level should be High
        assert_eq!(
            del.danger_level(),
            nopaldb::query::nql::parser::ast::DangerLevel::High
        );
    }
}

#[test]
fn test_delete_comparison_operators() {
    let query = r#"delete (p:Product) where p.stock <= 0"#;
    let result = parse(query);

    assert!(
        result.is_ok(),
        "Failed to parse DELETE with <= operator: {:?}",
        result.err()
    );
}
