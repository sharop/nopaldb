// tests/nql_update_tests.rs
//
// Tests for UPDATE statement parsing

use nopaldb::query::nql::parser::ast::Statement;
use nopaldb::query::nql::parser::parse;

#[test]
fn test_update_simple() {
    let query = r#"update (u:User) set u.verified = true where u.email_confirmed = true"#;
    let result = parse(query);

    assert!(result.is_ok(), "Failed to parse UPDATE: {:?}", result.err());

    match result.unwrap() {
        Statement::Update(upd) => {
            assert!(!upd.assignments.is_empty(), "Should have assignments");
            assert!(upd.filter.is_some(), "Should have WHERE clause");
        }
        _ => panic!("Expected Update statement"),
    }
}

#[test]
fn test_update_multiple_assignments() {
    let query = r#"update (p:Person) set p.status = "active", p.updated_at = 1234567890"#;
    let result = parse(query);

    assert!(
        result.is_ok(),
        "Failed to parse UPDATE with multiple assignments: {:?}",
        result.err()
    );

    if let Statement::Update(upd) = result.unwrap() {
        assert_eq!(upd.assignments.len(), 2, "Should have 2 assignments");
    }
}

#[test]
fn test_update_with_limit() {
    let query = r#"update (u:User) set u.migrated = true where u.version = 1 limit 1000"#;
    let result = parse(query);

    assert!(
        result.is_ok(),
        "Failed to parse UPDATE with LIMIT: {:?}",
        result.err()
    );

    if let Statement::Update(upd) = result.unwrap() {
        assert!(upd.limit.is_some());
        assert_eq!(upd.limit.unwrap().limit, 1000);
    }
}

#[test]
fn test_update_without_where() {
    // Should parse but be High danger
    let query = r#"update (n:Node) set n.version = 2"#;
    let result = parse(query);

    assert!(
        result.is_ok(),
        "Failed to parse UPDATE without WHERE: {:?}",
        result.err()
    );

    if let Statement::Update(upd) = result.unwrap() {
        assert!(upd.filter.is_none());
        assert_eq!(
            upd.danger_level(),
            nopaldb::query::nql::parser::ast::DangerLevel::High
        );
    }
}

#[test]
fn test_update_assignment_structure() {
    let query = r#"update (p:Person) set p.age = 30 where p.name = "Alice""#;
    let result = parse(query);

    assert!(result.is_ok());

    if let Statement::Update(upd) = result.unwrap() {
        let assignment = &upd.assignments[0];
        assert_eq!(assignment.variable, "p");
        assert_eq!(assignment.property, "age");
    }
}
