// tests/taxonomy_integration_test.rs
//
// Integration tests for IndexType::Taxonomy — verifies the full flow:
//   1. Create Class nodes + subClassOf edges
//   2. CREATE INDEX ... TYPE taxonomy via NQL
//   3. Index survives a database restart (load_indices rebuild)

use nopaldb::Graph;
use nopaldb::types::{Edge, Node, NodeKind};

/// Helper: open a Graph, add Animal/Mammal/Dog hierarchy, create taxonomy index,
/// then close and reopen to verify rebuild.
#[tokio::test]
async fn test_taxonomy_index_create_and_persist() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let db_path = temp_dir.path().to_str().unwrap();

    let (animal_id, mammal_id, dog_id);

    // ----- Phase 1: create data + index -----
    {
        let graph = Graph::open(db_path).await?;

        // Add Class nodes.
        let mut animal = Node::new("Class");
        animal.kind = NodeKind::Class;
        animal.properties.insert(
            "name".to_string(),
            nopaldb::types::PropertyValue::String("Animal".to_string()),
        );
        animal_id = graph.add_node(animal).await?;

        let mut mammal = Node::new("Class");
        mammal.kind = NodeKind::Class;
        mammal.properties.insert(
            "name".to_string(),
            nopaldb::types::PropertyValue::String("Mammal".to_string()),
        );
        mammal_id = graph.add_node(mammal).await?;

        let mut dog = Node::new("Class");
        dog.kind = NodeKind::Class;
        dog.properties.insert(
            "name".to_string(),
            nopaldb::types::PropertyValue::String("Dog".to_string()),
        );
        dog_id = graph.add_node(dog).await?;

        // Add subClassOf edges: Animal ← Mammal ← Dog
        graph
            .add_edge(Edge::new(animal_id, mammal_id, "subClassOf"))
            .await?;
        graph
            .add_edge(Edge::new(mammal_id, dog_id, "subClassOf"))
            .await?;

        // Create taxonomy index via NQL.
        graph
            .execute_statement("create index on Class(subClassOf) type taxonomy")
            .await?;

        // Verify index is listed.
        let indexes = graph.list_indexes().await;
        assert!(
            indexes.iter().any(|m| m.name == "Class_subClassOf"),
            "taxonomy index should be listed after CREATE INDEX"
        );
    }

    // ----- Phase 2: reopen and verify rebuild -----
    {
        let graph = Graph::open(db_path).await?;

        // Index should have been rebuilt by load_indices().
        let indexes = graph.list_indexes().await;
        assert!(
            indexes.iter().any(|m| m.name == "Class_subClassOf"),
            "taxonomy index should persist across restarts"
        );

        // The rebuilt index should have 3 Class nodes.
        let meta = indexes
            .iter()
            .find(|m| m.name == "Class_subClassOf")
            .unwrap();
        assert_eq!(
            meta.index_type,
            nopaldb::index::IndexType::Taxonomy,
            "index type should be Taxonomy"
        );
        // Size reflects registered class nodes.
        assert_eq!(
            meta.size, 3,
            "rebuilt taxonomy index should have 3 class nodes"
        );

        // Confirm the node IDs are still valid.
        graph.get_node(animal_id).await?;
        graph.get_node(mammal_id).await?;
        graph.get_node(dog_id).await?;
    }

    Ok(())
}

/// Verify that a duplicate CREATE INDEX returns an error.
#[tokio::test]
async fn test_taxonomy_index_duplicate_rejected() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let db_path = temp_dir.path().to_str().unwrap();

    let graph = Graph::open(db_path).await?;

    graph
        .execute_statement("create index on Class(subClassOf) type taxonomy")
        .await?;

    let result = graph
        .execute_statement("create index on Class(subClassOf) type taxonomy")
        .await;

    assert!(
        result.is_err(),
        "duplicate taxonomy index creation should fail"
    );

    Ok(())
}
