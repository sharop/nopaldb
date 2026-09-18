//! 0.6.0 (#131): redb es el motor por defecto y `StorageEngine::Auto` elige
//! el motor del directorio si ya hay una base. Una base sled creada con 0.5.x
//! se abre con el binario nuevo sin tocar código; un motor explícito sobre un
//! directorio ajeno sigue siendo error, con la receta de migración.

use nopaldb::{Graph, Node, PropertyValue, Storage, StorageEngine, StorageOptions, StorageProfile};

fn with(engine: StorageEngine) -> StorageOptions {
    StorageOptions { engine, ..Default::default() }
}

async fn create_db(dir: &std::path::Path, engine: StorageEngine) -> nopaldb::NodeId {
    let graph = Graph::open_with_options(dir, with(engine)).await.unwrap();
    let id = graph
        .add_node(Node::new("Planta").with_property("nombre", PropertyValue::String("nopal".into())))
        .await
        .unwrap();
    graph.close().await.unwrap();
    id
}

#[tokio::test]
async fn auto_opens_an_existing_sled_database_with_sled() {
    let dir = tempfile::tempdir().unwrap();
    let id = create_db(dir.path(), StorageEngine::Sled).await;
    assert!(dir.path().join("conf").exists() && dir.path().join("db").exists());

    // `Graph::open` = opciones por defecto = Auto.
    let graph = Graph::open(dir.path()).await.unwrap();
    assert_eq!(graph.storage().backend_name(), "sled");
    assert_eq!(graph.get_node(id).await.unwrap().label, "Planta");
    graph.close().await.unwrap();
    drop(graph); // el lock se suelta al drop, no en close()

    // También los constructores con perfil (hasta 0.5.24 forzaban sled).
    let graph = Graph::open_with_profile(dir.path(), StorageProfile::Mobile).await.unwrap();
    assert_eq!(graph.storage().backend_name(), "sled");
}

#[tokio::test]
async fn auto_opens_an_existing_redb_database_with_redb() {
    let dir = tempfile::tempdir().unwrap();
    let id = create_db(dir.path(), StorageEngine::Redb).await;
    assert!(dir.path().join("nopal.redb").exists());
    let graph = Graph::open(dir.path()).await.unwrap();
    assert_eq!(graph.storage().backend_name(), "redb");
    assert_eq!(graph.get_node(id).await.unwrap().label, "Planta");
}

#[tokio::test]
async fn auto_creates_new_databases_with_redb() {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path().join("nueva")).await.unwrap();
    assert_eq!(graph.storage().backend_name(), "redb");
    drop(graph);
    let mem = Graph::in_memory().await.unwrap();
    assert_eq!(mem.storage().backend_name(), "redb");
    let mem = Graph::in_memory_with_profile(StorageProfile::Server).await.unwrap();
    assert_eq!(mem.storage().backend_name(), "redb");
}

#[tokio::test]
async fn an_explicit_engine_over_the_other_engines_database_is_an_error_that_says_how_to_migrate() {
    let dir = tempfile::tempdir().unwrap();
    create_db(dir.path(), StorageEngine::Sled).await;
    let err = match Graph::open_with_options(dir.path(), with(StorageEngine::Redb)).await {
        Ok(_) => panic!("redb explícito sobre una base sled debe fallar"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("base sled") && err.contains("migra") || err.contains("mígrala"), "{err}");

    let dir2 = tempfile::tempdir().unwrap();
    create_db(dir2.path(), StorageEngine::Redb).await;
    let err = match Graph::open_with_options(dir2.path(), with(StorageEngine::Sled)).await {
        Ok(_) => panic!("sled explícito sobre una base redb debe fallar"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("base redb"), "{err}");
}

#[tokio::test]
async fn copy_database_with_auto_detects_the_source_engine() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let id = create_db(src.path(), StorageEngine::Sled).await;
    let report = Storage::copy_database(src.path(), with(StorageEngine::Auto), dst.path().join("redb"), with(StorageEngine::Auto))
        .await
        .unwrap();
    assert!(report.verified);
    let graph = Graph::open(dst.path().join("redb")).await.unwrap();
    assert_eq!(graph.storage().backend_name(), "redb");
    assert_eq!(graph.get_node(id).await.unwrap().label, "Planta");

    // Auto sobre un origen sin base no inventa una migración vacía.
    let empty = tempfile::tempdir().unwrap();
    let err = Storage::copy_database(empty.path(), with(StorageEngine::Auto), dst.path().join("otra"), with(StorageEngine::Auto))
        .await
        .err()
        .expect("sin base en el origen debe fallar")
        .to_string();
    assert!(err.contains("no hay una base"), "{err}");
}
