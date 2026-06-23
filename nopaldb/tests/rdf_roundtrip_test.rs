// tests/rdf_roundtrip_test.rs
//
// Round-trip tests: Graph → export_turtle() → import_turtle() → same Graph.
//
// Required features: owl-import
// Run with: cargo test --features owl-import --test rdf_roundtrip_test

use nopaldb::graph::Graph;
use nopaldb::types::{Node, NodeKind, PropertyValue};
use tempfile::TempDir;

async fn open_temp_graph() -> (Graph, TempDir) {
    let dir = TempDir::new().unwrap();
    let graph = Graph::open(dir.path().to_str().unwrap()).await.unwrap();
    (graph, dir)
}

// ---------------------------------------------------------------------------
// Test 1 — round-trip mínimo: una sola clase
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_roundtrip_single_class() {
    let (graph, _dir) = open_temp_graph().await;

    let ttl_in = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
:Person rdf:type owl:Class .
"#;

    graph.import_turtle(ttl_in).await.unwrap();

    let exported = graph.export_turtle().await.unwrap();

    // El output debe contener el bloque de prefijos y la clase.
    assert!(
        exported.contains("@prefix owl:"),
        "debe incluir prefijo owl"
    );
    assert!(
        exported.contains("@prefix rdf:"),
        "debe incluir prefijo rdf"
    );
    assert!(
        exported.contains(":Person rdf:type owl:Class ."),
        "debe exportar la clase Person"
    );

    // Reimport en grafo limpio → debe dar 1 clase.
    let (graph2, _dir2) = open_temp_graph().await;
    let report = graph2.import_turtle(&exported).await.unwrap();

    assert_eq!(report.classes_added, 1, "reimport debe dar 1 clase");
    assert_eq!(report.subclass_edges_added, 0);
    let nodes = graph2.get_nodes_by_label("Person").await.unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].kind, NodeKind::Class);
}

// ---------------------------------------------------------------------------
// Test 2 — round-trip con jerarquía: Animal > Mammal > Dog
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_roundtrip_hierarchy() {
    let (graph, _dir) = open_temp_graph().await;

    let ttl_in = r#"
@prefix owl:  <http://www.w3.org/2002/07/owl#> .
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
:Animal rdf:type owl:Class .
:Mammal rdf:type owl:Class .
:Dog    rdf:type owl:Class .
:Mammal rdfs:subClassOf :Animal .
:Dog    rdfs:subClassOf :Mammal .
"#;

    let first = graph.import_turtle(ttl_in).await.unwrap();
    assert_eq!(first.classes_added, 3);
    assert_eq!(first.subclass_edges_added, 2);

    let exported = graph.export_turtle().await.unwrap();

    assert!(exported.contains(":Animal rdf:type owl:Class ."));
    assert!(exported.contains(":Mammal rdf:type owl:Class ."));
    assert!(exported.contains(":Dog rdf:type owl:Class ."));
    assert!(exported.contains("rdfs:subClassOf"));

    // Reimport → mismas métricas.
    let (graph2, _dir2) = open_temp_graph().await;
    let report = graph2.import_turtle(&exported).await.unwrap();

    assert_eq!(report.classes_added, 3, "3 clases en reimport");
    assert_eq!(report.subclass_edges_added, 2, "2 subClassOf en reimport");
}

// ---------------------------------------------------------------------------
// Test 3 — round-trip con individuos y data properties
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_roundtrip_individuals_with_properties() {
    let (graph, _dir) = open_temp_graph().await;

    let ttl_in = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
:Person rdf:type owl:Class .
:Alice  rdf:type :Person .
:Alice  :age "30" .
:Alice  :name "Alice" .
"#;

    graph.import_turtle(ttl_in).await.unwrap();

    let exported = graph.export_turtle().await.unwrap();

    // El export debe contener el tipo xsd para el entero.
    assert!(
        exported.contains("xsd:integer"),
        "age debe exportarse como xsd:integer"
    );
    assert!(
        exported.contains(":Alice"),
        "individuo Alice debe estar en el export"
    );

    // Reimport en grafo limpio.
    let (graph2, _dir2) = open_temp_graph().await;
    let report = graph2.import_turtle(&exported).await.unwrap();

    assert_eq!(report.classes_added, 1, "1 clase en reimport");
    assert_eq!(report.instances_added, 1, "1 instancia en reimport");

    // Verificar que la propiedad age se preservó como Int.
    let person_nodes = graph2.get_nodes_by_label("Person").await.unwrap();
    let alice = person_nodes
        .iter()
        .find(|n| n.kind == NodeKind::Individual)
        .expect("debe existir el individuo Alice");
    assert_eq!(
        alice.properties.get("age"),
        Some(&PropertyValue::Int(30)),
        "age debe round-tripear como Int(30)"
    );
}

// ---------------------------------------------------------------------------
// Test 4 — export no emite nodos ordinarios (Individual sin propiedad "iri")
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_export_excludes_ordinary_nodes() {
    let (graph, _dir) = open_temp_graph().await;

    // Nodo ordinario de NopalDB — sin IRI, sin owl:Class.
    let mut tx_node = Node::new("Transaction");
    tx_node
        .properties
        .insert("amount".to_string(), PropertyValue::Int(5000));
    graph.add_node(tx_node).await.unwrap();

    // Importar también una clase OWL.
    let ttl_owl = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
:LegalEntity rdf:type owl:Class .
"#;
    graph.import_turtle(ttl_owl).await.unwrap();

    let exported = graph.export_turtle().await.unwrap();

    // La clase OWL sí debe aparecer.
    assert!(
        exported.contains(":LegalEntity rdf:type owl:Class ."),
        "clase OWL debe exportarse"
    );
    // El nodo ordinario no debe aparecer.
    assert!(
        !exported.contains("Transaction"),
        "nodos ordinarios NO deben exportarse"
    );
    assert!(
        !exported.contains("amount"),
        "propiedades de nodos ordinarios NO deben exportarse"
    );
}

// ---------------------------------------------------------------------------
// Test 5 — round-trip diamond + idempotencia del segundo reimport
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_roundtrip_diamond_idempotent() {
    let (graph, _dir) = open_temp_graph().await;

    let ttl_in = r#"
@prefix owl:  <http://www.w3.org/2002/07/owl#> .
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
:A rdf:type owl:Class .
:B rdf:type owl:Class .
:C rdf:type owl:Class .
:D rdf:type owl:Class .
:B rdfs:subClassOf :A .
:C rdfs:subClassOf :A .
:D rdfs:subClassOf :B .
:D rdfs:subClassOf :C .
"#;

    graph.import_turtle(ttl_in).await.unwrap();
    let exported = graph.export_turtle().await.unwrap();

    // Primer reimport en grafo limpio.
    let (graph2, _dir2) = open_temp_graph().await;
    let r1 = graph2.import_turtle(&exported).await.unwrap();
    assert_eq!(r1.classes_added, 4);
    assert_eq!(r1.subclass_edges_added, 4);

    // Segundo reimport sobre el mismo grafo → idempotente (no duplica nodos).
    graph2.import_turtle(&exported).await.unwrap();

    // El número de nodos Class no debe duplicarse.
    for label in &["A", "B", "C", "D"] {
        let nodes = graph2.get_nodes_by_label(label).await.unwrap();
        let class_count = nodes.iter().filter(|n| n.kind == NodeKind::Class).count();
        assert_eq!(class_count, 1, "clase {} no debe duplicarse", label);
    }
}

// ---------------------------------------------------------------------------
// Test 6 — export_owl_file escribe a disco y reimport funciona
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_export_owl_file_roundtrip() {
    let (graph, dir) = open_temp_graph().await;

    let ttl_in = r#"
@prefix owl:  <http://www.w3.org/2002/07/owl#> .
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
:Vehicle rdf:type owl:Class .
:Car     rdf:type owl:Class .
:Car     rdfs:subClassOf :Vehicle .
"#;

    graph.import_turtle(ttl_in).await.unwrap();

    let out_path = dir.path().join("exported.ttl");
    graph.export_owl_file(&out_path).await.unwrap();

    assert!(out_path.exists(), "el archivo .ttl debe existir en disco");

    // Leer y reimport.
    let (graph2, _dir2) = open_temp_graph().await;
    let report = graph2.import_owl_file(&out_path).await.unwrap();

    assert_eq!(report.classes_added, 2, "Vehicle y Car");
    assert_eq!(report.subclass_edges_added, 1, "Car ⊑ Vehicle");
}
