// tests/nql_upsert_edges_test.rs
//
// `upsert` en NQL con relaciones (F3): cada nodo se encuentra o crea por su
// índice único y cada arista por (origen, tipo, destino), en una sola
// transacción. NQL ya puede conectar dos nodos existentes.

use nopaldb::index::IndexType;
use nopaldb::types::{Edge, NodeId};
use nopaldb::{Graph, NopalError, NqlResult, PropertyValue, Result, WriteResult};

async fn graph() -> Result<Graph> {
    let g = Graph::in_memory().await?;
    g.create_unique_index("Persona", "email", IndexType::Hash).await?;
    Ok(g)
}

async fn upsert(g: &Graph, nql: &str) -> Result<WriteResult> {
    match g.execute_statement(nql).await? {
        NqlResult::Write(w) => Ok(w),
        other => panic!("{other:?}"),
    }
}

async fn id_of(g: &Graph, email: &str) -> NodeId {
    let nodes = g.get_nodes_by_label("Persona").await.unwrap();
    let hits: Vec<_> = nodes
        .iter()
        .filter(|n| n.properties.get("email") == Some(&PropertyValue::String(email.into())))
        .collect();
    assert_eq!(hits.len(), 1, "{email}");
    hits[0].id
}

async fn edges(g: &Graph, from: &str, edge_type: &str) -> Vec<Edge> {
    let id = id_of(g, from).await;
    let mut out = g.get_outgoing_edges(id).await.unwrap();
    out.retain(|e| e.edge_type == edge_type);
    out
}

/// (created, updated, unchanged) of nodes, then (created, updated) of edges.
fn counts(w: &WriteResult) -> ((usize, usize, usize), (usize, usize)) {
    ((w.nodes_created, w.nodes_updated, w.nodes_unchanged), (w.edges_created, w.edges_updated))
}

const AMIGOS: &str = r#"upsert (a:Persona {email: "a@ejemplo.org", nombre: "Ana"})-[r:AMIGO_DE {desde: 2020}]->(b:Persona {email: "b@ejemplo.org"})"#;

#[tokio::test]
async fn creates_both_nodes_and_the_edge_then_writes_nothing() -> Result<()> {
    let g = graph().await?;
    let w = upsert(&g, AMIGOS).await?;
    assert_eq!(counts(&w), ((2, 0, 0), (1, 0)));
    assert_eq!(w.created_ids.len(), 2);

    let e = edges(&g, "a@ejemplo.org", "AMIGO_DE").await;
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].target, id_of(&g, "b@ejemplo.org").await);
    assert_eq!(e[0].properties["desde"], PropertyValue::Int(2020));

    let wal = g.wal_bytes().await;
    let w = upsert(&g, AMIGOS).await?;
    assert_eq!(counts(&w), ((0, 0, 2), (0, 0)));
    assert_eq!((w.matched_ids.len(), w.properties_changed), (2, 0));
    assert_eq!(g.wal_bytes().await, wal, "nothing changed, nothing logged");
    assert_eq!((g.node_count().await?, g.edge_count().await?), (2, 1));
    Ok(())
}

/// `add` always creates its nodes; `upsert` finds them, so it connects two
/// nodes that already exist without copying them.
#[tokio::test]
async fn connects_nodes_that_already_exist() -> Result<()> {
    let g = graph().await?;
    upsert(&g, r#"upsert (a:Persona {email: "a@ejemplo.org", nombre: "Ana"})"#).await?;
    upsert(&g, r#"upsert (b:Persona {email: "b@ejemplo.org", nombre: "Beto"})"#).await?;

    let w = upsert(&g, r#"upsert (a:Persona {email: "a@ejemplo.org"})-[:AMIGO_DE]->(b:Persona {email: "b@ejemplo.org"})"#).await?;
    assert_eq!(counts(&w), ((0, 0, 2), (1, 0)));
    assert_eq!((g.node_count().await?, g.edge_count().await?), (2, 1));
    // The rest of a found node's literal is not applied: it keeps its name.
    let a = g.get_node(id_of(&g, "a@ejemplo.org").await).await?;
    assert_eq!(a.properties["nombre"], PropertyValue::String("Ana".into()));
    Ok(())
}

#[tokio::test]
async fn set_reaches_the_edge_and_its_literal_only_applies_on_create() -> Result<()> {
    let g = graph().await?;
    let q = r#"upsert (a:Persona {email: "a@ejemplo.org"})-[r:VISITA {veces: 0, desde: 2020}]->(b:Persona {email: "b@ejemplo.org"}) set r.veces = r.veces + 1"#;
    upsert(&g, q).await?;
    let w = upsert(&g, &q.replace("2020", "1999")).await?;
    assert_eq!(counts(&w), ((0, 0, 2), (0, 1)));
    assert_eq!(w.properties_changed, 1);

    let e = edges(&g, "a@ejemplo.org", "VISITA").await;
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].properties["veces"], PropertyValue::Int(2));
    assert_eq!(e[0].properties["desde"], PropertyValue::Int(2020));

    // A `set` on a node and on the edge in one statement.
    let w = upsert(&g, r#"upsert (a:Persona {email: "a@ejemplo.org"})-[r:VISITA]->(b:Persona {email: "b@ejemplo.org"}) set a.activo = true, r.veces = 10"#).await?;
    assert_eq!(counts(&w), ((0, 1, 1), (0, 1)));
    Ok(())
}

#[tokio::test]
async fn the_arrow_says_which_way_the_edge_goes() -> Result<()> {
    let g = graph().await?;
    upsert(&g, r#"upsert (a:Persona {email: "a@ejemplo.org"})<-[:SIGUE_A]-(b:Persona {email: "b@ejemplo.org"})"#).await?;
    assert!(edges(&g, "a@ejemplo.org", "SIGUE_A").await.is_empty());
    let e = edges(&g, "b@ejemplo.org", "SIGUE_A").await;
    assert_eq!(e.len(), 1);
    assert_eq!(e[0].target, id_of(&g, "a@ejemplo.org").await);

    // The other direction is another edge.
    let w = upsert(&g, r#"upsert (a:Persona {email: "a@ejemplo.org"})-[:SIGUE_A]->(b:Persona {email: "b@ejemplo.org"})"#).await?;
    assert_eq!(counts(&w), ((0, 0, 2), (1, 0)));
    Ok(())
}

#[tokio::test]
async fn chains_and_reused_variables() -> Result<()> {
    let g = graph().await?;
    let w = upsert(
        &g,
        r#"upsert (a:Persona {email: "a@ejemplo.org"})-[:CONOCE]->(b:Persona {email: "b@ejemplo.org"})<-[:CONOCE]-(c:Persona {email: "c@ejemplo.org"})"#,
    )
    .await?;
    assert_eq!(counts(&w), ((3, 0, 0), (2, 0)));
    let b = id_of(&g, "b@ejemplo.org").await;
    assert_eq!(edges(&g, "c@ejemplo.org", "CONOCE").await[0].target, b);

    // `(a)` bare later in the pattern is the same node: a cycle of two edges.
    let w = upsert(
        &g,
        r#"upsert (a:Persona {email: "a@ejemplo.org"})-[:CONOCE]->(b:Persona {email: "b@ejemplo.org"})-[:CONOCE]->(a)"#,
    )
    .await?;
    assert_eq!(counts(&w), ((0, 0, 2), (1, 0)));
    assert_eq!(edges(&g, "b@ejemplo.org", "CONOCE").await[0].target, id_of(&g, "a@ejemplo.org").await);
    assert_eq!(g.node_count().await?, 3);
    Ok(())
}

/// Every error is raised before anything is written: a pattern is created
/// whole or not at all.
#[tokio::test]
async fn bad_patterns_write_nothing() -> Result<()> {
    let g = graph().await?;
    for bad in [
        // Each edge needs a direction and a type, and is one edge.
        r#"upsert (a:Persona {email: "a@ejemplo.org"})-[:AMIGO_DE]-(b:Persona {email: "b@ejemplo.org"})"#,
        r#"upsert (a:Persona {email: "a@ejemplo.org"})-[r]->(b:Persona {email: "b@ejemplo.org"})"#,
        r#"upsert (a:Persona {email: "a@ejemplo.org"})-[:AMIGO_DE]->{1,2}(b:Persona {email: "b@ejemplo.org"})"#,
        // Every node needs its key; the second one has no unique index.
        r#"upsert (a:Persona {email: "a@ejemplo.org"})-[:TRABAJA_EN]->(e:Empresa {rfc: "X1"})"#,
        r#"upsert (a:Persona {email: "a@ejemplo.org"})-[:AMIGO_DE]->(b:Persona)"#,
        // A reused variable is written bare.
        r#"upsert (a:Persona {email: "a@ejemplo.org"})-[:AMIGO_DE]->(a:Persona {email: "b@ejemplo.org"})"#,
        // `set` only on variables of the pattern, never on a key.
        r#"upsert (a:Persona {email: "a@ejemplo.org"})-[r:AMIGO_DE]->(b:Persona {email: "b@ejemplo.org"}) set c.x = 1"#,
        r#"upsert (a:Persona {email: "a@ejemplo.org"})-[r:AMIGO_DE]->(b:Persona {email: "b@ejemplo.org"}) set b.email = "c@ejemplo.org""#,
        // The same edge twice.
        r#"upsert (a:Persona {email: "a@ejemplo.org"})-[:AMIGO_DE]->(b:Persona {email: "b@ejemplo.org"})<-[:AMIGO_DE]-(a)"#,
    ] {
        match g.execute_statement(bad).await {
            Err(NopalError::SemanticError(_)) => {}
            other => panic!("{bad}: {other:?}"),
        }
    }
    assert_eq!((g.node_count().await?, g.edge_count().await?), (0, 0));
    Ok(())
}

#[tokio::test]
async fn ambiguous_edges_and_one_key_twice_are_errors() -> Result<()> {
    let g = graph().await?;
    upsert(&g, AMIGOS).await?;
    let (a, b) = (id_of(&g, "a@ejemplo.org").await, id_of(&g, "b@ejemplo.org").await);
    g.add_edge(Edge::new(a, b, "AMIGO_DE")).await?;
    assert!(matches!(g.execute_statement(AMIGOS).await, Err(NopalError::AmbiguousUpsertKey(_))));

    // Two variables with one key would be one node created twice.
    let e = g
        .execute_statement(r#"upsert (x:Persona {email: "c@ejemplo.org"})-[:AMIGO_DE]->(y:Persona {email: "c@ejemplo.org"})"#)
        .await;
    assert!(matches!(e, Err(NopalError::AmbiguousUpsertKey(_))), "{e:?}");
    assert_eq!(g.node_count().await?, 2);
    Ok(())
}

/// Every upsert takes the locks of both endpoints' keys, so concurrent
/// upserts of one pattern leave two nodes and one edge, and every `set`
/// counts.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_upserts_of_one_pattern_leave_one_edge() -> Result<()> {
    let g = graph().await?;
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let g = g.clone();
        tasks.push(tokio::spawn(async move {
            g.execute_statement(
                r#"upsert (a:Persona {email: "a@ejemplo.org"})-[r:VISITA {veces: 0}]->(b:Persona {email: "b@ejemplo.org"}) set r.veces = r.veces + 1"#,
            )
            .await
        }));
    }
    for t in tasks {
        t.await.unwrap()?;
    }
    assert_eq!((g.node_count().await?, g.edge_count().await?), (2, 1));
    let e = edges(&g, "a@ejemplo.org", "VISITA").await;
    assert_eq!(e[0].properties["veces"], PropertyValue::Int(16));
    Ok(())
}

#[tokio::test]
async fn explain_lists_each_node_and_edge() -> Result<()> {
    let g = graph().await?;
    let plan = match g.execute_statement(&format!("explain {AMIGOS} set r.visto = true")).await? {
        NqlResult::Explain(p) => p,
        other => panic!("{other:?}"),
    };
    assert!(plan.contains("node a: find by Persona.email (unique index Persona_email)"), "{plan}");
    assert!(plan.contains("node b: find by Persona.email"), "{plan}");
    assert!(plan.contains("edge r a -> b: find by (source, AMIGO_DE, target)"), "{plan}");
    assert_eq!(g.node_count().await?, 0, "explain does not write");
    Ok(())
}
