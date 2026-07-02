// tests/nql_create_tests.rs
//
// Tests for ADD (CREATE) statement parsing

use nopaldb::query::nql::parser::parse;
use nopaldb::query::nql::parser::ast::{Statement, PatternElement};

#[test]
fn test_add_simple_node() {
    let query = r#"add (p:Person {name: "Alice", age: 30})"#;
    let result = parse(query);
    
    assert!(result.is_ok(), "Failed to parse ADD: {:?}", result.err());
    
    match result.unwrap() {
        Statement::Add(add) => {
            assert!(!add.pattern.elements.is_empty());
            
            // Verify node has properties
            if let PatternElement::Node(node) = &add.pattern.elements[0] {
                assert_eq!(node.label, Some("Person".to_string()));
                assert_eq!(node.variable, Some("p".to_string()));
                assert!(!node.properties.is_empty(), "Properties should be parsed");
            } else {
                panic!("Expected Node element");
            }
        }
        _ => panic!("Expected Add statement"),
    }
}

#[test]
fn test_add_node_without_properties() {
    let query = r#"add (p:Person)"#;
    let result = parse(query);
    
    assert!(result.is_ok(), "Failed to parse ADD: {:?}", result.err());
    assert!(matches!(result.unwrap(), Statement::Add(_)));
}

#[test]
fn test_add_node_with_label_only() {
    let query = r#"add (:Product)"#;
    let result = parse(query);
    
    assert!(result.is_ok(), "Failed to parse ADD with label only: {:?}", result.err());
}

#[test]
fn test_add_multiple_properties() {
    let query = r#"add (u:User {name: "Bob", email: "bob@test.com", active: true, score: 100})"#;
    let result = parse(query);
    
    assert!(result.is_ok(), "Failed to parse ADD with multiple properties: {:?}", result.err());
    
    if let Statement::Add(add) = result.unwrap() {
        if let PatternElement::Node(node) = &add.pattern.elements[0] {
            assert_eq!(node.properties.len(), 4, "Should have 4 properties");
        }
    }
}
