// tests/nql_sketch_tests.rs
//
// Tests for SKETCH/COMMIT statement parsing

use nopaldb::query::nql::parser::ast::Statement;
use nopaldb::query::nql::parser::parse;

#[test]
fn test_sketch_delete() {
    let query = r#"
        sketch cleanup =
            delete (u:User)
            where u.last_login < 1640995200
    "#;

    let result = parse(query);
    assert!(
        result.is_ok(),
        "Failed to parse SKETCH with DELETE: {:?}",
        result.err()
    );

    match result.unwrap() {
        Statement::Sketch(sketch) => {
            assert_eq!(sketch.name, "cleanup");
            assert!(matches!(*sketch.operation, Statement::Delete(_)));
        }
        _ => panic!("Expected Sketch statement"),
    }
}

#[test]
fn test_sketch_update() {
    let query = r#"
        sketch verify_users =
            update (u:User)
            set u.verified = true, u.verified_at = 1640995200
            where u.email_confirmed = true
    "#;

    let result = parse(query);
    assert!(
        result.is_ok(),
        "Failed to parse SKETCH with UPDATE: {:?}",
        result.err()
    );

    match result.unwrap() {
        Statement::Sketch(sketch) => {
            assert_eq!(sketch.name, "verify_users");
            assert!(matches!(*sketch.operation, Statement::Update(_)));
        }
        _ => panic!("Expected Sketch statement"),
    }
}

#[test]
fn test_sketch_query() {
    let query = r#"
        sketch top_cities =
            find p.city, count(*) as total
            from (p:Person)
            group by p.city
            having count(*) > 1000
            order by total
            limit 10
    "#;

    let result = parse(query);
    assert!(
        result.is_ok(),
        "Failed to parse SKETCH with Query: {:?}",
        result.err()
    );

    match result.unwrap() {
        Statement::Sketch(sketch) => {
            assert_eq!(sketch.name, "top_cities");
            assert!(matches!(*sketch.operation, Statement::Query(_)));
        }
        _ => panic!("Expected Sketch statement"),
    }
}

#[test]
fn test_commit() {
    let query = "commit cleanup";

    let result = parse(query);
    assert!(result.is_ok(), "Failed to parse COMMIT: {:?}", result.err());

    match result.unwrap() {
        Statement::Commit(commit) => {
            assert_eq!(commit.sketch_name, "cleanup");
        }
        _ => panic!("Expected Commit statement"),
    }
}

#[test]
fn test_direct_query_still_works() {
    let query = r#"
        find p.name, p.age
        from (p:Person)
        where p.age > 25
        limit 10
    "#;

    let result = parse(query);
    assert!(
        result.is_ok(),
        "Direct queries should still work: {:?}",
        result.err()
    );

    match result.unwrap() {
        Statement::Query(_) => {
            // Success
        }
        _ => panic!("Expected Query statement"),
    }
}
