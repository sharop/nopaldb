// tests/wal_redo_integrity_test.rs
//
// Regresiones de integridad del redo del WAL (issue #65 — hallazgos H3/H4
// del oráculo de invariantes + deleted_edges sin registro WAL). Cada caso
// reabre la base SIN checkpoint previo: el WAL llega vivo al open y el redo
// es el sujeto bajo prueba. Dominio ficticio (vivero), sin datos reales.
//
//   (a) H3 — un commit que devuelve Err jamás se materializa (ni en la
//       sesión viva ni tras reopen): la prevalidación pre-WAL lo rechaza
//       antes de que su write-set toque el log.
//   (b) H4 — alta directa → DeleteNodeTx → AddNodeTx → reopen: el nodo
//       re-alteado sobrevive (antes, el redo re-aplicaba el delete viejo y
//       se saltaba la re-alta "ya aplicada").
//   (c) deleted_edges — una arista borrada por tx sigue borrada tras el
//       reopen (antes no había registro DeleteEdge y el redo la resucitaba
//       re-aplicando su InsertEdge).
//   (d) compat — base sin la marca `wal_applied_upto` (simula un WAL escrito
//       por un binario anterior): reabre bien y el replay en orden-de-log,
//       ahora CON DeleteEdge, converge al estado correcto.
//   (e) estabilidad — dos reopens consecutivos sobre un mundo mixto
//       directo/tx producen exactamente el mismo estado.

use nopaldb::{Edge, EdgeId, Graph, Node, NodeId, PropertyValue};
use std::collections::BTreeMap;
use uuid::Uuid;

fn nid(x: u128) -> NodeId {
    Uuid::from_u128(0x65_0000_0000 + x)
}

fn planta(x: u128, riego: i64) -> Node {
    Node::with_id(nid(x), "Planta").with_property("riego", PropertyValue::Int(riego))
}

/// (a) H3: tx{add_node(a), add_edge(a→b)} con b inexistente.
#[tokio::test]
async fn h3_commit_fallido_no_se_materializa() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (nid(0xA), nid(0xB)); // b jamás se crea

    {
        let graph = Graph::open(dir.path()).await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();
        tx.add_node(planta(0xA, 1)).await.unwrap();
        tx.add_edge(Edge::new(a, b, "riega")).unwrap();
        assert!(
            tx.commit().await.is_err(),
            "commit con endpoint inexistente debe fallar"
        );
        // La prevalidación es pre-WAL y pre-apply: ni siquiera en la sesión
        // viva debe existir el nodo del commit fallido.
        assert!(
            graph.get_node(a).await.is_err(),
            "el nodo de un commit con Err no debe existir en la sesión viva"
        );
    }

    let graph = Graph::open(dir.path()).await.unwrap();
    assert!(
        graph.get_node(a).await.is_err(),
        "H3: el redo materializó un commit que reportó Err"
    );
    assert!(graph.get_all_edges().await.unwrap().is_empty());
}

/// (b) H4: alta directa → DeleteNodeTx → AddNodeTx → reopen.
#[tokio::test]
async fn h4_realta_por_tx_sobrevive_al_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let x = nid(0x54);

    {
        let graph = Graph::open(dir.path()).await.unwrap();
        graph.add_node(planta(0x54, 1)).await.unwrap();

        let mut tx = graph.begin_transaction().await.unwrap();
        tx.delete_node(x).unwrap();
        tx.commit().await.unwrap();

        let mut tx = graph.begin_transaction().await.unwrap();
        tx.add_node(planta(0x54, 7)).await.unwrap();
        tx.commit().await.unwrap();
    }

    let graph = Graph::open(dir.path()).await.unwrap();
    let node = graph
        .get_node(x)
        .await
        .expect("H4: el redo borró un nodo commiteado al reabrir");
    assert_eq!(node.properties.get("riego"), Some(&PropertyValue::Int(7)));
}

/// Puebla una base con nodos+arista por tx y borra la arista por tx.
/// Devuelve el id de la arista borrada.
async fn poblar_y_borrar_arista(graph: &Graph) -> EdgeId {
    let mut tx = graph.begin_transaction().await.unwrap();
    tx.add_node(planta(0xC1, 1)).await.unwrap();
    tx.add_node(planta(0xC2, 2)).await.unwrap();
    let edge_id = tx.add_edge(Edge::new(nid(0xC1), nid(0xC2), "riega")).unwrap();
    tx.commit().await.unwrap();

    let mut tx = graph.begin_transaction().await.unwrap();
    tx.delete_edge(edge_id).unwrap();
    tx.commit().await.unwrap();

    edge_id
}

/// (c) deleted_edges: el borrado por tx sobrevive al reopen.
#[tokio::test]
async fn delete_edge_por_tx_persiste_tras_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let edge_id = {
        let graph = Graph::open(dir.path()).await.unwrap();
        poblar_y_borrar_arista(&graph).await
    };

    let graph = Graph::open(dir.path()).await.unwrap();
    assert!(
        graph.get_edge(edge_id).await.is_err(),
        "el redo resucitó una arista borrada por tx"
    );
    assert!(graph.get_node(nid(0xC1)).await.is_ok());
    assert!(graph.get_node(nid(0xC2)).await.is_ok());
}

/// (d) compat: sin la marca (WAL "viejo"), el replay completo con guardas
/// converge — el registro DeleteEdge va DESPUÉS de InsertEdge en el log y
/// re-aplicarlos en orden reproduce el estado final.
#[tokio::test]
async fn base_sin_marca_reabre_y_replaya_en_orden() {
    let dir = tempfile::tempdir().unwrap();
    let edge_id = {
        let graph = Graph::open(dir.path()).await.unwrap();
        let edge_id = poblar_y_borrar_arista(&graph).await;
        // Simula una base escrita por un binario anterior a la marca.
        graph
            .storage()
            .delete_meta(nopaldb::storage::META_WAL_APPLIED_UPTO)
            .await
            .unwrap();
        edge_id
    };

    let graph = Graph::open(dir.path()).await.unwrap();
    assert!(
        graph.get_edge(edge_id).await.is_err(),
        "replay sin marca: la arista borrada por tx resucitó"
    );
    assert!(graph.get_node(nid(0xC1)).await.is_ok());
    assert!(graph.get_node(nid(0xC2)).await.is_ok());
}

/// Foto del estado: nodos (id → riego) + aristas (id → extremos).
async fn foto(graph: &Graph) -> (BTreeMap<NodeId, Option<PropertyValue>>, BTreeMap<EdgeId, (NodeId, NodeId)>) {
    let nodos = graph
        .get_all_nodes()
        .await
        .unwrap()
        .into_iter()
        .map(|n| (n.id, n.properties.get("riego").cloned()))
        .collect();
    let aristas = graph
        .get_all_edges()
        .await
        .unwrap()
        .into_iter()
        .map(|e| (e.id, (e.source, e.target)))
        .collect();
    (nodos, aristas)
}

/// (e) doble reopen estable sobre un mundo mixto directo/tx.
#[tokio::test]
async fn doble_reopen_es_estable() {
    let dir = tempfile::tempdir().unwrap();

    {
        let graph = Graph::open(dir.path()).await.unwrap();
        // Mundo mixto: alta directa, delete por tx, re-alta por tx, y una
        // segunda planta enlazada por tx.
        graph.add_node(planta(0xE1, 1)).await.unwrap();

        let mut tx = graph.begin_transaction().await.unwrap();
        tx.delete_node(nid(0xE1)).unwrap();
        tx.commit().await.unwrap();

        let mut tx = graph.begin_transaction().await.unwrap();
        tx.add_node(planta(0xE1, 3)).await.unwrap();
        tx.commit().await.unwrap();

        let mut tx = graph.begin_transaction().await.unwrap();
        tx.add_node(planta(0xE2, 5)).await.unwrap();
        tx.add_edge(Edge::new(nid(0xE2), nid(0xE1), "poda")).unwrap();
        tx.commit().await.unwrap();
    }

    let primera = {
        let graph = Graph::open(dir.path()).await.unwrap();
        foto(&graph).await
    };
    let segunda = {
        let graph = Graph::open(dir.path()).await.unwrap();
        foto(&graph).await
    };

    assert_eq!(
        primera, segunda,
        "el estado cambió entre dos reopens consecutivos (redo no idempotente)"
    );
    assert_eq!(
        primera.0.get(&nid(0xE1)),
        Some(&Some(PropertyValue::Int(3))),
        "la re-alta por tx debe sobrevivir a todos los reopens"
    );
}
