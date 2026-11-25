// tests/cypher_read_only_test.rs
//
// Cypher de solo lectura (feature `cypher`): `to_nql` da el NQL que se
// ejecuta, y `execute_cypher` da lo que Cypher daría. Cada caso fija las dos
// cosas: la traducción (la tabla de docs/en/CYPHER.md sale de aquí) y las
// filas.
//
// Dominio ficticio: un zoo.

use nopaldb::query::cypher::to_nql;
use nopaldb::types::{Node, PropertyValue};
use nopaldb::{Edge, Graph, Result};

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.into())
}

async fn zoo() -> Result<Graph> {
    let g = Graph::in_memory().await?;
    let animal = |nombre: &str, especie: &str, zona: &str| {
        Node::new("Animal").with_property("nombre", s(nombre)).with_property("especie", s(especie)).with_property("zona", s(zona))
    };
    let michi = g.add_node(animal("michi", "felino", "norte").with_property("edad", PropertyValue::Int(3))).await?;
    let tom = g.add_node(animal("tom", "felino", "norte").with_property("edad", PropertyValue::Int(5))).await?;
    let lola = g.add_node(animal("lola", "ave", "sur")).await?;
    let ana = g.add_node(Node::new("Cuidador").with_property("nombre", s("ana"))).await?;
    g.add_edge(Edge::new(ana, michi, "CUIDA")).await?;
    g.add_edge(Edge::new(ana, lola, "CUIDA")).await?;
    g.add_edge(Edge::new(michi, tom, "JUEGA_CON")).await?;
    g.add_edge(Edge::new(tom, lola, "JUEGA_CON")).await?;
    Ok(g)
}

/// Las filas de `cypher`, una cadena por fila con las columnas `cols`
/// separadas por `|`; ordenadas salvo que la consulta tenga ORDER BY.
async fn rows(g: &Graph, cypher: &str, cols: &[&str]) -> Vec<String> {
    let r = g.execute_cypher(cypher).await.unwrap_or_else(|e| panic!("{cypher}: {e}"));
    let mut out: Vec<String> = r
        .rows()
        .iter()
        .map(|row| {
            cols.iter()
                .map(|c| match row.get(c) {
                    Some(PropertyValue::String(v)) => v.clone(),
                    Some(PropertyValue::Int(i)) => i.to_string(),
                    Some(v) => format!("{v:?}"),
                    None => "-".into(),
                })
                .collect::<Vec<_>>()
                .join("|")
        })
        .collect();
    if !cypher.to_ascii_uppercase().contains("ORDER BY") {
        out.sort();
    }
    out
}

/// Las consultas de referencia: Cypher, el NQL que produce y sus filas.
#[tokio::test]
async fn canonical_queries_translate_and_run() -> Result<()> {
    let g = zoo().await?;
    let cases: &[(&str, &str, &[&str], &[&str])] = &[
        (
            "MATCH (a:Animal) WHERE a.edad > 3 RETURN a.nombre",
            "find a.nombre from (a:Animal) where a.edad > 3",
            &["a.nombre"],
            &["tom"],
        ),
        (
            "MATCH (c:Cuidador)-[:CUIDA]->(a) RETURN a.nombre ORDER BY a.nombre",
            "find a.nombre from (c:Cuidador)-[:CUIDA]->(a) order by a.nombre",
            &["a.nombre"],
            &["lola", "michi"],
        ),
        // Agrupación implícita: NQL agrupa por las columnas sin agregado.
        (
            "MATCH (a:Animal) RETURN a.zona, count(*) AS n ORDER BY n DESC",
            "find a.zona, count(*) as n from (a:Animal) order by n desc",
            &["a.zona", "n"],
            &["norte|2", "sur|1"],
        ),
        (
            "MATCH (a:Animal {nombre: 'michi'})-[:JUEGA_CON*1..2]->(b) RETURN b.nombre",
            "find b.nombre from (a:Animal {nombre: \"michi\"})-[:JUEGA_CON]->{1,2}(b)",
            &["b.nombre"],
            &["lola", "tom"],
        ),
        (
            "MATCH (a:Animal {nombre: 'tom'})-[:JUEGA_CON]-(b) RETURN b.nombre",
            "find b.nombre from (a:Animal {nombre: \"tom\"})-[:JUEGA_CON]-(b)",
            &["b.nombre"],
            &["lola", "michi"],
        ),
        (
            "MATCH (a:Animal)<-[:CUIDA]-(c) RETURN DISTINCT c.nombre",
            "find distinct c.nombre from (a:Animal)<-[:CUIDA]-(c)",
            &["c.nombre"],
            &["ana"],
        ),
        (
            "MATCH (c:Cuidador) MATCH (c)-[:CUIDA]->(a) WHERE a.especie = 'ave' RETURN c.nombre, a.nombre",
            "find c.nombre, a.nombre from (c:Cuidador), (c)-[:CUIDA]->(a) where a.especie = \"ave\"",
            &["c.nombre", "a.nombre"],
            &["ana|lola"],
        ),
        (
            "MATCH (a:Animal) RETURN a.nombre ORDER BY a.edad DESC, a.nombre SKIP 1 LIMIT 2",
            "find a.nombre from (a:Animal) order by a.edad desc, a.nombre limit 2 offset 1",
            &["a.nombre"],
            // Como en Cypher, los null van al final en ASC y al principio en
            // DESC: lola (sin edad), tom, michi.
            &["tom", "michi"],
        ),
        (
            "MATCH (a:Animal) WHERE a.especie IN ['ave', 'reptil'] OR a.edad <> 3 RETURN a.nombre",
            "find a.nombre from (a:Animal) where (a.especie in [\"ave\", \"reptil\"]) or (a.edad != 3)",
            &["a.nombre"],
            &["lola", "tom"],
        ),
        (
            "MATCH (a:Animal) RETURN min(a.edad) AS menor, max(a.edad) AS mayor, sum(a.edad) AS total",
            "find min(a.edad) as menor, max(a.edad) as mayor, sum(a.edad) as total from (a:Animal)",
            &["menor", "mayor", "total"],
            // Diferencia con Cypher: `sum` de enteros da Float en NQL.
            &["3|5|Float(8.0)"],
        ),
        (
            "MATCH (a:Animal WHERE a.zona = 'norte')-[r:JUEGA_CON]->(b) RETURN type(r) AS t, b.nombre",
            "find r.type as t, b.nombre from (a:Animal)-[r:JUEGA_CON]->(b) where a.zona = \"norte\"",
            &["t", "b.nombre"],
            &["JUEGA_CON|lola", "JUEGA_CON|tom"],
        ),
    ];
    for (cypher, nql, cols, expected) in cases {
        assert_eq!(to_nql(cypher)?, *nql, "{cypher}");
        assert_eq!(rows(&g, cypher, cols).await, *expected, "{cypher}");
    }
    Ok(())
}

/// `IS NULL` cubre la propiedad ausente, como en Cypher; `IS NOT NULL` no.
#[tokio::test]
async fn is_null_covers_a_missing_property() -> Result<()> {
    let g = zoo().await?;
    assert_eq!(rows(&g, "MATCH (a:Animal) WHERE a.edad IS NULL RETURN a.nombre", &["a.nombre"]).await, ["lola"]);
    assert_eq!(
        rows(&g, "MATCH (a:Animal) WHERE a.edad IS NOT NULL RETURN a.nombre", &["a.nombre"]).await,
        ["michi", "tom"]
    );
    Ok(())
}

/// Diferencia documentada: NQL tiene lógica de dos valores. Con `edad`
/// ausente, Cypher evalúa `NOT a.edad = 3` a null y descarta a lola; NQL la
/// incluye. Para pedir lo de Cypher: `a.edad IS NOT NULL AND NOT a.edad = 3`.
#[tokio::test]
async fn not_over_a_missing_property_differs_from_cypher() -> Result<()> {
    let g = zoo().await?;
    let not = "MATCH (a:Animal) WHERE NOT a.edad = 3 RETURN a.nombre";
    assert_eq!(rows(&g, not, &["a.nombre"]).await, ["lola", "tom"]);
    let guarded = "MATCH (a:Animal) WHERE a.edad IS NOT NULL AND NOT a.edad = 3 RETURN a.nombre";
    assert_eq!(rows(&g, guarded, &["a.nombre"]).await, ["tom"]);
    Ok(())
}

/// `RETURN a` da las columnas de `all(a)`; `id(a)` da el id de NopalDB.
#[tokio::test]
async fn whole_nodes_and_ids() -> Result<()> {
    let g = zoo().await?;
    let r = g.execute_cypher("MATCH (a:Cuidador) RETURN a, id(a) AS i").await?;
    assert_eq!(to_nql("MATCH (a:Cuidador) RETURN a, id(a) AS i")?, "find all(a), a.id as i from (a:Cuidador)");
    let row = &r.rows()[0];
    assert_eq!(row.get("a.nombre"), Some(&s("ana")));
    assert_eq!(row.get("a.label"), Some(&s("Cuidador")));
    assert_eq!(row.get("a.id"), row.get("i"));
    Ok(())
}

/// Un string pasa a NQL con sus comillas, barras y saltos de línea.
#[tokio::test]
async fn strings_keep_their_characters() -> Result<()> {
    let g = Graph::in_memory().await?;
    let raro = "di \"hola\"\\ adiós\n\tñandú 'x'";
    g.add_node(Node::new("Nota").with_property("texto", s(raro))).await?;
    let cypher = r#"MATCH (n:Nota) WHERE n.texto = 'di "hola"\\ adiós\n\tñandú \'x\'' RETURN count(*) AS c"#;
    assert_eq!(rows(&g, cypher, &["c"]).await, ["1"]);
    Ok(())
}

/// Las demás formas aceptadas: la traducción, y que NQL la acepte.
#[tokio::test]
async fn other_accepted_forms() -> Result<()> {
    let g = zoo().await?;
    for (cypher, nql) in [
        ("MATCH (a)-[*..3]->(b) RETURN count(b)", "find count(b) from (a)-[]->{1,3}(b)"),
        ("MATCH (a)-[:R*2]-(b) RETURN avg(b.x) AS m", "find avg(b.x) as m from (a)-[:R]-{2,2}(b)"),
        ("MATCH (a)-[:A|:B]->(b) RETURN b.x", "find b.x from (a)-[:A|B]->(b)"),
        ("MATCH (a) WHERE a.x = 0x1F AND a.y = -2.5 RETURN elementId(a)", "find a.id from (a) where (a.x = 31) and (a.y = -2.5)"),
        ("MATCH (a) RETURN * OFFSET 3 LIMIT 1", "find * from (a) limit 1 offset 3"),
        ("MATCH (a) WHERE NOT (a.x > 1 OR a.y < 2) RETURN a.x * (a.y + 1) AS z", "find a.x * (a.y + 1) as z from (a) where !((a.x > 1) or (a.y < 2))"),
    ] {
        assert_eq!(to_nql(cypher)?, nql, "{cypher}");
        g.execute_cypher(cypher).await.unwrap_or_else(|e| panic!("{cypher}: {e}"));
    }
    Ok(())
}

/// Lo que no tiene equivalente es un error que lo nombra, nunca una
/// traducción aproximada.
#[test]
fn unsupported_cypher_is_an_error() {
    for (cypher, says) in [
        ("CREATE (n:Animal {nombre: 'rex'})", "read-only"),
        ("MATCH (n) SET n.x = 1 RETURN n", "read-only"),
        ("MATCH (n) DETACH DELETE n", "read-only"),
        ("MERGE (n:Animal {nombre: 'rex'})", "read-only"),
        ("OPTIONAL MATCH (n) RETURN n.x", "OPTIONAL MATCH"),
        ("MATCH (n) WITH n RETURN n.x", "WITH"),
        ("UNWIND [1, 2] AS x RETURN x", "UNWIND"),
        ("MATCH (n) RETURN n.x UNION MATCH (m) RETURN m.x", "UNION"),
        ("MATCH (n) WHERE n.x = $p RETURN n.x", "parameter"),
        ("MATCH (n) WHERE n.nombre STARTS WITH 'mi' RETURN n.x", "operator"),
        ("MATCH (n) WHERE n.nombre =~ 'mi.*' RETURN n.x", "operator"),
        ("MATCH (n) RETURN count(DISTINCT n.x)", "DISTINCT inside an aggregate"),
        ("MATCH (n) RETURN toUpper(n.x)", "the function `toupper`"),
        ("MATCH (n:Animal:Felino) RETURN n.x", "several labels"),
        ("MATCH (n {x: 1 + 1}) RETURN n.x", "non-literal value"),
        ("MATCH p = (a)-[]->(b) RETURN a.x", "path variable"),
        ("MATCH (a)-[*]->(b) RETURN a.x", "at least 1 hop and an upper bound"),
        ("MATCH (a)-[*0..2]->(b) RETURN a.x", "at least 1 hop and an upper bound"),
        ("MATCH (a)-[*2..]->(b) RETURN a.x", "at least 1 hop and an upper bound"),
        ("MATCH (a)-[r*1..2]->(b) RETURN a.x", "variable-length"),
        ("MATCH (n) RETURN n.x SKIP 2", "SKIP without LIMIT"),
        ("MATCH (n) RETURN n AS m", "alias on a whole node"),
        ("MATCH (n) RETURN n, count(*)", "a whole node next to an aggregate"),
        ("MATCH (`un nodo`) RETURN `un nodo`.x", "this name"),
        ("MATCH (n)", "must end with RETURN"),
        ("MATCH (a), (b), shortestPath((a)-[*]-(b)) RETURN a.x", "path"),
        ("MATCH (n RETURN n", "Cypher:"),
    ] {
        let err = to_nql(cypher).expect_err(cypher).to_string();
        assert!(err.contains(says), "{cypher}: {err}");
    }
}
