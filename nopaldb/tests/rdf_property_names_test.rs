// tests/rdf_property_names_test.rs
//
// Two data predicates with the same local name keep separate properties
// (#412): the first owns `fecha`, the second writes `vet:fecha`, with a
// warning. The graph remembers which predicate owns each key, so later
// imports agree and the export writes each value with its own predicate.
// Until 0.11.1 both values merged into `fecha: [..]`. Fictional domain: a zoo.

use std::collections::BTreeMap;

use nopaldb::types::PropertyValue;
use nopaldb::{Graph, Result};

const ZOO: &str = "http://zoo.example/";
const VET: &str = "http://vet.example/";

const MICHI: &str = r#"
@prefix zoo: <http://zoo.example/> .
@prefix vet: <http://vet.example/> .
zoo:michi zoo:fecha "2020-01-01" ;
          vet:fecha "2024-05-05" .
"#;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.into())
}

async fn props(g: &Graph, iri: &str) -> BTreeMap<String, PropertyValue> {
    let id = g.get_all_nodes_by_property("iri", &s(iri)).await.unwrap()[0];
    let node = g.get_node(id).await.unwrap();
    node.properties.into_iter().filter(|(k, _)| k != "iri").collect()
}

fn catalog(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[tokio::test]
async fn same_local_name_keeps_two_properties() -> Result<()> {
    let g = Graph::in_memory().await?;
    let report = g.import_turtle(MICHI).await?;

    let michi = props(&g, &format!("{ZOO}michi")).await;
    assert_eq!(michi.get("fecha"), Some(&s("2020-01-01")), "{michi:?}");
    assert_eq!(michi.get("vet:fecha"), Some(&s("2024-05-05")), "{michi:?}");

    assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
    let w = &report.warnings[0];
    assert!(w.contains(&format!("{VET}fecha")) && w.contains(&format!("{ZOO}fecha")) && w.contains("`vet:fecha`"), "{w}");

    let expected = catalog(&[("fecha", &format!("{ZOO}fecha")), ("vet:fecha", &format!("{VET}fecha"))]);
    assert_eq!(g.rdf_properties().await?, expected);

    // Re-importing assumes nothing new and changes nothing.
    let again = g.import_turtle(MICHI).await?;
    assert!(again.warnings.is_empty(), "{:?}", again.warnings);
    assert_eq!(props(&g, &format!("{ZOO}michi")).await, michi);
    assert_eq!(g.rdf_properties().await?, expected);
    Ok(())
}

/// Ownership outlives the document: a later import that only uses `vet:fecha`
/// still writes it apart from `fecha`, also after reopening the graph.
#[tokio::test]
async fn later_imports_and_reopens_keep_the_owner() -> Result<()> {
    let dir = tempfile::tempdir().unwrap();
    {
        let g = Graph::open(dir.path()).await?;
        g.import_turtle(MICHI).await?;
        g.close().await?;
    }
    let g = Graph::open(dir.path()).await?;
    let report = g
        .import_turtle(
            r#"@prefix vet: <http://vet.example/> .
            <http://zoo.example/firulais> vet:fecha "2025-02-02" ."#,
        )
        .await?;
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    let firulais = props(&g, &format!("{ZOO}firulais")).await;
    assert_eq!(firulais.get("vet:fecha"), Some(&s("2025-02-02")), "{firulais:?}");
    assert!(!firulais.contains_key("fecha"), "{firulais:?}");
    g.close().await?;
    Ok(())
}

/// The export writes each value with the predicate it came from (not both
/// under the export namespace), and importing that export into an empty graph
/// gives the same properties back.
#[tokio::test]
async fn export_writes_each_original_predicate() -> Result<()> {
    let g = Graph::in_memory().await?;
    g.import_turtle(MICHI).await?;
    let export = g.export_turtle().await?;
    assert!(export.turtle.contains("zoo:fecha \"2020-01-01\""), "{}", export.turtle);
    assert!(export.turtle.contains("vet:fecha \"2024-05-05\""), "{}", export.turtle);

    let copy = Graph::in_memory().await?;
    copy.import_turtle(&export.turtle).await?;
    assert_eq!(props(&copy, &format!("{ZOO}michi")).await, props(&g, &format!("{ZOO}michi")).await);
    Ok(())
}

/// A predicate whose local name is a key the importer writes itself (`iri`,
/// `rdfs_label`, …) no longer overwrites it.
#[tokio::test]
async fn reserved_keys_are_not_overwritten() -> Result<()> {
    let g = Graph::in_memory().await?;
    let report = g
        .import_turtle(
            r#"@prefix zoo: <http://zoo.example/> .
            zoo:michi zoo:iri "no es un IRI" ; zoo:nombre "Michi" ."#,
        )
        .await?;
    let michi = props(&g, &format!("{ZOO}michi")).await;
    assert_eq!(michi.get("zoo:iri"), Some(&s("no es un IRI")), "{michi:?}");
    assert_eq!(michi.get("nombre"), Some(&s("Michi")), "{michi:?}");
    assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
    assert!(report.warnings[0].contains("reservada"), "{}", report.warnings[0]);
    Ok(())
}
