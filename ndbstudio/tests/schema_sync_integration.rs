use nopaldb::Graph;
use tempfile::tempdir;

#[tokio::test]
async fn write_then_rebuild_schema_reflects_updates() {
    let tmp_dir = tempdir().expect("failed to create temp dir");
    let graph = Graph::open(tmp_dir.path())
        .await
        .expect("failed to open graph");

    graph
        .execute_statement(r#"add (a:Person {name: "Alice"})-[:KNOWS]->(b:Person {name: "Bob"})"#)
        .await
        .expect("failed to execute add statement");

    graph
        .rebuild_schema()
        .await
        .expect("failed to rebuild schema");

    let schema = graph.get_schema().await.expect("failed to load schema");

    assert!(schema.node_labels.contains(&"Person".to_string()));
    assert!(schema.edge_types.contains(&"KNOWS".to_string()));

    assert_eq!(schema.node_counts.get("Person").copied().unwrap_or_default(), 2);
    assert_eq!(schema.edge_counts.get("KNOWS").copied().unwrap_or_default(), 1);
    assert_eq!(schema.total_nodes, 2);
    assert_eq!(schema.total_edges, 1);
}
