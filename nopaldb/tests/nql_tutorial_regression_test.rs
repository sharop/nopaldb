// tests/nql_tutorial_regression_test.rs
//
// Regression tests para los 4 bugs detectados durante la creación de tutoriales:
//   #53 — GROUP BY drops AS alias para n.label
//   #54 — Pattern aggregation ORDER BY + LIMIT no respeta orden
//   #55 — WHERE en edge properties no filtra (Int vs Float type mismatch)
//   #56 — instanceOf/subClassOf pasan todos los nodos

use nopaldb::{Graph, Result};
use nopaldb::types::{Node, Edge, PropertyValue};

fn int(n: i64) -> PropertyValue { PropertyValue::Int(n) }
fn float(f: f64) -> PropertyValue { PropertyValue::Float(f) }

// ---------------------------------------------------------------------------
// #53 — GROUP BY AS alias debe aparecer en columnas y filas del resultado
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_group_by_alias_in_columns() -> Result<()> {
    let graph = Graph::in_memory().await?;
    let mut tx = graph.begin_transaction().await?;

    tx.add_node(Node::new("Person").with_property("age", int(30))).await?;
    tx.add_node(Node::new("Person").with_property("age", int(25))).await?;
    tx.add_node(Node::new("Account").with_property("balance", int(1000))).await?;

    tx.commit().await?;

    let result = graph.execute_nql(
        "find n.label as etiqueta, count(*) as total \
         from (n) \
         group by n.label \
         order by total desc"
    ).await?;

    // La columna debe llamarse "etiqueta", no "n.label"
    assert!(
        result.columns.contains(&"etiqueta".to_string()),
        "columna 'etiqueta' debe existir — columnas actuales: {:?}", result.columns
    );
    assert!(
        result.columns.contains(&"total".to_string()),
        "columna 'total' debe existir"
    );

    // Cada fila debe tener el valor bajo la clave "etiqueta"
    for row in result.rows() {
        assert!(
            row.get("etiqueta").is_some(),
            "fila sin clave 'etiqueta': todas las claves: {:?}", result.columns
        );
    }

    // Debe haber 2 grupos: Person (2) y Account (1)
    assert_eq!(result.len(), 2, "deben ser exactamente 2 grupos");

    Ok(())
}

// ---------------------------------------------------------------------------
// #54 — Pattern aggregation ORDER BY + LIMIT debe retornar top-N ordenado
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_pattern_agg_order_by_limit() -> Result<()> {
    let graph = Graph::in_memory().await?;
    let mut tx = graph.begin_transaction().await?;

    // b recibe 3 transfers, c recibe 2, d recibe 1
    let a = tx.add_node(Node::new("Account")).await?;
    let b = tx.add_node(Node::new("Account")).await?;
    let c = tx.add_node(Node::new("Account")).await?;
    let d = tx.add_node(Node::new("Account")).await?;
    let _e = tx.add_node(Node::new("Account")).await?;

    tx.add_edge(Edge::new(a, b, "TRANSFERS").with_property("amount", int(100)))?;
    tx.add_edge(Edge::new(a, b, "TRANSFERS").with_property("amount", int(200)))?;
    tx.add_edge(Edge::new(a, b, "TRANSFERS").with_property("amount", int(300)))?;
    tx.add_edge(Edge::new(a, c, "TRANSFERS").with_property("amount", int(400)))?;
    tx.add_edge(Edge::new(a, c, "TRANSFERS").with_property("amount", int(500)))?;
    tx.add_edge(Edge::new(a, d, "TRANSFERS").with_property("amount", int(600)))?;

    tx.commit().await?;

    let result = graph.execute_nql(
        "find b.id, count(*) as inbound \
         from (a:Account) -[:TRANSFERS]-> (b:Account) \
         group by b.id \
         order by inbound desc \
         limit 2"
    ).await?;

    // Con LIMIT 2, solo deben retornarse 2 filas
    assert_eq!(result.len(), 2, "LIMIT 2 debe retornar exactamente 2 filas");

    // La primera fila debe ser b (3 inbound), la segunda c (2)
    let first_inbound = result.rows()[0].get("inbound");
    let second_inbound = result.rows()[1].get("inbound");

    assert_eq!(first_inbound, Some(&PropertyValue::Int(3)),
        "primera fila debe tener inbound=3 (cuenta b), got: {:?}", first_inbound);
    assert_eq!(second_inbound, Some(&PropertyValue::Int(2)),
        "segunda fila debe tener inbound=2 (cuenta c), got: {:?}", second_inbound);

    Ok(())
}

// ---------------------------------------------------------------------------
// #55 — WHERE en edge properties con amount Float debe filtrar correctamente
//
// Antes del fix: PropertyValue::Float(x) > PropertyValue::Int(900_000) usaba
// type_rank (Float=3, Int=2) → siempre true, sin importar el valor numérico.
// Después del fix: coerción numérica correcta.
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_edge_where_filter_float() -> Result<()> {
    let graph = Graph::in_memory().await?;
    let mut tx = graph.begin_transaction().await?;

    let a = tx.add_node(Node::new("Account")).await?;
    let b = tx.add_node(Node::new("Account")).await?;
    let c = tx.add_node(Node::new("Account")).await?;

    // Amounts almacenados como Float (escenario real: valores monetarios con decimales)
    tx.add_edge(Edge::new(a, b, "TRANSFERS").with_property("amount", float(1_500_000.0)))?;
    tx.add_edge(Edge::new(a, b, "TRANSFERS").with_property("amount", float(500_000.0)))?;
    tx.add_edge(Edge::new(a, c, "TRANSFERS").with_property("amount", float(2_000_000.0)))?;
    tx.add_edge(Edge::new(a, c, "TRANSFERS").with_property("amount", float(100_000.0)))?;

    tx.commit().await?;

    // El literal NQL `900000` se parsea como Int; la comparación Float > Int
    // debe ser numérica, no por type_rank.
    let result = graph.execute_nql(
        "find a.id, b.id, e.amount \
         from (a:Account) -[e:TRANSFERS]-> (b:Account) \
         where e.amount > 900000"
    ).await?;

    // Solo deben aparecer los 2 transfers > 900_000: 2_000_000.0 y 1_500_000.0
    assert_eq!(
        result.len(), 2,
        "WHERE e.amount > 900000 debe retornar 2 filas, got {}: {:?}",
        result.len(), result.rows()
    );

    for row in result.rows() {
        let amount = row.get("e.amount").expect("columna e.amount debe existir");
        let amount_f = match amount {
            PropertyValue::Float(f) => *f,
            PropertyValue::Int(i) => *i as f64,
            other => panic!("tipo inesperado: {:?}", other),
        };
        assert!(
            amount_f > 900_000.0,
            "todas las filas deben tener amount > 900_000, got {}", amount_f
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// #56 — instanceOf/subClassOf ya no pasan todos los nodos cuando no hay
//        taxonomía registrada. Antes del fix: devolvía todos los nodos (true).
//        Después del fix: devuelve 0 filas (false, correctamente).
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_instanceof_no_longer_passes_all_nodes() -> Result<()> {
    let graph = Graph::in_memory().await?;
    let mut tx = graph.begin_transaction().await?;

    tx.add_node(Node::new("Person").with_property("name", PropertyValue::String("Alice".into()))).await?;
    tx.add_node(Node::new("Person").with_property("name", PropertyValue::String("Bob".into()))).await?;
    tx.add_node(Node::new("Account").with_property("balance", int(500))).await?;

    tx.commit().await?;

    // Sin taxonomía registrada, instanceOf(n, "Person") NO debe retornar todos los nodos.
    // Antes del fix retornaba los 3 nodos (eval_condition devolvía true para FunctionCall).
    // Después del fix retorna 0 (eval_condition devuelve false; eval_condition_with_graph
    // intenta taxonomy lookup → None → false).
    let result = graph.execute_nql(
        r#"find n.label from (n) where instanceOf(n, "Person")"#
    ).await?;

    assert!(
        result.len() < 3,
        "instanceOf sin taxonomía NO debe retornar todos los nodos (era el bug #56), got {} filas",
        result.len()
    );

    Ok(())
}
