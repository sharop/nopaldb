// tests/unique_index_test.rs
//
// Índices únicos (M1-8): ningún camino de escritura deja dos nodos de una
// etiqueta con el mismo valor, el chequeo ocurre antes del WAL, y la marca
// sobrevive a reabrir sin cambiar `metadata.bin`. También: `create unique
// index` en NQL, y `upsert` que reintenta cuando pierde la clave.

use std::collections::{HashMap, HashSet};

use nopaldb::index::IndexType;
use nopaldb::{
    Graph, LinkSpec, Node, NodeId, NopalError, NqlResult, PropertyValue, Result, UpsertOutcome, UpsertRequest,
};

fn persona(email: &str) -> Node {
    Node::new("Persona").with_property("email", email)
}

fn is_violation<T: std::fmt::Debug>(r: Result<T>) -> bool {
    matches!(r, Err(NopalError::UniqueViolation(_)))
}

async fn emails(g: &Graph) -> Vec<String> {
    let mut v: Vec<String> = g
        .get_nodes_by_label("Persona")
        .await
        .unwrap()
        .into_iter()
        .filter_map(|n| match n.properties.get("email") {
            Some(PropertyValue::String(s)) => Some(s.clone()),
            _ => None,
        })
        .collect();
    v.sort();
    v
}

async fn unique_graph() -> Result<Graph> {
    let g = Graph::in_memory().await?;
    g.create_unique_index("Persona", "email", IndexType::Hash).await?;
    Ok(g)
}

#[tokio::test]
async fn every_write_path_refuses_a_repeated_value() -> Result<()> {
    let g = unique_graph().await?;
    g.add_node(persona("s@ejemplo.org")).await?;

    // Escritura directa.
    assert!(is_violation(g.add_node(persona("s@ejemplo.org")).await));

    // Transacción: nada del commit se escribe.
    let mut tx = g.begin_transaction().await?;
    tx.add_node(persona("j@ejemplo.org")).await?;
    tx.add_node(persona("s@ejemplo.org")).await?;
    assert!(is_violation(tx.commit().await));

    // Dos nodos nuevos con el mismo valor en la misma transacción.
    let mut tx = g.begin_transaction().await?;
    tx.add_node(persona("k@ejemplo.org")).await?;
    tx.add_node(persona("k@ejemplo.org")).await?;
    assert!(is_violation(tx.commit().await));

    // NQL.
    assert!(is_violation(g.execute_statement(r#"add (p:Persona {email: "s@ejemplo.org"})"#).await));

    // Carga masiva: el lote entero se rechaza.
    let mut loader = g.bulk_loader(10);
    loader.add_node(persona("m@ejemplo.org")).await?;
    loader.add_node(persona("m@ejemplo.org")).await?;
    assert!(is_violation(loader.finish().await));

    assert_eq!(emails(&g).await, ["s@ejemplo.org"]);
    Ok(())
}

#[tokio::test]
async fn updates_keep_or_move_their_own_value() -> Result<()> {
    let g = unique_graph().await?;
    let a = g.add_node(persona("a@ejemplo.org")).await?;
    let b = g.add_node(persona("b@ejemplo.org")).await?;

    // Reescribir un nodo con su propio valor no choca consigo mismo.
    g.add_node(Node::with_id(a, "Persona").with_property("email", "a@ejemplo.org").with_property("n", 1))
        .await?;
    // Tomar el valor de otro, sí.
    assert!(is_violation(
        g.add_node(Node::with_id(a, "Persona").with_property("email", "b@ejemplo.org")).await
    ));

    // Intercambio en una transacción: cada valor queda libre dentro del set.
    let mut tx = g.begin_transaction().await?;
    tx.add_node(Node::with_id(a, "Persona").with_property("email", "b@ejemplo.org")).await?;
    tx.add_node(Node::with_id(b, "Persona").with_property("email", "a@ejemplo.org")).await?;
    tx.commit().await?;
    assert_eq!(emails(&g).await, ["a@ejemplo.org", "b@ejemplo.org"]);

    // Cambiar la etiqueta libera el valor.
    g.add_node(Node::with_id(a, "Exalumno").with_property("email", "b@ejemplo.org")).await?;
    g.add_node(persona("b@ejemplo.org")).await?;
    Ok(())
}

#[tokio::test]
async fn deleting_frees_the_value() -> Result<()> {
    let g = unique_graph().await?;
    let a = g.add_node(persona("a@ejemplo.org")).await?;

    // Borrar y volver a crear en la misma transacción.
    let mut tx = g.begin_transaction().await?;
    tx.delete_node(a)?;
    tx.add_node(persona("a@ejemplo.org")).await?;
    tx.commit().await?;

    // Y en dos pasos.
    let id = g.get_nodes_by_label("Persona").await?[0].id;
    g.delete_node(id).await?;
    g.add_node(persona("a@ejemplo.org")).await?;
    assert_eq!(emails(&g).await, ["a@ejemplo.org"]);
    Ok(())
}

#[tokio::test]
async fn missing_null_and_other_labels_are_not_constrained() -> Result<()> {
    let g = unique_graph().await?;
    g.add_node(Node::new("Persona")).await?;
    g.add_node(Node::new("Persona")).await?;
    g.add_node(Node::new("Persona").with_property("email", PropertyValue::Null)).await?;
    g.add_node(Node::new("Persona").with_property("email", PropertyValue::Null)).await?;
    g.add_node(persona("s@ejemplo.org")).await?;
    g.add_node(Node::new("Empresa").with_property("email", "s@ejemplo.org")).await?;
    assert_eq!(g.get_nodes_by_label("Persona").await?.len(), 5);
    Ok(())
}

/// Commits concurrentes de la misma clave: el applier los valida en el mismo
/// lote, antes de aplicar ninguno. Exactamente uno gana.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_creates_of_one_value_leave_one_node() -> Result<()> {
    let g = unique_graph().await?;
    let mut tasks = Vec::new();
    for i in 0..8 {
        let g = g.clone();
        tasks.push(tokio::spawn(async move {
            let mut tx = g.begin_transaction().await?;
            tx.add_node(persona("s@ejemplo.org").with_property("intento", i)).await?;
            tx.commit().await
        }));
    }
    let mut ok = 0;
    let mut refused = 0;
    for t in tasks {
        match t.await.unwrap() {
            Ok(()) => ok += 1,
            Err(NopalError::UniqueViolation(_)) => refused += 1,
            Err(e) => panic!("{e}"),
        }
    }
    assert_eq!((ok, refused), (1, 7));
    assert_eq!(emails(&g).await, ["s@ejemplo.org"]);
    Ok(())
}

#[tokio::test]
async fn creating_over_repeated_values_fails_and_leaves_no_index() -> Result<()> {
    let g = Graph::in_memory().await?;
    let a = g.add_node(persona("s@ejemplo.org")).await?;
    let b = g.add_node(persona("s@ejemplo.org")).await?;
    let err = g.create_unique_index("Persona", "email", IndexType::Hash).await.unwrap_err();
    let msg = err.to_string();
    assert!(matches!(err, NopalError::UniqueViolation(_)), "{msg}");
    assert!(msg.contains(&a.to_string()) && msg.contains(&b.to_string()), "{msg}");
    assert!(g.list_indexes().await.is_empty());
    // Sin la marca, nada se rechaza.
    g.add_node(persona("s@ejemplo.org")).await?;

    assert!(g.create_unique_index("Persona", "email", IndexType::FullText).await.is_err());
    Ok(())
}

#[tokio::test]
async fn the_flag_survives_reopening_and_dropping_clears_it() -> Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    {
        let g = Graph::open(&path).await?;
        g.create_unique_index("Persona", "email", IndexType::BTree).await?;
        g.add_node(persona("s@ejemplo.org")).await?;
        g.close().await?;
    }
    {
        let g = Graph::open(&path).await?;
        let meta = g.list_indexes().await;
        assert!(meta.len() == 1 && meta[0].unique, "{meta:?}");
        assert!(is_violation(g.add_node(persona("s@ejemplo.org")).await));
        g.drop_index("Persona_email").await?;
        g.add_node(persona("s@ejemplo.org")).await?;
        g.close().await?;
    }
    let g = Graph::open(&path).await?;
    assert!(g.list_indexes().await.is_empty());
    g.add_node(persona("s@ejemplo.org")).await?;
    Ok(())
}

/// Una versión sin índices únicos abre la base, conserva el índice como uno
/// común y puede escribir duplicados. Al volver: se listan y los nuevos se
/// siguen rechazando.
#[tokio::test]
async fn duplicates_written_without_the_flag_are_reported_not_fatal() -> Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let (a, b): (NodeId, NodeId);
    {
        // Lo que dejaría esa versión: un índice común con un valor repetido
        // y el catálogo `unique_indexes` que no sabe leer.
        let g = Graph::open(&path).await?;
        g.create_index("Persona", "email", IndexType::Hash).await?;
        a = g.add_node(persona("s@ejemplo.org")).await?;
        b = g.add_node(persona("s@ejemplo.org")).await?;
        g.storage().put_meta_bytes("unique_indexes", br#"["Persona_email"]"#).await?;
        g.close().await?;
    }
    let g = Graph::open(&path).await?;
    let dups = g.unique_index_duplicates("Persona_email").await?;
    assert_eq!(dups.len(), 1);
    assert_eq!(dups[0].1.iter().copied().collect::<HashSet<_>>(), HashSet::from([a, b]));
    assert!(is_violation(g.add_node(persona("s@ejemplo.org")).await));

    g.delete_node(b).await?;
    assert!(g.unique_index_duplicates("Persona_email").await?.is_empty());
    Ok(())
}

/// La marca no viaja en `metadata.bin`: el archivo conserva la forma que
/// leen las versiones anteriores, que si no pudieran leerlo empezarían sin
/// índices y lo reescribirían en el siguiente `create_index`.
#[tokio::test]
async fn metadata_file_keeps_its_shape() -> Result<()> {
    let plain = tempfile::tempdir().unwrap();
    let unique = tempfile::tempdir().unwrap();
    for (dir, is_unique) in [(&plain, false), (&unique, true)] {
        let g = Graph::open(dir.path().join("db")).await?;
        if is_unique {
            g.create_unique_index("Persona", "email", IndexType::Hash).await?;
        } else {
            g.create_index("Persona", "email", IndexType::Hash).await?;
        }
        g.close().await?;
    }
    let read = |d: &tempfile::TempDir| std::fs::read(d.path().join("db/indexes/metadata.bin")).unwrap();
    // Solo difiere el `created_at` (8 bytes al final de la entrada).
    let (p, u) = (read(&plain), read(&unique));
    assert_eq!(p.len(), u.len());
    let differing = p.iter().zip(&u).filter(|(x, y)| x != y).count();
    assert!(differing <= 8, "{differing} bytes differ");
    Ok(())
}

#[tokio::test]
async fn nql_creates_unique_indexes() -> Result<()> {
    let g = Graph::in_memory().await?;
    let r = g.execute_statement("create unique index on Persona(email)").await?;
    assert!(matches!(&r, NqlResult::Index(m) if m == "Unique index created: Persona_email"), "{r:?}");
    let meta = g.list_indexes().await;
    assert!(meta.len() == 1 && meta[0].unique && meta[0].index_type == IndexType::Hash, "{meta:?}");
    g.execute_statement(r#"add (p:Persona {email: "s@ejemplo.org"})"#).await?;
    assert!(is_violation(g.execute_statement(r#"add (p:Persona {email: "s@ejemplo.org"})"#).await));

    g.execute_statement("CREATE UNIQUE INDEX ON Empresa(rfc) TYPE btree").await?;
    let stats = g.stats().await?;
    let flags: Vec<(String, bool)> = stats.indexes.iter().map(|i| (i.name.clone(), i.unique)).collect();
    assert_eq!(flags, [("Empresa_rfc".to_string(), true), ("Persona_email".to_string(), true)]);
    assert!(g.describe_index("Empresa_rfc").await.unwrap().metadata.unique);

    // Un índice común sigue siendo común.
    g.execute_statement("create index on Persona(nombre)").await?;
    assert!(!g.describe_index("Persona_nombre").await.unwrap().metadata.unique);

    for bad in [
        "create unique index on Nota(cuerpo) type fulltext",
        "create unique index on Nota(tema) type taxonomy",
        r#"create unique index on Nota(cuerpo) with (language = "spanish")"#,
    ] {
        assert!(matches!(g.execute_statement(bad).await, Err(NopalError::QueryParseError(_))), "{bad}");
    }

    // Sobre valores repetidos falla sin crear nada.
    g.execute_statement(r#"add (c:Ciudad {nombre: "Oaxaca"})"#).await?;
    g.execute_statement(r#"add (c:Ciudad {nombre: "Oaxaca"})"#).await?;
    assert!(is_violation(g.execute_statement("create unique index on Ciudad(nombre)").await));
    assert!(g.describe_index("Ciudad_nombre").await.is_none());
    Ok(())
}

#[tokio::test]
async fn copying_the_base_keeps_the_flag() -> Result<()> {
    use nopaldb::storage::{Storage, StorageOptions};

    let src = tempfile::tempdir().unwrap();
    {
        let g = Graph::open(src.path()).await?;
        g.create_unique_index("Persona", "email", IndexType::Hash).await?;
        g.add_node(persona("s@ejemplo.org")).await?;
        g.close().await?;
    }
    let dst = tempfile::tempdir().unwrap();
    let copy = dst.path().join("copia");
    let report = Storage::copy_database(src.path(), StorageOptions::default(), &copy, StorageOptions::default()).await?;
    assert!(report.verified);
    let g = Graph::open(&copy).await?;
    assert!(g.list_indexes().await[0].unique);
    assert!(is_violation(g.add_node(persona("s@ejemplo.org")).await));
    Ok(())
}

fn upsert(email: &str, intento: i64) -> UpsertRequest {
    UpsertRequest {
        label: "Persona".into(),
        key: "email".into(),
        props: HashMap::from([
            ("email".to_string(), PropertyValue::String(email.into())),
            ("intento".to_string(), PropertyValue::Int(intento)),
        ]),
        embedding: None,
        links: Vec::new(),
    }
}

/// `add_node` no toma el candado por clave de `upsert`: puede crear la clave
/// entre la búsqueda y el commit del upsert. Con índice único ese commit se
/// rechaza y el upsert reintenta, encuentra al ganador y lo actualiza; nunca
/// devuelve `UniqueViolation`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn upserts_that_lose_their_key_to_a_plain_write_update_the_winner() -> Result<()> {
    let g = unique_graph().await?;
    let mut tasks = Vec::new();
    for ronda in 0..20 {
        let email = format!("r{ronda}@ejemplo.org");
        for i in 0..6 {
            let (g, email) = (g.clone(), email.clone());
            tasks.push(tokio::spawn(async move {
                if i % 2 == 0 {
                    g.upsert_node(upsert(&email, i)).await.map(|(o, _)| Some(o))
                } else {
                    let mut tx = g.begin_transaction().await?;
                    tx.add_node(persona(&email)).await?;
                    match tx.commit().await {
                        Err(NopalError::UniqueViolation(_)) => Ok(None),
                        r => r.map(|_| None),
                    }
                }
            }));
        }
    }
    for t in tasks {
        if let Some(outcome) = t.await.unwrap()? {
            assert_ne!(outcome, UpsertOutcome::Unchanged, "cada upsert escribe su `intento`");
        }
    }
    let e = emails(&g).await;
    assert_eq!(e.len(), 20, "{e:?}");
    assert_eq!(e.iter().collect::<HashSet<_>>().len(), 20);
    Ok(())
}

/// Los stubs de `links` tampoco toman candado: dos upserts de notas distintas
/// que enlazan al mismo destino inexistente lo crearían dos veces. Con índice
/// único en la clave del destino, el que pierde reintenta y enlaza al stub del
/// otro.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn link_stubs_of_a_unique_key_are_created_once() -> Result<()> {
    let g = Graph::in_memory().await?;
    g.create_unique_index("Nota", "clave", IndexType::Hash).await?;
    let nota = |clave: String, destino: String| UpsertRequest {
        label: "Nota".into(),
        key: "clave".into(),
        props: HashMap::from([("clave".to_string(), PropertyValue::String(clave))]),
        embedding: None,
        links: vec![LinkSpec {
            edge_type: "MENCIONA".into(),
            target_label: "Nota".into(),
            target_key: "clave".into(),
            target_key_value: PropertyValue::String(destino),
            props: HashMap::new(),
            create_target_stub: true,
        }],
    };
    let mut tasks = Vec::new();
    for ronda in 0..20 {
        for i in 0..4 {
            let g = g.clone();
            let req = nota(format!("n{ronda}-{i}"), format!("destino{ronda}"));
            tasks.push(tokio::spawn(async move { g.upsert_node(req).await }));
        }
    }
    for t in tasks {
        t.await.unwrap()?;
    }
    let notas = g.get_nodes_by_label("Nota").await?;
    assert_eq!(notas.len(), 20 * 5, "4 notas y un solo destino por ronda");
    assert_eq!(g.get_all_edges().await?.len(), 20 * 4);
    Ok(())
}
