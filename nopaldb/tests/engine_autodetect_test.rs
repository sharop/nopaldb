//! 0.6.0 (#131) hizo de redb el motor por defecto y `StorageEngine::Auto`
//! elegía el motor del directorio. 0.8.0 (#141) retiró sled: redb es el
//! único motor, y una base sled de 0.5.x (sus archivos `conf` y `db`) se
//! sigue reconociendo solo para fallar con la receta de migración (migrar
//! con NopalDB 0.7, la última versión que lee sled), en vez de con un error
//! críptico o, peor, creando una base redb vacía al lado.

use nopaldb::{Graph, Node, PropertyValue, Storage, StorageEngine, StorageOptions, StorageProfile};

fn with(engine: StorageEngine) -> StorageOptions {
    StorageOptions { engine, ..Default::default() }
}

async fn create_db(dir: &std::path::Path) -> nopaldb::NodeId {
    let graph = Graph::open_with_options(dir, with(StorageEngine::Redb)).await.unwrap();
    let id = graph
        .add_node(Node::new("Planta").with_property("nombre", PropertyValue::String("nopal".into())))
        .await
        .unwrap();
    graph.close().await.unwrap();
    id
}

/// Lo que deja una base sled en disco: basta para reconocerla.
fn fake_sled_db(dir: &std::path::Path) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("conf"), b"segment_size: 524288\n").unwrap();
    std::fs::write(dir.join("db"), b"").unwrap();
}

async fn open_error(dir: &std::path::Path, engine: StorageEngine) -> String {
    match Graph::open_with_options(dir, with(engine)).await {
        Ok(_) => panic!("{engine:?} sobre una base sled debe fallar"),
        Err(e) => e.to_string(),
    }
}

#[tokio::test]
async fn auto_opens_an_existing_redb_database_with_redb() {
    let dir = tempfile::tempdir().unwrap();
    let id = create_db(dir.path()).await;
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
async fn a_sled_database_fails_with_the_migration_recipe_whatever_the_engine_asked() {
    let dir = tempfile::tempdir().unwrap();
    fake_sled_db(dir.path());
    for engine in [StorageEngine::Auto, StorageEngine::Redb, StorageEngine::Sled] {
        let err = open_error(dir.path(), engine).await;
        assert!(err.contains("contains a sled database"), "{engine:?}: {err}");
        assert!(err.contains("nopaldb==0.7") && err.contains("Graph.migrate"), "{engine:?}: {err}");
    }
    // Ningún intento dejó una base redb al lado.
    assert!(!dir.path().join("nopal.redb").exists());
    // Ni los constructores con perfil (hasta 0.5.24 forzaban sled).
    let Err(err) = Graph::open_with_profile(dir.path(), StorageProfile::Mobile).await else {
        panic!("open_with_profile sobre una base sled debe fallar");
    };
    assert!(err.to_string().contains("nopaldb==0.7"), "{err}");
}

#[tokio::test]
async fn asking_for_sled_on_a_new_directory_says_it_was_removed() {
    let dir = tempfile::tempdir().unwrap();
    let Err(err) = Graph::open_with_options(dir.path().join("nueva"), with(StorageEngine::Sled)).await else {
        panic!("sled ya no existe");
    };
    let err = err.to_string();
    assert!(err.contains("removed in NopalDB 0.8.0"), "{err}");
    assert!(!dir.path().join("nueva").join("nopal.redb").exists());
}

#[tokio::test]
async fn copy_database_with_auto_copies_a_redb_database() {
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();
    let id = create_db(src.path()).await;
    let report = Storage::copy_database(src.path(), with(StorageEngine::Auto), dst.path().join("copia"), with(StorageEngine::Auto))
        .await
        .unwrap();
    assert!(report.verified);
    let graph = Graph::open(dst.path().join("copia")).await.unwrap();
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

    // Un origen sled: la receta, no una copia.
    let sled = tempfile::tempdir().unwrap();
    fake_sled_db(sled.path());
    let err = Storage::copy_database(sled.path(), with(StorageEngine::Auto), dst.path().join("de_sled"), with(StorageEngine::Auto))
        .await
        .err()
        .expect("un origen sled debe fallar")
        .to_string();
    assert!(err.contains("nopaldb==0.7"), "{err}");
}
