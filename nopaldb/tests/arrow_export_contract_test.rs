//! What the Arrow analytical export promises: `graph_to_arrow`,
//! `nodes_to_arrow_with_properties` and `edges_to_arrow_with_properties`.
//!
//! Until 0.9.0:
//! - a label-scoped export carried every edge of the graph, so the
//!   "subgraph" had edges to nodes it did not contain;
//! - the property columns came out in hash order, so two exports of the same
//!   graph could differ;
//! - lists, objects and bytes became `<complex>` / `<bytes:N>`, a property
//!   with only such values got no column, a value of another type in an
//!   integer column became null, and an explicit `Null` became the text
//!   `"null"`;
//! - a property named `id` or `label` shadowed the structural column;
//! - an export without nodes was an error, unlike the edge export.
#![cfg(feature = "analytics")]

use std::collections::HashSet;

use arrow::array::{Array, BooleanArray, Int64Array, StringArray};
use arrow::datatypes::DataType;
use arrow::record_batch::RecordBatch;
use nopaldb::arrow_export::{graph_to_arrow, nodes_to_arrow_with_properties};
use nopaldb::types::{Edge, Node, PropertyValue};
use nopaldb::Graph;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

fn names(batch: &RecordBatch) -> Vec<String> {
    batch.schema().fields().iter().map(|f| f.name().clone()).collect()
}

fn text(batch: &RecordBatch, col: &str) -> Vec<Option<String>> {
    let a = batch.column_by_name(col).unwrap_or_else(|| panic!("no column {col}: {:?}", names(batch)));
    let a = a.as_any().downcast_ref::<StringArray>().unwrap_or_else(|| panic!("{col} is not Utf8"));
    (0..a.len()).map(|i| (!a.is_null(i)).then(|| a.value(i).to_string())).collect()
}

#[tokio::test]
async fn a_label_scoped_export_is_the_induced_subgraph() {
    let g = Graph::in_memory().await.unwrap();
    let a1 = g.add_node(Node::new("Planta")).await.unwrap();
    let a2 = g.add_node(Node::new("Planta")).await.unwrap();
    let b1 = g.add_node(Node::new("Plaga")).await.unwrap();
    let b2 = g.add_node(Node::new("Plaga")).await.unwrap();
    let inside = g.add_edge(Edge::new(a1, a2, "JUNTO_A")).await.unwrap();
    let selfloop = g.add_edge(Edge::new(a1, a1, "MISMA")).await.unwrap();
    let parallel = g.add_edge(Edge::new(a1, a2, "JUNTO_A")).await.unwrap();
    g.add_edge(Edge::new(b1, a1, "ATACA")).await.unwrap(); // crosses the scope
    g.add_edge(Edge::new(b1, b2, "CON")).await.unwrap(); // unrelated

    let (nodes, edges) = graph_to_arrow(&g, Some("Planta")).await.unwrap();
    assert_eq!(nodes.num_rows(), 2);
    let got: HashSet<String> = text(&edges, "id").into_iter().flatten().collect();
    let want: HashSet<String> = [inside, selfloop, parallel].iter().map(|id| id.to_string()).collect();
    assert_eq!(got, want, "only edges with both endpoints in the scope; self-loops and parallel edges kept");

    // Without a label: every edge.
    let (_, all) = graph_to_arrow(&g, None).await.unwrap();
    assert_eq!(all.num_rows(), 5);
}

#[tokio::test]
async fn the_schema_does_not_depend_on_insertion_order() {
    let props = [("zeta", PropertyValue::Int(1)), ("alfa", s("a")), ("mu", PropertyValue::Bool(true)), ("beta", PropertyValue::Float(1.5))];
    let mut schemas = Vec::new();
    for order in [[0, 1, 2, 3], [3, 2, 1, 0], [2, 0, 3, 1]] {
        let g = Graph::in_memory().await.unwrap();
        let mut node = Node::new("P");
        for i in order {
            node = node.with_property(props[i].0, props[i].1.clone());
        }
        g.add_node(node).await.unwrap();
        let (nodes, _) = graph_to_arrow(&g, None).await.unwrap();
        schemas.push(names(&nodes));
    }
    assert_eq!(schemas[0], vec!["id", "label", "alfa", "beta", "mu", "zeta"]);
    assert!(schemas.iter().all(|s| *s == schemas[0]), "{schemas:?}");
}

#[tokio::test]
async fn complex_mixed_and_binary_values_are_json_not_placeholders() {
    let nodes = vec![
        Node::new("P")
            .with_property("solo_lista", PropertyValue::List(vec![PropertyValue::Int(1), s("x")]))
            .with_property("mezcla", PropertyValue::Int(3))
            .with_property("objeto", PropertyValue::Object(vec![("k".into(), PropertyValue::Int(1))]))
            .with_property("bytes", PropertyValue::Bytes(vec![1, 2]))
            .with_property("entero", PropertyValue::Int(7)),
        Node::new("P")
            .with_property("mezcla", PropertyValue::List(vec![PropertyValue::Int(1)]))
            .with_property("entero", PropertyValue::Int(8)),
    ];
    let batch = nodes_to_arrow_with_properties(&nodes, None).unwrap();

    assert_eq!(text(&batch, "solo_lista")[0].as_deref(), Some(r#"[1,"x"]"#), "a property with only lists gets a column");
    assert_eq!(text(&batch, "mezcla"), vec![Some("3".into()), Some("[1]".into())], "mixed → text, nothing nulled");
    assert_eq!(text(&batch, "objeto")[0].as_deref(), Some(r#"{"k":1}"#));
    assert_eq!(text(&batch, "bytes")[0].as_deref(), Some("[1,2]"));
    let col = batch.column_by_name("entero").unwrap();
    assert_eq!(col.data_type(), &DataType::Int64, "one scalar type keeps its type");
    let ints = col.as_any().downcast_ref::<Int64Array>().unwrap();
    assert_eq!((ints.value(0), ints.value(1)), (7, 8));
    for cell in text(&batch, "solo_lista").into_iter().chain(text(&batch, "mezcla")).flatten() {
        assert!(!cell.contains("<complex>") && !cell.contains("<bytes"), "{cell}");
    }
}

#[tokio::test]
async fn explicit_null_is_an_arrow_null_like_a_missing_property() {
    let nodes = vec![
        Node::new("P").with_property("nombre", s("nopal")).with_property("activo", PropertyValue::Bool(true)),
        Node::new("P").with_property("nombre", PropertyValue::Null).with_property("activo", PropertyValue::Null),
        Node::new("P"),
    ];
    let batch = nodes_to_arrow_with_properties(&nodes, None).unwrap();
    assert_eq!(text(&batch, "nombre"), vec![Some("nopal".into()), None, None], "not the text \"null\"");
    let b = batch.column_by_name("activo").unwrap().as_any().downcast_ref::<BooleanArray>().unwrap();
    assert!(b.value(0) && b.is_null(1) && b.is_null(2));
}

#[tokio::test]
async fn properties_named_like_structural_columns_do_not_shadow_them() {
    let g = Graph::in_memory().await.unwrap();
    let id = g
        .add_node(Node::new("P").with_property("id", s("mi-id")).with_property("label", s("mi-etiqueta")))
        .await
        .unwrap();
    let other = g.add_node(Node::new("P")).await.unwrap();
    g.add_edge(Edge::new(id, other, "T").with_property("source", s("manual")).with_property("edge_type", s("x")))
        .await
        .unwrap();
    let (nodes, edges) = graph_to_arrow(&g, None).await.unwrap();

    let ns = names(&nodes);
    assert_eq!(ns.iter().filter(|n| *n == "id").count(), 1, "{ns:?}");
    assert!(ns.contains(&"property.id".into()) && ns.contains(&"property.label".into()), "{ns:?}");
    assert!(text(&nodes, "id").contains(&Some(id.to_string())), "the structural id is the node id");
    assert!(text(&nodes, "property.id").contains(&Some("mi-id".into())));

    let es = names(&edges);
    assert!(es.contains(&"property.source".into()) && es.contains(&"property.edge_type".into()), "{es:?}");
    assert_eq!(text(&edges, "source")[0].as_deref(), Some(id.to_string().as_str()));
}

#[tokio::test]
async fn empty_exports_have_their_schema_instead_of_an_error() {
    let g = Graph::in_memory().await.unwrap();
    let (nodes, edges) = graph_to_arrow(&g, None).await.unwrap();
    assert_eq!((nodes.num_rows(), edges.num_rows()), (0, 0));
    assert_eq!(names(&nodes), vec!["id", "label"]);

    g.add_node(Node::new("Planta")).await.unwrap();
    let (nodes, _) = graph_to_arrow(&g, Some("NoExiste")).await.unwrap();
    assert_eq!(nodes.num_rows(), 0);
    assert_eq!(names(&nodes), vec!["id", "label"]);
}
