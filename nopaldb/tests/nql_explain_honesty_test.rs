// EXPLAIN honesto: el plan reportado sale de la MISMA decisión que despacha
// la ejecución (Executor::index_fast_path_decision). Esta matriz cubre las
// divergencias que existían cuando EXPLAIN y ejecución eran caminos
// independientes — cada caso asserta que el texto del plan describe la
// estrategia que la ejecución realmente toma.

use nopaldb::{Graph, Node, NqlResult, Result};

async fn graph_with_people() -> Result<Graph> {
    let graph = Graph::in_memory().await?;
    for (name, age) in [("Ana", 30i64), ("Beto", 25), ("Cata", 35)] {
        graph
            .add_node(Node::new("Person").with_property("name", name).with_property("age", age))
            .await?;
    }
    Ok(graph)
}

async fn explain(graph: &Graph, q: &str) -> Result<String> {
    match graph.execute_statement(q).await? {
        NqlResult::Explain(plan) => Ok(plan),
        other => panic!("Expected Explain, got: {}", other.summary()),
    }
}

#[tokio::test]
async fn eq_with_index_reports_seek_even_on_small_label() -> Result<()> {
    // Divergencia #7 (invertida) resuelta: el planner de costos habría
    // preferido scan en un label de 3 nodos, pero la ejecución SIEMPRE usa
    // el índice cuando el fast-path aplica — EXPLAIN ahora lo dice.
    let graph = graph_with_people().await?;
    graph.execute_statement("create index on Person(name) type hash").await?;

    let plan = explain(&graph, r#"explain find p.name from (p:Person) where p.name = "Ana""#).await?;
    assert!(plan.contains("INDEX SEEK"), "plan: {plan}");
    assert!(plan.contains("Person_name"), "plan: {plan}");

    // Y la consulta ejecuta correcto por ese camino
    let rows = graph.execute_nql(r#"find p.name from (p:Person) where p.name = "Ana""#).await?;
    assert_eq!(rows.rows().len(), 1);
    Ok(())
}

#[tokio::test]
async fn range_predicate_reports_scan_not_seek() -> Result<()> {
    // Divergencia #1 resuelta: antes EXPLAIN ignoraba el operador y
    // reportaba IndexSeek para rangos; la ejecución solo tiene fast-path Eq.
    let graph = graph_with_people().await?;
    graph.execute_statement("create index on Person(age) type btree").await?;

    let plan = explain(&graph, "explain find p.name from (p:Person) where p.age > 26").await?;
    assert!(plan.contains("LABEL SCAN"), "plan: {plan}");
    assert!(!plan.contains("INDEX SEEK"), "plan: {plan}");

    let rows = graph.execute_nql("find p.name from (p:Person) where p.age > 26").await?;
    assert_eq!(rows.rows().len(), 2); // Ana(30), Cata(35)
    Ok(())
}

#[tokio::test]
async fn no_index_reports_scan_with_reason() -> Result<()> {
    let graph = graph_with_people().await?;

    let plan = explain(&graph, r#"explain find p.name from (p:Person) where p.name = "Ana""#).await?;
    assert!(plan.contains("LABEL SCAN"), "plan: {plan}");
    assert!(plan.contains("no existe índice"), "plan: {plan}");
    Ok(())
}

#[cfg(feature = "fulltext")]
#[tokio::test]
async fn fulltext_index_reports_scan_for_equality() -> Result<()> {
    // Divergencia #5 resuelta: un índice fulltext con predicado `=` truena
    // en runtime y cae a scan; antes EXPLAIN lo reportaba como seek porque
    // solo compilaba el nombre contra la metadata.
    let graph = graph_with_people().await?;
    graph.execute_statement("create index on Person(name) type fulltext").await?;

    let plan = explain(&graph, r#"explain find p.name from (p:Person) where p.name = "Ana""#).await?;
    assert!(plan.contains("LABEL SCAN"), "plan: {plan}");
    assert!(plan.contains("FullText"), "plan: {plan}");
    Ok(())
}

#[tokio::test]
async fn no_where_reports_scan() -> Result<()> {
    let graph = graph_with_people().await?;
    let plan = explain(&graph, "explain find p.name from (p:Person)").await?;
    assert!(plan.contains("LABEL SCAN"), "plan: {plan}");
    assert!(plan.contains("WHERE"), "plan: {plan}");
    Ok(())
}

#[tokio::test]
async fn unlabeled_query_explains_as_full_scan_instead_of_error() -> Result<()> {
    // Divergencia #9 resuelta: EXPLAIN erroraba («Node has no label») en
    // queries sin label que ejecutan perfectamente.
    let graph = graph_with_people().await?;
    let plan = explain(&graph, "explain find n from (n)").await?;
    assert!(plan.contains("FULL SCAN"), "plan: {plan}");

    let rows = graph.execute_nql("find n from (n)").await?;
    assert_eq!(rows.rows().len(), 3);
    Ok(())
}

#[tokio::test]
async fn relationship_pattern_reports_pipeline() -> Result<()> {
    // Divergencia #4 resuelta: el fast-path se apaga con relaciones; antes
    // EXPLAIN solo miraba el nodo ancla y podía afirmar un seek.
    let graph = graph_with_people().await?;
    graph.execute_statement("create index on Person(name) type hash").await?;

    let plan = explain(
        &graph,
        r#"explain find p.name, q.name from (p:Person)-[:KNOWS]->(q:Person) where p.name = "Ana""#,
    )
    .await?;
    assert!(plan.contains("PATTERN PIPELINE"), "plan: {plan}");
    assert!(!plan.contains("INDEX SEEK"), "plan: {plan}");
    Ok(())
}

#[tokio::test]
async fn explain_of_write_gives_clear_message_not_ast_dump() -> Result<()> {
    // Divergencia #10 resuelta: antes regresaba el Debug dump del AST.
    // La gramática NQL no acepta `explain <write>`, así que este camino solo
    // es alcanzable por API: parsear el write y pasarlo a execute_explain.
    let graph = graph_with_people().await?;
    let stmt = nopaldb::parse(r#"add (n:Person {name: "Dora"})"#)?;
    let executor = nopaldb::Executor::new(&graph);
    let plan = executor.execute_explain(stmt).await?;
    assert!(plan.contains("plan de lectura no disponible"), "plan: {plan}");
    assert!(plan.contains("ADD"), "plan: {plan}");
    assert!(!plan.contains("Add(AddStmt"), "no debe haber Debug dump: {plan}");
    Ok(())
}
