// tests/nql_path_function_test.rs
//
// `path(a, b)` y `shortestPath(a, b)` en NQL sobre variables del FROM (#416,
// fase 2): `path` da el camino (`{nodes, edges, length}` o nulo) y
// `shortestPath` la distancia (`-1.0` sin camino), con las opciones
// `edge_types`, `direction` y `max_depth`. Hasta 0.11.3 `shortestPath` solo
// aceptaba UUIDs literales y con dos patrones en el FROM era un error.
//
// Dominio ficticio: el zoo de shortest_path_edges_test.
//   michi -COME-> raton, michi -JUEGA_CON-> raton, raton -COME-> queso,
//   michi -PERSIGUE-> pajaro -PERSIGUE-> mosca -PERSIGUE-> queso

use std::collections::HashMap;

use nopaldb::types::{EdgeId, NodeId};
use nopaldb::{Edge, Graph, Node, PropertyValue, Result};

struct Zoo {
    g: Graph,
    id: HashMap<&'static str, NodeId>,
    edge: HashMap<(&'static str, &'static str, &'static str), EdgeId>,
}

async fn zoo() -> Result<Zoo> {
    let g = Graph::in_memory().await?;
    let mut id = HashMap::new();
    for nombre in ["michi", "raton", "queso", "pajaro", "mosca", "firulais"] {
        let n = g.add_node(Node::new("Animal").with_property("nombre", PropertyValue::String(nombre.into()))).await?;
        id.insert(nombre, n);
    }
    let mut edge = HashMap::new();
    for (from, kind, to) in [
        ("michi", "COME", "raton"),
        ("michi", "JUEGA_CON", "raton"),
        ("raton", "COME", "queso"),
        ("michi", "PERSIGUE", "pajaro"),
        ("pajaro", "PERSIGUE", "mosca"),
        ("mosca", "PERSIGUE", "queso"),
    ] {
        edge.insert((from, kind, to), g.add_edge(Edge::new(id[from], id[to], kind)).await?);
    }
    Ok(Zoo { g, id, edge })
}

const PAIR: &str = r#"from (a:Animal {nombre: "michi"}), (b:Animal {nombre: "queso"})"#;

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.into())
}

fn field<'v>(v: &'v PropertyValue, key: &str) -> &'v PropertyValue {
    let PropertyValue::Object(entries) = v else { panic!("not an object: {v:?}") };
    entries.iter().find(|(k, _)| k == key).map(|(_, v)| v).unwrap_or_else(|| panic!("no `{key}` in {v:?}"))
}

fn list(v: &PropertyValue) -> &[PropertyValue] {
    let PropertyValue::List(items) = v else { panic!("not a list: {v:?}") };
    items
}

impl Zoo {
    async fn one(&self, nql: &str, col: &str) -> PropertyValue {
        let r = self.g.execute_nql(nql).await.unwrap_or_else(|e| panic!("{nql}: {e}"));
        assert_eq!(r.rows().len(), 1, "{nql}");
        r.rows()[0].get(col).cloned().unwrap_or_else(|| panic!("{nql}: no `{col}` in {:?}", r.rows()[0]))
    }

    fn node_ids(&self, path: &PropertyValue, names: &[&str]) -> bool {
        let got: Vec<&PropertyValue> = list(field(path, "nodes")).iter().map(|n| field(n, "id")).collect();
        let want: Vec<PropertyValue> = names.iter().map(|n| s(&self.id[n].to_string())).collect();
        got.into_iter().eq(want.iter())
    }
}

#[tokio::test]
async fn path_gives_nodes_edges_and_length() -> Result<()> {
    let z = zoo().await?;
    let p = z.one(&format!("find path(a, b) as p {PAIR}"), "p").await;
    assert!(z.node_ids(&p, &["michi", "raton", "queso"]), "{p:?}");
    assert_eq!(field(&p, "length"), &PropertyValue::Int(2));
    let edges = list(field(&p, "edges"));
    let nodes = list(field(&p, "nodes"));
    assert_eq!(edges.len(), 2);
    for (i, e) in edges.iter().enumerate() {
        assert_eq!(field(e, "source"), field(&nodes[i], "id"), "edges[{i}]");
        assert_eq!(field(e, "target"), field(&nodes[i + 1], "id"), "edges[{i}]");
    }
    // La arista exacta por la que pasa, de las dos que hay entre michi y raton.
    assert_eq!(field(&edges[0], "id"), &s(&z.edge[&("michi", "COME", "raton")].to_string()));
    assert_eq!(field(&edges[0], "type"), &s("COME"));
    assert_eq!(field(&nodes[0], "label"), &s("Animal"));
    // Sin alias, la columna se llama como la llamada.
    let r = z.g.execute_nql(&format!("find path(a, b) {PAIR}")).await?;
    assert_eq!(r.columns, ["path(a, b)"]);
    Ok(())
}

#[tokio::test]
async fn options_filter_like_the_rust_api() -> Result<()> {
    let z = zoo().await?;
    let persigue = z.one(&format!(r#"find path(a, b, edge_types = ["PERSIGUE"]) as p {PAIR}"#), "p").await;
    assert!(z.node_ids(&persigue, &["michi", "pajaro", "mosca", "queso"]), "{persigue:?}");
    let juega = z
        .one(r#"find path(a, b, edge_types = ["JUEGA_CON"]) as p from (a:Animal {nombre: "michi"}), (b:Animal {nombre: "raton"})"#, "p")
        .await;
    assert_eq!(field(&list(field(&juega, "edges"))[0], "id"), &s(&z.edge[&("michi", "JUEGA_CON", "raton")].to_string()));
    // Saliente no hay vuelta; con `in` y `both`, sí.
    let back = r#"from (a:Animal {nombre: "queso"}), (b:Animal {nombre: "michi"})"#;
    assert_eq!(z.one(&format!("find path(a, b) as p {back}"), "p").await, PropertyValue::Null);
    for dir in ["in", "both"] {
        let p = z.one(&format!(r#"find path(a, b, direction = "{dir}") as p {back}"#), "p").await;
        assert!(z.node_ids(&p, &["queso", "raton", "michi"]), "{dir}: {p:?}");
    }
    let short = format!(r#"find path(a, b, edge_types = ["PERSIGUE"], max_depth = 2) as p {PAIR}"#);
    assert_eq!(z.one(&short, "p").await, PropertyValue::Null);
    Ok(())
}

#[tokio::test]
async fn shortest_path_takes_variables_and_keeps_its_distance() -> Result<()> {
    let z = zoo().await?;
    assert_eq!(z.one(&format!("find shortestPath(a, b) as d {PAIR}"), "d").await, PropertyValue::Float(2.0));
    let persigue = format!(r#"find shortestPath(a, b, edge_types = ["PERSIGUE"]) as d {PAIR}"#);
    assert_eq!(z.one(&persigue, "d").await, PropertyValue::Float(3.0));
    let none = r#"find shortestPath(a, b) as d from (a:Animal {nombre: "michi"}), (b:Animal {nombre: "firulais"})"#;
    assert_eq!(z.one(none, "d").await, PropertyValue::Float(-1.0));
    // El de UUIDs literales (feature `algorithms`) lo cubre
    // nql_regression_agg_algo_bool_test.
    Ok(())
}

/// Una fila por par: cada animal con su camino hasta el queso, también con
/// el `where`, y con un patrón de un salto en el FROM.
#[tokio::test]
async fn one_value_per_row() -> Result<()> {
    let z = zoo().await?;
    let r = z
        .g
        .execute_nql(
            r#"find a.nombre, shortestPath(a, b) as d from (a:Animal), (b:Animal {nombre: "queso"})
               where a.nombre != "queso" order by a.nombre"#,
        )
        .await?;
    let rows: Vec<(String, PropertyValue)> =
        r.rows().iter().map(|row| (row.get_string("a.nombre").unwrap(), row.get("d").cloned().unwrap())).collect();
    let f = PropertyValue::Float;
    let expected = [("firulais", f(-1.0)), ("michi", f(2.0)), ("mosca", f(1.0)), ("pajaro", f(2.0)), ("raton", f(1.0))]
        .map(|(n, d)| (n.to_string(), d));
    assert_eq!(rows, expected);

    let hop = z
        .one(r#"find path(a, c) as p from (a:Animal {nombre: "michi"})-[:PERSIGUE]->(b), (c:Animal {nombre: "queso"})"#, "p")
        .await;
    assert_eq!(field(&hop, "length"), &PropertyValue::Int(2));
    let one_hop = z.one(r#"find path(b, a, direction = "in") as p from (a:Animal {nombre: "michi"})-[:PERSIGUE]->(b)"#, "p").await;
    assert!(z.node_ids(&one_hop, &["pajaro", "michi"]), "{one_hop:?}");
    Ok(())
}

#[tokio::test]
async fn misuse_is_an_error_that_says_why() -> Result<()> {
    let z = zoo().await?;
    for (nql, says) in [
        (r#"find path(a, a) from (a:Animal {nombre: "michi"})"#.to_string(), "two node variables of the FROM"),
        (format!("find path(a, c) {PAIR}"), "`c` is not a node variable"),
        (format!("find path(a) {PAIR}"), "takes two node variables"),
        (format!("find path(a, b.nombre) {PAIR}"), "takes two node variables"),
        (format!(r#"find path(a, b, edge_types = "COME") {PAIR}"#), "`edge_types` must be a non-empty list"),
        (format!(r#"find path(a, b, direction = "up") {PAIR}"#), "`direction` must be"),
        (format!("find path(a, b, max_depth = -1) {PAIR}"), "`max_depth` must be an integer"),
        (format!("find path(a, b, weight = 1) {PAIR}"), "unknown option `weight`"),
        (format!("find a.nombre {PAIR} where shortestPath(a, b) > 1"), "go only in `find`"),
        (format!("find count(*), path(a, b) {PAIR}"), "not supported in aggregations"),
    ] {
        let err = z.g.execute_nql(&nql).await.expect_err(&nql).to_string();
        assert!(err.contains(says), "{nql}: {err}");
    }
    Ok(())
}
