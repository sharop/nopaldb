// tests/nql_update_durability_test.rs
//
// NQL `update … set` pasa por la transacción de la sentencia: versión MVCC,
// registro en el WAL e índices mantenidos por el commit. Hasta 0.9.4 escribía
// directo en el motor (`storage_insert_node` / `storage_insert_edge`): sin
// versión nueva en `history()`, un índice hash de usuario que seguía
// devolviendo el nodo por el valor viejo, y un crash antes del siguiente
// checkpoint perdía el UPDATE confirmado, porque el WAL no tenía registro con
// el que rehacerlo. Dominio ficticio (vivero).

use nopaldb::index::IndexType;
use nopaldb::{Edge, EdgeId, Graph, Node, NodeId, PropertyValue};
use uuid::Uuid;

fn nid(x: u128) -> NodeId {
    Uuid::from_u128(0x0D0E_0000_0000 + x)
}

fn int(v: i64) -> PropertyValue {
    PropertyValue::Int(v)
}

/// Dos macetas y un riego entre ellas, sembrados por transacción (cadena
/// MVCC desde el principio).
async fn sembrar(graph: &Graph) -> EdgeId {
    let mut tx = graph.begin_transaction().await.unwrap();
    tx.add_node(Node::with_id(nid(1), "Maceta").with_property("riego", int(1)))
        .await
        .unwrap();
    tx.add_node(Node::with_id(nid(2), "Maceta").with_property("riego", int(5)))
        .await
        .unwrap();
    let edge = Edge::new(nid(1), nid(2), "COMPARTE").with_property("litros", int(3));
    let edge_id = tx.add_edge(edge).unwrap();
    tx.commit().await.unwrap();
    edge_id
}

/// El UPDATE deja una versión nueva en la historia y cierra la anterior en el
/// mismo instante en que abre la nueva.
#[tokio::test]
async fn update_crea_version_mvcc_y_cierra_la_anterior() {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await.unwrap();
    sembrar(&graph).await;

    let antes = graph.history(nid(1)).await.unwrap();
    assert_eq!(antes.len(), 1, "la siembra deja una versión");

    graph
        .execute_statement("update (m:Maceta) set m.riego = 2 where m.riego = 1")
        .await
        .unwrap();

    let mut historia = graph.history(nid(1)).await.unwrap();
    historia.sort_by_key(|v| v.version); // `history` llega de la más nueva a la más vieja
    assert_eq!(
        historia.len(),
        2,
        "el UPDATE debe dejar una versión MVCC nueva; historia = {:?}",
        historia
    );
    let (vieja, nueva) = (&historia[0], &historia[1]);
    assert_eq!(vieja.node_data.properties.get("riego"), Some(&int(1)));
    assert_eq!(nueva.node_data.properties.get("riego"), Some(&int(2)));
    assert!(nueva.valid_from > vieja.valid_from, "la versión nueva es posterior");
    assert_eq!(
        vieja.valid_to,
        Some(nueva.valid_from),
        "la versión anterior se cierra con el ts del commit del UPDATE"
    );
    assert_eq!(nueva.valid_to, None, "la versión nueva es la vigente");

    let actual = graph.get_node(nid(1)).await.unwrap();
    assert_eq!(actual.properties.get("riego"), Some(&int(2)));
}

/// Las dos capas de índice quedan coherentes: el índice hash de usuario y el
/// de propiedades encuentran el nodo por el valor nuevo y ya no por el viejo.
#[tokio::test]
async fn update_mantiene_indice_hash_y_de_propiedades() {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await.unwrap();
    graph.create_index("Maceta", "riego", IndexType::Hash).await.unwrap();
    sembrar(&graph).await;

    graph
        .execute_statement("update (m:Maceta) set m.riego = 2 where m.riego = 1")
        .await
        .unwrap();

    let por_nuevo = graph.find_nodes_indexed("Maceta", "riego", int(2)).await.unwrap();
    assert_eq!(
        por_nuevo.iter().map(|n| n.id).collect::<Vec<_>>(),
        vec![nid(1)],
        "el índice hash de usuario debe tener el valor nuevo"
    );
    let por_viejo = graph.find_nodes_indexed("Maceta", "riego", int(1)).await.unwrap();
    assert!(
        por_viejo.is_empty(),
        "el índice hash de usuario no debe devolver el nodo por el valor viejo: {:?}",
        por_viejo.iter().map(|n| n.id).collect::<Vec<_>>()
    );

    assert_eq!(graph.get_all_nodes_by_property("riego", &int(2)).await.unwrap(), vec![nid(1)]);
    assert!(graph.get_all_nodes_by_property("riego", &int(1)).await.unwrap().is_empty());

    // Y el WHERE de NQL por la propiedad indexada.
    let r = graph
        .execute_nql("find m.riego from (m:Maceta) where m.riego = 2")
        .await
        .unwrap();
    assert_eq!(r.len(), 1);
    let r = graph
        .execute_nql("find m.riego from (m:Maceta) where m.riego = 1")
        .await
        .unwrap();
    assert_eq!(r.len(), 0);
}

/// UPDATE de una arista por patrón: el valor cambia, la arista no se
/// duplica en la adyacencia ni en el conteo.
#[tokio::test]
async fn update_de_arista_no_duplica_adyacencia() {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await.unwrap();
    let edge_id = sembrar(&graph).await;

    graph
        .execute_statement("update (a:Maceta)-[r:COMPARTE]->(b:Maceta) set r.litros = 7")
        .await
        .unwrap();

    let edge = graph.get_edge(edge_id).await.unwrap();
    assert_eq!(edge.properties.get("litros"), Some(&int(7)));
    assert_eq!(graph.edge_count().await.unwrap(), 1);
    let salientes = graph.get_outgoing_edges(nid(1)).await.unwrap();
    assert_eq!(salientes.len(), 1, "la reescritura no duplica la adyacencia");
    assert_eq!(salientes[0].properties.get("litros"), Some(&int(7)));
}

/// `limit` sigue acotando cuántos nodos entran al write-set.
#[tokio::test]
async fn update_respeta_limit() {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await.unwrap();
    sembrar(&graph).await;

    graph
        .execute_statement("update (m:Maceta) set m.riego = 9 limit 1")
        .await
        .unwrap();
    let con_9 = graph.get_all_nodes_by_property("riego", &int(9)).await.unwrap();
    assert_eq!(con_9.len(), 1, "limit 1 actualiza exactamente un nodo");
}

// ─── Crash real: el proceso muere sin close ni destructores ───────────────

const ENV_DB_DIR: &str = "NOPAL_NQL_UPDATE_CRASH_DIR";
const MARCA: &str = "update_confirmado";

/// Hijo: siembra, confirma los UPDATE de nodo y de arista, anota que
/// volvieron Ok y aborta el proceso (sin `close`, sin checkpoint, sin drop).
/// Ignorado en corridas normales; solo corre cuando el padre lo relanza.
#[tokio::test]
#[ignore = "solo corre como proceso hijo de update_sobrevive_a_un_crash"]
async fn nql_update_crash_child() {
    let Some(dir) = std::env::var_os(ENV_DB_DIR) else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let graph = Graph::open(&dir).await.expect("open child");
    sembrar(&graph).await;

    graph
        .execute_statement("update (m:Maceta) set m.riego = 2 where m.riego = 1")
        .await
        .expect("update nodo");
    graph
        .execute_statement("update (a:Maceta)-[r:COMPARTE]->(b:Maceta) set r.litros = 7")
        .await
        .expect("update arista");

    std::fs::write(dir.join(MARCA), b"ok").expect("marca");
    std::process::abort();
}

#[cfg(unix)]
#[tokio::test]
async fn update_sobrevive_a_un_crash() {
    let dir = tempfile::tempdir().unwrap();
    let exe = std::env::current_exe().expect("current_exe");
    let status = std::process::Command::new(&exe)
        .args(["nql_update_crash_child", "--ignored", "--exact", "--nocapture"])
        .env(ENV_DB_DIR, dir.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("spawn child");
    assert!(!status.success(), "el hijo debe morir por abort");
    assert!(
        dir.path().join(MARCA).exists(),
        "el hijo no llegó a confirmar los UPDATE (status {:?})",
        status
    );

    let graph = Graph::open(dir.path()).await.unwrap();
    let nodo = graph.get_node(nid(1)).await.unwrap();
    assert_eq!(
        nodo.properties.get("riego"),
        Some(&int(2)),
        "el UPDATE de nodo confirmado se perdió en el crash"
    );
    assert_eq!(graph.history(nid(1)).await.unwrap().len(), 2, "el redo rehace la versión");
    assert_eq!(graph.get_all_nodes_by_property("riego", &int(2)).await.unwrap(), vec![nid(1)]);
    assert!(graph.get_all_nodes_by_property("riego", &int(1)).await.unwrap().is_empty());

    let aristas = graph.get_outgoing_edges(nid(1)).await.unwrap();
    assert_eq!(aristas.len(), 1);
    assert_eq!(
        aristas[0].properties.get("litros"),
        Some(&int(7)),
        "el UPDATE de arista confirmado se perdió en el crash"
    );

    // Un segundo open no cambia nada (el redo es idempotente).
    graph.close().await.unwrap();
    drop(graph);
    let graph = Graph::open(dir.path()).await.unwrap();
    assert_eq!(graph.get_node(nid(1)).await.unwrap().properties.get("riego"), Some(&int(2)));
    assert_eq!(graph.history(nid(1)).await.unwrap().len(), 2);
    let arista = &graph.get_outgoing_edges(nid(1)).await.unwrap()[0];
    assert_eq!(arista.properties.get("litros"), Some(&int(7)));
}
