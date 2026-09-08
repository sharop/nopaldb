// tests/shacl_turtle_test.rs
//
// SHACL shapes loaded from Turtle (#98) and validated against a graph imported
// from Turtle: the expected violations and nothing else, with component, path
// and value; what the validator does not implement is reported, not silent.
// Fictional domain: a cookbook.

use std::collections::BTreeSet;

use nopaldb::types::PropertyValue;
use nopaldb::{Graph, Result};

const DATA: &str = include_str!("fixtures/recetario.ttl");
const SHAPES: &str = include_str!("fixtures/recetario_shapes.ttl");

async fn graph_with_data() -> Result<Graph> {
    let graph = Graph::in_memory().await?;
    let report = graph.import_turtle(DATA).await?;
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    Ok(graph)
}

fn iri(local: &str) -> PropertyValue {
    PropertyValue::String(format!("http://cocina.example/{local}"))
}

#[tokio::test]
async fn shapes_from_turtle_find_exactly_the_planted_defects() -> Result<()> {
    let graph = graph_with_data().await?;
    let (report, shapes) = graph.validate_shapes(SHAPES).await?;

    assert_eq!(shapes.shapes, 1);
    assert_eq!(shapes.property_shapes, 4);
    assert_eq!(shapes.constraints, 3 + 2 + 1 + 2);
    assert_eq!(shapes.ignored.len(), 1, "{:?}", shapes.ignored);
    assert!(shapes.ignored[0].contains("sh:closed"), "{}", shapes.ignored[0]);
    assert!(shapes.warnings.is_empty(), "{:?}", shapes.warnings);

    assert!(!report.conforms);
    assert!(report.notes.is_empty(), "{:?}", report.notes);

    // Resolve focus nodes back to their IRIs.
    let mut found: Vec<(String, String, Option<String>, Option<PropertyValue>)> = Vec::new();
    for v in &report.violations {
        let node = graph.get_node(v.focus_node).await?;
        let who = node.properties["iri"].as_str().unwrap().rsplit('/').next().unwrap().to_string();
        found.push((who, v.constraint.clone(), v.path.clone(), v.value.clone()));
        assert_eq!(v.shape_name, "Receta bien descrita");
    }
    found.sort();
    let expected = vec![
        ("dificultad_rara".to_string(), "sh:InConstraintComponent".to_string(), Some("dificultad".to_string()), Some(PropertyValue::String("imposible".into()))),
        ("sin_nombre".to_string(), "sh:MinCountConstraintComponent".to_string(), Some("nombre".to_string()), None),
        ("tiempo_negativo".to_string(), "sh:MinInclusiveConstraintComponent".to_string(), Some("tiempoMin".to_string()), Some(PropertyValue::Int(-5))),
        ("un_ingrediente".to_string(), "sh:MinCountConstraintComponent".to_string(), Some("usa".to_string()), None),
        ("usa_utensilio".to_string(), "sh:ClassConstraintComponent".to_string(), Some("usa".to_string()), Some(iri("olla"))),
    ];
    assert_eq!(found, expected);
    Ok(())
}

#[tokio::test]
async fn correct_data_conforms_and_subclass_instances_are_reached_by_label_only() -> Result<()> {
    // Only the three correct recipes.
    let graph = Graph::in_memory().await?;
    let correct: String = DATA.lines().take_while(|l| !l.starts_with("# Un defecto")).collect::<Vec<_>>().join("\n");
    graph.import_turtle(&correct).await?;
    let (report, _) = graph.validate_shapes(SHAPES).await?;
    assert!(report.conforms, "{:?}", report.violations);

    // `pan_de_muerto` is a Postre ⊑ Receta. Today sh:targetClass resolves by
    // label (issue #99 makes it taxonomy-aware), so a shape on Receta does
    // not reach it: make that explicit so #99 has a test to flip.
    let shape = SHAPES.replace("sh:minCount 2 ; sh:class :Ingrediente", "sh:minCount 3 ; sh:class :Ingrediente");
    let (report, _) = graph.validate_shapes(&shape).await?;
    let focus: BTreeSet<String> = {
        let mut s = BTreeSet::new();
        for v in &report.violations {
            let node = graph.get_node(v.focus_node).await?;
            s.insert(node.label.clone());
        }
        s
    };
    assert_eq!(focus, BTreeSet::new(), "the two Receta recipes have 3 ingredients; the Postre with 2 is not a focus node yet");
    Ok(())
}

#[tokio::test]
async fn datatype_distinguishes_text_from_number_and_lists_count_per_element() -> Result<()> {
    let graph = Graph::in_memory().await?;
    graph
        .import_turtle(
            r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix :    <http://cocina.example/> .
:Receta a owl:Class .
:a a :Receta ; :tiempoMin "3" ; :etiqueta "dulce", "frío" .
:b a :Receta ; :tiempoMin "3"^^xsd:integer ; :etiqueta "dulce" .
"#,
        )
        .await?;
    let (report, shapes) = graph
        .validate_shapes(
            r#"
@prefix sh:  <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix :    <http://cocina.example/> .
:S sh:targetClass :Receta ;
   sh:property [ sh:path :tiempoMin ; sh:datatype xsd:integer ] ,
               [ sh:path :etiqueta ; sh:minCount 2 ] .
"#,
        )
        .await?;
    assert!(shapes.ignored.is_empty(), "{:?}", shapes.ignored);
    let mut got: Vec<(String, String)> = Vec::new();
    for v in &report.violations {
        let node = graph.get_node(v.focus_node).await?;
        got.push((node.properties["iri"].as_str().unwrap().rsplit('/').next().unwrap().to_string(), v.constraint.clone()));
    }
    got.sort();
    assert_eq!(
        got,
        vec![
            ("a".to_string(), "sh:DatatypeConstraintComponent".to_string()),
            ("b".to_string(), "sh:MinCountConstraintComponent".to_string()),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn malformed_shapes_are_an_error_and_nothing_runs() -> Result<()> {
    let graph = graph_with_data().await?;
    let err = graph.validate_shapes("@prefix sh: <http://www.w3.org/ns/shacl#> .\n:S sh:minCount .").await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("RDF parse error") && msg.contains("line 2"), "{msg}");
    Ok(())
}

#[tokio::test]
async fn shapes_file_and_target_node_by_iri() -> Result<()> {
    let graph = graph_with_data().await?;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shapes.ttl");
    std::fs::write(
        &path,
        r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix :   <http://cocina.example/> .
:Uno sh:targetNode :sin_nombre, :nadie ; sh:property [ sh:path :nombre ; sh:minCount 1 ] .
"#,
    )
    .unwrap();
    let (report, shapes) = graph.validate_shapes_file(&path).await?;
    assert_eq!(shapes.shapes, 1);
    assert_eq!(report.violations.len(), 1);
    assert_eq!(report.violations[0].shape_name, "Uno");
    assert_eq!(report.notes.len(), 1, "{:?}", report.notes);
    assert!(report.notes[0].contains("cocina.example/nadie"));
    Ok(())
}
