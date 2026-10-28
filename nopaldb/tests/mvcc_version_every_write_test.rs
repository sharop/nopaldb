// tests/mvcc_version_every_write_test.rs
//
// Toda escritura versiona (0.10.1). Hasta 0.10.0 quedaban sin versión MVCC:
//
// - `Graph::add_node` directo: solo el registro. Peor aún, su redo del WAL
//   SÍ versionaba, así que la misma escritura tenía historia tras un crash
//   y no sin él;
// - el `BulkLoader` (`add_nodes_batch`/`add_edges_batch`): ni versión, ni
//   avance del reloj lógico, ni `last_commit_timestamp`;
// - las aristas re-escritas con un id existente, o recreadas tras borrarlas,
//   reescribían su v1 en lugar de continuar la cadena.
//
// Y una base escrita así se ADOPTA al abrirla: lo que no tenía versión la
// recibe en un timestamp F con suelo de historia, y por debajo del suelo
// `get_node_at` contesta `HistoryUnavailable`.
//
// Este archivo solo usa API que ya existía en 0.10.0 (las metas de la
// adopción se nombran con su cadena), para que en esa versión falle en las
// aserciones y no al compilar. Dominio ficticio: un huerto con jardineros.

use nopaldb::error::NopalError;
use nopaldb::{Edge, EdgeId, Graph, Node, NodeId, PropertyValue};
use uuid::Uuid;

fn nid(x: u128) -> NodeId {
    Uuid::from_u128(0x6a2d_0000_0000 + x)
}

fn eid(x: u128) -> EdgeId {
    Uuid::from_u128(0x6a2d_e000_0000 + x)
}

fn planta(x: u128, nombre: &str, riegos: i64) -> Node {
    Node::with_id(nid(x), "Planta")
        .with_property("nombre", PropertyValue::String(nombre.to_string()))
        .with_property("riegos", PropertyValue::Int(riegos))
}

fn riegos(node: &Node) -> Option<&PropertyValue> {
    node.properties.get("riegos")
}

/// `(version, valid_from, valid_to, timestamp, prev_version)` de cada versión.
type Huella = Vec<(u64, u64, Option<u64>, u64, Option<u64>)>;

async fn huella_nodo(graph: &Graph, id: NodeId) -> Huella {
    graph
        .history(id)
        .await
        .unwrap()
        .iter()
        .map(|v| (v.version, v.valid_from, v.valid_to, v.timestamp, v.prev_version))
        .collect()
}

async fn huella_arista(graph: &Graph, id: EdgeId) -> Huella {
    graph
        .edge_history(id)
        .await
        .unwrap()
        .iter()
        .map(|v| (v.version, v.valid_from, v.valid_to, v.timestamp, v.prev_version))
        .collect()
}

// ─── add_node directo ──────────────────────────────────────────────────────

/// El alta directa deja v1 en su propio ts (el de `last_commit_timestamp`),
/// legible con `get_node_at`; re-escribir el mismo id cierra v1 y escribe v2.
#[tokio::test]
async fn add_node_directo_versiona_y_encadena() {
    let graph = Graph::in_memory().await.unwrap();
    let antes = graph.last_commit_timestamp();

    let id = graph.add_node(planta(0x1, "nopal", 1)).await.unwrap();
    let t1 = graph.last_commit_timestamp();
    assert!(t1 > antes, "la escritura directa cuenta para last_commit_timestamp");

    let historia = graph.history(id).await.unwrap();
    assert_eq!(historia.len(), 1, "add_node directo debe dejar una versión");
    assert_eq!((historia[0].version, historia[0].valid_from, historia[0].valid_to), (1, t1, None));
    assert_eq!(riegos(&graph.get_node_at(id, t1).await.unwrap()), Some(&PropertyValue::Int(1)));
    // Antes de su alta, el nodo no existía (tiene historia: no es "desconocido").
    let err = graph.get_node_at(id, t1 - 1).await.unwrap_err();
    assert!(matches!(err, NopalError::NodeNotFound(_)), "got {err:?}");

    // Re-escritura directa del mismo id: la cadena continúa.
    graph.add_node(planta(0x1, "nopal", 2)).await.unwrap();
    let t2 = graph.last_commit_timestamp();
    assert!(t2 > t1);
    let historia = graph.history(id).await.unwrap(); // más reciente primero
    assert_eq!(historia.len(), 2);
    assert_eq!((historia[0].version, historia[0].prev_version, historia[0].valid_from), (2, Some(1), t2));
    assert_eq!(historia[1].valid_to, Some(t2), "v1 se cierra en el ts de la re-escritura");
    assert_eq!(riegos(&graph.get_node_at(id, t1).await.unwrap()), Some(&PropertyValue::Int(1)));
    assert_eq!(riegos(&graph.get_node_at(id, t2).await.unwrap()), Some(&PropertyValue::Int(2)));
    assert_eq!(riegos(&graph.as_of(t2).get_node(id).await.unwrap()), Some(&PropertyValue::Int(2)));

    // Un update por transacción sigue la misma cadena (v3).
    let mut tx = graph.begin_transaction().await.unwrap();
    tx.add_node(planta(0x1, "nopal", 3)).await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(graph.history(id).await.unwrap()[0].version, 3);

    // Borrar y recrear directo: v4, sin re-fechar el cierre del borrado.
    graph.delete_node(id).await.unwrap();
    let cierre = graph.history(id).await.unwrap()[0].valid_to.expect("borrado cierra v3");
    graph.add_node(planta(0x1, "nopal", 4)).await.unwrap();
    let historia = graph.history(id).await.unwrap();
    assert_eq!(historia[0].version, 4);
    assert_eq!(historia[1].valid_to, Some(cierre));
    assert!(historia[0].valid_from > cierre);
}

/// Crash antes del checkpoint con solo escrituras directas de nodos: el
/// redo debe reconstruir EXACTAMENTE la historia que la base mostraba
/// (mismas versiones y timestamps). En 0.10.0 la base original no tenía
/// historia y la recuperada sí.
#[tokio::test]
async fn redo_de_add_node_directo_deja_la_misma_historia() {
    let original = tempfile::tempdir().unwrap();
    let ids = [nid(0x10), nid(0x11)];
    let esperado = {
        let graph = Graph::open(original.path()).await.unwrap();
        graph.add_node(planta(0x10, "maguey", 1)).await.unwrap();
        graph.add_node(planta(0x11, "biznaga", 1)).await.unwrap();
        graph.add_node(planta(0x10, "maguey", 2)).await.unwrap();
        let mut h = Vec::new();
        for id in ids {
            h.push(huella_nodo(&graph, id).await);
        }
        h
        // Drop sin close ni checkpoint: el WAL queda vivo.
    };
    assert_eq!(esperado[0].len(), 2, "dos escrituras directas = dos versiones");
    assert_eq!(esperado[1].len(), 1);

    // Solo sobrevive el WAL.
    let recuperada = tempfile::tempdir().unwrap();
    std::fs::copy(original.path().join("nopal.wal"), recuperada.path().join("nopal.wal")).unwrap();
    {
        let graph = Graph::open(recuperada.path()).await.unwrap();
        let mut h = Vec::new();
        for id in ids {
            h.push(huella_nodo(&graph, id).await);
        }
        assert_eq!(h, esperado, "el redo de add_node directo cambió la historia");
        assert_eq!(
            riegos(&graph.get_node(ids[0]).await.unwrap()),
            Some(&PropertyValue::Int(2))
        );
        // Sin la marca de progreso, el siguiente open repite el log entero.
        graph.storage().delete_meta(nopaldb::storage::META_WAL_APPLIED_UPTO).await.unwrap();
    }
    let graph = Graph::open(recuperada.path()).await.unwrap();
    for (id, h) in ids.iter().zip(&esperado) {
        assert_eq!(&huella_nodo(&graph, *id).await, h, "repetir el redo duplicó versiones");
    }
    drop(graph);

    // La original, reabierta (su WAL se reproduce sobre lo ya aplicado).
    let graph = Graph::open(original.path()).await.unwrap();
    for (id, h) in ids.iter().zip(&esperado) {
        assert_eq!(&huella_nodo(&graph, *id).await, h);
    }
}

// ─── BulkLoader ────────────────────────────────────────────────────────────

/// Los lotes del bulk versionan nodos y aristas con UN ts por lote, avanzan
/// el reloj y `last_commit_timestamp`; un id repetido en el lote deja una
/// sola versión (la de su última aparición) y un id existente continúa su
/// cadena.
#[tokio::test]
async fn bulk_loader_versiona_y_avanza_el_reloj() {
    let dir = tempfile::tempdir().unwrap();
    let (ts_nodos, ts_aristas) = {
        let graph = Graph::open(dir.path()).await.unwrap();
        let antes = graph.last_commit_timestamp();

        let mut loader = graph.bulk_loader(100);
        loader.add_node(planta(0x20, "cempasuchil", 1)).await.unwrap();
        loader.add_node(planta(0x21, "dalia", 1)).await.unwrap();
        // Repetido en el mismo lote: gana la última aparición.
        loader.add_node(planta(0x20, "cempasuchil", 7)).await.unwrap();
        loader
            .add_edge(Edge::with_id(eid(0x1), nid(0x20), nid(0x21), "acompaña"))
            .await
            .unwrap();
        loader.finish().await.unwrap();

        let h20 = graph.history(nid(0x20)).await.unwrap();
        let h21 = graph.history(nid(0x21)).await.unwrap();
        assert_eq!(h20.len(), 1, "el bulk debe versionar (una versión por id)");
        assert_eq!(h21.len(), 1);
        assert_eq!(riegos(&h20[0].node_data), Some(&PropertyValue::Int(7)), "gana la última aparición");
        let ts_nodos = h20[0].valid_from;
        assert_eq!(h21[0].valid_from, ts_nodos, "un timestamp por lote");
        assert!(ts_nodos > antes, "el bulk avanza el reloj");

        let he = graph.edge_history(eid(0x1)).await.unwrap();
        assert_eq!(he.len(), 1);
        let ts_aristas = he[0].valid_from;
        assert!(ts_aristas > ts_nodos, "cada lote toma su propio timestamp");
        assert!(graph.last_commit_timestamp() >= ts_aristas, "el bulk cuenta para last_commit_timestamp");
        let ahora = graph.last_commit_timestamp();
        assert_eq!(
            riegos(&graph.as_of(ahora).get_node(nid(0x20)).await.unwrap()),
            Some(&PropertyValue::Int(7))
        );
        assert!(graph.get_node_at(nid(0x20), ts_nodos - 1).await.is_err());

        // Segunda carga sobre los mismos ids: la cadena continúa.
        let mut loader = graph.bulk_loader(100);
        loader.add_node(planta(0x21, "dalia", 2)).await.unwrap();
        loader.finish().await.unwrap();
        let h21 = graph.history(nid(0x21)).await.unwrap();
        assert_eq!(h21.len(), 2);
        assert_eq!((h21[0].version, h21[0].prev_version), (2, Some(1)));
        assert_eq!(h21[1].valid_to, Some(h21[0].valid_from));
        graph.close().await.unwrap();
        (ts_nodos, ts_aristas)
    };

    // El reloj quedó persistido: lo siguiente va después de lo cargado.
    let graph = Graph::open(dir.path()).await.unwrap();
    assert!(graph.last_commit_timestamp() > ts_aristas);
    graph.add_node(planta(0x22, "girasol", 1)).await.unwrap();
    let t = graph.history(nid(0x22)).await.unwrap()[0].valid_from;
    assert!(t > ts_nodos && t > ts_aristas, "el reloj retrocedió tras el bulk");
}

// ─── Cadena de aristas ─────────────────────────────────────────────────────

/// Re-escribir una arista existente encadena v2 (cerrando v1); recrearla
/// tras borrarla sigue en v3 sin re-fechar el cierre del borrado.
#[tokio::test]
async fn re_alta_de_arista_continua_la_cadena() {
    let graph = Graph::in_memory().await.unwrap();
    let a = graph.add_node(planta(0x30, "jitomate", 1)).await.unwrap();
    let b = graph.add_node(planta(0x31, "albahaca", 1)).await.unwrap();
    let id = eid(0x30);
    let arista = |dosis: i64| {
        Edge::with_id(id, a, b, "protege").with_property("dosis", PropertyValue::Int(dosis))
    };

    graph.add_edge(arista(1)).await.unwrap();
    graph.add_edge(arista(2)).await.unwrap();
    let h = graph.edge_history(id).await.unwrap(); // más antigua primero
    assert_eq!(h.len(), 2, "re-escribir una arista debe encadenar v2, no pisar v1");
    assert_eq!((h[1].version, h[1].prev_version), (2, Some(1)));
    assert_eq!(h[0].valid_to, Some(h[1].valid_from), "v1 se cierra en el ts de v2");
    assert_eq!(h[0].edge_data.properties.get("dosis"), Some(&PropertyValue::Int(1)));

    graph.delete_edge(id).await.unwrap();
    let cierre = graph.edge_history(id).await.unwrap()[1].valid_to.expect("borrar cierra v2");

    // Por transacción, para cubrir también ese camino.
    let mut tx = graph.begin_transaction().await.unwrap();
    tx.add_edge(arista(3)).unwrap();
    tx.commit().await.unwrap();
    let h = graph.edge_history(id).await.unwrap();
    assert_eq!(h.len(), 3, "recrear tras borrar debe seguir desde la versión más alta");
    assert_eq!((h[2].version, h[2].prev_version), (3, Some(2)));
    assert_eq!(h[1].valid_to, Some(cierre), "no se re-fecha el cierre del borrado");
    assert!(h[2].valid_from > cierre);
    assert_eq!(graph.get_edge(id).await.unwrap().properties.get("dosis"), Some(&PropertyValue::Int(3)));
    // Una sola entrada en la adyacencia pese a las tres altas.
    assert_eq!(graph.neighbors(a, nopaldb::Direction::Outgoing).await.unwrap(), vec![b]);
}

/// El redo (dos veces) de alta, re-alta, borrado y recreación de una arista
/// reconstruye la misma cadena, sin versiones duplicadas.
#[tokio::test]
async fn redo_de_la_cadena_de_aristas_no_duplica() {
    let original = tempfile::tempdir().unwrap();
    let id = eid(0x40);
    let esperado = {
        let graph = Graph::open(original.path()).await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();
        tx.add_node(planta(0x40, "frijol", 1)).await.unwrap();
        tx.add_node(planta(0x41, "maíz", 1)).await.unwrap();
        tx.commit().await.unwrap();
        let arista = |n: i64| {
            Edge::with_id(id, nid(0x41), nid(0x40), "sostiene").with_property("n", PropertyValue::Int(n))
        };
        graph.add_edge(arista(1)).await.unwrap();
        graph.add_edge(arista(2)).await.unwrap();
        graph.delete_edge(id).await.unwrap();
        graph.add_edge(arista(3)).await.unwrap();
        huella_arista(&graph, id).await
    };
    assert_eq!(esperado.len(), 3);

    let recuperada = tempfile::tempdir().unwrap();
    std::fs::copy(original.path().join("nopal.wal"), recuperada.path().join("nopal.wal")).unwrap();
    {
        let graph = Graph::open(recuperada.path()).await.unwrap();
        assert_eq!(huella_arista(&graph, id).await, esperado, "el redo cambió la cadena de la arista");
        graph.storage().delete_meta(nopaldb::storage::META_WAL_APPLIED_UPTO).await.unwrap();
    }
    let graph = Graph::open(recuperada.path()).await.unwrap();
    assert_eq!(huella_arista(&graph, id).await, esperado, "repetir el redo duplicó versiones");
    drop(graph);
    let graph = Graph::open(original.path()).await.unwrap();
    assert_eq!(huella_arista(&graph, id).await, esperado);
}

// ─── Adopción de bases sin versionar ───────────────────────────────────────

/// Metas de la adopción, por su nombre (existen desde 0.10.1).
const META_ADOPTADA: &str = "history_adopted";

/// Una base "de 0.10.0": registros sin versión escritos con las primitivas
/// de bajo nivel de `Storage` (lo que dejaban `add_node` directo y el bulk),
/// una cadena abierta de un nodo ya borrado, una cadena pisada por una
/// escritura sin versión, una arista sin versión y una versión abierta de
/// una arista ya borrada; más un nodo con historia transaccional sana.
struct Huerto {
    sana: NodeId,
    t_sana_v1: u64,
    sueltos: Vec<NodeId>,
    borrado: NodeId,
    pisado: NodeId,
    arista_suelta: EdgeId,
    arista_borrada: EdgeId,
    reloj: u64,
}

async fn huerto_sin_versionar(graph: &Graph) -> Huerto {
    let storage = graph.storage();

    // Historia sana: dos commits.
    let mut tx = graph.begin_transaction().await.unwrap();
    tx.add_node(planta(0x50, "romero", 1)).await.unwrap();
    tx.add_node(planta(0x51, "tomillo", 1)).await.unwrap();
    tx.add_node(planta(0x52, "laurel", 1)).await.unwrap();
    tx.add_node(planta(0x53, "salvia", 1)).await.unwrap();
    tx.add_edge(Edge::with_id(eid(0x52), nid(0x50), nid(0x53), "aroma")).unwrap();
    tx.commit().await.unwrap();
    let t_sana_v1 = graph.history(nid(0x50)).await.unwrap()[0].valid_from;
    let mut tx = graph.begin_transaction().await.unwrap();
    tx.add_node(planta(0x50, "romero", 2)).await.unwrap();
    tx.commit().await.unwrap();

    // Registros sin versión: alta directa y bulk de ≤ 0.10.0.
    storage.insert_node(&planta(0x54, "ruda", 1)).await.unwrap();
    storage
        .insert_nodes_batch(&[planta(0x55, "epazote", 1), planta(0x56, "hierbabuena", 1)])
        .await
        .unwrap();
    // Arista sin versión (bulk de ≤ 0.10.0: registro + adyacencia).
    storage
        .insert_edges_batch(&[Edge::with_id(eid(0x50), nid(0x54), nid(0x55), "repele")])
        .await
        .unwrap();

    // Nodo borrado antes de que el borrado cerrara su versión: el registro
    // desaparece y su cadena queda abierta.
    storage.delete_node(nid(0x51)).await.unwrap();
    // Cadena pisada por una escritura sin versión (add_node directo sobre un
    // nodo con historia, en ≤ 0.10.0).
    storage.insert_node(&planta(0x52, "laurel", 9)).await.unwrap();
    // Arista borrada sin cerrar su versión (registro fuera, current abierto).
    storage.delete_edge(eid(0x52)).await.unwrap();
    storage.debug_clear_adjacency().await.unwrap(); // el open la reconstruye

    // Una base de 0.10.0 no tiene la marca de adopción.
    storage.delete_meta(META_ADOPTADA).await.unwrap();

    Huerto {
        sana: nid(0x50),
        t_sana_v1,
        sueltos: vec![nid(0x54), nid(0x55), nid(0x56)],
        borrado: nid(0x51),
        pisado: nid(0x52),
        arista_suelta: eid(0x50),
        arista_borrada: eid(0x52),
        reloj: graph.last_commit_timestamp(),
    }
}

fn es_desconocida(r: nopaldb::Result<Node>) -> bool {
    matches!(r, Err(NopalError::HistoryUnavailable(_)))
}

#[tokio::test]
async fn adopcion_versiona_lo_que_no_tenia_version() {
    let dir = tempfile::tempdir().unwrap();
    let h = {
        let graph = Graph::open(dir.path()).await.unwrap();
        let h = huerto_sin_versionar(&graph).await;
        graph.close().await.unwrap();
        h
    };

    let graph = Graph::open(dir.path()).await.unwrap();

    // Cada registro sin versión recibe v1 en el MISMO F, posterior a todo.
    let f = graph.history(h.sueltos[0]).await.unwrap().first().map(|v| v.valid_from)
        .expect("la adopción debe versionar los registros sin cadena");
    assert!(f > h.reloj, "F se toma del reloj, después de todo lo escrito");
    assert!(graph.last_commit_timestamp() >= f, "la adopción cuenta como escritura");
    for &id in &h.sueltos {
        let hist = graph.history(id).await.unwrap();
        assert_eq!(hist.len(), 1);
        assert_eq!((hist[0].version, hist[0].valid_from, hist[0].valid_to), (1, f, None));
        assert!(graph.get_node_at(id, f).await.is_ok());
        // Por debajo del suelo: desconocido, nunca "no existía".
        assert!(es_desconocida(graph.get_node_at(id, f - 1).await), "{id} por debajo del suelo");
        assert!(es_desconocida(graph.get_node_at(id, 0).await));
    }
    assert!(matches!(
        graph.as_of(f - 1).get_nodes(&[h.sana, h.sueltos[0]]).await,
        Err(NopalError::HistoryUnavailable(_))
    ));

    // La historia sana conserva su pasado real (sin suelo).
    assert_eq!(graph.history(h.sana).await.unwrap().len(), 2);
    assert_eq!(
        riegos(&graph.get_node_at(h.sana, h.t_sana_v1).await.unwrap()),
        Some(&PropertyValue::Int(1))
    );

    // Cadena abierta de un nodo ya borrado: se cierra en F.
    let hist = graph.history(h.borrado).await.unwrap();
    assert_eq!(hist[0].valid_to, Some(f));
    assert!(matches!(graph.get_node_at(h.borrado, f).await, Err(NopalError::NodeNotFound(_))));
    assert!(es_desconocida(graph.get_node_at(h.borrado, f - 1).await));

    // Cadena pisada: v2 en F con lo que dice el registro; v1 cerrada en F.
    let hist = graph.history(h.pisado).await.unwrap();
    assert_eq!(hist.len(), 2);
    assert_eq!((hist[0].version, hist[0].valid_from), (2, f));
    assert_eq!(riegos(&hist[0].node_data), Some(&PropertyValue::Int(9)));
    assert_eq!(hist[1].valid_to, Some(f));

    // Arista sin versión: v1 en F. Versión abierta de arista borrada: cerrada en F.
    let he = graph.edge_history(h.arista_suelta).await.unwrap();
    assert_eq!(he.len(), 1);
    assert_eq!((he[0].valid_from, he[0].valid_to), (f, None));
    let he = graph.edge_history(h.arista_borrada).await.unwrap();
    assert_eq!(he.last().unwrap().valid_to, Some(f));
    // Las lecturas por tipo en el pasado tampoco inventan.
    assert!(matches!(
        graph.storage().get_versioned_edges_of_type_at("repele", f - 1).await,
        Err(NopalError::HistoryUnavailable(_))
    ));
    assert_eq!(graph.storage().get_versioned_edges_of_type_at("repele", f).await.unwrap().len(), 1);

    assert!(graph.storage().get_meta_u64(META_ADOPTADA).await.unwrap().is_some());

    // Lo escrito después va por encima de F.
    graph.add_node(planta(0x57, "manzanilla", 1)).await.unwrap();
    assert!(graph.history(nid(0x57)).await.unwrap()[0].valid_from > f);
}

/// La pasada corre una vez: reabrir no toca nada. Y es reanudable: sin la
/// marca (crash antes de escribirla) repite sin efectos sobre lo adoptado y
/// adopta lo que faltara con el MISMO F persistido.
#[tokio::test]
async fn adopcion_corre_una_vez_y_se_reanuda_con_el_mismo_f() {
    let dir = tempfile::tempdir().unwrap();
    let h = {
        let graph = Graph::open(dir.path()).await.unwrap();
        let h = huerto_sin_versionar(&graph).await;
        graph.close().await.unwrap();
        h
    };
    let todos: Vec<NodeId> = h
        .sueltos
        .iter()
        .copied()
        .chain([h.sana, h.borrado, h.pisado])
        .collect();
    let aristas = [h.arista_suelta, h.arista_borrada];

    let huellas = |graph: Graph, todos: Vec<NodeId>| async move {
        let mut n = Vec::new();
        for id in &todos {
            n.push(huella_nodo(&graph, *id).await);
        }
        let mut e = Vec::new();
        for id in aristas {
            e.push(huella_arista(&graph, id).await);
        }
        (n, e)
    };

    let (primera, f) = {
        let graph = Graph::open(dir.path()).await.unwrap();
        let f = graph.history(h.sueltos[0]).await.unwrap()[0].valid_from;
        let hs = huellas(graph.clone(), todos.clone()).await;
        graph.close().await.unwrap();
        (hs, f)
    };

    // Segundo open: nada cambia.
    {
        let graph = Graph::open(dir.path()).await.unwrap();
        assert_eq!(huellas(graph.clone(), todos.clone()).await, primera, "la adopción corrió dos veces");
        // Simula una pasada interrumpida antes de su marca, con un registro
        // aún sin adoptar (escrito, como en 0.10.0, sin versión).
        graph.storage().insert_node(&planta(0x58, "orégano", 1)).await.unwrap();
        graph.storage().delete_meta(META_ADOPTADA).await.unwrap();
        graph.close().await.unwrap();
    }

    let graph = Graph::open(dir.path()).await.unwrap();
    assert_eq!(
        huellas(graph.clone(), todos.clone()).await,
        primera,
        "re-adoptar lo ya adoptado debe ser un no-op"
    );
    let hist = graph.history(nid(0x58)).await.unwrap();
    assert_eq!(hist.len(), 1, "la reanudación adopta lo que faltaba");
    assert_eq!(hist[0].valid_from, f, "la reanudación usa el mismo F");
    assert!(es_desconocida(graph.get_node_at(nid(0x58), f - 1).await));
}
