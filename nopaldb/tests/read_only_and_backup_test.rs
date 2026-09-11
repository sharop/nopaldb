// Apertura de solo-lectura y backup en frío.
//
// Lo que se verifica NO es que se pueda leer mientras otro escribe — eso
// ningún motor embebido de los que soporta NopalDB lo da hoy, y prometerlo
// sería mentir. Lo que se verifica es:
//
//   1. que un handle de solo-lectura NO pueda modificar la base, por ninguna
//      de las vías (nodos, aristas, transacciones, embeddings, índices);
//   2. que el flujo real para "leer sin tocar la base viva" —copiar en frío
//      y abrir la copia— funcione de punta a punta y preserve los datos.
//
// Ese segundo punto es la respuesta honesta al problema que motivó todo
// esto, y por eso está aquí junto al primero y no en un test aparte.

use nopaldb::types::{Node, NodeId, PropertyValue};
use nopaldb::{Edge, Graph};

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

/// Base con contenido variado, cerrada limpio.
async fn base_con_datos(dir: &std::path::Path) -> (NodeId, NodeId) {
    let g = Graph::open(dir).await.unwrap();
    let a = g
        .add_node(Node::new("Doc").with_property("name", s("nopal")))
        .await
        .unwrap();
    let b = g
        .add_node(Node::new("Doc").with_property("name", s("agave")))
        .await
        .unwrap();
    g.add_edge(Edge::new(a, b, "CITES")).await.unwrap();
    #[cfg(feature = "embeddings")]
    g.add_node_embedding(a, vec![1.0, 0.0, 0.0], "m").await.unwrap();
    g.close().await.unwrap();
    (a, b)
}

/// Se lee todo lo que hay: nodos, aristas y vecinos.
#[tokio::test]
async fn read_only_reads_everything() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = base_con_datos(dir.path()).await;

    let g = Graph::open_read_only(dir.path()).await.unwrap();
    assert!(g.is_read_only());

    assert_eq!(
        g.get_node(a).await.unwrap().properties.get("name"),
        Some(&s("nopal"))
    );
    assert_eq!(g.get_all_nodes().await.unwrap().len(), 2);
    assert_eq!(g.get_all_edges().await.unwrap().len(), 1);
    assert_eq!(
        g.neighbors(a, nopaldb::Direction::Outgoing).await.unwrap(),
        vec![b]
    );
    g.close().await.unwrap();
}

/// Ninguna vía de escritura pasa: ni directa, ni transaccional, ni de
/// embeddings, ni de índices. Y falla con error, nunca con panic.
#[tokio::test]
async fn read_only_rejects_every_write_path() {
    let dir = tempfile::tempdir().unwrap();
    let (a, _b) = base_con_datos(dir.path()).await;

    let g = Graph::open_read_only(dir.path()).await.unwrap();

    // (1) escritura directa
    let err = g.add_node(Node::new("Nuevo")).await.unwrap_err();
    assert!(
        format!("{err}").contains("solo-lectura"),
        "el error debe decir por qué: {err}"
    );

    // (2) borrado
    assert!(g.delete_node(a).await.is_err(), "delete_node debe fallar");

    // (3) transacción
    let mut tx = g.begin_transaction().await.unwrap();
    tx.add_node(Node::new("EnTx")).await.unwrap();
    assert!(
        tx.commit().await.is_err(),
        "el commit de una tx debe fallar en solo-lectura"
    );

    // (4) embeddings — no pasan por el applier, así que los cubre el sello
    //     del engine y no la guarda temprana.
    #[cfg(feature = "embeddings")]
    assert!(
        g.add_node_embedding(a, vec![0.0, 1.0, 0.0], "m").await.is_err(),
        "add_node_embedding debe fallar"
    );

    // (5) índices — escriben su metadata directo al filesystem, así que no
    //     los ve ninguno de los dos sellos: llevan guarda propia.
    assert!(
        g.create_index("Doc", "name", nopaldb::index::IndexType::Hash)
            .await
            .is_err(),
        "create_index debe fallar"
    );

    g.close().await.unwrap();
}

/// Y lo más importante: tras rechazar todo, la base sigue intacta en disco.
/// Un rechazo que dejara escrituras a medias sería peor que no tener el modo.
#[tokio::test]
async fn rejected_writes_leave_no_trace() {
    let dir = tempfile::tempdir().unwrap();
    let (a, _b) = base_con_datos(dir.path()).await;

    {
        let g = Graph::open_read_only(dir.path()).await.unwrap();
        let _ = g.add_node(Node::new("Fantasma")).await;
        let _ = g.delete_node(a).await;
        let mut tx = g.begin_transaction().await.unwrap();
        let _ = tx.add_node(Node::new("FantasmaTx")).await;
        let _ = tx.commit().await;
        #[cfg(feature = "embeddings")]
        let _ = g.add_node_embedding(a, vec![9.0, 9.0, 9.0], "m").await;
        g.close().await.unwrap();
    }

    // Reabrir para ESCRITURA: el replay del WAL corre aquí. Si algún rechazo
    // hubiera dejado un registro en el WAL, aparecería ahora.
    let g = Graph::open(dir.path()).await.unwrap();
    let nodos = g.get_all_nodes().await.unwrap();
    assert_eq!(nodos.len(), 2, "no puede haber nodos nuevos: {nodos:?}");
    assert!(g.get_node(a).await.is_ok(), "el nodo borrado en vano sigue ahí");
    #[cfg(feature = "embeddings")]
    assert_eq!(
        g.get_node_embedding(a, "m").await.unwrap().vector,
        vec![1.0, 0.0, 0.0],
        "el embedding no pudo ser sobrescrito"
    );
    g.close().await.unwrap();
}

/// Una base abierta para escribir NO se puede abrir en solo-lectura: el modo
/// no da acceso concurrente y el error lo dice. Es la mitad del contrato que
/// más fácil se malinterpreta.
#[tokio::test]
async fn read_only_does_not_grant_concurrent_access() {
    let dir = tempfile::tempdir().unwrap();
    base_con_datos(dir.path()).await;

    let escritor = Graph::open(dir.path()).await.unwrap();
    let Err(err) = Graph::open_read_only(dir.path()).await else {
        panic!("no debe poder abrirse en solo-lectura con un escritor vivo");
    };
    let msg = format!("{err}");
    // El escritor vive en ESTE proceso: desde 0.5.19 el error lo dice así y
    // explica que el lock se suelta al drop (antes culpaba a "otro proceso").
    assert!(
        msg.contains("ya está abierta en este proceso"),
        "debe explicar que hay otro handle en este proceso, no un error críptico: {msg}"
    );
    escritor.close().await.unwrap();
}

/// El flujo completo de backup en frío: copiar, abrir la copia en
/// solo-lectura, y comprobar que los datos están.
///
/// Esto es lo que de verdad resuelve "quiero leer sin molestar al escritor".
#[tokio::test]
async fn cold_backup_round_trip() {
    use nopaldb::storage::{Storage, StorageOptions};

    let origen = tempfile::tempdir().unwrap();
    let (a, b) = base_con_datos(origen.path()).await;

    let destino = tempfile::tempdir().unwrap();
    let backup = destino.path().join("copia");

    let reporte = Storage::copy_database(
        origen.path(),
        StorageOptions::default(),
        &backup,
        StorageOptions::default(),
    )
    .await
    .expect("el backup en frío debe funcionar");
    assert!(reporte.verified, "la copia se verifica sola");

    // La copia abre y tiene los datos.
    let copia = Graph::open_read_only(&backup).await.unwrap();
    assert_eq!(copia.get_all_nodes().await.unwrap().len(), 2);
    assert_eq!(copia.get_all_edges().await.unwrap().len(), 1);
    assert_eq!(
        copia.get_node(a).await.unwrap().properties.get("name"),
        Some(&s("nopal"))
    );
    assert_eq!(
        copia.neighbors(a, nopaldb::Direction::Outgoing).await.unwrap(),
        vec![b]
    );
    copia.close().await.unwrap();

    // Y el ORIGEN sigue usable después del backup.
    let g = Graph::open(origen.path()).await.unwrap();
    assert_eq!(g.get_all_nodes().await.unwrap().len(), 2);
    g.close().await.unwrap();
}

/// El backup se niega a mezclar bases: destino no vacío es un error, no una
/// fusión silenciosa.
#[tokio::test]
async fn cold_backup_refuses_to_overwrite() {
    use nopaldb::storage::{Storage, StorageOptions};

    let origen = tempfile::tempdir().unwrap();
    base_con_datos(origen.path()).await;
    let destino = tempfile::tempdir().unwrap();
    let backup = destino.path().join("copia");

    Storage::copy_database(origen.path(), StorageOptions::default(), &backup, StorageOptions::default())
        .await
        .unwrap();

    let err = Storage::copy_database(
        origen.path(),
        StorageOptions::default(),
        &backup,
        StorageOptions::default(),
    )
    .await
    .expect_err("un destino con datos debe rechazarse");
    assert!(format!("{err}").contains("no está vacío"), "{err}");
}
