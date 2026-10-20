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

    assert_eq!(report.classes_added, 2, "should have added Person and Student");
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
    assert!(!result.rows.is_empty(), "NQL query should return Person nodes");
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
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

:Person rdf:type owl:Class .
:Alice  rdf:type :Person .
:Alice  :age "30"^^xsd:integer .
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
        // El IRI se guarda expandido: `:Alice` con el prefijo vacío por defecto.
        Some(&nopaldb::types::PropertyValue::String("http://example.org/ontology#Alice".to_string()))
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
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

:Animal rdf:type owl:Class .
:Dog    rdf:type owl:Class .
:Dog    rdfs:subClassOf :Animal .
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
    let a_id = a_nodes.iter().find(|n| n.kind == nopaldb::types::NodeKind::Class)
        .map(|n| n.id).expect("Class A must exist");
    let c_id = c_nodes.iter().find(|n| n.kind == nopaldb::types::NodeKind::Class)
        .map(|n| n.id).expect("Class C must exist");

    // CR1: C ⊑ B ⊑ A  → C ⊑ A transitively
    assert!(
        reasoner.is_subclass_of(c_id, a_id),
        "EL reasoner must infer C ⊑ A via transitivity"
    );
}

// ---------------------------------------------------------------------------
// Test 5 — `:rosa :creceEn :jardin` es una arista que NQL puede recorrer
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_resource_object_creates_edge_queryable_by_nql() {
    let (graph, _dir) = open_temp_graph().await;

    let ttl = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix :    <http://plantas.example/> .

:Planta a owl:Class .
:Lugar  a owl:Class .
:rosa   a :Planta ; :nombre "rosa" ; :creceEn :jardin .
:cactus a :Planta ; :nombre "cactus" ; :creceEn :maceta .
:jardin a :Lugar ; :nombre "jardín" .
:maceta a :Lugar ; :nombre "maceta" .
"#;
    let report = graph.import_turtle(ttl).await.unwrap();
    assert_eq!(report.edges_created, 6, "4 instanceOf + 2 creceEn");
    assert_eq!(report.placeholders_created, 0);

    let result = graph
        .execute_nql(r#"find p.nombre, l.nombre from (p:Planta)-[:creceEn]->(l:Lugar) where p.nombre = "rosa""#)
        .await
        .unwrap();
    assert_eq!(result.len(), 1, "{result:?}");
    assert_eq!(
        result.rows()[0].get("l.nombre"),
        Some(&nopaldb::types::PropertyValue::String("jardín".into()))
    );

    // Y las aristas instanceOf también se recorren.
    let result = graph
        .execute_nql(r#"find p.nombre from (p)-[:instanceOf]->(c) where c.label = "Planta" order by p.nombre"#)
        .await
        .unwrap();
    assert_eq!(result.len(), 2, "{result:?}");
}

// ---------------------------------------------------------------------------
// Test 6 — una arista `subClassOf` entre individuos no envenena la taxonomía
// ---------------------------------------------------------------------------
//
// `rebuild_taxonomy_from_graph` corre en cada `Graph::open`. Antes de 0.5.10
// tomaba TODA arista `subClassOf`; con aristas por predicado de usuario, un
// `:a :subClassOf :b` entre individuos entraría a la jerarquía. El importer
// califica ese predicado (`:subClassOf`) y el rebuild exige extremos Class.
#[cfg(feature = "reasoner")]
#[tokio::test]
async fn test_user_subclassof_between_individuals_survives_reopen() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap().to_string();

    let ttl = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix :    <http://plantas.example/> .

:Planta a owl:Class .
:Arbol  a owl:Class ; rdfs:subClassOf :Planta .
:roble  a :Arbol .
:haya   a :Arbol .
:roble  :subClassOf :haya .
"#;
    {
        let graph = Graph::open(&path).await.unwrap();
        let report = graph.import_turtle(ttl).await.unwrap();
        assert!(report.warnings.iter().any(|w| w.contains(":subClassOf")), "{:?}", report.warnings);
        graph.close().await.unwrap();
    }

    let graph = Graph::open(&path).await.unwrap();
    // instanceOf transitivo sigue correcto: 2 árboles son plantas, y nada más.
    let result = graph
        .execute_nql(r#"find n.label from (n) where instanceOf(n, "Planta")"#)
        .await
        .unwrap();
    assert_eq!(result.len(), 2, "{result:?}");
    // La jerarquía de clases es la declarada: solo Arbol bajo Planta.
    let result = graph
        .execute_nql(r#"find c.label from (c) where subClassOf(c, "Planta")"#)
        .await
        .unwrap();
    let labels: Vec<_> = result.rows().iter().filter_map(|r| r.get("c.label").cloned()).collect();
    assert_eq!(labels, vec![nopaldb::types::PropertyValue::String("Arbol".into())], "{result:?}");
}

// ---------------------------------------------------------------------------
// #235 — owl:disjointWith / owl:AllDisjointClasses reach the reasoner
// ---------------------------------------------------------------------------

const DISJOINT_TTL: &str = r#"
@prefix : <http://cocina.example/> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

:Cosa a owl:Class .
:Receta a owl:Class ; rdfs:subClassOf :Cosa .
:Utensilio a owl:Class ; rdfs:subClassOf :Cosa .
:Hibrido a owl:Class ; rdfs:subClassOf :Receta , :Utensilio .
:Receta owl:disjointWith :Utensilio .
:Utensilio owl:disjointWith :Receta .
"#;

async fn class_id(graph: &Graph, label: &str) -> nopaldb::types::NodeId {
    graph.get_nodes_by_label(label).await.unwrap()[0].id
}

#[tokio::test]
async fn test_disjoint_with_becomes_one_edge_and_feeds_the_reasoner() {
    use nopaldb::reasoner::{ELReasoner, Unsatisfiable, EDGE_DISJOINT_WITH};
    let (graph, _dir) = open_temp_graph().await;
    let report = graph.import_turtle(DISJOINT_TTL).await.unwrap();

    // Stated both ways: one edge for the unordered pair.
    assert_eq!(report.disjoint_pairs_added, 1);
    assert_eq!(report.triples_skipped, 0, "owl:disjointWith is consumed, not skipped");
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let edges = graph.get_edges_by_label(EDGE_DISJOINT_WITH).await.unwrap();
    assert_eq!(edges.len(), 1);

    // Second import: nothing new.
    let again = graph.import_turtle(DISJOINT_TTL).await.unwrap();
    assert_eq!(again.disjoint_pairs_added, 0);

    let (receta, utensilio, hibrido) = (
        class_id(&graph, "Receta").await,
        class_id(&graph, "Utensilio").await,
        class_id(&graph, "Hibrido").await,
    );
    let mut reasoner = ELReasoner::from_graph_at(&graph, u64::MAX).await.unwrap();
    assert_eq!(reasoner.unsatisfiable_classes(), vec![hibrido]);
    match reasoner.why_unsatisfiable(hibrido) {
        Some(Unsatisfiable::Disjoint { left, right }) => {
            let mut pair = [left, right];
            pair.sort();
            let mut want = [receta, utensilio];
            want.sort();
            assert_eq!(pair, want);
        }
        other => panic!("expected the disjoint pair, got {other:?}"),
    }
}

#[tokio::test]
async fn test_all_disjoint_classes_of_four_writes_six_pairs() {
    use nopaldb::reasoner::{ELReasoner, EDGE_DISJOINT_WITH};
    let (graph, _dir) = open_temp_graph().await;
    let ttl = r#"
@prefix : <http://cocina.example/> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
:Sopa a owl:Class . :Postre a owl:Class . :Bebida a owl:Class . :Salsa a owl:Class .
[] a owl:AllDisjointClasses ; owl:members ( :Sopa :Postre :Bebida :Salsa ) .
:Atole a owl:Class ; rdfs:subClassOf :Bebida , :Postre .
"#;
    let report = graph.import_turtle(ttl).await.unwrap();
    assert_eq!(report.disjoint_pairs_added, 6);
    assert_eq!(report.triples_skipped, 0, "the AllDisjointClasses node and its list are consumed");
    assert_eq!(graph.get_edges_by_label(EDGE_DISJOINT_WITH).await.unwrap().len(), 6);

    let atole = class_id(&graph, "Atole").await;
    let mut reasoner = ELReasoner::from_graph_at(&graph, u64::MAX).await.unwrap();
    assert_eq!(reasoner.unsatisfiable_classes(), vec![atole]);
}

#[tokio::test]
async fn test_subclass_of_owl_nothing_is_a_warning_not_a_node() {
    let (graph, _dir) = open_temp_graph().await;
    let ttl = r#"
@prefix : <http://cocina.example/> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
:Imposible a owl:Class ; rdfs:subClassOf owl:Nothing .
"#;
    let report = graph.import_turtle(ttl).await.unwrap();
    assert_eq!(report.classes_added, 1, "no node for owl:Nothing");
    assert_eq!(report.subclass_edges_added, 0);
    assert!(graph.get_nodes_by_label("Nothing").await.unwrap().is_empty());
    assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
    assert!(report.warnings[0].contains("owl:Nothing"), "{}", report.warnings[0]);
    assert_eq!(report.triples_skipped, 1, "the triple is reported, not stored");
}

#[tokio::test]
async fn test_disjoint_with_round_trips_through_export() {
    let (graph, _dir) = open_temp_graph().await;
    graph.import_turtle(DISJOINT_TTL).await.unwrap();
    let exported = graph.export_turtle().await.unwrap().turtle;
    assert!(exported.contains("owl:disjointWith") || exported.contains("http://www.w3.org/2002/07/owl#disjointWith"), "{exported}");

    let (other, _dir2) = open_temp_graph().await;
    let report = other.import_turtle(&exported).await.unwrap();
    assert_eq!(report.disjoint_pairs_added, 1);
}

#[tokio::test]
async fn test_owl_nothing_edge_cases_keep_the_class_and_warn() {
    let (graph, _dir) = open_temp_graph().await;
    let ttl = r#"
@prefix : <http://cocina.example/> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
:Fantasma rdfs:subClassOf owl:Nothing .
owl:Nothing rdfs:subClassOf :Fantasma .
:Fantasma owl:disjointWith :Fantasma .
:Fantasma owl:disjointWith owl:Nothing .
"#;
    let report = graph.import_turtle(ttl).await.unwrap();
    // The class whose only mention is `subClassOf owl:Nothing` still exists,
    // so the user can assert the axiom in the reasoner.
    assert_eq!(report.classes_added, 1, "{report:?}");
    assert_eq!(graph.get_nodes_by_label("Fantasma").await.unwrap().len(), 1);
    assert!(graph.get_nodes_by_label("Nothing").await.unwrap().is_empty(), "⊥ is never a node");
    assert_eq!(report.subclass_edges_added, 0);
    assert_eq!(report.disjoint_pairs_added, 0);
    // Four statements, four warnings (plus the usual one for a class created
    // without `a owl:Class`), four skipped: nothing vanishes in silence.
    assert_eq!(report.warnings.len(), 5, "{:?}", report.warnings);
    assert_eq!(report.warnings.iter().filter(|w| w.contains("owl:Nothing")).count(), 3, "{:?}", report.warnings);
    assert!(report.warnings.iter().any(|w| w.contains("consigo misma")), "{:?}", report.warnings);
    assert_eq!(report.triples_skipped, 4, "{report:?}");
}
