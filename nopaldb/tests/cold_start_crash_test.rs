// Arranque en frío interrumpido: matar el proceso mientras se CREA la base
// no puede dejar el directorio inservible.
//
// Es el escenario del primer arranque de un despliegue nuevo — OOM, kill del
// contenedor, timeout de deploy, corte de luz. No hay datos que perder
// todavía; lo que está en juego es que la aplicación vuelva a arrancar.
//
// Se probó que el riesgo se limita a la creación: una base ya establecida
// sobrevive el mismo kill sin problema (segundo test). Por eso el arreglo
// cubre solo el arranque en frío, y por eso este test lo verifica de las dos
// formas — que la ventana mala esté cerrada y que la buena siga estándolo.
//
// Patrón self-exec, como el resto de harnesses de crash: el padre relanza
// este binario filtrando el hijo (#[ignore]) y lo mata con SIGKILL.
//
// El test no sabe de motores: usa el que el build traiga, y los DOS tenían
// el agujero. redb de par en par (determinista dentro de la ventana) y sled
// con una ventana mucho más estrecha —~3 de 20 barridos lo pescaban, con
// `Read corrupted data` en vez de `invalid data`— que casi pasa por sano en
// una primera medición de 64 intentos. Cada motor lo cierra a su manera
// (rename atómico en redb, descarte de la creación incompleta en sled)
// porque su layout en disco es distinto; el invariante que se exige aquí es
// el mismo para ambos.

#![cfg(unix)]

use nopaldb::{Graph, Node, PropertyValue};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const ENV_DB_DIR: &str = "NOPAL_COLD_START_DB_DIR";

/// Hijo: abre la base y escribe sin parar hasta que lo maten.
#[tokio::test]
#[ignore = "solo corre como proceso hijo del harness"]
async fn cold_start_child() {
    let Some(dir) = std::env::var_os(ENV_DB_DIR) else {
        return;
    };
    let graph = Graph::open(Path::new(&dir)).await.expect("child open");
    let mut i: i64 = 0;
    loop {
        i += 1;
        let _ = graph
            .add_node(Node::new("N").with_property("i", PropertyValue::Int(i)))
            .await;
    }
}

fn spawn_child(exe: &Path, dir: &Path) -> Child {
    Command::new(exe)
        .args(["cold_start_child", "--ignored", "--exact"])
        .env(ENV_DB_DIR, dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn child")
}

/// Barrido de retardos que cubre toda la creación: en NINGUNO puede quedar
/// una base que no abra.
///
/// El barrido va de 1ms a 70ms porque el instante exacto en que la creación
/// termina depende de la máquina — en un runner lento tarda decenas de
/// milisegundos, en uno rápido menos de diez. Fijar un solo retardo probaría
/// la máquina donde corre, no el invariante.
#[tokio::test(flavor = "multi_thread")]
async fn kill_while_creating_leaves_a_usable_directory() {
    let exe = std::env::current_exe().expect("current_exe");
    let mut broken: Vec<(u64, String)> = Vec::new();

    for ms in [1u64, 2, 3, 4, 5, 6, 8, 10, 15, 20, 30, 50, 70] {
        for _ in 0..3 {
            let dir = tempfile::tempdir().unwrap();
            let mut child = spawn_child(&exe, dir.path());
            tokio::time::sleep(Duration::from_millis(ms)).await;
            child.kill().expect("SIGKILL child");
            let _ = child.wait();

            match Graph::open(dir.path()).await {
                Ok(graph) => {
                    // Además de abrir, debe quedar utilizable.
                    graph
                        .add_node(Node::new("After").with_property("ok", PropertyValue::Bool(true)))
                        .await
                        .expect("la base debe aceptar escrituras tras el arranque fallido");
                    drop(graph);
                }
                Err(e) => broken.push((ms, format!("{e}"))),
            }
        }
    }

    assert!(
        broken.is_empty(),
        "un kill durante la creación dejó la base inabrible en {} casos: {:?}",
        broken.len(),
        broken
    );
}

/// La otra mitad del invariante: una base ESTABLECIDA sobrevive el mismo
/// kill. Sin este test, el primero se podría "pasar" borrando el directorio
/// en cada apertura fallida — y nadie lo notaría hasta perder datos.
#[tokio::test(flavor = "multi_thread")]
async fn kill_over_established_db_preserves_data() {
    let exe = std::env::current_exe().expect("current_exe");

    for ms in [2u64, 5, 10, 30] {
        let dir = tempfile::tempdir().unwrap();
        let marker = uuid::Uuid::new_v4();

        {
            let graph = Graph::open(dir.path()).await.expect("crear base");
            graph
                .add_node(Node::with_id(marker, "Seed").with_property("keep", PropertyValue::Bool(true)))
                .await
                .expect("seed");
            graph.close().await.expect("cierre limpio");
        }

        let mut child = spawn_child(&exe, dir.path());
        tokio::time::sleep(Duration::from_millis(ms)).await;
        child.kill().expect("SIGKILL child");
        let _ = child.wait();

        let graph = Graph::open(dir.path())
            .await
            .unwrap_or_else(|e| panic!("base establecida debe abrir tras el kill (ms={ms}): {e}"));
        graph
            .get_node(marker)
            .await
            .unwrap_or_else(|e| panic!("el nodo previo al kill debe seguir ahí (ms={ms}): {e}"));
    }
}
