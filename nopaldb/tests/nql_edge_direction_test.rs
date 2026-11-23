// tests/nql_edge_direction_test.rs
//
// Dirección de las relaciones en NQL (#395): `-[]-` y `--` son "cualquier
// dirección" al leer, las mezclas de puntas son error de parseo, y ADD crea
// la arista en el sentido de la flecha.

use nopaldb::query::nql::parser::ast::{Direction, PatternElement, Statement};
use nopaldb::query::nql::parser::parse;
use nopaldb::{Graph, NqlResult, Result};

fn direction(nql: &str) -> Direction {
    let stmt = parse(nql).unwrap_or_else(|e| panic!("{nql}: {e}"));
    let Statement::Query(q) = stmt else { panic!("{nql}: no es consulta") };
    q.from.patterns[0]
        .elements
        .iter()
        .find_map(|e| match e {
            PatternElement::Relationship(r) => Some(r.direction.clone()),
            _ => None,
        })
        .unwrap()
}

#[test]
fn every_arrow_has_one_meaning() {
    for (arrow, expected) in [
        ("-[]->", Direction::Outgoing),
        ("-[:K]->", Direction::Outgoing),
        ("->", Direction::Outgoing),
        ("-> [:K] ->", Direction::Outgoing),
        ("<-[]-", Direction::Incoming),
        ("<-[r:K]-", Direction::Incoming),
        ("<-", Direction::Incoming),
        ("<- [:K] <-", Direction::Incoming),
        ("<-[]->", Direction::Bidirectional),
        ("<->", Direction::Bidirectional),
        ("-[]-", Direction::Bidirectional),
        ("-[:K]-", Direction::Bidirectional),
        ("-[:K]-{1,2}", Direction::Bidirectional),
        ("--", Direction::Bidirectional),
    ] {
        let nql = format!("find b from (a) {arrow} (b)");
        assert_eq!(direction(&nql), expected, "{arrow}");
    }
}

#[test]
fn mixed_arrowheads_are_parse_errors() {
    for arrow in ["->[]<-", "->[]-", "-[]<-", "->[:K]<-", "-> [:K] -"] {
        let nql = format!("find b from (a) {arrow} (b)");
        let err = parse(&nql).err().unwrap_or_else(|| panic!("{arrow} parseó"));
        let msg = err.to_string();
        assert!(msg.contains("invalid relationship arrow") && msg.contains("-[]-"), "{arrow}: {msg}");
    }
}

/// a → b → c
async fn chain() -> Result<Graph> {
    let g = Graph::in_memory().await?;
    g.execute_statement(r#"add (a:P {name: "a"}) -[:K]-> (b:P {name: "b"}) -[:K]-> (c:P {name: "c"})"#)
        .await?;
    Ok(g)
}

async fn names(g: &Graph, nql: &str) -> Vec<String> {
    let r = g.execute_nql(nql).await.unwrap_or_else(|e| panic!("{nql}: {e}"));
    let mut v: Vec<String> = r.rows().iter().filter_map(|row| row.get_string("y.name")).collect();
    v.sort();
    v
}

#[tokio::test]
async fn undirected_reads_match_both_sides() -> Result<()> {
    let g = chain().await?;
    for (arrow, expected) in [
        ("-[]->", vec!["c"]),
        ("<-[]-", vec!["a"]),
        ("-> [:K] ->", vec!["c"]),
        ("<- [:K] <-", vec!["a"]),
        ("<-[]->", vec!["a", "c"]),
        ("-[]-", vec!["a", "c"]),
        ("-[:K]-", vec!["a", "c"]),
        ("--", vec!["a", "c"]),
    ] {
        let nql = format!(r#"find y.name from (x) {arrow} (y) where x.name = "b""#);
        assert_eq!(names(&g, &nql).await, expected, "{arrow}");
    }
    // Longitud variable: desde a, en cualquier dirección, b y c.
    let nql = r#"find distinct y.name from (x) -[:K]-{1,2} (y) where x.name = "a""#;
    assert_eq!(names(&g, nql).await, ["b", "c"]);
    Ok(())
}

/// Con los dos extremos libres, cada arista sale una vez por orientación,
/// como en Cypher: Sergio → Juan da (Sergio, Juan) y (Juan, Sergio).
#[tokio::test]
async fn undirected_pattern_with_both_ends_free_yields_each_orientation() -> Result<()> {
    let g = Graph::in_memory().await?;
    g.execute_statement(r#"add (s:P {name: "Sergio"}) -[:AMIGO_DE]-> (j:P {name: "Juan"})"#).await?;
    for arrow in ["-[:AMIGO_DE]-", "<-[:AMIGO_DE]->"] {
        let r = g.execute_nql(&format!("find a.name, b.name from (a:P) {arrow} (b:P)")).await?;
        let mut pairs: Vec<_> = r
            .rows()
            .iter()
            .map(|row| (row.get_string("a.name").unwrap(), row.get_string("b.name").unwrap()))
            .collect();
        pairs.sort();
        let expected = [("Juan", "Sergio"), ("Sergio", "Juan")].map(|(a, b)| (a.to_string(), b.to_string()));
        assert_eq!(pairs, expected, "{arrow}");
    }
    Ok(())
}

async fn edges(g: &Graph) -> Vec<(String, String)> {
    let r = g.execute_nql("find s.name, t.name from (s) -[:K]-> (t)").await.unwrap();
    let mut v: Vec<_> = r
        .rows()
        .iter()
        .map(|row| (row.get_string("s.name").unwrap(), row.get_string("t.name").unwrap()))
        .collect();
    v.sort();
    v
}

#[tokio::test]
async fn add_creates_the_edge_the_arrow_points_to() -> Result<()> {
    let g = Graph::in_memory().await?;
    g.execute_statement(r#"add (a:P {name: "a"}) -[:K]-> (b:P {name: "b"})"#).await?;
    g.execute_statement(r#"add (c:P {name: "c"}) <-[:K]- (d:P {name: "d"})"#).await?;
    // En cadena: e → f ← g.
    g.execute_statement(r#"add (e:P {name: "e"}) -[:K]-> (f:P {name: "f"}) <-[:K]- (g:P {name: "g"})"#)
        .await?;
    let expected = [("a", "b"), ("d", "c"), ("e", "f"), ("g", "f")]
        .map(|(s, t)| (s.to_string(), t.to_string()));
    assert_eq!(edges(&g).await, expected);
    Ok(())
}

#[tokio::test]
async fn add_without_a_single_direction_is_rejected() -> Result<()> {
    let g = Graph::in_memory().await?;
    for arrow in ["-[:K]-", "<-[:K]->"] {
        let nql = format!(r#"add (a:P {{name: "a"}}) {arrow} (b:P {{name: "b"}})"#);
        let err = g.execute_statement(&nql).await.err().unwrap_or_else(|| panic!("{arrow} escribió"));
        assert!(err.to_string().contains("ADD needs a direction"), "{arrow}: {err}");
    }
    // Nada se escribió a medias.
    let r = g.execute_nql("find count(*) as n from (p:P)").await?;
    assert_eq!(r.rows()[0].get("n"), Some(&nopaldb::PropertyValue::Int(0)));
    Ok(())
}

#[tokio::test]
async fn delete_and_update_accept_undirected_patterns() -> Result<()> {
    let g = chain().await?;
    let NqlResult::Write(w) = g
        .execute_statement(r#"update (x:P) -[:K]- (y:P) set y.vecino = true where x.name = "b""#)
        .await?
    else {
        panic!("update no devolvió WriteResult")
    };
    assert_eq!(w.nodes_updated, 2, "{w:?}");
    assert_eq!(names(&g, "find y.name from (y:P) where y.vecino = true").await, ["a", "c"]);

    // DELETE con patrón borra las aristas que casan: las dos de b.
    let NqlResult::Write(w) = g.execute_statement(r#"delete (x:P) -[:K]- (y:P) where x.name = "b""#).await?
    else {
        panic!("delete no devolvió WriteResult")
    };
    assert_eq!(w.edges_deleted, 2, "{w:?}");
    assert_eq!(edges(&g).await, Vec::<(String, String)>::new());
    Ok(())
}

/// The quantifier also goes on a short arrow (#415), with the meaning of its
/// form with empty brackets: `<->{1,2}` is `<-[]->{1,2}`. Until 0.11.0 only
/// the bracket form parsed.
#[tokio::test]
async fn short_arrows_take_a_quantifier() -> Result<()> {
    let g = chain().await?;
    for (short, long, from, expected) in [
        ("->{1,2}", "-[]->{1,2}", "a", vec!["b", "c"]),
        ("<-{1,2}", "<-[]-{1,2}", "c", vec!["a", "b"]),
        ("<->{1,2}", "<-[]->{1,2}", "b", vec!["a", "c"]),
        ("--{1,2}", "-[]-{1,2}", "a", vec!["b", "c"]),
        ("->{2}", "-[]->{2}", "a", vec!["c"]),
    ] {
        let q = |arrow: &str| format!(r#"find distinct y.name from (x) {arrow} (y) where x.name = "{from}""#);
        assert_eq!(names(&g, &q(short)).await, expected, "{short}");
        assert_eq!(names(&g, &q(long)).await, expected, "{long}");
        // No space between the arrow and its quantifier is the usual spelling.
        let tight = format!(r#"find distinct y.name from (x){short}(y) where x.name = "{from}""#);
        assert_eq!(names(&g, &tight).await, expected, "{tight}");
    }
    Ok(())
}

/// A Cypher length (`*1..2`) is not NQL; the parse error names the NQL form,
/// keeping the arrows and the relationship spec.
#[test]
fn cypher_length_gets_a_hint() {
    for (cypher, hint) in [
        ("-[:K*1..2]->", "`-[:K]->{1,2}`"),
        ("<-[:K*1..3]-", "`<-[:K]-{1,3}`"),
        ("-[*2]-", "`-[]-{2,2}`"),
        ("-[r:K*..3]->", "`-[r:K]->{1,3}`"),
        ("-[:K*]->", "needs an upper bound: `-[:K]->{1,5}`"),
    ] {
        let nql = format!("find b from (a){cypher}(b)");
        let err = parse(&nql).expect_err(&nql).to_string();
        assert!(err.contains("is Cypher, not NQL") && err.contains(hint), "{cypher}: {err}");
    }
    // Any other parse error is left alone.
    let err = parse("find b from (a)-[:K]->(b) wher").unwrap_err().to_string();
    assert!(!err.contains("hint"), "{err}");
}
