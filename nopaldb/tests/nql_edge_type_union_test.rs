// tests/nql_edge_type_union_test.rs
//
// Unión de tipos de arista en NQL (#414): `-[:A|B]->` es una arista de
// cualquiera de los tipos, con y sin cuantificador, en un salto, en cadena y
// en DELETE/UPDATE con patrón. ADD y UPSERT la rechazan: una arista nueva
// tiene un tipo. Hasta 0.11.2 `-[:A|B]->` era error de parseo.
//
// Dominio ficticio: un zoo. michi -COME-> raton, michi -PERSIGUE-> pajaro,
// michi -DUERME_CON-> firulais, raton -COME-> queso, pajaro -PERSIGUE-> mosca.

use nopaldb::query::nql::parser::ast::{PatternElement, Statement};
use nopaldb::query::nql::parser::parse;
use nopaldb::{Edge, Graph, Node, NqlResult, PropertyValue, Result};

fn types(nql: &str) -> (Option<String>, Vec<String>) {
    let stmt = parse(nql).unwrap_or_else(|e| panic!("{nql}: {e}"));
    let Statement::Query(q) = stmt else { panic!("{nql}: no es consulta") };
    q.from.patterns[0]
        .elements
        .iter()
        .find_map(|e| match e {
            PatternElement::Relationship(r) => Some((r.variable.clone(), r.rel_types.clone())),
            _ => None,
        })
        .unwrap()
}

#[test]
fn the_union_parses_in_every_form() {
    let ab = || vec!["A".to_string(), "B".to_string()];
    for (rel, variable, expected) in [
        ("-[:A]->", None, vec!["A".to_string()]),
        ("-[:A|B]->", None, ab()),
        ("-[:A|:B]->", None, ab()),
        ("-[r:A|B]->", Some("r"), ab()),
        ("-[r:A | B {w: 1}]->", Some("r"), ab()),
        ("<-[:A|B]-", None, ab()),
        ("-[:A|B]-{1,3}", None, ab()),
        ("-[:A|B|C]->{2}", None, vec!["A".into(), "B".into(), "C".into()]),
        ("-[r]->", Some("r"), vec![]),
        ("->", None, vec![]),
    ] {
        let nql = format!("find b from (a) {rel} (b)");
        assert_eq!(types(&nql), (variable.map(str::to_string), expected), "{rel}");
    }
    for bad in ["-[:A|]->", "-[:|B]->", "-[:A||B]->"] {
        assert!(parse(&format!("find b from (a) {bad} (b)")).is_err(), "{bad} parseó");
    }
}

async fn zoo() -> Result<Graph> {
    let g = Graph::in_memory().await?;
    let mut ids = std::collections::HashMap::new();
    for nombre in ["michi", "raton", "pajaro", "firulais", "mosca", "queso"] {
        let id = g.add_node(Node::new("Animal").with_property("nombre", PropertyValue::String(nombre.into()))).await?;
        ids.insert(nombre, id);
    }
    for (from, kind, to) in [
        ("michi", "COME", "raton"),
        ("michi", "PERSIGUE", "pajaro"),
        ("michi", "DUERME_CON", "firulais"),
        ("raton", "COME", "queso"),
        ("pajaro", "PERSIGUE", "mosca"),
    ] {
        g.add_edge(Edge::new(ids[from], ids[to], kind)).await?;
    }
    Ok(g)
}

async fn names(g: &Graph, nql: &str, col: &str) -> Vec<String> {
    let r = g.execute_nql(nql).await.unwrap_or_else(|e| panic!("{nql}: {e}"));
    let mut v: Vec<String> = r.rows().iter().map(|row| row.get_string(col).unwrap()).collect();
    v.sort();
    v
}

#[tokio::test]
async fn one_hop_follows_either_type() -> Result<()> {
    let g = zoo().await?;
    let q = r#"find b.nombre from (a:Animal {nombre: "michi"}) -[:COME|PERSIGUE]-> (b)"#;
    assert_eq!(names(&g, q, "b.nombre").await, ["pajaro", "raton"]);
    // El tipo de cada fila es el de su arista.
    let r = g
        .execute_nql(r#"find b.nombre, r.type from (a:Animal {nombre: "michi"}) -[r:COME|PERSIGUE]-> (b)"#)
        .await?;
    let mut rows: Vec<(String, String)> = r
        .rows()
        .iter()
        .map(|row| (row.get_string("b.nombre").unwrap(), row.get_string("r.type").unwrap()))
        .collect();
    rows.sort();
    assert_eq!(rows, [("pajaro".into(), "PERSIGUE".into()), ("raton".into(), "COME".into())]);
    // Un solo tipo sigue igual, y un tipo de la unión que no existe no estorba.
    let one = r#"find b.nombre from (a:Animal {nombre: "michi"}) -[:COME]-> (b)"#;
    assert_eq!(names(&g, one, "b.nombre").await, ["raton"]);
    let ghost = r#"find b.nombre from (a:Animal {nombre: "michi"}) -[:COME|VUELA]-> (b)"#;
    assert_eq!(names(&g, ghost, "b.nombre").await, ["raton"]);
    // Entrante.
    let inc = r#"find a.nombre from (b:Animal {nombre: "mosca"}) <-[:COME|PERSIGUE]- (a)"#;
    assert_eq!(names(&g, inc, "a.nombre").await, ["pajaro"]);
    Ok(())
}

#[tokio::test]
async fn chains_and_quantifiers_follow_either_type() -> Result<()> {
    let g = zoo().await?;
    let chain = r#"find c.nombre from (a:Animal {nombre: "michi"}) -[:COME|PERSIGUE]-> (b) -[:COME|PERSIGUE]-> (c)"#;
    assert_eq!(names(&g, chain, "c.nombre").await, ["mosca", "queso"]);
    // La unión en un salto y un tipo en el otro.
    let mixed = r#"find c.nombre from (a:Animal {nombre: "michi"}) -[:COME|PERSIGUE]-> (b) -[:PERSIGUE]-> (c)"#;
    assert_eq!(names(&g, mixed, "c.nombre").await, ["mosca"]);
    let path = r#"find b.nombre from (a:Animal {nombre: "michi"}) -[:COME|PERSIGUE]->{1,2} (b)"#;
    assert_eq!(names(&g, path, "b.nombre").await, ["mosca", "pajaro", "queso", "raton"]);
    let two = r#"find b.nombre from (a:Animal {nombre: "michi"}) -[:COME|PERSIGUE]->{2} (b)"#;
    assert_eq!(names(&g, two, "b.nombre").await, ["mosca", "queso"]);
    Ok(())
}

#[tokio::test]
async fn delete_with_a_union_removes_edges_of_either_type() -> Result<()> {
    let g = zoo().await?;
    let NqlResult::Write(w) = g
        .execute_statement(r#"delete (a:Animal) -[:COME|PERSIGUE]-> (b) where a.nombre = "michi""#)
        .await?
    else {
        panic!("delete no devolvió WriteResult")
    };
    assert_eq!(w.edges_deleted, 2, "{w:?}");
    let left = r#"find b.nombre from (a:Animal {nombre: "michi"}) -> (b)"#;
    assert_eq!(names(&g, left, "b.nombre").await, ["firulais"]);
    Ok(())
}

#[tokio::test]
async fn writes_reject_a_union() -> Result<()> {
    let g = Graph::in_memory().await?;
    let add = r#"add (a:Animal {nombre: "michi"}) -[:COME|PERSIGUE]-> (b:Animal {nombre: "raton"})"#;
    let err = g.execute_statement(add).await.expect_err(add).to_string();
    assert!(err.contains("ADD") && err.contains("`:COME|PERSIGUE` is a union"), "{err}");
    let count = g.execute_nql("find count(*) as n from (p:Animal)").await?;
    assert_eq!(count.rows()[0].get("n"), Some(&PropertyValue::Int(0)), "ADD no escribe a medias");

    g.execute_statement("create unique index on Animal(nombre)").await?;
    let upsert = r#"upsert (a:Animal {nombre: "michi"}) -[:COME|PERSIGUE]-> (b:Animal {nombre: "raton"})"#;
    let err = g.execute_statement(upsert).await.expect_err(upsert).to_string();
    assert!(err.contains("UPSERT") && err.contains("`:COME|PERSIGUE` is a union"), "{err}");
    Ok(())
}

#[tokio::test]
async fn explain_names_the_union() -> Result<()> {
    let g = zoo().await?;
    let explain = |nql: &'static str| {
        let g = g.clone();
        async move {
            match g.execute_statement(nql).await.unwrap() {
                NqlResult::Explain(plan) => plan,
                other => panic!("{nql}: {}", other.summary()),
            }
        }
    };
    let union = explain(r#"explain find b.nombre from (a:Animal {nombre: "michi"}) -[:COME|PERSIGUE]-> (b)"#).await;
    assert!(union.contains("PATTERN PIPELINE") && union.contains("Edge types: `:COME|PERSIGUE`"), "{union}");
    let single = explain(r#"explain find b.nombre from (a:Animal {nombre: "michi"}) -[:COME]-> (b)"#).await;
    assert!(!single.contains("Edge types"), "sin unión el plan no cambia: {single}");
    Ok(())
}
