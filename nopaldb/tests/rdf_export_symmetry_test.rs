// tests/rdf_export_symmetry_test.rs
//
// The exporter is the mirror of the importer (#70): import → export → import
// yields an isomorphic graph (nodes by IRI, edges by (subject, predicate,
// object), literals by value and type), under the document's own namespaces,
// and whatever cannot be written is listed in `ExportReport::skipped`.
// Fictional domain: a cookbook.

use std::collections::{BTreeMap, BTreeSet};

use nopaldb::types::{Edge, Node, NodeKind, PropertyValue};
use nopaldb::{Graph, Result};

const COCINA: &str = r#"
@prefix owl:  <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .
@prefix :     <http://cocina.example/> .
@prefix geo:  <http://lugares.example/> .

:Bebida    a owl:Class ; rdfs:label "Bebida" ; rdfs:comment "Todo lo que se sirve en taza o vaso." .
:Infusion  a owl:Class ; rdfs:subClassOf :Bebida .
:Postre    a owl:Class .
geo:Lugar  a owl:Class .

:café_de_olla a :Infusion, :Bebida ;
    :nombre "Café de olla" ;
    :temperatura "85"^^xsd:integer ;
    :precio "35.5"^^xsd:decimal ;
    :caliente "true"^^xsd:boolean ;
    :ingrediente "canela", "piloncillo" ;
    :origen geo:Veracruz ;
    :acompaña :pan_de_muerto .

:pan_de_muerto a :Postre ; :nombre "Pan de muerto" ; :servidoEn geo:Puebla .
geo:Veracruz   a geo:Lugar ; rdfs:label "Veracruz" .
:agua          a :Bebida .
"#;

/// Everything the bridge promises to keep, keyed by IRI so two graphs can be
/// compared regardless of node ids.
#[derive(Debug, PartialEq)]
struct Shape {
    /// iri → (kind as text, label, properties)
    nodes: BTreeMap<String, (String, String, BTreeMap<String, PropertyValue>)>,
    edges: BTreeSet<(String, String, String)>,
}

fn iri_of(n: &Node) -> Option<String> {
    match n.properties.get("iri") {
        Some(PropertyValue::String(s)) => Some(s.clone()),
        _ => None,
    }
}

async fn shape(graph: &Graph) -> Shape {
    let nodes = graph.get_all_nodes().await.unwrap();
    let by_id: BTreeMap<_, _> = nodes.iter().filter_map(|n| iri_of(n).map(|i| (n.id, i))).collect();
    let mut out = Shape { nodes: BTreeMap::new(), edges: BTreeSet::new() };
    for n in &nodes {
        let Some(iri) = iri_of(n) else { continue };
        let props: BTreeMap<String, PropertyValue> = n
            .properties
            .iter()
            .filter(|(k, _)| k.as_str() != "iri")
            .map(|(k, v)| {
                // Repeated predicates land as a List in stored order; order is
                // not RDF semantics, so compare them as sets.
                let v = match v {
                    PropertyValue::List(items) => {
                        let mut items = items.clone();
                        items.sort_by_key(|x| format!("{x:?}"));
                        PropertyValue::List(items)
                    }
                    other => other.clone(),
                };
                (k.clone(), v)
            })
            .collect();
        out.nodes.insert(iri, (format!("{:?}", n.kind), n.label.clone(), props));
    }
    for e in graph.get_all_edges().await.unwrap() {
        if let (Some(s), Some(t)) = (by_id.get(&e.source), by_id.get(&e.target)) {
            let pred = match e.properties.get("iri") {
                Some(PropertyValue::String(p)) => p.clone(),
                _ => format!("<no iri> {}", e.edge_type),
            };
            out.edges.insert((s.clone(), format!("{} {pred}", e.edge_type), t.clone()));
        }
    }
    out
}

#[tokio::test]
async fn test_import_export_import_is_isomorphic() -> Result<()> {
    let a = Graph::in_memory().await?;
    let first = a.import_turtle(COCINA).await?;
    assert!(first.warnings.is_empty(), "{:?}", first.warnings);
    assert_eq!(first.placeholders_created, 1, "geo:Puebla is never described");

    let export = a.export_turtle().await?;
    assert!(export.report.skipped.is_empty(), "{:?}", export.report.skipped);

    let b = Graph::in_memory().await?;
    let second = b.import_turtle(&export.turtle).await?;
    assert!(second.warnings.is_empty(), "re-import must need no assumptions: {:?}", second.warnings);
    assert_eq!(second.triples_skipped, 0);
    assert_eq!(second.placeholders_created, 1);

    assert_eq!(shape(&a).await, shape(&b).await);

    // Namespaces are the document's, not a hardcoded one; non-ASCII survives.
    assert!(export.turtle.contains("@prefix : <http://cocina.example/> ."), "{}", export.turtle);
    assert!(export.turtle.contains("@prefix geo: <http://lugares.example/> ."), "{}", export.turtle);
    assert!(export.turtle.contains(":café_de_olla a :Infusion , :Bebida"), "label class first: {}", export.turtle);
    assert!(!export.turtle.contains("example.org/ontology"), "{}", export.turtle);
    assert_eq!(b.rdf_prefixes().await?.get("geo").map(String::as_str), Some("http://lugares.example/"));
    Ok(())
}

#[tokio::test]
async fn test_export_report_counts_match_the_document() -> Result<()> {
    let graph = Graph::in_memory().await?;
    graph.import_turtle(COCINA).await?;
    let export = graph.export_turtle().await?;
    let r = &export.report;
    assert_eq!(r.classes, 4);
    assert_eq!(r.subclass_edges, 1);
    assert_eq!(r.individuals, 4, "café, pan, Veracruz, agua; Puebla is only an object");
    assert_eq!(r.type_triples, 5, "café has two types");
    assert_eq!(r.edges, 3, "origen, acompaña, servidoEn");
    // Bebida: label, comment · café: nombre, temperatura, precio, caliente, 2×ingrediente · pan: nombre · Veracruz: label
    assert_eq!(r.literals, 10);
    assert_eq!(r.triples_written, r.classes + r.subclass_edges + r.type_triples + r.edges + r.literals);
    assert!(r.skipped.is_empty());

    // The document parses with a standard Turtle parser and has exactly that many triples.
    let parsed: Vec<_> = oxttl::TurtleParser::new().for_slice(export.turtle.as_bytes()).collect();
    assert!(parsed.iter().all(|t| t.is_ok()), "{parsed:?}");
    assert_eq!(parsed.len(), r.triples_written);
    Ok(())
}

#[tokio::test]
async fn test_export_is_deterministic_and_reimport_writes_nothing() -> Result<()> {
    let graph = Graph::in_memory().await?;
    graph.import_turtle(COCINA).await?;
    let one = graph.export_turtle().await?;
    let two = graph.export_turtle().await?;
    assert_eq!(one, two);

    // Re-importing a graph's own export into it is a no-op.
    let nodes_before = graph.get_all_nodes().await?.len();
    let edges_before = graph.get_all_edges().await?.len();
    let report = graph.import_turtle(&one.turtle).await?;
    assert_eq!(report.classes_added, 0);
    assert_eq!(report.instances_added, 0);
    assert_eq!(report.edges_created, 0);
    assert_eq!(report.placeholders_created, 0);
    assert_eq!(graph.get_all_nodes().await?.len(), nodes_before);
    assert_eq!(graph.get_all_edges().await?.len(), edges_before);
    Ok(())
}

#[tokio::test]
async fn test_unrepresentable_values_and_edges_are_reported_not_dropped_silently() -> Result<()> {
    let graph = Graph::in_memory().await?;
    graph.import_turtle(COCINA).await?;

    // Add, by hand, what RDF cannot carry: bytes, a nested object, an edge
    // with properties, and an edge to an ordinary node without IRI.
    let cafe = graph
        .get_all_nodes()
        .await?
        .into_iter()
        .find(|n| iri_of(n).as_deref() == Some("http://cocina.example/café_de_olla"))
        .unwrap();
    let mut updated = cafe.clone();
    updated.properties.insert("foto".into(), PropertyValue::Bytes(vec![1, 2, 3]));
    updated.properties.insert("meta".into(), PropertyValue::Object(vec![("k".into(), PropertyValue::Int(1))]));
    updated.properties.insert("nan".into(), PropertyValue::Float(f64::NAN));
    graph.add_node(updated).await?;

    let veracruz = graph
        .get_all_nodes()
        .await?
        .into_iter()
        .find(|n| iri_of(n).as_deref() == Some("http://lugares.example/Veracruz"))
        .unwrap();
    graph
        .add_edge(Edge::new(cafe.id, veracruz.id, "vendidoEn").with_property("desde", PropertyValue::Int(1990)))
        .await?;
    let plain = graph.add_node(Node::new("Nota").with_property("texto", PropertyValue::String("x".into()))).await?;
    graph.add_edge(Edge::new(cafe.id, plain, "anotadoEn")).await?;

    let export = graph.export_turtle().await?;
    let skipped = export.report.skipped.join("\n");
    assert!(skipped.contains("`foto`") && skipped.contains("Bytes"), "{skipped}");
    assert!(skipped.contains("`meta`") && skipped.contains("Object"), "{skipped}");
    assert!(skipped.contains("`nan`"), "{skipped}");
    assert!(skipped.contains("vendidoEn") && skipped.contains("`desde`"), "{skipped}");
    assert!(skipped.contains("anotadoEn") && skipped.contains("sin `iri`"), "{skipped}");
    assert_eq!(export.report.skipped.len(), 5, "{skipped}");

    // The edge without `iri` still exports, under the export namespace, and
    // the ordinary node stays out of the document.
    assert!(export.turtle.contains(":vendidoEn geo:Veracruz"), "{}", export.turtle);
    assert!(!export.turtle.contains("Nota") && !export.turtle.contains("anotadoEn"), "{}", export.turtle);
    Ok(())
}

#[tokio::test]
async fn test_graph_born_in_nopaldb_exports_under_default_namespace() -> Result<()> {
    // No Turtle ever imported: classes and individuals built by hand, with the
    // `iri` the bridge expects, get the default namespace as empty prefix.
    let graph = Graph::in_memory().await?;
    let mut planta = Node::new("Planta");
    planta.kind = NodeKind::Class;
    planta.properties.insert("iri".into(), PropertyValue::String("http://example.org/ontology#Planta".into()));
    let planta_id = graph.add_node(planta).await?;
    let roble = Node::new("Planta")
        .with_property("iri", PropertyValue::String("http://example.org/ontology#roble".into()))
        .with_property("altura", PropertyValue::Int(20));
    let roble_id = graph.add_node(roble).await?;
    graph.add_edge(Edge::new(roble_id, planta_id, "instanceOf")).await?;

    let export = graph.export_turtle().await?;
    assert!(export.turtle.contains("@prefix : <http://example.org/ontology#> ."), "{}", export.turtle);
    assert!(export.turtle.contains(":roble a :Planta"), "{}", export.turtle);
    assert!(export.turtle.contains(":altura 20"), "{}", export.turtle);
    assert!(export.report.skipped.is_empty());

    let other = Graph::in_memory().await?;
    let report = other.import_turtle(&export.turtle).await?;
    assert_eq!((report.classes_added, report.instances_added, report.edges_created), (1, 1, 1));
    Ok(())
}
