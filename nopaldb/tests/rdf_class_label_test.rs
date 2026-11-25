// tests/rdf_class_label_test.rs
//
// #411: al importar Turtle, la clase `zoo:Felino` lleva el label `Felino`,
// el mismo que sus instancias. Desde 0.11.5 un patrón con label `(n:Felino)`
// no admite nodos de clase (`NodeKind::Class`), salvo que la consulta
// pregunte por la clase de esa variable con `is_class(n)` o
// `subClassOf(n, …)`; `get_label_count` y `nodes_per_label` cuentan igual.
// Cada test recorre una ruta de lectura o escritura distinta.
//
// Dominio ficticio: un zoo.

use nopaldb::types::{Node, NodeKind, PropertyValue};
use nopaldb::{Edge, Graph, NqlResult, Result};

const ZOO: &str = r#"
@prefix owl:  <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix zoo:  <http://zoo.example/> .
zoo:Animal a owl:Class .
zoo:Felino a owl:Class ; rdfs:subClassOf zoo:Animal .
zoo:michi a zoo:Felino .
zoo:tom   a zoo:Felino .
zoo:michi zoo:juegaCon zoo:tom .
"#;

const FELINO: &str = "http://zoo.example/Felino";
const MICHI: &str = "http://zoo.example/michi";
const TOM: &str = "http://zoo.example/tom";

async fn zoo() -> Result<Graph> {
    let g = Graph::in_memory().await?;
    g.import_turtle(ZOO).await?;
    Ok(g)
}

async fn col(g: &Graph, nql: &str, c: &str) -> Vec<String> {
    let r = g.execute_nql(nql).await.unwrap_or_else(|e| panic!("{nql}: {e}"));
    let mut v: Vec<String> = r.rows().iter().map(|row| row.get_string(c).unwrap_or_default()).collect();
    v.sort();
    v
}

async fn class_id(g: &Graph) -> String {
    let ids = g.get_all_nodes_by_property("iri", &PropertyValue::String(FELINO.into())).await.unwrap();
    ids[0].to_string()
}

#[tokio::test]
async fn a_labeled_pattern_reads_the_instances() -> Result<()> {
    let g = zoo().await?;
    assert_eq!(col(&g, "find n.iri from (n:Felino)", "n.iri").await, [MICHI, TOM]);
    let count = g.execute_nql("find count(*) as c from (n:Felino)").await?;
    assert_eq!(count.rows()[0].get("c"), Some(&PropertyValue::Int(2)));
    assert_eq!(g.get_label_count("Felino").await?, 2);
    assert_eq!(g.get_stats().await?.nodes_per_label.get("Felino"), Some(&2));
    // Sin label nada cambia: la clase sigue ahí.
    let unlabeled = format!(r#"find n.iri from (n) where n.iri = "{FELINO}""#);
    assert_eq!(col(&g, &unlabeled, "n.iri").await, [FELINO]);
    // La lectura cruda de Rust sí la incluye.
    assert_eq!(g.get_nodes_by_label("Felino").await?.len(), 3);
    Ok(())
}

#[tokio::test]
async fn is_class_and_sub_class_of_ask_for_the_class() -> Result<()> {
    let g = zoo().await?;
    assert_eq!(col(&g, "find n.iri from (n:Felino) where is_class(n)", "n.iri").await, [FELINO]);
    assert_eq!(col(&g, "find n.iri from (n:Felino) where not is_class(n)", "n.iri").await, [MICHI, TOM]);
    let either = format!(r#"find n.iri from (n:Felino) where is_class(n) or n.iri = "{MICHI}""#);
    assert_eq!(col(&g, &either, "n.iri").await, [FELINO, MICHI]);
    // subClassOf ya pide nodos de clase: sigue encontrando la clase por su label.
    let sub = r#"find c.iri from (c:Felino) where subClassOf(c, "zoo:Animal")"#;
    assert_eq!(col(&g, sub, "c.iri").await, [FELINO]);
    Ok(())
}

/// Las rutas rápidas del patrón de un nodo: búsqueda por id y por propiedad.
#[tokio::test]
async fn id_and_property_lookups_follow_the_rule() -> Result<()> {
    let g = zoo().await?;
    let id = class_id(&g).await;
    assert!(col(&g, &format!(r#"find n.iri from (n:Felino) where n.id = "{id}""#), "n.iri").await.is_empty());
    let by_id = format!(r#"find n.iri from (n:Felino) where n.id = "{id}" and is_class(n)"#);
    assert_eq!(col(&g, &by_id, "n.iri").await, [FELINO]);
    g.execute_statement("create index on Felino(iri)").await?;
    let by_iri = format!(r#"find n.iri from (n:Felino) where n.iri = "{FELINO}""#);
    assert!(col(&g, &by_iri, "n.iri").await.is_empty());
    let inline = format!(r#"find n.iri from (n:Felino {{iri: "{FELINO}"}})"#);
    assert!(col(&g, &inline, "n.iri").await.is_empty());
    let both = format!(r#"find n.iri from (n:Felino) where n.iri in ["{FELINO}", "{MICHI}"]"#);
    assert_eq!(col(&g, &both, "n.iri").await, [MICHI]);
    Ok(())
}

/// Patrones con aristas: el origen, el destino de un salto, dos saltos y
/// varios patrones.
#[tokio::test]
async fn patterns_with_edges_follow_the_rule() -> Result<()> {
    let g = zoo().await?;
    // michi -instanceOf-> Felino: el destino con label es la clase.
    assert!(col(&g, "find c.iri from (i:Felino)-[:instanceOf]->(c:Felino)", "c.iri").await.is_empty());
    let asked = "find c.iri from (i:Felino)-[:instanceOf]->(c:Felino) where is_class(c)";
    assert_eq!(col(&g, asked, "c.iri").await, [FELINO, FELINO]);
    assert_eq!(col(&g, "find c.iri from (i:Felino)-[:instanceOf]->(c)", "c.iri").await, [FELINO, FELINO]);
    // El origen de un salto: Felino -subClassOf-> Animal sale de la clase.
    assert!(col(&g, "find b.iri from (a:Felino)-[:subClassOf]->(b)", "b.iri").await.is_empty());
    let from_class = "find b.iri from (a:Felino)-[:subClassOf]->(b) where is_class(a)";
    assert_eq!(col(&g, from_class, "b.iri").await, ["http://zoo.example/Animal"]);
    // El origen: la clase no tiene aristas juegaCon, las instancias sí.
    assert_eq!(col(&g, "find b.iri from (a:Felino)-[:juegaCon]->(b:Felino)", "b.iri").await, [TOM]);
    // Dos saltos (ruta lineal) y varios patrones.
    let two = "find c.iri from (a:Felino)-[:juegaCon]->(b:Felino)-[:instanceOf]->(c:Felino)";
    assert!(col(&g, two, "c.iri").await.is_empty());
    let r = g.execute_nql("find a.iri, b.iri from (a:Felino), (b:Felino)").await?;
    assert_eq!(r.rows().len(), 4, "2 × 2 instancias, sin la clase");
    Ok(())
}

#[tokio::test]
async fn delete_and_update_leave_the_class_alone() -> Result<()> {
    let g = zoo().await?;
    let NqlResult::Write(w) = g.execute_statement("update (n:Felino) set n.visto = true").await? else {
        panic!("update")
    };
    assert_eq!(w.nodes_updated, 2, "{w:?}");
    let NqlResult::Write(w) = g.execute_statement("delete (n:Felino)-[:juegaCon]->(m:Felino)").await? else {
        panic!("delete")
    };
    assert_eq!(w.edges_deleted, 1, "{w:?}");
    let NqlResult::Write(w) = g.execute_statement("delete (n:Felino)").await? else { panic!("delete") };
    assert_eq!(w.nodes_deleted, 2, "{w:?}");
    assert_eq!(col(&g, "find n.iri from (n:Felino) where is_class(n)", "n.iri").await, [FELINO]);
    assert_eq!(g.get_label_count("Felino").await?, 0);
    // Pedida, la clase también se borra.
    let NqlResult::Write(w) = g.execute_statement("delete (n:Felino) where is_class(n)").await? else {
        panic!("delete")
    };
    assert_eq!(w.nodes_deleted, 1, "{w:?}");
    assert!(g.get_nodes_by_label("Felino").await?.is_empty());
    Ok(())
}

/// La regla es por `NodeKind`, no por el importador; y el conteo de clases
/// sobrevive a reabrir la base.
#[tokio::test]
async fn hand_made_classes_and_reopening() -> Result<()> {
    let dir = tempfile::tempdir().unwrap();
    {
        let g = Graph::open(dir.path()).await?;
        let mut perro = Node::new("Perro");
        perro.kind = NodeKind::Class;
        let clase = g.add_node(perro).await?;
        let firulais = g.add_node(Node::new("Perro").with_property("nombre", PropertyValue::String("firulais".into()))).await?;
        g.add_edge(Edge::new(firulais, clase, "instanceOf")).await?;
        assert_eq!(col(&g, "find n.nombre from (n:Perro)", "n.nombre").await, ["firulais"]);
        assert_eq!(g.get_label_count("Perro").await?, 1);
        g.close().await?;
    }
    let g = Graph::open(dir.path()).await?;
    assert_eq!(g.get_label_count("Perro").await?, 1);
    assert_eq!(col(&g, "find count(*) as c from (n:Perro) where is_class(n)", "c").await.len(), 1);
    g.close().await?;
    Ok(())
}

#[tokio::test]
async fn is_class_misuse_is_an_error() -> Result<()> {
    let g = zoo().await?;
    for (nql, says) in [
        ("find is_class(n) from (n:Felino)", "goes in `where`"),
        ("find n.iri from (n:Felino) order by is_class(n)", "goes in `where`"),
        (r#"find n.iri from (n:Felino) where is_class(n.iri)"#, "takes one node variable"),
        (r#"find n.iri from (n:Felino) where is_class()"#, "takes one node variable"),
    ] {
        let err = g.execute_nql(nql).await.expect_err(nql).to_string();
        assert!(err.contains(says), "{nql}: {err}");
    }
    Ok(())
}
