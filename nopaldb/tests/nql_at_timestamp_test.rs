// tests/nql_at_timestamp_test.rs
//
// NQL `find … at timestamp <ts>` (#298): la consulta corre sobre el grafo
// válido en `ts`, materializado desde la historia MVCC. Hasta 0.10.1 la
// cláusula era un error con nombre (y hasta 0.9.3 devolvía el presente sin
// aviso). Dominio ficticio: una cocina con recetas e ingredientes.

use nopaldb::error::NopalError;
use nopaldb::mvcc::GCConfig;
use nopaldb::query::nql::NqlResult;
use nopaldb::{Edge, Graph, Node, PropertyValue};

async fn names(graph: &Graph, nql: &str) -> Vec<String> {
    let result = graph.execute_nql(nql).await.unwrap_or_else(|e| panic!("{nql}: {e}"));
    let mut out: Vec<String> = result
        .rows()
        .iter()
        .map(|r| match r.values.values().next() {
            Some(PropertyValue::String(s)) => s.clone(),
            Some(PropertyValue::Int(i)) => i.to_string(),
            other => format!("{other:?}"),
        })
        .collect();
    out.sort();
    out
}

async fn write(graph: &Graph, nql: &str) -> u64 {
    graph.execute_nql(nql).await.unwrap_or_else(|e| panic!("{nql}: {e}"));
    graph.last_commit_timestamp()
}

fn history_unavailable(r: nopaldb::Result<nopaldb::query::nql::QueryResult>) -> String {
    match r {
        Err(NopalError::HistoryUnavailable(m)) => m,
        other => panic!("expected HistoryUnavailable, got {other:?}"),
    }
}

/// El caso del issue: `update` y luego leer el valor viejo.
#[tokio::test]
async fn lee_el_valor_de_entonces() {
    let graph = Graph::in_memory().await.unwrap();
    let t1 = write(&graph, r#"add (r:Receta {nombre: "mole", paso: "v1"})"#).await;
    let t2 = write(&graph, r#"update (r:Receta) set r.paso = "v2" where r.nombre = "mole""#).await;
    assert!(t2 > t1);

    let q = |ts: u64| format!("find r.paso from (r:Receta) at timestamp {ts}");
    assert_eq!(names(&graph, &q(t1)).await, ["v1"]);
    assert_eq!(names(&graph, &q(t2)).await, ["v2"]);
    // Antes de crearla no existía.
    assert!(names(&graph, &q(t1 - 1)).await.is_empty());
    // Sin la cláusula, el presente.
    assert_eq!(names(&graph, "find r.paso from (r:Receta)").await, ["v2"]);
}

/// Un nodo borrado sigue en el pasado, con sus aristas; y una arista nueva
/// no aparece antes de crearse.
#[tokio::test]
async fn borrados_y_aristas_en_el_pasado() {
    let graph = Graph::in_memory().await.unwrap();
    let t1 = write(
        &graph,
        r#"add (r:Receta {nombre: "pozole"})-[:LLEVA {gramos: 500}]->(i:Ingrediente {nombre: "maiz"})"#,
    )
    .await;
    let t2 = write(
        &graph,
        r#"add (r:Receta {nombre: "tamal"})-[:LLEVA {gramos: 300}]->(i:Ingrediente {nombre: "masa"})"#,
    )
    .await;
    let t3 = write(&graph, r#"delete (i:Ingrediente) where i.nombre = "maiz""#).await;

    let q = |ts: u64| {
        format!("find i.nombre from (r:Receta)-[:LLEVA]->(i:Ingrediente) at timestamp {ts}")
    };
    assert_eq!(names(&graph, &q(t1)).await, ["maiz"]);
    assert_eq!(names(&graph, &q(t2)).await, ["maiz", "masa"]);
    assert_eq!(names(&graph, &q(t3)).await, ["masa"]);

    // Propiedades de la arista, también las de entonces.
    let g = format!("find e.gramos from (r:Receta)-[e:LLEVA]->(i:Ingrediente) at timestamp {t2}");
    assert_eq!(names(&graph, &g).await, ["300", "500"]);

    // Agregación sobre el pasado.
    let c = format!("find count(i) as n from (i:Ingrediente) at timestamp {t2}");
    assert_eq!(names(&graph, &c).await, ["2"]);
}

/// En el último commit (y después) es el estado actual.
#[tokio::test]
async fn en_el_ultimo_commit_es_el_presente() {
    let graph = Graph::in_memory().await.unwrap();
    write(&graph, r#"add (r:Receta {nombre: "sopa"})-[:LLEVA]->(i:Ingrediente {nombre: "fideo"})"#).await;
    write(&graph, r#"add (r:Receta {nombre: "arroz"})"#).await;
    let t = write(&graph, r#"update (r:Receta) set r.nombre = "arroz rojo" where r.nombre = "arroz""#).await;

    for q in [
        "find r.nombre from (r:Receta)",
        "find i.nombre from (r:Receta)-[:LLEVA]->(i:Ingrediente)",
        "find count(*) as n from (x)",
    ] {
        let now = names(&graph, q).await;
        assert_eq!(names(&graph, &format!("{q} at timestamp {t}")).await, now, "{q}");
        assert_eq!(names(&graph, &format!("{q} at timestamp {}", t + 1000)).await, now, "{q}");
    }
}

/// Lo cargado con el BulkLoader existe desde su lote, no antes.
#[tokio::test]
async fn bulk_desde_su_lote() {
    let graph = Graph::in_memory().await.unwrap();
    let t0 = write(&graph, r#"add (r:Receta {nombre: "flan"})"#).await;
    let a = Node::new("Ingrediente").with_property("nombre", PropertyValue::String("huevo".into()));
    let b = Node::new("Ingrediente").with_property("nombre", PropertyValue::String("leche".into()));
    let (ida, idb) = (a.id, b.id);
    let mut loader = graph.bulk_loader(10);
    loader.add_node(a).await.unwrap();
    loader.add_node(b).await.unwrap();
    loader.add_edge(Edge::new(ida, idb, "COMBINA")).await.unwrap();
    loader.finish().await.unwrap();
    let t1 = graph.last_commit_timestamp();
    assert!(t1 > t0);

    let q = |ts: u64| format!("find i.nombre from (i:Ingrediente) at timestamp {ts}");
    assert!(names(&graph, &q(t0)).await.is_empty());
    assert_eq!(names(&graph, &q(t1)).await, ["huevo", "leche"]);
    let e = format!("find b.nombre from (a:Ingrediente)-[:COMBINA]->(b:Ingrediente) at timestamp {t1}");
    assert_eq!(names(&graph, &e).await, ["leche"]);
}

/// Bajo el horizonte del GC la historia ya no está: error, no filas de menos.
#[tokio::test]
async fn bajo_el_horizonte_del_gc() {
    let graph = Graph::in_memory().await.unwrap();
    let t1 = write(&graph, r#"add (r:Receta {nombre: "atole", paso: "v1"})"#).await;
    write(&graph, r#"update (r:Receta) set r.paso = "v2" where r.paso = "v1""#).await;
    let t3 = write(&graph, r#"update (r:Receta) set r.paso = "v3" where r.paso = "v2""#).await;
    graph.gc(GCConfig::before_timestamp(t3)).await.unwrap();
    let horizon = graph.gc_horizon().unwrap();
    assert!(horizon > t1, "the GC should have collected something");

    let msg = history_unavailable(
        graph.execute_nql(&format!("find r.paso from (r:Receta) at timestamp {t1}")).await,
    );
    assert!(msg.contains("GC horizon"), "{msg}");
    assert_eq!(names(&graph, &format!("find r.paso from (r:Receta) at timestamp {t3}")).await, ["v3"]);
}

/// Una base adoptada (escrita sin versión antes de 0.10.1): antes del suelo
/// no se sabe qué había, así que es error con nombre.
#[tokio::test]
async fn bajo_el_suelo_de_una_base_adoptada() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cocina");
    let t_before;
    {
        let graph = Graph::open(&path).await.unwrap();
        t_before = write(&graph, r#"add (r:Receta {nombre: "chocolate"})"#).await;
        // Sin versión ni marca, como escribía una versión anterior.
        let viejo = Node::new("Receta").with_property("nombre", PropertyValue::String("champurrado".into()));
        graph.storage().insert_node(&viejo).await.unwrap();
        // Que el próximo open vuelva a adoptar.
        graph.storage().delete_meta(nopaldb::storage::META_HISTORY_ADOPTED).await.unwrap();
        graph.close().await.unwrap();
    }
    let graph = Graph::open(&path).await.unwrap();
    let floor = graph.last_commit_timestamp();
    assert!(floor > t_before);

    let msg = history_unavailable(
        graph.execute_nql(&format!("find r.nombre from (r:Receta) at timestamp {t_before}")).await,
    );
    assert!(msg.contains("history starts at"), "{msg}");
    assert_eq!(
        names(&graph, &format!("find r.nombre from (r:Receta) at timestamp {floor}")).await,
        ["champurrado", "chocolate"]
    );
}

/// Un nodo escrito con una primitiva de bajo nivel no tiene historia: el
/// grafo de entonces no se conoce.
#[tokio::test]
async fn sin_historia_es_error() {
    let graph = Graph::in_memory().await.unwrap();
    let t = write(&graph, r#"add (r:Receta {nombre: "caldo"})"#).await;
    let suelto = Node::new("Receta").with_property("nombre", PropertyValue::String("suelto".into()));
    graph.storage().insert_node(&suelto).await.unwrap();
    let msg = history_unavailable(graph.execute_nql(&format!("find r.nombre from (r:Receta) at timestamp {t}")).await);
    assert!(msg.contains("no history"), "{msg}");
}

/// Lo que lee datos sin versión (vectores, texto completo, comunidades) es
/// un error con nombre, no el estado actual.
#[tokio::test]
async fn funciones_sin_historia_rechazadas() {
    let graph = Graph::in_memory().await.unwrap();
    let t = write(&graph, r#"add (r:Receta {nombre: "salsa"})"#).await;
    for (q, f) in [
        (format!(r#"find r.nombre from (r:Receta) where similar_to(r, "ref") at timestamp {t}"#), "similar_to"),
        (
            format!(r#"find r.nombre from (r:Receta) where hybrid(r, "texto", "ref", "m") at timestamp {t}"#),
            "hybrid",
        ),
        (format!("find r.nombre from (r:Receta) where has_embedding(r) at timestamp {t}"), "has_embedding"),
        (format!("find score(r) from (r:Receta) at timestamp {t}"), "score"),
    ] {
        match graph.execute_nql(&q).await {
            Err(NopalError::SemanticError(m)) => {
                assert!(m.contains(f) && m.contains("AT TIMESTAMP"), "{q}: {m}")
            }
            other => panic!("{q}: expected SemanticError, got {other:?}"),
        }
    }
}

/// EXPLAIN dice que materializó y cuánto; el export corre sobre el pasado.
#[tokio::test]
async fn explain_y_export() {
    let graph = Graph::in_memory().await.unwrap();
    let t1 = write(&graph, r#"add (r:Receta {nombre: "taco"})-[:LLEVA]->(i:Ingrediente {nombre: "tortilla"})"#).await;
    write(&graph, r#"add (r:Receta {nombre: "gordita"})"#).await;

    let NqlResult::Explain(plan) = graph
        .execute_statement(&format!("explain find r.nombre from (r:Receta) at timestamp {t1}"))
        .await
        .unwrap()
    else {
        panic!("expected an EXPLAIN")
    };
    assert!(plan.contains(&format!("materialized at timestamp {t1} (2 nodes, 1 edges)")), "{plan}");

    let NqlResult::Profile(profile) = graph
        .execute_statement(&format!("profile find r.nombre from (r:Receta) at timestamp {t1}"))
        .await
        .unwrap()
    else {
        panic!("expected a PROFILE")
    };
    assert_eq!(profile.rows_returned, 1);
    assert!(profile.plan.contains("Time travel"), "{}", profile.plan);

    let NqlResult::Export { data, rows_exported, .. } = graph
        .execute_statement(&format!("find r.nombre from (r:Receta) at timestamp {t1} export json"))
        .await
        .unwrap()
    else {
        panic!("expected an export")
    };
    assert_eq!(rows_exported, 1);
    assert!(data.contains("taco") && !data.contains("gordita"), "{data}");
}

/// Un `Executor` solo corre sobre el grafo actual: con la cláusula puesta
/// es error, nunca el presente.
#[tokio::test]
async fn executor_suelto_no_ignora_la_clausula() {
    use nopaldb::query::nql::parser::ast::Statement;
    use nopaldb::query::nql::{parse, Executor};
    let graph = Graph::in_memory().await.unwrap();
    write(&graph, r#"add (r:Receta {nombre: "esquite"})"#).await;
    let Statement::Query(q) = parse("find r.nombre from (r:Receta) at timestamp 1").unwrap() else {
        panic!()
    };
    match Executor::new(&graph).execute(q).await {
        Err(NopalError::SemanticError(m)) => assert!(m.contains("AT TIMESTAMP"), "{m}"),
        other => panic!("expected SemanticError, got {other:?}"),
    }
}

/// `instanceOf` en el pasado: la taxonomía se reconstruye sobre el grafo
/// materializado (si no, daría `false` sin aviso), con los prefijos de la
/// base para nombrar la clase como `prefijo:Local`.
#[cfg(all(feature = "reasoner", feature = "owl-import"))]
#[tokio::test]
async fn instanceof_en_el_pasado() {
    const HUERTO: &str = r#"
@prefix owl:   <http://www.w3.org/2002/07/owl#> .
@prefix rdfs:  <http://www.w3.org/2000/01/rdf-schema#> .
@prefix huerto: <http://huerto.example/clases#> .
@prefix :      <http://huerto.example/> .

huerto:Planta   a owl:Class .
huerto:Hortaliza a owl:Class ; rdfs:subClassOf huerto:Planta .
:chile   a huerto:Hortaliza .
:nopal   a huerto:Planta .
"#;
    let graph = Graph::in_memory().await.unwrap();
    graph.import_turtle(HUERTO).await.unwrap();
    let t1 = graph.last_commit_timestamp();
    graph
        .import_turtle(
            "@prefix huerto: <http://huerto.example/clases#> .\n\
             @prefix : <http://huerto.example/> .\n\
             :jitomate a huerto:Hortaliza .\n",
        )
        .await
        .unwrap();
    let t2 = graph.last_commit_timestamp();

    let q = |ts: u64| {
        format!(r#"find n.iri from (n) where instanceOf(n, "huerto:Planta") at timestamp {ts}"#)
    };
    let iri = |s: &str| format!("http://huerto.example/{s}");
    assert_eq!(names(&graph, &q(t1)).await, [iri("chile"), iri("nopal")]);
    assert_eq!(names(&graph, &q(t2)).await, [iri("chile"), iri("jitomate"), iri("nopal")]);
    // Igual que la consulta actual en el último commit.
    assert_eq!(
        names(&graph, &q(t2)).await,
        names(&graph, r#"find n.iri from (n) where instanceOf(n, "huerto:Planta")"#).await
    );
}
