// tests/schema_test.rs

use nopaldb::{Edge, Graph, Node, PropertyValue};
use std::collections::HashMap;

#[tokio::test]
async fn test_get_labels() {
    let graph = Graph::in_memory().await.unwrap();
    let mut tx = graph.begin_transaction().await.unwrap();

    tx.add_node(Node::new("Person")
        .with_property("name",PropertyValue::String("Alice".into())))
        .await.unwrap();

    tx.add_node(Node::new("Entity")
        .with_property("name",PropertyValue::String("Corp".into())))
        .await.unwrap();

    tx.commit().await.unwrap();

    let labels = graph.get_labels().await.unwrap();
    assert_eq!(labels.len(), 2);
    assert!(labels.contains(&"Person".to_string()));
    assert!(labels.contains(&"Entity".to_string()));
}

#[tokio::test]
async fn test_get_schema() {
    let graph = Graph::in_memory().await.unwrap();
    let mut tx = graph.begin_transaction().await.unwrap();

    let mut props = HashMap::new();
    props.insert("name".to_string(), PropertyValue::String("Alice".into()));
    props.insert("age".to_string(), PropertyValue::Int(30.into()));
    let alice = tx.add_node(Node::new("Person")
        .with_properties(props)).await.unwrap();

    let mut props2 = HashMap::new();
    props2.insert("name".to_string(), PropertyValue::String("Bob".into()));
    let bob=tx.add_node(Node::new("Person")
        .with_properties(props2)).await.unwrap();

    let mut edge_props = HashMap::new();
    edge_props.insert("since".to_string(), PropertyValue::Int(2020.into()));
    tx.add_edge(Edge::new(alice, bob, "KNOWS")
        .with_properties(edge_props))
        .unwrap();

    tx.commit().await.unwrap();

    let schema = graph.get_schema().await.unwrap();

    assert_eq!(schema.total_nodes, 2);
    assert_eq!(schema.total_edges, 1);
    assert_eq!(schema.node_labels.len(), 1);
    assert_eq!(schema.edge_types.len(), 1);
    assert!(schema.node_labels.contains(&"Person".to_string()));
    assert!(schema.edge_types.contains(&"KNOWS".to_string()));
}

#[tokio::test]
async fn test_get_label_properties() {
    let graph = Graph::in_memory().await.unwrap();
    let mut tx = graph.begin_transaction().await.unwrap();

    let mut props = HashMap::new();
    props.insert("name".to_string(), PropertyValue::String("Alice".into()));
    props.insert("age".to_string(), PropertyValue::Int(30.into()));
    props.insert("email".to_string(), PropertyValue::String("alice@example.com".into()));
    tx.add_node(Node::new("Person")
        .with_properties(props)).await.unwrap();

    tx.commit().await.unwrap();

    let properties = graph.get_label_properties("Person").await.unwrap();
    assert_eq!(properties.len(), 3);
    assert!(properties.contains(&"name".to_string()));
    assert!(properties.contains(&"age".to_string()));
    assert!(properties.contains(&"email".to_string()));
}

#[tokio::test]
async fn test_label_count() {
    let graph = Graph::in_memory().await.unwrap();
    let mut tx = graph.begin_transaction().await.unwrap();

    for i in 0..5 {
        let mut props = HashMap::new();
        props.insert("id".to_string(), PropertyValue::Int(i.into()));
        tx.add_node(Node::new("Person").with_properties(props)).await.unwrap();
    }

    tx.commit().await.unwrap();

    let count = graph.get_label_count("Person").await.unwrap();
    assert_eq!(count, 5);
}