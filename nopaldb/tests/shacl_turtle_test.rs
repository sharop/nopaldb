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
async fn correct_data_conforms_and_subclass_instances_are_focus_nodes() -> Result<()> {
    // Only the three correct recipes.
    let graph = Graph::in_memory().await?;
    let correct: String = DATA.lines().take_while(|l| !l.starts_with("# Un defecto")).collect::<Vec<_>>().join("\n");
    graph.import_turtle(&correct).await?;
    let (report, _) = graph.validate_shapes(SHAPES).await?;
    assert!(report.conforms, "{:?}", report.violations);

    // `pan_de_muerto` is a Postre ⊑ Receta with two ingredients: a shape on
    // Receta demanding three reaches it through the hierarchy (#99).
    let shape = SHAPES.replace("sh:minCount 2 ; sh:class :Ingrediente", "sh:minCount 3 ; sh:class :Ingrediente");
    let (report, _) = graph.validate_shapes(&shape).await?;
    let focus: BTreeSet<String> = {
        let mut s = BTreeSet::new();
        for v in &report.violations {
            let node = graph.get_node(v.focus_node).await?;
            s.insert(node.properties["iri"].as_str().unwrap().rsplit('/').next().unwrap().to_string());
        }
        s
    };
    assert_eq!(focus, BTreeSet::from(["pan_de_muerto".to_string()]));

    // The class itself is never a focus node, even though it carries the label.
    for v in &report.violations {
        assert_ne!(graph.get_node(v.focus_node).await?.kind, nopaldb::types::NodeKind::Class);
    }
    Ok(())
}

#[tokio::test]
async fn target_class_by_label_prefix_and_iri_agree_and_exact_label_opts_out() -> Result<()> {
    use nopaldb::shacl::{ConstraintType, PathSpec, PropertyShape, ShaclValidator, Shape, Target, TargetMode};

    let graph = graph_with_data().await?;
    // Every Receta (and Postre) must have 3 ingredients: the ones with fewer are the focus nodes that fail.
    let shape_for = |target: &str| {
        Shape::new("Tres")
            .with_target(Target::Class(target.into()))
            .with_property_shape(PropertyShape::new(PathSpec::Predicate("usa".into()), vec![ConstraintType::MinCount(3)]))
    };
    let mut results = Vec::new();
    for target in ["Receta", ":Receta", "http://cocina.example/Receta"] {
        let report = ShaclValidator::from_shapes(vec![shape_for(target)]).validate(&graph).await?;
        let mut who: Vec<String> = Vec::new();
        for v in &report.violations {
            who.push(graph.get_node(v.focus_node).await?.properties["iri"].as_str().unwrap().rsplit('/').next().unwrap().to_string());
        }
        who.sort();
        results.push(who);
    }
    assert_eq!(results[0], results[1], "label vs :prefixed");
    assert_eq!(results[0], results[2], "label vs IRI");
    // pan_de_muerto (Postre ⊑ Receta, 2 ingredients) is in, through the hierarchy.
    assert!(results[0].contains(&"pan_de_muerto".to_string()), "{:?}", results[0]);
    assert!(results[0].contains(&"un_ingrediente".to_string()));
    assert!(!results[0].contains(&"cafe_de_olla".to_string()), "3 ingredients: conforms");

    // ExactLabel reproduces the pre-0.5.15 scan: the Postre is out, and an IRI finds nobody.
    let report = ShaclValidator::from_shapes(vec![shape_for("Receta")])
        .with_target_mode(TargetMode::ExactLabel)
        .validate(&graph)
        .await?;
    let mut who: Vec<String> = Vec::new();
    for v in &report.violations {
        who.push(graph.get_node(v.focus_node).await?.label.clone());
    }
    assert!(who.iter().all(|l| l == "Receta"), "{who:?}");
    assert!(!who.is_empty());
    let report = ShaclValidator::from_shapes(vec![shape_for("http://cocina.example/Receta")])
        .with_target_mode(TargetMode::ExactLabel)
        .validate(&graph)
        .await?;
    assert_eq!(report.violations.len(), who.len(), "ExactLabel falls back to the local name of an IRI");
    Ok(())
}

#[tokio::test]
async fn multi_typed_individual_is_a_focus_node_of_every_type() -> Result<()> {
    let graph = Graph::in_memory().await?;
    graph
        .import_turtle(
            r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix :    <http://cocina.example/> .
:Receta a owl:Class .  :Vegana a owl:Class .
:ensalada a :Receta, :Vegana .
:asado    a :Receta .
"#,
        )
        .await?;
    let (report, _) = graph
        .validate_shapes(
            r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix :   <http://cocina.example/> .
:R sh:targetClass :Receta ; sh:property [ sh:path :nombre ; sh:minCount 1 ] .
:V sh:targetClass :Vegana ; sh:property [ sh:path :sinCarne ; sh:minCount 1 ] .
"#,
        )
        .await?;
    let mut got: Vec<(String, String)> = Vec::new();
    for v in &report.violations {
        let node = graph.get_node(v.focus_node).await?;
        got.push((node.properties["iri"].as_str().unwrap().rsplit('/').next().unwrap().to_string(), v.shape_name.clone()));
    }
    got.sort();
    assert_eq!(
        got,
        vec![("asado".to_string(), "R".to_string()), ("ensalada".to_string(), "R".to_string()), ("ensalada".to_string(), "V".to_string())],
        "ensalada is labelled Receta (first type) but is also a focus node of the Vegana shape"
    );
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

// ---------------------------------------------------------------------------
// #100: logical constraints, sh:node, sh:severity / sh:message, pattern errors.
// ---------------------------------------------------------------------------

const LOGIC_DATA: &str = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix :    <http://cocina.example/> .
:Receta a owl:Class .  :Ingrediente a owl:Class .  :Utensilio a owl:Class .
:canela a :Ingrediente ; :nombre "Canela" .
:cafe   a :Ingrediente .
:olla   a :Utensilio ; :nombre "Olla" .
:entero  a :Receta ; :tiempoMin "20"^^xsd:integer ; :estado "cocido" ; :usa :canela .
:decimal a :Receta ; :tiempoMin "20.5"^^xsd:decimal ; :estado "cocido" ; :usa :canela .
:texto   a :Receta ; :tiempoMin "20" ; :estado "cocido" ; :usa :canela .
:crudo   a :Receta ; :tiempoMin "5"^^xsd:integer ; :estado "crudo" ; :usa :canela .
:sin_nombre_ing a :Receta ; :tiempoMin "5"^^xsd:integer ; :estado "cocido" ; :usa :cafe .
:con_olla a :Receta ; :tiempoMin "5"^^xsd:integer ; :estado "cocido" ; :usa :canela, :olla .
"#;

const LOGIC_SHAPES: &str = r#"
@prefix sh:  <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix :    <http://cocina.example/> .
:IngredienteShape a sh:NodeShape ;
  sh:class :Ingrediente ;
  sh:property [ sh:path :nombre ; sh:minCount 1 ] .
:RecetaShape a sh:NodeShape ; sh:targetClass :Receta ;
  sh:property [ sh:path :tiempoMin ; sh:or ( [ sh:datatype xsd:integer ] [ sh:datatype xsd:decimal ] ) ] ,
              [ sh:path :estado ; sh:not [ sh:in ( "crudo" ) ] ; sh:severity sh:Warning ; sh:message "mejor cocido" ] ,
              [ sh:path :usa ; sh:node :IngredienteShape ] .
"#;

async fn who(graph: &Graph, v: &nopaldb::shacl::ConstraintViolation) -> Result<String> {
    Ok(graph.get_node(v.focus_node).await?.properties["iri"].as_str().unwrap().rsplit('/').next().unwrap().to_string())
}

#[tokio::test]
async fn or_not_node_severity_and_message_from_turtle() -> Result<()> {
    let graph = Graph::in_memory().await?;
    graph.import_turtle(LOGIC_DATA).await?;
    let (report, shapes) = graph.validate_shapes(LOGIC_SHAPES).await?;
    assert!(shapes.ignored.is_empty(), "{:?}", shapes.ignored);

    let mut got: Vec<(String, String, String, usize)> = Vec::new();
    for v in &report.violations {
        got.push((who(&graph, v).await?, v.constraint.clone(), format!("{:?}", v.severity), v.nested.len()));
    }
    got.sort();
    assert_eq!(
        got,
        vec![
            ("con_olla".to_string(), "sh:NodeConstraintComponent".to_string(), "Violation".to_string(), 1),
            ("crudo".to_string(), "sh:NotConstraintComponent".to_string(), "Warning".to_string(), 0),
            ("sin_nombre_ing".to_string(), "sh:NodeConstraintComponent".to_string(), "Violation".to_string(), 1),
            ("texto".to_string(), "sh:OrConstraintComponent".to_string(), "Violation".to_string(), 2),
        ]
    );
    // The `or` explains both branches; the `node` explains which constraint of the referenced shape failed.
    let texto = report.violations.iter().find(|v| v.constraint == "sh:OrConstraintComponent").unwrap();
    assert!(texto.nested.iter().all(|n| n.constraint == "sh:DatatypeConstraintComponent"));
    assert_eq!(texto.value, Some(PropertyValue::String("20".into())));
    let olla = report.violations.iter().find(|v| v.value == Some(iri("olla"))).unwrap();
    assert_eq!(olla.nested[0].constraint, "sh:ClassConstraintComponent");
    assert_eq!(olla.nested[0].shape_name, "IngredienteShape");
    let cafe = report.violations.iter().find(|v| v.value == Some(iri("cafe"))).unwrap();
    assert_eq!(cafe.nested[0].constraint, "sh:MinCountConstraintComponent");
    assert_eq!(cafe.nested[0].path.as_deref(), Some("nombre"));
    // sh:message is the author's text; sh:severity Warning does not break conformance by itself.
    let crudo = report.violations.iter().find(|v| v.constraint == "sh:NotConstraintComponent").unwrap();
    assert_eq!(crudo.message, "mejor cocido");
    assert!(!report.conforms, "the Violations do");

    // Only the warning left: conforms.
    let only_warning = LOGIC_SHAPES.replace("[ sh:path :tiempoMin ; sh:or ( [ sh:datatype xsd:integer ] [ sh:datatype xsd:decimal ] ) ] ,\n", "").replace(" ,\n              [ sh:path :usa ; sh:node :IngredienteShape ]", "");
    let (report, _) = graph.validate_shapes(&only_warning).await?;
    assert_eq!(report.violations.len(), 1, "{:?}", report.violations);
    assert!(report.conforms);
    Ok(())
}

#[tokio::test]
async fn and_xone_and_deactivated_from_turtle() -> Result<()> {
    let graph = Graph::in_memory().await?;
    graph.import_turtle(LOGIC_DATA).await?;
    let shapes = r#"
@prefix sh:  <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix :    <http://cocina.example/> .
:Rapida a sh:NodeShape ; sh:targetClass :Receta ;
  sh:property [ sh:path :tiempoMin ; sh:and ( [ sh:datatype xsd:integer ] [ sh:maxInclusive 10 ] ) ] .
:Exclusiva a sh:NodeShape ; sh:targetClass :Receta ;
  sh:property [ sh:path :tiempoMin ; sh:xone ( [ sh:datatype xsd:integer ] [ sh:minInclusive 10 ] ) ] .
:Apagada a sh:NodeShape ; sh:targetClass :Receta ; sh:deactivated true ;
  sh:property [ sh:path :nombre ; sh:minCount 1 ] .
"#;
    let (report, sr) = graph.validate_shapes(shapes).await?;
    assert!(sr.ignored.is_empty(), "{:?}", sr.ignored);
    let mut got: Vec<(String, String)> = Vec::new();
    for v in &report.violations {
        got.push((v.shape_name.clone(), who(&graph, v).await?));
    }
    got.sort();
    assert_eq!(
        got,
        vec![
            // and: integer AND <= 10 → entero (20) and decimal/texto fail; the three 5-minute ones pass.
            ("Exclusiva".to_string(), "entero".to_string()),   // integer AND >= 10: both → not exactly one
            ("Exclusiva".to_string(), "texto".to_string()),    // neither
            ("Rapida".to_string(), "decimal".to_string()),
            ("Rapida".to_string(), "entero".to_string()),
            ("Rapida".to_string(), "texto".to_string()),
        ],
        "Apagada validates nothing"
    );
    Ok(())
}

#[tokio::test]
async fn invalid_pattern_is_a_load_error_naming_the_shape() -> Result<()> {
    let graph = graph_with_data().await?;
    let err = graph
        .validate_shapes(
            r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix :   <http://cocina.example/> .
:RecetaShape sh:targetClass :Receta ; sh:property [ sh:path :nombre ; sh:pattern "[" ] .
"#,
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains(":RecetaShape") && err.contains("'['"), "{err}");
    Ok(())
}
