// tests/mvcc_delete_closes_versions_test.rs
//
// Borrar un nodo CIERRA su historia MVCC: la versión vigente del nodo y la
// de cada arista que cae en cascada reciben `valid_to = ts del borrado`, por
// todos los caminos (directo, transacción, NQL `delete`, `delete_node_by_key`
// y redo del WAL). Antes la historia quedaba abierta y `as_of` posterior al
// borrado seguía mostrando el nodo y sus aristas.
//
// El redo del WAL, además, reaplica las aristas con el timestamp REGISTRADO
// del commit: antes `add_edge`/`delete_edge` tomaban uno nuevo del reloj y la
// historia reconstruida tras un crash no coincidía con la que ya se había
// mostrado. Dominio ficticio (vivero), sin datos reales.

use nopaldb::{Edge, EdgeId, Graph, Node, NodeId, PropertyValue};
use uuid::Uuid;

fn nid(x: u128) -> NodeId {
    Uuid::from_u128(0x0de1_0000_0000 + x)
}

fn planta(x: u128, nombre: &str) -> Node {
    Node::with_id(nid(x), "Planta")
        .with_property("nombre", PropertyValue::String(nombre.to_string()))
}

/// Vivero de tres plantas por transacción (todas con cadena MVCC) y
/// aristas `a→b`, `c→a` y un auto-lazo `a→a`, todas incidentes a `a`.
async fn vivero(graph: &Graph) -> (NodeId, NodeId, NodeId, Vec<EdgeId>) {
    let (a, b, c) = (nid(0xA), nid(0xB), nid(0xC));
    let mut tx = graph.begin_transaction().await.unwrap();
    tx.add_node(planta(0xA, "agave")).await.unwrap();
    tx.add_node(planta(0xB, "biznaga")).await.unwrap();
    tx.add_node(planta(0xC, "cempasuchil")).await.unwrap();
    let e1 = tx.add_edge(Edge::new(a, b, "poliniza")).unwrap();
    let e2 = tx.add_edge(Edge::new(c, a, "sombrea")).unwrap();
    let e3 = tx.add_edge(Edge::new(a, a, "injerta")).unwrap();
    tx.commit().await.unwrap();
    (a, b, c, vec![e1, e2, e3])
}

/// Comprueba que `id` quedó cerrado en un único `ts_d` (que se devuelve),
/// legible antes y ausente después, y que cada arista de `cascada` se
/// cerró con ese MISMO ts.
async fn assert_cerrado(graph: &Graph, id: NodeId, cascada: &[EdgeId]) -> u64 {
    let historia = graph.history(id).await.unwrap();
    let ultima = historia.first().expect("el nodo tenía cadena MVCC");
    let ts_d = ultima
        .valid_to
        .expect("borrar el nodo debe cerrar su versión vigente (valid_to)");
    assert!(ts_d > ultima.valid_from, "cierre posterior al alta");

    // El pasado sigue legible; desde el borrado, el nodo no existe.
    assert!(
        graph.get_node_at_strict(id, ts_d - 1).await.is_ok(),
        "antes del borrado el nodo debe verse en su historia"
    );
    assert!(
        graph.get_node_at_strict(id, ts_d).await.is_err(),
        "en el ts del borrado ya no hay versión válida"
    );
    assert!(
        graph.as_of(ts_d + 10).get_node(id).await.is_err(),
        "as_of posterior al borrado no debe ver el nodo"
    );

    for &e in cascada {
        let historia_e = graph.edge_history(e).await.unwrap();
        let ultima_e = historia_e.last().expect("la arista tenía versión");
        assert_eq!(
            ultima_e.valid_to,
            Some(ts_d),
            "la arista {e} borrada en cascada debe cerrarse con el ts del borrado del nodo"
        );
        assert!(graph.get_edge(e).await.is_err());
    }
    ts_d
}

#[tokio::test]
async fn borrado_directo_cierra_nodo_y_aristas() {
    let graph = Graph::in_memory().await.unwrap();
    let (a, b, c, aristas) = vivero(&graph).await;

    graph.delete_node(a).await.unwrap();

    assert_cerrado(&graph, a, &aristas).await;
    // Los otros extremos siguen vivos y con la versión abierta.
    for x in [b, c] {
        assert!(graph.get_node(x).await.is_ok());
        assert_eq!(graph.history(x).await.unwrap()[0].valid_to, None);
    }
}

#[tokio::test]
async fn borrado_por_transaccion_cierra_nodo_y_aristas() {
    let graph = Graph::in_memory().await.unwrap();
    let (a, _, _, aristas) = vivero(&graph).await;

    let mut tx = graph.begin_transaction().await.unwrap();
    tx.delete_node(a).unwrap();
    tx.commit().await.unwrap();

    assert_cerrado(&graph, a, &aristas).await;
}

#[tokio::test]
async fn borrado_nql_cierra_nodo_y_aristas() {
    let graph = Graph::in_memory().await.unwrap();
    let (a, _, _, aristas) = vivero(&graph).await;

    graph
        .execute_statement(r#"delete (p:Planta) where p.nombre = "agave""#)
        .await
        .unwrap();

    assert!(graph.get_node(a).await.is_err(), "el delete NQL no borró el nodo");
    assert_cerrado(&graph, a, &aristas).await;
}

#[tokio::test]
async fn borrado_por_clave_cierra_nodo_y_aristas() {
    let dir = tempfile::tempdir().unwrap();
    let graph = Graph::open(dir.path()).await.unwrap();
    let (a, _, _, aristas) = vivero(&graph).await;

    let borrado = graph
        .delete_node_by_key("Planta", "nombre", &PropertyValue::String("agave".into()))
        .await
        .unwrap();
    assert_eq!(borrado, Some(a));

    assert_cerrado(&graph, a, &aristas).await;
}

/// Un nodo sin cadena MVCC se borra sin error y sin historia que cerrar; su
/// arista versionada sí se cierra. Desde 0.10.1 el alta directa versiona, así
/// que el registro sin cadena se fabrica con la primitiva de bajo nivel
/// `Storage::insert_node` (lo que dejaba `add_node` en ≤ 0.10.0).
#[tokio::test]
async fn borrar_nodo_sin_cadena_no_falla() {
    let graph = Graph::in_memory().await.unwrap();
    let a = nid(0x1);
    graph.storage().insert_node(&planta(0x1, "nopal")).await.unwrap();
    let b = graph.add_node(planta(0x2, "maguey")).await.unwrap();
    let e = graph.add_edge(Edge::new(a, b, "acompaña")).await.unwrap();

    graph.delete_node(a).await.unwrap();

    assert!(graph.get_node(a).await.is_err());
    assert!(graph.history(a).await.unwrap().is_empty());
    let historia_e = graph.edge_history(e).await.unwrap();
    assert!(historia_e.last().unwrap().valid_to.is_some(), "la arista quedó abierta");
}

/// Recrear por transacción un nodo borrado continúa su cadena (v2) sin
/// re-fechar el cierre del borrado: entre el borrado y la re-alta el nodo
/// no existe en la historia.
#[tokio::test]
async fn recrear_tras_borrado_continua_la_cadena() {
    let graph = Graph::in_memory().await.unwrap();
    let (a, _, _, _) = vivero(&graph).await;
    graph.delete_node(a).await.unwrap();
    let ts_d = graph.history(a).await.unwrap()[0].valid_to.unwrap();

    let mut tx = graph.begin_transaction().await.unwrap();
    tx.add_node(planta(0xA, "agave azul")).await.unwrap();
    tx.commit().await.unwrap();

    let historia = graph.history(a).await.unwrap();
    assert_eq!(historia.len(), 2, "la re-alta debe ser v2, no pisar v1");
    let (v2, v1) = (&historia[0], &historia[1]);
    assert_eq!((v1.version, v2.version), (1, 2));
    assert_eq!(v1.valid_to, Some(ts_d), "la re-alta re-fechó el cierre del borrado");
    assert!(v2.valid_from > ts_d);
    assert_eq!(v2.valid_to, None);
    assert!(graph.get_node_at_strict(a, ts_d).await.is_err());
    assert!(graph.get_node(a).await.is_ok());
}

// ─── Redo del WAL ──────────────────────────────────────────────────────────

/// `(version, valid_from, valid_to, timestamp)` de cada versión.
type Huella = Vec<(u64, u64, Option<u64>, u64)>;

async fn huella_nodo(graph: &Graph, id: NodeId) -> Huella {
    graph
        .history(id)
        .await
        .unwrap()
        .iter()
        .map(|v| (v.version, v.valid_from, v.valid_to, v.timestamp))
        .collect()
}

async fn huella_arista(graph: &Graph, id: EdgeId) -> Huella {
    graph
        .edge_history(id)
        .await
        .unwrap()
        .iter()
        .map(|v| (v.version, v.valid_from, v.valid_to, v.timestamp))
        .collect()
}

async fn huellas(graph: &Graph, nodos: &[NodeId], aristas: &[EdgeId]) -> (Vec<Huella>, Vec<Huella>) {
    let mut n = Vec::new();
    for &id in nodos {
        n.push(huella_nodo(graph, id).await);
    }
    let mut e = Vec::new();
    for &id in aristas {
        e.push(huella_arista(graph, id).await);
    }
    (n, e)
}

/// Crash antes de aplicar nada al KV: solo el WAL sobrevive. Se simula
/// copiando `nopal.wal` a un directorio vacío; el open lo reproduce entero
/// y la historia reconstruida debe ser IDÉNTICA a la que la base original
/// mostraba (mismos valid_from/valid_to), con los nodos borrados cerrados.
/// Reabrir de nuevo — con y sin la marca de progreso — no añade versiones.
#[tokio::test]
async fn redo_del_wal_reconstruye_la_historia_con_los_ts_registrados() {
    let original = tempfile::tempdir().unwrap();
    let (nodos, aristas, esperado) = {
        let graph = Graph::open(original.path()).await.unwrap();
        let (a, b, c, mut aristas) = vivero(&graph).await;
        // Arista directa con ts propio, borrada después por tx.
        let e4 = graph.add_edge(Edge::new(b, c, "riega")).await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();
        tx.delete_edge(e4).unwrap();
        tx.commit().await.unwrap();
        aristas.push(e4);
        // Arista directa borrada directamente.
        let e5 = graph.add_edge(Edge::new(c, b, "abona")).await.unwrap();
        graph.delete_edge(e5).await.unwrap();
        aristas.push(e5);
        // Borrado directo de `a` (cascada a→b, c→a, a→a) y por tx de `c`.
        graph.delete_node(a).await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();
        tx.delete_node(c).unwrap();
        tx.commit().await.unwrap();

        let nodos = vec![a, b, c];
        let esperado = huellas(&graph, &nodos, &aristas).await;
        (nodos, aristas, esperado)
        // Drop sin close ni checkpoint: el WAL queda vivo.
    };
    // Sanidad de lo que la base original mostraba: nodos borrados cerrados.
    assert!(esperado.0[0][0].2.is_some() && esperado.0[2][0].2.is_some());
    assert!(esperado.1.iter().all(|h| h.last().unwrap().2.is_some()));

    let recuperada = tempfile::tempdir().unwrap();
    std::fs::copy(
        original.path().join("nopal.wal"),
        recuperada.path().join("nopal.wal"),
    )
    .unwrap();

    {
        let graph = Graph::open(recuperada.path()).await.unwrap();
        assert!(graph.get_node(nodos[0]).await.is_err());
        assert!(graph.get_node(nodos[1]).await.is_ok());
        assert!(graph.get_node(nodos[2]).await.is_err());
        assert_eq!(
            huellas(&graph, &nodos, &aristas).await,
            esperado,
            "el redo reconstruyó la historia con timestamps distintos a los del commit"
        );
        // Sin la marca, el segundo open repite el log entero con las guardas.
        graph
            .storage()
            .delete_meta(nopaldb::storage::META_WAL_APPLIED_UPTO)
            .await
            .unwrap();
    }
    let graph = Graph::open(recuperada.path()).await.unwrap();
    assert_eq!(
        huellas(&graph, &nodos, &aristas).await,
        esperado,
        "repetir el redo cambió la historia (no idempotente)"
    );
    drop(graph);

    // La base original, reabierta, tampoco cambia.
    let graph = Graph::open(original.path()).await.unwrap();
    assert_eq!(huellas(&graph, &nodos, &aristas).await, esperado);
}

/// El mismo crash, solo con aristas (sin borrar nodos): aísla el segundo
/// defecto — el redo de InsertEdge/DeleteEdge tomaba ts nuevos del reloj.
#[tokio::test]
async fn redo_de_aristas_conserva_valid_from_y_valid_to() {
    let original = tempfile::tempdir().unwrap();
    let (aristas, esperado) = {
        let graph = Graph::open(original.path()).await.unwrap();
        let (_, b, c, mut aristas) = vivero(&graph).await;
        let e4 = graph.add_edge(Edge::new(b, c, "riega")).await.unwrap();
        let e5 = graph.add_edge(Edge::new(c, b, "abona")).await.unwrap();
        graph.delete_edge(e5).await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();
        tx.delete_edge(aristas[0]).unwrap();
        tx.commit().await.unwrap();
        // Un nodo más después: el reloj sigue avanzando tras las aristas.
        graph.add_node(planta(0xD, "dalia")).await.unwrap();
        aristas.extend([e4, e5]);
        let esperado = huellas(&graph, &[], &aristas).await.1;
        (aristas, esperado)
    };

    let recuperada = tempfile::tempdir().unwrap();
    std::fs::copy(
        original.path().join("nopal.wal"),
        recuperada.path().join("nopal.wal"),
    )
    .unwrap();
    let graph = Graph::open(recuperada.path()).await.unwrap();
    assert_eq!(
        huellas(&graph, &[], &aristas).await.1,
        esperado,
        "el redo re-fechó las versiones de las aristas"
    );
}
