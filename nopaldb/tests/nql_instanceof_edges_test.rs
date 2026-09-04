// tests/nql_instanceof_edges_test.rs
//
// `instanceOf` / `subClassOf` in NQL over a graph imported from Turtle (#93):
// direct instances count (the pre-0.5.11 bug), every declared type counts
// (the `instanceOf` edges), and the class can be named by label,
// `prefix:Local` or full IRI. Domain is fictional botany.

use nopaldb::types::{Node, PropertyValue};
use nopaldb::{Graph, Result};

const FLORA: &str = r#"
@prefix owl:   <http://www.w3.org/2002/07/owl#> .
@prefix rdfs:  <http://www.w3.org/2000/01/rdf-schema#> .
@prefix flora: <http://plantas.example/flora#> .
@prefix fauna: <http://plantas.example/fauna#> .
@prefix :      <http://plantas.example/> .

flora:Planta          a owl:Class .
flora:Arbol           a owl:Class ; rdfs:subClassOf flora:Planta .
flora:PlantaMedicinal a owl:Class ; rdfs:subClassOf flora:Planta .
flora:Rosa            a owl:Class ; rdfs:subClassOf flora:Planta .
fauna:Rosa            a owl:Class .
fauna:Insecto         a owl:Class .

:roble         a flora:Arbol .
:musgo         a flora:Planta .
:sauce         a flora:Arbol, flora:PlantaMedicinal .
:rosa_roja     a flora:Rosa .
:rosa_mosqueta a fauna:Rosa .
:abeja         a fauna:Insecto ; :visita :rosa_roja, :sauce .
"#;

async fn labels(graph: &Graph, nql: &str) -> Vec<String> {
    let result = graph.execute_nql(nql).await.unwrap_or_else(|e| panic!("{nql}: {e}"));
    let mut out: Vec<String> = result
        .rows()
        .iter()
        .filter_map(|r| r.values.values().next())
        .filter_map(|v| match v {
            PropertyValue::String(s) => Some(s.clone()),
            other => Some(format!("{other:?}")),
        })
        .collect();
    out.sort();
    out
}

async fn iris(graph: &Graph, class: &str) -> Vec<String> {
    labels(graph, &format!(r#"find n.iri from (n) where instanceOf(n, "{class}")"#)).await
}

fn iri(local: &str) -> String {
    format!("http://plantas.example/{local}")
}

// ---------------------------------------------------------------------------
// The bug: direct instances of the queried class were never returned.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_instanceof_includes_direct_instances() -> Result<()> {
    let graph = Graph::in_memory().await?;
    let report = graph.import_turtle(FLORA).await?;
    assert!(report.warnings.iter().all(|w| w.contains("Rosa")), "{:?}", report.warnings);

    // Planta: roble and sauce (Arbol), rosa_roja (Rosa), and musgo, a direct instance.
    assert_eq!(
        iris(&graph, "Planta").await,
        vec![iri("musgo"), iri("roble"), iri("rosa_roja"), iri("sauce")]
    );
    // Arbol has no subclasses: only direct instances, which used to give 0 rows.
    assert_eq!(iris(&graph, "Arbol").await, vec![iri("roble"), iri("sauce")]);
    Ok(())
}

// ---------------------------------------------------------------------------
// Every declared type counts, not only the one that became the label.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_instanceof_sees_every_type_of_a_multityped_node() -> Result<()> {
    let graph = Graph::in_memory().await?;
    graph.import_turtle(FLORA).await?;

    // sauce is labelled Arbol (first type) but is also a PlantaMedicinal.
    assert_eq!(iris(&graph, "PlantaMedicinal").await, vec![iri("sauce")]);
    let sauce = labels(&graph, r#"find n.label from (n) where instanceOf(n, "PlantaMedicinal")"#).await;
    assert_eq!(sauce, vec!["Arbol".to_string()]);
    Ok(())
}

// ---------------------------------------------------------------------------
// Three ways to name the class: label, prefix:Local, full IRI.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_instanceof_accepts_label_prefix_and_iri() -> Result<()> {
    let graph = Graph::in_memory().await?;
    graph.import_turtle(FLORA).await?;

    let by_label = iris(&graph, "Arbol").await;
    let by_prefix = iris(&graph, "flora:Arbol").await;
    let by_iri = iris(&graph, "http://plantas.example/flora#Arbol").await;
    assert_eq!(by_label, vec![iri("roble"), iri("sauce")]);
    assert_eq!(by_prefix, by_label);
    assert_eq!(by_iri, by_label);

    // Unknown class in any form: no rows, no error.
    assert!(iris(&graph, "Hongo").await.is_empty());
    assert!(iris(&graph, "flora:Hongo").await.is_empty());
    assert!(iris(&graph, "http://plantas.example/flora#Hongo").await.is_empty());
    assert!(iris(&graph, "nadie:Arbol").await.is_empty());
    Ok(())
}

// ---------------------------------------------------------------------------
// Two classes with the same local name: each qualified form finds its own.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_instanceof_distinguishes_same_local_name_across_namespaces() -> Result<()> {
    let graph = Graph::in_memory().await?;
    graph.import_turtle(FLORA).await?;

    assert_eq!(iris(&graph, "flora:Rosa").await, vec![iri("rosa_roja")]);
    assert_eq!(iris(&graph, "fauna:Rosa").await, vec![iri("rosa_mosqueta")]);
    assert_eq!(iris(&graph, "http://plantas.example/fauna#Rosa").await, vec![iri("rosa_mosqueta")]);
    // The bare label belongs to the class declared first (flora), as the importer labels them.
    assert_eq!(iris(&graph, "Rosa").await, vec![iri("rosa_roja")]);
    Ok(())
}

// ---------------------------------------------------------------------------
// subClassOf shares the resolution and stays strict.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_subclassof_accepts_the_same_forms_and_is_strict() -> Result<()> {
    let graph = Graph::in_memory().await?;
    graph.import_turtle(FLORA).await?;

    let expected = vec!["Arbol".to_string(), "PlantaMedicinal".to_string(), "Rosa".to_string()];
    for class in ["Planta", "flora:Planta", "http://plantas.example/flora#Planta"] {
        let got = labels(&graph, &format!(r#"find c.label from (c) where subClassOf(c, "{class}")"#)).await;
        assert_eq!(got, expected, "subClassOf(c, {class:?})");
    }
    // A class is not a subclass of itself; Arbol has none.
    assert!(labels(&graph, r#"find c.label from (c) where subClassOf(c, "Arbol")"#).await.is_empty());
    Ok(())
}

// ---------------------------------------------------------------------------
// Path filters take the same three forms.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_path_filters_accept_prefixed_and_iri_class_names() -> Result<()> {
    let graph = Graph::in_memory().await?;
    graph.import_turtle(FLORA).await?;

    for class in ["PlantaMedicinal", "flora:PlantaMedicinal", "http://plantas.example/flora#PlantaMedicinal"] {
        let got = labels(
            &graph,
            &format!(r#"find n.iri from (a:Insecto)-[:visita]->(n) where path_end_instanceOf("{class}")"#),
        )
        .await;
        assert_eq!(got, vec![iri("sauce")], "path_end_instanceOf({class:?})");
    }
    // Both visited nodes are plants, direct or by inheritance.
    let got = labels(
        &graph,
        r#"find n.iri from (a:Insecto)-[:visita]->(n) where path_end_instanceOf("flora:Planta")"#,
    )
    .await;
    assert_eq!(got, vec![iri("rosa_roja"), iri("sauce")]);
    Ok(())
}

// ---------------------------------------------------------------------------
// A node without `instanceOf` edges (built by hand, or by an import older
// than 0.5.10) is still read by its label — now including direct instances.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_nodes_without_type_edges_fall_back_to_label() -> Result<()> {
    let graph = Graph::in_memory().await?;
    graph.import_turtle(FLORA).await?;

    let mut tx = graph.begin_transaction().await?;
    tx.add_node(Node::new("Arbol").with_property("name", PropertyValue::String("encino".into()))).await?;
    tx.add_node(Node::new("Planta").with_property("name", PropertyValue::String("helecho".into()))).await?;
    tx.add_node(Node::new("Piedra").with_property("name", PropertyValue::String("cuarzo".into()))).await?;
    tx.commit().await?;

    // Imported plants have no `name`; only the hand-made nodes project a string.
    async fn names(graph: &Graph, class: &str) -> Vec<String> {
        let nql = format!(r#"find n.name from (n) where instanceOf(n, "{class}")"#);
        let result = graph.execute_nql(&nql).await.unwrap();
        let mut out: Vec<String> = result
            .rows()
            .iter()
            .filter_map(|r| match r.get("n.name") {
                Some(PropertyValue::String(s)) => Some(s.clone()),
                _ => None,
            })
            .collect();
        out.sort();
        out
    }
    assert_eq!(names(&graph, "Planta").await, vec!["encino".to_string(), "helecho".to_string()]);
    assert_eq!(names(&graph, "flora:Arbol").await, vec!["encino".to_string()]);
    assert!(names(&graph, "Piedra").await.is_empty(), "Piedra is not a class");
    Ok(())
}

// ---------------------------------------------------------------------------
// Reopening the database rebuilds types, IRIs and prefixes from storage.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_everything_survives_reopen() -> Result<()> {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap().to_string();
    {
        let graph = Graph::open(&path).await?;
        graph.import_turtle(FLORA).await?;
        graph.close().await?;
    }
    let graph = Graph::open(&path).await?;
    assert_eq!(
        iris(&graph, "Planta").await,
        vec![iri("musgo"), iri("roble"), iri("rosa_roja"), iri("sauce")]
    );
    assert_eq!(iris(&graph, "PlantaMedicinal").await, vec![iri("sauce")]);
    assert_eq!(iris(&graph, "fauna:Rosa").await, vec![iri("rosa_mosqueta")]);
    assert_eq!(iris(&graph, "http://plantas.example/flora#Arbol").await, vec![iri("roble"), iri("sauce")]);
    Ok(())
}
