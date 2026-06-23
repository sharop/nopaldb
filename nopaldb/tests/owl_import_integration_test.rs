// tests/owl_import_integration_test.rs
//
// End-to-end integration tests for the OWL/Turtle → Graph pipeline.
//
// Required features: owl-import
// Run with: cargo test --features owl-import --test owl_import_integration_test

use nopaldb::graph::Graph;
use tempfile::TempDir;

async fn open_temp_graph() -> (Graph, TempDir) {
    let dir = TempDir::new().unwrap();
    let graph = Graph::open(dir.path().to_str().unwrap()).await.unwrap();
    (graph, dir)
}

// ---------------------------------------------------------------------------
// Test 1 — import via Graph::import_turtle method
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_import_owl_via_graph_method() {
    let (graph, _dir) = open_temp_graph().await;

    let ttl = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

:Person  rdf:type owl:Class .
:Student rdf:type owl:Class .
:Student rdfs:subClassOf :Person .
"#;

    let report = graph.import_turtle(ttl).await.unwrap();

    assert_eq!(
        report.classes_added, 2,
        "should have added Person and Student"
    );
    assert_eq!(report.subclass_edges_added, 1, "Student ⊑ Person");

    // Nodes exist in graph
    let person_nodes = graph.get_nodes_by_label("Person").await.unwrap();
    assert_eq!(person_nodes.len(), 1);

    let student_nodes = graph.get_nodes_by_label("Student").await.unwrap();
    assert_eq!(student_nodes.len(), 1);

    // NQL: query class nodes
    let result = graph
        .execute_nql("find p.label from (p:Person)")
        .await
        .unwrap();
    assert!(
        !result.rows.is_empty(),
        "NQL query should return Person nodes"
    );
}

// ---------------------------------------------------------------------------
// Test 2 — import instances with data properties
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_import_instances_with_properties() {
    let (graph, _dir) = open_temp_graph().await;

    let ttl = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .

:Person rdf:type owl:Class .
:Alice  rdf:type :Person .
:Alice  :age "30" .
:Alice  :name "Alice" .
"#;

    let report = graph.import_turtle(ttl).await.unwrap();

    assert_eq!(report.classes_added, 1, "Person class added");
    assert_eq!(report.instances_added, 1, "Alice individual added");

    // Individual node exists
    let person_nodes = graph.get_nodes_by_label("Person").await.unwrap();
    // Filter to individuals (not the class node)
    use nopaldb::types::NodeKind;
    let individuals: Vec<_> = person_nodes
        .iter()
        .filter(|n| n.kind != NodeKind::Class)
        .collect();
    assert_eq!(individuals.len(), 1, "one individual of type Person");

    let alice = &individuals[0];
    assert_eq!(
        alice.properties.get("iri"),
        Some(&nopaldb::types::PropertyValue::String(":Alice".to_string()))
    );
    assert_eq!(
        alice.properties.get("age"),
        Some(&nopaldb::types::PropertyValue::Int(30))
    );
}

// ---------------------------------------------------------------------------
// Test 3 — import same TTL twice → no duplicate nodes
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_import_is_idempotent_at_graph_level() {
    let (graph, _dir) = open_temp_graph().await;

    let ttl = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .

:Animal rdf:type owl:Class .
:Dog    rdf:type owl:Class .
:Dog    rdf:subClassOf :Animal .
:Fido   rdf:type :Dog .
"#;

    graph.import_turtle(ttl).await.unwrap();
    graph.import_turtle(ttl).await.unwrap();

    // Classes should not be duplicated
    let animal_nodes = graph.get_nodes_by_label("Animal").await.unwrap();
    assert_eq!(animal_nodes.len(), 1, "Animal class must not be duplicated");

    let dog_nodes = graph.get_nodes_by_label("Dog").await.unwrap();
    // 1 class node + at most 1 individual (Fido)
    let class_count = dog_nodes
        .iter()
        .filter(|n| n.kind == nopaldb::types::NodeKind::Class)
        .count();
    assert_eq!(class_count, 1, "Dog class must not be duplicated");
}

// ---------------------------------------------------------------------------
// Test 4 — import hierarchy then run EL reasoner CR1
// ---------------------------------------------------------------------------
#[cfg(feature = "reasoner")]
#[tokio::test]
async fn test_import_then_reasoner_classify() {
    use nopaldb::reasoner::ELReasoner;

    let (graph, _dir) = open_temp_graph().await;

    let ttl = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

:A rdf:type owl:Class .
:B rdf:type owl:Class .
:C rdf:type owl:Class .
:B rdfs:subClassOf :A .
:C rdfs:subClassOf :B .
"#;

    graph.import_turtle(ttl).await.unwrap();

    // Build reasoner from graph snapshot at current time.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 1;

    let mut reasoner = ELReasoner::from_graph_at(&graph, ts).await.unwrap();
    reasoner.classify_all();

    // Retrieve node IDs from the graph to cross-reference with the reasoner.
    let a_nodes = graph.get_nodes_by_label("A").await.unwrap();
    let c_nodes = graph.get_nodes_by_label("C").await.unwrap();
    let a_id = a_nodes
        .iter()
        .find(|n| n.kind == nopaldb::types::NodeKind::Class)
        .map(|n| n.id)
        .expect("Class A must exist");
    let c_id = c_nodes
        .iter()
        .find(|n| n.kind == nopaldb::types::NodeKind::Class)
        .map(|n| n.id)
        .expect("Class C must exist");

    // CR1: C ⊑ B ⊑ A  → C ⊑ A transitively
    assert!(
        reasoner.is_subclass_of(c_id, a_id),
        "EL reasoner must infer C ⊑ A via transitivity"
    );
}
