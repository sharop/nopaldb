// tests/shacl_validation_test.rs
//! Tests de integracion para SHACL Core.
//!
//! Requiere: `--features shacl`
//!
//! Ejecutar con:
//!   cargo test --test shacl_validation_test --features shacl

use nopaldb::Graph;
use nopaldb::shacl::{
    ConstraintType, DatatypeKind, PathSpec, PropertyShape, ShaclValidator, Shape, Target,
};
use nopaldb::types::{Node, PropertyValue};

async fn make_graph() -> Graph {
    Graph::in_memory()
        .await
        .expect("failed to create in-memory graph")
}

// --- T-1: minCount — nodo valido vs invalido ---

#[tokio::test]
async fn test_min_count_valid_node() {
    let graph = make_graph().await;
    let mut tx = graph.begin_transaction().await.unwrap();
    tx.add_node(Node::new("Person").with_property("age", PropertyValue::Int(25)))
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let shape = Shape::new("PersonShape")
        .with_target(Target::Class("Person".into()))
        .with_property_shape(PropertyShape::new(
            PathSpec::Property("age".into()),
            vec![ConstraintType::MinCount(1)],
        ));

    let report = ShaclValidator::from_shapes(vec![shape])
        .validate(&graph)
        .await
        .unwrap();

    assert!(report.conforms, "Nodo valido debe conformar");
}

#[tokio::test]
async fn test_min_count_missing_property() {
    let graph = make_graph().await;
    let mut tx = graph.begin_transaction().await.unwrap();
    tx.add_node(Node::new("Person")).await.unwrap();
    tx.commit().await.unwrap();

    let shape = Shape::new("PersonShape")
        .with_target(Target::Class("Person".into()))
        .with_property_shape(PropertyShape::new(
            PathSpec::Property("age".into()),
            vec![ConstraintType::MinCount(1)],
        ));

    let report = ShaclValidator::from_shapes(vec![shape])
        .validate(&graph)
        .await
        .unwrap();

    assert!(!report.conforms);
    assert_eq!(report.violations.len(), 1);
    assert_eq!(
        report.violations[0].path.as_deref(),
        Some("age"),
        "La violacion debe indicar el path 'age'"
    );
}

// --- T-2: datatype — Int vs String ---

#[tokio::test]
async fn test_datatype_int_expected_string_given() {
    let graph = make_graph().await;
    let mut tx = graph.begin_transaction().await.unwrap();
    tx.add_node(
        Node::new("Sensor").with_property("reading", PropertyValue::String("not-a-number".into())),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let shape = Shape::new("SensorShape")
        .with_target(Target::Class("Sensor".into()))
        .with_property_shape(PropertyShape::new(
            PathSpec::Property("reading".into()),
            vec![
                ConstraintType::MinCount(1),
                ConstraintType::Datatype(DatatypeKind::Int),
            ],
        ));

    let report = ShaclValidator::from_shapes(vec![shape])
        .validate(&graph)
        .await
        .unwrap();

    assert!(!report.conforms);
    let has_datatype_violation = report
        .violations
        .iter()
        .any(|v| v.message.contains("datatype") || v.message.contains("tipo"));
    assert!(
        has_datatype_violation,
        "Debe haber una violacion de datatype"
    );
}

// --- T-3: pattern (regex) ---

#[tokio::test]
async fn test_pattern_email_valid() {
    let graph = make_graph().await;
    let mut tx = graph.begin_transaction().await.unwrap();
    tx.add_node(
        Node::new("User").with_property("email", PropertyValue::String("alice@example.com".into())),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let shape = Shape::new("UserShape")
        .with_target(Target::Class("User".into()))
        .with_property_shape(PropertyShape::new(
            PathSpec::Property("email".into()),
            vec![
                ConstraintType::MinCount(1),
                ConstraintType::Pattern(r"^[^@\s]+@[^@\s]+\.[^@\s]+$".into()),
            ],
        ));

    let report = ShaclValidator::from_shapes(vec![shape])
        .validate(&graph)
        .await
        .unwrap();

    assert!(report.conforms, "Email valido debe conformar");
}

#[tokio::test]
async fn test_pattern_email_invalid() {
    let graph = make_graph().await;
    let mut tx = graph.begin_transaction().await.unwrap();
    tx.add_node(
        Node::new("User").with_property("email", PropertyValue::String("not-an-email".into())),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let shape = Shape::new("UserShape")
        .with_target(Target::Class("User".into()))
        .with_property_shape(PropertyShape::new(
            PathSpec::Property("email".into()),
            vec![
                ConstraintType::MinCount(1),
                ConstraintType::Pattern(r"^[^@\s]+@[^@\s]+\.[^@\s]+$".into()),
            ],
        ));

    let report = ShaclValidator::from_shapes(vec![shape])
        .validate(&graph)
        .await
        .unwrap();

    assert!(!report.conforms, "Email invalido no debe conformar");
}

// --- T-4: sh:in (enumeracion) ---

#[tokio::test]
async fn test_in_constraint_valid() {
    let graph = make_graph().await;
    let mut tx = graph.begin_transaction().await.unwrap();
    tx.add_node(
        Node::new("Account").with_property("status", PropertyValue::String("active".into())),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let allowed = vec![
        PropertyValue::String("active".into()),
        PropertyValue::String("inactive".into()),
        PropertyValue::String("suspended".into()),
    ];

    let shape = Shape::new("AccountShape")
        .with_target(Target::Class("Account".into()))
        .with_property_shape(PropertyShape::new(
            PathSpec::Property("status".into()),
            vec![ConstraintType::In(allowed)],
        ));

    let report = ShaclValidator::from_shapes(vec![shape])
        .validate(&graph)
        .await
        .unwrap();

    assert!(report.conforms);
}

#[tokio::test]
async fn test_in_constraint_invalid() {
    let graph = make_graph().await;
    let mut tx = graph.begin_transaction().await.unwrap();
    tx.add_node(
        Node::new("Account").with_property("status", PropertyValue::String("pending".into())),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let allowed = vec![
        PropertyValue::String("active".into()),
        PropertyValue::String("inactive".into()),
    ];

    let shape = Shape::new("AccountShape")
        .with_target(Target::Class("Account".into()))
        .with_property_shape(PropertyShape::new(
            PathSpec::Property("status".into()),
            vec![ConstraintType::In(allowed)],
        ));

    let report = ShaclValidator::from_shapes(vec![shape])
        .validate(&graph)
        .await
        .unwrap();

    assert!(!report.conforms);
    assert_eq!(report.violations.len(), 1);
}

// --- T-5: sh:targetClass ---

#[tokio::test]
async fn test_target_class_multiple_nodes() {
    let graph = make_graph().await;
    let mut tx = graph.begin_transaction().await.unwrap();
    // Alice tiene age, Bob no
    tx.add_node(
        Node::new("Person")
            .with_property("name", PropertyValue::String("Alice".into()))
            .with_property("age", PropertyValue::Int(30)),
    )
    .await
    .unwrap();
    tx.add_node(Node::new("Person").with_property("name", PropertyValue::String("Bob".into())))
        .await
        .unwrap();
    // Company no aplica al shape de Person
    tx.add_node(Node::new("Company").with_property("name", PropertyValue::String("ACME".into())))
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let shape = Shape::new("PersonShape")
        .with_target(Target::Class("Person".into()))
        .with_property_shape(PropertyShape::new(
            PathSpec::Property("age".into()),
            vec![ConstraintType::MinCount(1)],
        ));

    let report = ShaclValidator::from_shapes(vec![shape])
        .validate(&graph)
        .await
        .unwrap();

    // Solo Bob viola; Alice y Company quedan fuera del scope o conforman
    assert!(!report.conforms);
    assert_eq!(
        report.violations.len(),
        1,
        "Solo Bob debe violar (Company no es Person)"
    );
}

// --- T-6: PathSpec::Edge (single-hop) ---

#[tokio::test]
async fn test_property_shape_edge_path() {
    let graph = make_graph().await;
    let mut tx = graph.begin_transaction().await.unwrap();
    let person_id = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Alice".into())))
        .await
        .unwrap();
    // Alice tiene arista KNOWS hacia Bob
    let bob_id = tx
        .add_node(Node::new("Person").with_property("name", PropertyValue::String("Bob".into())))
        .await
        .unwrap();
    tx.add_edge(nopaldb::types::Edge {
        id: uuid::Uuid::new_v4(),
        source: person_id,
        target: bob_id,
        edge_type: "KNOWS".into(),
        properties: Default::default(),
    })
    .unwrap();
    tx.commit().await.unwrap();

    // Shape: Person debe tener al menos 1 relacion KNOWS saliente
    let shape = Shape::new("PersonKnowsShape")
        .with_target(Target::Class("Person".into()))
        .with_property_shape(PropertyShape::new(
            PathSpec::Edge("KNOWS".into()),
            vec![ConstraintType::MinCount(1)],
        ));

    let report = ShaclValidator::from_shapes(vec![shape])
        .validate(&graph)
        .await
        .unwrap();

    // Alice conforma (tiene KNOWS), Bob no (no tiene KNOWS saliente)
    assert!(!report.conforms);
    assert_eq!(report.violations.len(), 1, "Bob no tiene KNOWS saliente");
}

// --- T-7: rango numerico ---

#[tokio::test]
async fn test_numeric_range_valid() {
    let graph = make_graph().await;
    let mut tx = graph.begin_transaction().await.unwrap();
    tx.add_node(Node::new("Product").with_property("price", PropertyValue::Float(9.99)))
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let shape = Shape::new("ProductShape")
        .with_target(Target::Class("Product".into()))
        .with_property_shape(PropertyShape::new(
            PathSpec::Property("price".into()),
            vec![
                ConstraintType::MinInclusive(0.01),
                ConstraintType::MaxInclusive(999.99),
            ],
        ));

    let report = ShaclValidator::from_shapes(vec![shape])
        .validate(&graph)
        .await
        .unwrap();

    assert!(report.conforms);
}

#[tokio::test]
async fn test_numeric_range_violation() {
    let graph = make_graph().await;
    let mut tx = graph.begin_transaction().await.unwrap();
    tx.add_node(Node::new("Product").with_property("price", PropertyValue::Float(-5.0)))
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let shape = Shape::new("ProductShape")
        .with_target(Target::Class("Product".into()))
        .with_property_shape(PropertyShape::new(
            PathSpec::Property("price".into()),
            vec![ConstraintType::MinInclusive(0.01)],
        ));

    let report = ShaclValidator::from_shapes(vec![shape])
        .validate(&graph)
        .await
        .unwrap();

    assert!(!report.conforms);
}

// --- T-8: API programatica from_shapes ---

#[tokio::test]
async fn test_programmatic_shapes_no_target_applies_to_all() {
    let graph = make_graph().await;
    let mut tx = graph.begin_transaction().await.unwrap();
    // Nodo de cualquier tipo
    tx.add_node(Node::new("Thing").with_property("id_code", PropertyValue::String("X99".into())))
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // Shape sin target = aplica a todos los nodos
    let shape = Shape::new("AllNodesShape").with_property_shape(PropertyShape::new(
        PathSpec::Property("id_code".into()),
        vec![
            ConstraintType::MinCount(1),
            ConstraintType::Datatype(DatatypeKind::Str),
        ],
    ));

    let report = ShaclValidator::from_shapes(vec![shape])
        .validate(&graph)
        .await
        .unwrap();

    assert!(
        report.conforms,
        "Nodo con id_code string valido debe conformar"
    );
}

#[tokio::test]
async fn test_validate_node_individual() {
    let graph = make_graph().await;
    let mut tx = graph.begin_transaction().await.unwrap();
    let node_id = tx.add_node(Node::new("Person")).await.unwrap();
    tx.commit().await.unwrap();

    let shape = Shape::new("PersonShape")
        .with_target(Target::Node(node_id))
        .with_property_shape(PropertyShape::new(
            PathSpec::Property("name".into()),
            vec![ConstraintType::MinCount(1)],
        ));

    let validator = ShaclValidator::from_shapes(vec![shape]);
    let violations = validator.validate_node(&graph, node_id).await.unwrap();

    assert_eq!(violations.len(), 1, "Person sin nombre debe violar");
}

#[tokio::test]
async fn test_from_graph_loads_shape_nodes() {
    let graph = make_graph().await;
    let mut tx = graph.begin_transaction().await.unwrap();
    // Crear un nodo sh:NodeShape en el grafo
    tx.add_node(
        Node::new("sh:NodeShape")
            .with_property("sh:name", PropertyValue::String("CompanyShape".into()))
            .with_property("sh:targetClass", PropertyValue::String("Company".into())),
    )
    .await
    .unwrap();
    tx.add_node(Node::new("Company").with_property("name", PropertyValue::String("ACME".into())))
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // from_graph debe cargar el shape
    let validator = ShaclValidator::from_graph(&graph).await.unwrap();
    // Sin property shapes, el shape no agrega violations pero debe existir
    let report = validator.validate(&graph).await.unwrap();
    assert!(report.conforms, "Sin constraints, todo debe conformar");
}
