//! El binario `nopaldb` (feature `cli`, #140): `migrate` deja una copia
//! verificada de una base y `engine` identifica el motor de un directorio.
//! Desde 0.8.0 (sled retirado, #141) redb es el único motor; una base sled
//! se reconoce para decir cómo migrarla. Se ejecuta el binario real
//! (`CARGO_BIN_EXE_nopaldb`).

use nopaldb::{Graph, Node, PropertyValue, StorageEngine, StorageOptions};
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_nopaldb"))
}

async fn make_db(dir: &std::path::Path) {
    let graph = Graph::open_with_options(dir, StorageOptions { engine: StorageEngine::Redb, ..Default::default() })
        .await
        .unwrap();
    for n in ["nopal", "maguey", "biznaga"] {
        graph
            .add_node(Node::new("Planta").with_property("nombre", PropertyValue::String(n.into())))
            .await
            .unwrap();
    }
    graph.close().await.unwrap();
}

#[tokio::test]
async fn migrate_copies_a_database_and_engine_identifies_it() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("plantas");
    let dst = tmp.path().join("plantas_copia");
    make_db(&src).await;

    let out = bin().args(["engine", src.to_str().unwrap()]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "redb");

    let out = bin()
        .args(["migrate", src.to_str().unwrap(), dst.to_str().unwrap(), "--to", "redb"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "stdout:\n{stdout}\nstderr:\n{}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("Verificación: OK"), "{stdout}");
    assert!(stdout.contains("entities"), "una línea por keyspace: {stdout}");

    let out = bin().args(["engine", dst.to_str().unwrap()]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "redb");

    // La copia abre con Auto y trae los datos.
    let graph = Graph::open(&dst).await.unwrap();
    assert_eq!(graph.storage().backend_name(), "redb");
    assert_eq!(graph.get_label_count("Planta").await.unwrap(), 3);
}

#[tokio::test]
async fn migrate_refuses_a_non_empty_destination_with_exit_code_2() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    make_db(&src).await;
    make_db(&dst).await;
    let out = bin()
        .args(["migrate", src.to_str().unwrap(), dst.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stderr).contains("la migración falló"));
}

#[test]
fn a_sled_database_is_identified_and_migrate_gives_the_recipe() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("plantas_sled");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("conf"), b"").unwrap();
    std::fs::write(src.join("db"), b"").unwrap();

    let out = bin().args(["engine", src.to_str().unwrap()]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "sled");

    let dst = tmp.path().join("plantas_redb");
    let out = bin().args(["migrate", src.to_str().unwrap(), dst.to_str().unwrap()]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("nopaldb==0.7") && stderr.contains("cargo install nopaldb"), "{stderr}");
}

#[test]
fn bad_arguments_exit_with_1_and_print_usage() {
    let out = bin().output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("uso:"));

    let out = bin().args(["migrate", "solo_uno"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("<src_dir>"));

    let out = bin().args(["migrate", "a", "b", "--to", "mongo"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("motor desconocido"));

    let out = bin().args(["engine", "/tmp/no/existe/nopal"]).output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("ninguno"));

    let out = bin().arg("--version").output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("nopaldb "));
}

#[tokio::test]
async fn stats_prints_every_section_of_a_closed_database() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("plantas");
    make_db(&dir).await;

    let out = bin().args(["stats", dir.to_str().unwrap()]).output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "stdout:\n{stdout}\nstderr:\n{}", String::from_utf8_lossy(&out.stderr));
    for needle in [
        "motor: redb",
        "solo lectura: sí",
        "Grafo: 3 nodos, 0 aristas",
        "Planta",
        "WAL:",
        "Último open:",
        "operaciones reproducidas: 0",
        "Índices de usuario: ninguno",
        "Índices HNSW en caché: ninguno",
        "GC: automático parado",
    ] {
        assert!(stdout.contains(needle), "falta `{needle}` en:\n{stdout}");
    }

    // Sin base: código 1 y ninguna creación accidental.
    let missing = tmp.path().join("nada");
    let out = bin().args(["stats", missing.to_str().unwrap()]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(!missing.exists(), "stats must not create a database");
}
