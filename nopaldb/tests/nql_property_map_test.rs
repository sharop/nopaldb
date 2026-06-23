// tests/nql_property_map_test.rs
//
// Pruebas de integración para NQL Phase D:
// Property maps inline como filtros pushdown en patrones FROM.
//
// Sintaxis cubierta:
//   (a:Label {key: value})           — source node con property filter
//   [:TYPE] -> (b:Label {key: value}) — target node con property filter
//   (a {k1: v1}) -> [:R] -> (b {k2: v2}) — ambos
//   multi-pattern: FROM p1, p2 con property maps

use nopaldb::Graph;
use nopaldb::types::{Edge, Node, PropertyValue};

/// Crea un grafo con empleados y empresas.
///
/// Nodos:
///   alice  :Person  {name: "Alice",  dept: "eng",  active: true,  age: 30}
///   bob    :Person  {name: "Bob",    dept: "sales", active: false, age: 25}
///   carol  :Person  {name: "Carol",  dept: "eng",  active: true,  age: 35}
///   acme   :Company {name: "Acme",   sector: "tech", public: true}
///   beta   :Company {name: "Beta",   sector: "finance", public: false}
///
/// Aristas WORKS_AT:
///   alice  -> acme
///   bob    -> beta
///   carol  -> acme
///
/// Aristas KNOWS:
///   alice -> bob
///   alice -> carol
async fn setup() -> (Graph, [uuid::Uuid; 5]) {
    let graph = Graph::in_memory().await.unwrap();

    let alice = Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()))
        .with_property("dept", PropertyValue::String("eng".into()))
        .with_property("active", PropertyValue::Bool(true))
        .with_property("age", PropertyValue::Int(30));

    let bob = Node::new("Person")
        .with_property("name", PropertyValue::String("Bob".into()))
        .with_property("dept", PropertyValue::String("sales".into()))
        .with_property("active", PropertyValue::Bool(false))
        .with_property("age", PropertyValue::Int(25));

    let carol = Node::new("Person")
        .with_property("name", PropertyValue::String("Carol".into()))
        .with_property("dept", PropertyValue::String("eng".into()))
        .with_property("active", PropertyValue::Bool(true))
        .with_property("age", PropertyValue::Int(35));

    let acme = Node::new("Company")
        .with_property("name", PropertyValue::String("Acme".into()))
        .with_property("sector", PropertyValue::String("tech".into()))
        .with_property("public", PropertyValue::Bool(true));

    let beta = Node::new("Company")
        .with_property("name", PropertyValue::String("Beta".into()))
        .with_property("sector", PropertyValue::String("finance".into()))
        .with_property("public", PropertyValue::Bool(false));

    for node in [&alice, &bob, &carol, &acme, &beta] {
        graph.add_node(node.clone()).await.unwrap();
    }

    // WORKS_AT
    graph
        .add_edge(Edge::new(alice.id, acme.id, "WORKS_AT"))
        .await
        .unwrap();
    graph
        .add_edge(Edge::new(bob.id, beta.id, "WORKS_AT"))
        .await
        .unwrap();
    graph
        .add_edge(Edge::new(carol.id, acme.id, "WORKS_AT"))
        .await
        .unwrap();

    // KNOWS
    graph
        .add_edge(Edge::new(alice.id, bob.id, "KNOWS"))
        .await
        .unwrap();
    graph
        .add_edge(Edge::new(alice.id, carol.id, "KNOWS"))
        .await
        .unwrap();

    (graph, [alice.id, bob.id, carol.id, acme.id, beta.id])
}

fn names(result: &nopaldb::query::nql::executor::result::QueryResult, col: &str) -> Vec<String> {
    let mut v: Vec<_> = result
        .rows
        .iter()
        .filter_map(|r| r.get(col))
        .filter_map(|v| {
            if let PropertyValue::String(s) = v {
                Some(s.clone())
            } else {
                None
            }
        })
        .collect();
    v.sort();
    v
}

// ── Source property map ──────────────────────────────────────────────────────

#[tokio::test]
async fn test_source_property_map_single_field() {
    let (graph, _) = setup().await;
    // Solo Alice y Carol son dept=eng
    let r = graph
        .execute_nql(r#"FIND a.name FROM (a:Person {dept: "eng"}) -> [:WORKS_AT] -> (b:Company)"#)
        .await
        .unwrap();
    assert_eq!(names(&r, "a.name"), vec!["Alice", "Carol"]);
}

#[tokio::test]
async fn test_source_property_map_bool_field() {
    let (graph, _) = setup().await;
    // active=true: Alice y Carol
    let r = graph
        .execute_nql("FIND a.name FROM (a:Person {active: true}) -> [:WORKS_AT] -> (b:Company)")
        .await
        .unwrap();
    assert_eq!(names(&r, "a.name"), vec!["Alice", "Carol"]);
}

#[tokio::test]
async fn test_source_property_map_no_match_returns_empty() {
    let (graph, _) = setup().await;
    let r = graph
        .execute_nql(
            r#"FIND a.name FROM (a:Person {dept: "marketing"}) -> [:WORKS_AT] -> (b:Company)"#,
        )
        .await
        .unwrap();
    assert!(
        r.rows.is_empty(),
        "no match expected, got {:?}",
        r.rows.len()
    );
}

// ── Target property map ──────────────────────────────────────────────────────

#[tokio::test]
async fn test_target_property_map_single_field() {
    let (graph, _) = setup().await;
    // Personas que trabajan en sector=tech (solo Acme): Alice y Carol
    let r = graph
        .execute_nql(
            r#"FIND a.name FROM (a:Person) -> [:WORKS_AT] -> (b:Company {sector: "tech"})"#,
        )
        .await
        .unwrap();
    assert_eq!(names(&r, "a.name"), vec!["Alice", "Carol"]);
}

#[tokio::test]
async fn test_target_property_map_bool_field() {
    let (graph, _) = setup().await;
    // Personas que trabajan en empresa public=false (Beta): solo Bob
    let r = graph
        .execute_nql("FIND a.name FROM (a:Person) -> [:WORKS_AT] -> (b:Company {public: false})")
        .await
        .unwrap();
    assert_eq!(names(&r, "a.name"), vec!["Bob"]);
}

#[tokio::test]
async fn test_target_property_map_no_match() {
    let (graph, _) = setup().await;
    let r = graph
        .execute_nql(
            r#"FIND a.name FROM (a:Person) -> [:WORKS_AT] -> (b:Company {sector: "energy"})"#,
        )
        .await
        .unwrap();
    assert!(r.rows.is_empty());
}

// ── Source + Target property maps combined ───────────────────────────────────

#[tokio::test]
async fn test_source_and_target_property_maps() {
    let (graph, _) = setup().await;
    // Personas dept=eng que trabajan en sector=tech: Alice y Carol
    let r = graph.execute_nql(
        r#"FIND a.name, b.name FROM (a:Person {dept: "eng"}) -> [:WORKS_AT] -> (b:Company {sector: "tech"})"#
    ).await.unwrap();
    assert_eq!(r.rows.len(), 2);
    assert_eq!(names(&r, "a.name"), vec!["Alice", "Carol"]);
    // Ambas trabajan en Acme
    for row in &r.rows {
        assert_eq!(
            row.get("b.name"),
            Some(&PropertyValue::String("Acme".into()))
        );
    }
}

#[tokio::test]
async fn test_source_and_target_maps_no_intersection() {
    let (graph, _) = setup().await;
    // dept=sales (Bob) trabajando en sector=tech (Acme) — no hay tal arista
    let r = graph.execute_nql(
        r#"FIND a.name FROM (a:Person {dept: "sales"}) -> [:WORKS_AT] -> (b:Company {sector: "tech"})"#
    ).await.unwrap();
    assert!(r.rows.is_empty());
}

// ── Interaction with WHERE ───────────────────────────────────────────────────

#[tokio::test]
async fn test_property_map_plus_where_clause() {
    let (graph, _) = setup().await;
    // dept=eng (Alice=30, Carol=35) + WHERE a.age > 32 → solo Carol
    let r = graph.execute_nql(
        r#"FIND a.name FROM (a:Person {dept: "eng"}) -> [:WORKS_AT] -> (b:Company) WHERE a.age > 32"#
    ).await.unwrap();
    assert_eq!(names(&r, "a.name"), vec!["Carol"]);
}

// ── Single-node queries (already worked, regression guard) ───────────────────

#[tokio::test]
async fn test_single_node_property_map_regression() {
    let (graph, _) = setup().await;
    let r = graph
        .execute_nql(r#"FIND n.name FROM (n:Person {dept: "eng"})"#)
        .await
        .unwrap();
    assert_eq!(names(&r, "n.name"), vec!["Alice", "Carol"]);
}

// ── Property map with KNOWS (no label on target) ─────────────────────────────

#[tokio::test]
async fn test_property_map_knows_relationship() {
    let (graph, _) = setup().await;
    // Alice conoce a: bob (active=false) y carol (active=true)
    // Filtrar target active=true: solo Carol
    let r = graph.execute_nql(
        r#"FIND a.name, b.name FROM (a:Person {name: "Alice"}) -> [:KNOWS] -> (b:Person {active: true})"#
    ).await.unwrap();
    assert_eq!(r.rows.len(), 1);
    assert_eq!(
        r.rows[0].get("b.name"),
        Some(&PropertyValue::String("Carol".into()))
    );
}
