// tests/nql_upsert_test.rs
//
// `upsert` en NQL (F2): la identidad sale del índice único del label; el
// literal se escribe solo al crear, `set` siempre, y sin cambios no se
// escribe nada.

use nopaldb::index::IndexType;
use nopaldb::query::nql::parse;
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

async fn persona(g: &Graph, id: &str) -> std::collections::HashMap<String, PropertyValue> {
    g.get_node(id.parse().unwrap()).await.unwrap().properties
}

fn int(i: i64) -> PropertyValue {
    PropertyValue::Int(i)
}

fn text(s: &str) -> PropertyValue {
    PropertyValue::String(s.into())
}

#[tokio::test]
async fn creates_then_updates_then_writes_nothing() -> Result<()> {
    let g = graph().await?;
    let q = r#"upsert (p:Persona {email: "s@ejemplo.org", nombre: "Sergio", visitas: 0}) set p.visitas = p.visitas + 1"#;

    let w = upsert(&g, q).await?;
    assert_eq!((w.nodes_created, w.nodes_updated, w.nodes_unchanged), (1, 0, 0));
    let id = w.created_ids[0].clone();
    assert!(w.matched_ids.is_empty());
    assert_eq!(w.properties_changed, 3);
    let p = persona(&g, &id).await;
    assert_eq!((p["nombre"].clone(), p["visitas"].clone()), (text("Sergio"), int(1)));

    // Encontrado: solo `set`, sobre lo que ya tiene; el resto del literal no se aplica.
    let w = upsert(&g, &q.replace("Sergio", "Otro")).await?;
    assert_eq!((w.nodes_created, w.nodes_updated, w.nodes_unchanged), (0, 1, 0));
    assert_eq!(w.matched_ids, std::slice::from_ref(&id));
    assert_eq!(w.properties_changed, 1);
    let p = persona(&g, &id).await;
    assert_eq!((p["nombre"].clone(), p["visitas"].clone()), (text("Sergio"), int(2)));

    // Sin `set` y ya existe: nada que escribir, ni en el WAL.
    let wal = g.wal_bytes().await;
    let w = upsert(&g, r#"upsert (p:Persona {email: "s@ejemplo.org", nombre: "Otro"})"#).await?;
    assert_eq!((w.nodes_created, w.nodes_updated, w.nodes_unchanged), (0, 0, 1));
    assert_eq!((w.matched_ids, w.properties_changed), (vec![id.clone()], 0));
    // Un `set` que deja el mismo valor tampoco escribe.
    let w = upsert(&g, r#"upsert (p:Persona {email: "s@ejemplo.org"}) set p.nombre = "Sergio""#).await?;
    assert_eq!(w.nodes_unchanged, 1);
    assert_eq!(g.wal_bytes().await, wal);

    assert_eq!(g.get_nodes_by_label("Persona").await?.len(), 1);
    Ok(())
}

#[tokio::test]
async fn keeps_the_properties_it_does_not_set() -> Result<()> {
    let g = graph().await?;
    let w = upsert(&g, r#"upsert (p:Persona {email: "a@ejemplo.org", ciudad: "Oaxaca"})"#).await?;
    let id = w.created_ids[0].clone();
    upsert(&g, r#"upsert (p:Persona {email: "a@ejemplo.org"}) set p.edad = 30"#).await?;
    let p = persona(&g, &id).await;
    assert_eq!((p["ciudad"].clone(), p["edad"].clone()), (text("Oaxaca"), int(30)));
    Ok(())
}

#[tokio::test]
async fn needs_a_unique_index_and_a_usable_key() -> Result<()> {
    let g = graph().await?;
    let err = |r: Result<NqlResult>| match r {
        Err(e) => e,
        Ok(r) => panic!("expected an error, got {r:?}"),
    };

    // Sin índice único: el mensaje trae el arreglo.
    let e = err(g.execute_statement(r#"upsert (e:Empresa {rfc: "X1", nombre: "Y"})"#).await);
    assert!(e.to_string().contains("create unique index on Empresa(nombre)"), "{e}");
    // Un índice común no basta.
    g.create_index("Empresa", "rfc", IndexType::Hash).await?;
    assert!(matches!(
        g.execute_statement(r#"upsert (e:Empresa {rfc: "X1"})"#).await,
        Err(NopalError::SemanticError(_))
    ));

    for bad in [
        r#"upsert (p:Persona {email: null})"#,
        r#"upsert (p {email: "s@ejemplo.org"})"#,
        r#"upsert (p:Persona)"#,
        r#"upsert (p:Persona {email: "s@ejemplo.org"}) set p.email = "otro@ejemplo.org""#,
        r#"upsert (p:Persona {email: "s@ejemplo.org"}) set q.nombre = "x""#,
        r#"upsert (p:Persona {email: "s@ejemplo.org"}) set p.grado = degree(p)"#,
    ] {
        assert!(matches!(g.execute_statement(bad).await, Err(NopalError::SemanticError(_))), "{bad}");
    }
    // Relaciones: F3.
    assert!(matches!(
        g.execute_statement(r#"upsert (a:Persona {email: "a@ejemplo.org"})-[:AMIGO_DE]->(b:Persona {email: "b@ejemplo.org"})"#).await,
        Err(NopalError::QueryParseError(_))
    ));
    assert!(g.get_nodes_by_label("Persona").await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn two_keys_must_agree() -> Result<()> {
    let g = graph().await?;
    g.create_unique_index("Persona", "curp", IndexType::Hash).await?;
    let a = upsert(&g, r#"upsert (p:Persona {email: "a@ejemplo.org", curp: "A"})"#).await?.created_ids[0].clone();
    upsert(&g, r#"upsert (p:Persona {email: "b@ejemplo.org", curp: "B"})"#).await?;

    // Las dos claves llevan al mismo nodo, o solo una existe: ese nodo.
    let w = upsert(&g, r#"upsert (p:Persona {email: "a@ejemplo.org", curp: "A"}) set p.n = 1"#).await?;
    assert_eq!(w.matched_ids, std::slice::from_ref(&a));
    // Cada una a un nodo distinto: ambiguo.
    assert!(matches!(
        g.execute_statement(r#"upsert (p:Persona {email: "a@ejemplo.org", curp: "B"})"#).await,
        Err(NopalError::AmbiguousUpsertKey(_))
    ));
    Ok(())
}

/// Con el candado por clave de `upsert_node` y el reintento ante el índice
/// único, upserts concurrentes de una clave dejan un nodo, y cada uno suma.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_upserts_of_one_key_leave_one_node() -> Result<()> {
    let g = graph().await?;
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let g = g.clone();
        tasks.push(tokio::spawn(async move {
            g.execute_statement(r#"upsert (p:Persona {email: "s@ejemplo.org", visitas: 0}) set p.visitas = p.visitas + 1"#)
                .await
        }));
    }
    for t in tasks {
        t.await.unwrap()?;
    }
    let nodes = g.get_nodes_by_label("Persona").await?;
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0].properties["visitas"], int(16));
    Ok(())
}

#[tokio::test]
async fn is_a_write_explains_and_cannot_be_sketched() -> Result<()> {
    let g = graph().await?;
    assert!(parse(r#"upsert (p:Persona {email: "s@ejemplo.org"})"#)?.is_write());

    let plan = match g.execute_statement(r#"explain upsert (p:Persona {email: "s@ejemplo.org"}) set p.n = 1"#).await? {
        NqlResult::Explain(p) => p,
        other => panic!("{other:?}"),
    };
    assert!(plan.contains("Persona.email (unique index Persona_email)"), "{plan}");
    assert!(g.get_nodes_by_label("Persona").await?.is_empty(), "explain does not write");

    assert!(g.execute_statement(r#"sketch s = upsert (p:Persona {email: "s@ejemplo.org"})"#).await.is_err());
    Ok(())
}
