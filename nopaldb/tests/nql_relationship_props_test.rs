// tests/nql_relationship_props_test.rs
//
// Tests de integración para property maps inline en RelationshipPattern (NQL Phase D+).
//
// Sintaxis cubierta:
//   -[r:TYPE {k: v}]->             variable + tipo + prop única
//   -[:TYPE {k: v}]->              tipo + prop (sin variable)
//   -[r:TYPE {k1: v1, k2: v2}]->   múltiples props en arista
//   combinación con nodo source filtrado por props
//   acceso a r.prop en FIND con filtro inline en arista

use nopaldb::Graph;
use nopaldb::types::{Edge, Node, PropertyValue};

/// Grafo de transferencias monetarias:
///
/// Nodos:
///   alice  :Person  {name: "Alice"}
///   bob    :Person  {name: "Bob"}
///   carol  :Person  {name: "Carol"}
///
/// Aristas TRANSFERS con propiedades:
///   alice -> bob   {amount: 5000, currency: "USD"}
///   alice -> carol {amount: 200,  currency: "USD"}
///   bob   -> carol {amount: 5000, currency: "EUR"}
async fn setup() -> Graph {
    let graph = Graph::in_memory().await.unwrap();

    let alice = Node::new("Person").with_property("name", PropertyValue::String("Alice".into()));
    let bob = Node::new("Person").with_property("name", PropertyValue::String("Bob".into()));
    let carol = Node::new("Person").with_property("name", PropertyValue::String("Carol".into()));

    for n in [&alice, &bob, &carol] {
        graph.add_node(n.clone()).await.unwrap();
    }

    graph
        .add_edge(
            Edge::new(alice.id, bob.id, "TRANSFERS")
                .with_property("amount", PropertyValue::Int(5000))
                .with_property("currency", PropertyValue::String("USD".into())),
        )
        .await
        .unwrap();

    graph
        .add_edge(
            Edge::new(alice.id, carol.id, "TRANSFERS")
                .with_property("amount", PropertyValue::Int(200))
                .with_property("currency", PropertyValue::String("USD".into())),
        )
        .await
        .unwrap();

    graph
        .add_edge(
            Edge::new(bob.id, carol.id, "TRANSFERS")
                .with_property("amount", PropertyValue::Int(5000))
                .with_property("currency", PropertyValue::String("EUR".into())),
        )
        .await
        .unwrap();

    graph
}

// Helper: extrae columna como Vec<String> ordenado
fn col_sorted(r: &nopaldb::query::nql::executor::result::QueryResult, col: &str) -> Vec<String> {
    let mut v: Vec<_> = r
        .rows
        .iter()
        .filter_map(|row| {
            if let Some(PropertyValue::String(s)) = row.get(col) {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect();
    v.sort();
    v
}

// --- Tests ---

#[tokio::test]
async fn test_rel_props_single_int_filter() {
    let graph = setup().await;

    // amount=5000 → alice->bob y bob->carol (2 resultados)
    let r = graph
        .execute_nql(
            r#"find a.name, b.name from (a:Person) -[r:TRANSFERS {amount: 5000}]-> (b:Person)"#,
        )
        .await
        .unwrap();

    assert_eq!(
        r.rows.len(),
        2,
        "Esperaba 2 transferencias con amount=5000, got {}",
        r.rows.len()
    );
}

#[tokio::test]
async fn test_rel_props_no_match() {
    let graph = setup().await;

    let r = graph
        .execute_nql(
            r#"find a.name, b.name from (a:Person) -[r:TRANSFERS {amount: 99999}]-> (b:Person)"#,
        )
        .await
        .unwrap();

    assert!(
        r.rows.is_empty(),
        "Esperaba 0 resultados, got {}",
        r.rows.len()
    );
}

#[tokio::test]
async fn test_rel_props_multi_filter() {
    let graph = setup().await;

    // amount=5000 AND currency="USD" → solo alice->bob
    let r = graph.execute_nql(
        r#"find a.name, b.name from (a:Person) -[r:TRANSFERS {amount: 5000, currency: "USD"}]-> (b:Person)"#
    ).await.unwrap();

    assert_eq!(
        r.rows.len(),
        1,
        "Esperaba 1 resultado, got {}",
        r.rows.len()
    );
    assert_eq!(
        r.rows[0].get("a.name"),
        Some(&PropertyValue::String("Alice".into()))
    );
    assert_eq!(
        r.rows[0].get("b.name"),
        Some(&PropertyValue::String("Bob".into()))
    );
}

#[tokio::test]
async fn test_rel_props_no_variable() {
    let graph = setup().await;

    // Sintaxis sin variable de arista: [:TRANSFERS {amount: 200}]
    let r = graph
        .execute_nql(
            r#"find a.name, b.name from (a:Person) -[:TRANSFERS {amount: 200}]-> (b:Person)"#,
        )
        .await
        .unwrap();

    assert_eq!(
        r.rows.len(),
        1,
        "Esperaba 1 resultado, got {}",
        r.rows.len()
    );
    assert_eq!(
        r.rows[0].get("b.name"),
        Some(&PropertyValue::String("Carol".into()))
    );
}

#[tokio::test]
async fn test_rel_props_combined_with_source_node_props() {
    let graph = setup().await;

    // Source node filtrado por name + arista filtrada por amount
    let r = graph.execute_nql(
        r#"find a.name, b.name from (a:Person {name: "Alice"}) -[r:TRANSFERS {amount: 5000}]-> (b:Person)"#
    ).await.unwrap();

    assert_eq!(
        r.rows.len(),
        1,
        "Esperaba 1 resultado, got {}",
        r.rows.len()
    );
    assert_eq!(
        r.rows[0].get("b.name"),
        Some(&PropertyValue::String("Bob".into()))
    );
}

#[tokio::test]
async fn test_rel_props_string_value() {
    let graph = setup().await;

    // Solo transferencias en EUR → bob->carol
    let r = graph
        .execute_nql(
            r#"find a.name, b.name from (a:Person) -[r:TRANSFERS {currency: "EUR"}]-> (b:Person)"#,
        )
        .await
        .unwrap();

    assert_eq!(
        r.rows.len(),
        1,
        "Esperaba 1 resultado, got {}",
        r.rows.len()
    );
    assert_eq!(col_sorted(&r, "a.name"), vec!["Bob"]);
}

#[tokio::test]
async fn test_rel_props_access_edge_var_with_inline_filter() {
    let graph = setup().await;

    // FIND r.amount con filtro currency="USD" → 2 filas (amounts: 200 y 5000)
    let r = graph.execute_nql(
        r#"find a.name, r.amount from (a:Person) -[r:TRANSFERS {currency: "USD"}]-> (b:Person)"#
    ).await.unwrap();

    assert_eq!(
        r.rows.len(),
        2,
        "Esperaba 2 transferencias en USD, got {}",
        r.rows.len()
    );

    let mut amounts: Vec<i64> = r
        .rows
        .iter()
        .filter_map(|row| {
            if let Some(PropertyValue::Int(v)) = row.get("r.amount") {
                Some(*v)
            } else {
                None
            }
        })
        .collect();
    amounts.sort();
    assert_eq!(amounts, vec![200, 5000]);
}
