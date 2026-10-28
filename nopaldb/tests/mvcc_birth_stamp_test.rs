// tests/mvcc_birth_stamp_test.rs
//
// Marca de nacimiento y suelo (0.10.1). El `BulkLoader` no copia cada nodo
// nuevo en una v1 (+62 % de disco): escribe una marca de NACIMIENTO, y el
// nodo sin cadena con marca tiene una versión implícita, su registro,
// válida desde la marca. La adopción de bases anteriores escribe un SUELO
// con la misma idea. Todo escritor que cambie o borre ese registro
// materializa antes la v1 desde él. Tabla de precedencia en `storage::Stamp`.
//
// Hasta 0.10.0 lo cargado por el bulk no tenía historia ninguna: estas
// pruebas fallan ahí en las aserciones (solo usan API existente, más las
// primitivas `debug_raw_*` que ya había). Dominio ficticio: recetas.

use nopaldb::error::NopalError;
use nopaldb::mvcc::GCConfig;
use nopaldb::{Edge, EdgeId, Graph, Node, NodeId, PropertyValue};
use uuid::Uuid;

fn nid(x: u128) -> NodeId {
    Uuid::from_u128(0x7ec1_0000_0000 + x)
}

fn eid(x: u128) -> EdgeId {
    Uuid::from_u128(0x7ec1_e000_0000 + x)
}

fn receta(x: u128, porciones: i64) -> Node {
    Node::with_id(nid(x), "Receta").with_property("porciones", PropertyValue::Int(porciones))
}

fn porciones(n: &Node) -> Option<&PropertyValue> {
    n.properties.get("porciones")
}

fn raw_node_key(tag: u8, id: NodeId) -> Vec<u8> {
    let mut k = vec![tag];
    k.extend_from_slice(id.as_bytes());
    k
}

fn raw_version_key(id: NodeId, version: u64) -> Vec<u8> {
    let mut k = raw_node_key(b'v', id);
    k.extend_from_slice(&version.to_be_bytes());
    k
}

fn raw_edge_version_key(id: EdgeId, version: u64) -> Vec<u8> {
    format!("{id}:v{version:020}").into_bytes()
}

fn tiene(graph: &Graph, keyspace: &str, key: &[u8]) -> bool {
    graph.storage().debug_raw_get(keyspace, key).unwrap().is_some()
}

async fn bulk_nodos(graph: &Graph, nodos: Vec<Node>) -> u64 {
    let mut loader = graph.bulk_loader(10_000);
    for n in nodos {
        loader.add_node(n).await.unwrap();
    }
    loader.finish().await.unwrap();
    graph.last_commit_timestamp()
}

async fn bulk_aristas(graph: &Graph, aristas: Vec<Edge>) -> u64 {
    let mut loader = graph.bulk_loader(10_000);
    for e in aristas {
        loader.add_edge(e).await.unwrap();
    }
    loader.finish().await.unwrap();
    graph.last_commit_timestamp()
}

fn desconocido<T: std::fmt::Debug>(r: nopaldb::Result<T>) -> bool {
    matches!(r, Err(NopalError::HistoryUnavailable(_)))
}

fn no_existe<T: std::fmt::Debug>(r: nopaldb::Result<T>) -> bool {
    matches!(r, Err(NopalError::NodeNotFound(_)))
}

/// Fila "sin cadena, NACIMIENTO T": antes de T no existía, desde T es su
/// registro; sin copia en `v|`. Para aristas, igual, y las lecturas por tipo
/// en el pasado las incluyen.
#[tokio::test]
async fn el_bulk_marca_el_nacimiento_sin_copiar() {
    let graph = Graph::in_memory().await.unwrap();
    let t = bulk_nodos(&graph, vec![receta(1, 4), receta(2, 6)]).await;
    let x = nid(1);

    let h = graph.history(x).await.unwrap();
    assert_eq!(h.len(), 1, "el nodo del bulk debe tener historia");
    assert_eq!((h[0].version, h[0].valid_from, h[0].valid_to), (1, t, None));
    assert!(!tiene(&graph, "history", &raw_version_key(x, 1)), "el bulk no debe copiar el nodo en una v1");
    assert!(tiene(&graph, "history", &raw_node_key(b'b', x)), "marca de nacimiento");

    assert!(no_existe(graph.get_node_at(x, t - 1).await), "antes de nacer no existía");
    assert_eq!(porciones(&graph.get_node_at(x, t).await.unwrap()), Some(&PropertyValue::Int(4)));
    assert!(graph.as_of(t).get_node(x).await.is_ok());
    assert!(graph.get_node_at_strict(x, t).await.is_ok(), "lectura de snapshot de una tx");

    let te = bulk_aristas(&graph, vec![Edge::with_id(eid(1), nid(1), nid(2), "acompaña")]).await;
    let he = graph.edge_history(eid(1)).await.unwrap();
    assert_eq!(he.len(), 1);
    assert_eq!((he[0].valid_from, he[0].valid_to), (te, None));
    assert!(!tiene(&graph, "versioned_edges", &raw_edge_version_key(eid(1), 1)));
    let storage = graph.storage();
    assert_eq!(storage.get_versioned_edges_of_type_at("acompaña", te).await.unwrap().len(), 1);
    assert!(storage.get_versioned_edges_of_type_at("acompaña", te - 1).await.unwrap().is_empty());

    // La exportación de historia también la ve.
    let batch = graph.history_to_arrow().await.unwrap();
    assert_eq!(batch.num_rows(), 2);
}

/// Cada escritor materializa la v1 implícita desde el registro, en el mismo
/// lote, antes de seguir: tx, `add_node` directo, borrado (y su cascada de
/// aristas), borrado de arista, re-alta de arista y un segundo bulk.
#[tokio::test]
async fn cada_escritor_materializa_la_v1() {
    let graph = Graph::in_memory().await.unwrap();
    let t = bulk_nodos(&graph, (1..=5).map(|x| receta(x, x as i64)).collect()).await;
    let te = bulk_aristas(
        &graph,
        vec![
            Edge::with_id(eid(1), nid(1), nid(2), "sigue"),
            Edge::with_id(eid(2), nid(3), nid(4), "sigue"),
            Edge::with_id(eid(3), nid(2), nid(3), "sigue"),
            Edge::with_id(eid(4), nid(4), nid(5), "sigue"),
        ],
    )
    .await;

    let comprueba_v1 = |h: &[nopaldb::mvcc::VersionedNode], desde: u64, hasta: u64, p: i64| {
        let v1 = h.iter().find(|v| v.version == 1).expect("v1 materializada");
        assert_eq!((v1.valid_from, v1.valid_to), (desde, Some(hasta)));
        assert_eq!(porciones(&v1.node_data), Some(&PropertyValue::Int(p)), "v1 = los datos del bulk");
    };

    // Transacción sobre nid(1).
    let mut tx = graph.begin_transaction().await.unwrap();
    tx.add_node(receta(1, 10)).await.unwrap();
    tx.commit().await.unwrap();
    let u = graph.last_commit_timestamp();
    let h = graph.history(nid(1)).await.unwrap();
    assert_eq!(h.len(), 2);
    comprueba_v1(&h, t, u, 1);
    assert_eq!((h[0].version, h[0].valid_from), (2, u));
    assert!(tiene(&graph, "history", &raw_version_key(nid(1), 1)));
    assert!(!tiene(&graph, "history", &raw_node_key(b'b', nid(1))), "la cadena gana: marca fuera");
    assert_eq!(porciones(&graph.get_node_at(nid(1), u - 1).await.unwrap()), Some(&PropertyValue::Int(1)));

    // add_node directo sobre nid(2).
    graph.add_node(receta(2, 20)).await.unwrap();
    let u2 = graph.last_commit_timestamp();
    comprueba_v1(&graph.history(nid(2)).await.unwrap(), t, u2, 2);

    // Borrar nid(3): v1 cerrada en el borrado; cascada de eid(2) y eid(3).
    graph.delete_node(nid(3)).await.unwrap();
    let d = graph.last_commit_timestamp();
    let h = graph.history(nid(3)).await.unwrap();
    assert_eq!(h.len(), 1);
    comprueba_v1(&h, t, d, 3);
    assert!(no_existe(graph.get_node_at(nid(3), d).await));
    assert_eq!(porciones(&graph.get_node_at(nid(3), d - 1).await.unwrap()), Some(&PropertyValue::Int(3)));
    for e in [eid(2), eid(3)] {
        let he = graph.edge_history(e).await.unwrap();
        assert_eq!(he.len(), 1, "la cascada materializa la v1 de {e}");
        assert_eq!((he[0].valid_from, he[0].valid_to), (te, Some(d)));
        assert!(tiene(&graph, "versioned_edges", &raw_edge_version_key(e, 1)));
    }
    let storage = graph.storage();
    assert_eq!(storage.get_versioned_edges_of_type_at("sigue", d - 1).await.unwrap().len(), 4);
    assert_eq!(storage.get_versioned_edges_of_type_at("sigue", d).await.unwrap().len(), 2);

    // Borrar la arista eid(1), y recrearla: v1 [te, x) y v2.
    graph.delete_edge(eid(1)).await.unwrap();
    let x = graph.last_commit_timestamp();
    graph.add_edge(Edge::with_id(eid(1), nid(1), nid(2), "sigue")).await.unwrap();
    let he = graph.edge_history(eid(1)).await.unwrap();
    assert_eq!(he.len(), 2);
    assert_eq!((he[0].valid_from, he[0].valid_to), (te, Some(x)));
    assert_eq!((he[1].version, he[1].prev_version), (2, Some(1)));

    // Re-alta directa de eid(4), viva: v1 [te, y) y v2.
    graph
        .add_edge(Edge::with_id(eid(4), nid(4), nid(5), "sigue").with_property("orden", 2))
        .await
        .unwrap();
    let y = graph.last_commit_timestamp();
    let he = graph.edge_history(eid(4)).await.unwrap();
    assert_eq!(he.len(), 2);
    assert_eq!((he[0].valid_from, he[0].valid_to, he[1].valid_from), (te, Some(y), y));

    // Segundo bulk sobre nid(4), que ya existe: plan versionado normal.
    let t2 = bulk_nodos(&graph, vec![receta(4, 40)]).await;
    comprueba_v1(&graph.history(nid(4)).await.unwrap(), t, t2, 4);
}

/// Filas de la tabla con suelo, y "sin cadena, sin marca": una base "de
/// 0.10.0" (registros escritos con la primitiva sin historia) adoptada al
/// abrir; luego una transacción sobre el nodo adoptado (con cadena y suelo).
#[tokio::test]
async fn filas_de_suelo_y_sin_marca() {
    let dir = tempfile::tempdir().unwrap();
    let viejo = nid(0x10);
    {
        let graph = Graph::open(dir.path()).await.unwrap();
        graph.storage().insert_node(&receta(0x10, 2)).await.unwrap();
        graph.storage().delete_meta("history_adopted").await.unwrap();
        graph.close().await.unwrap();
    }
    let graph = Graph::open(dir.path()).await.unwrap();
    let f = graph.history(viejo).await.unwrap()[0].valid_from;

    // Sin cadena, SUELO F: solo el suelo, sin copia.
    assert!(tiene(&graph, "history", &raw_node_key(b'f', viejo)));
    assert!(!tiene(&graph, "history", &raw_version_key(viejo, 1)), "la adopción no debe copiar el nodo");
    assert!(desconocido(graph.get_node_at(viejo, f - 1).await));
    assert!(graph.get_node_at(viejo, f).await.is_ok());

    // Con cadena Y suelo: bajo F sigue desconocido; desde F, la cadena.
    let mut tx = graph.begin_transaction().await.unwrap();
    tx.add_node(receta(0x10, 3)).await.unwrap();
    tx.commit().await.unwrap();
    let u = graph.last_commit_timestamp();
    assert!(tiene(&graph, "history", &raw_node_key(b'f', viejo)), "el suelo persiste con cadena");
    assert!(desconocido(graph.get_node_at(viejo, f - 1).await));
    assert_eq!(porciones(&graph.get_node_at(viejo, f).await.unwrap()), Some(&PropertyValue::Int(2)));
    assert_eq!(porciones(&graph.get_node_at(viejo, u).await.unwrap()), Some(&PropertyValue::Int(3)));
    // Una primitiva sin historia que lo pise ya no toca el suelo (hay cadena).
    graph.storage().insert_node(&receta(0x10, 4)).await.unwrap();
    assert!(tiene(&graph, "history", &raw_node_key(b'f', viejo)));

    // Sin cadena, sin marca (primitiva sin historia en una base ya
    // adoptada): desconocido siempre.
    let suelto = nid(0x11);
    graph.storage().insert_node(&receta(0x11, 1)).await.unwrap();
    let ahora = graph.last_commit_timestamp();
    assert!(desconocido(graph.get_node_at(suelto, ahora).await));
    assert!(desconocido(graph.get_node_at(suelto, 0).await));
    assert!(graph.history(suelto).await.unwrap().is_empty());
}

/// Punto 4: una primitiva sin historia que pisa un nodo del bulk (sin
/// cadena) retira su marca: el nodo pasa a "sin historia" en vez de
/// presentar los datos nuevos como si existieran desde la marca.
#[tokio::test]
async fn la_primitiva_que_pisa_retira_la_marca() {
    let graph = Graph::in_memory().await.unwrap();
    let t = bulk_nodos(&graph, vec![receta(0x20, 1)]).await;
    graph.storage().insert_node(&receta(0x20, 99)).await.unwrap();
    assert!(!tiene(&graph, "history", &raw_node_key(b'b', nid(0x20))));
    assert!(desconocido(graph.get_node_at(nid(0x20), t).await), "no se inventa el pasado");
}

/// Crash real tras el bulk: el motor conserva el bulk (finish() hace
/// checkpoint durable) pero pierde la transacción posterior, que solo
/// sobrevive en el WAL. El redo materializa la v1 desde la marca con el ts
/// del commit: historia = [datos del bulk desde T hasta u, v2].
#[tokio::test]
async fn redo_tras_bulk_materializa_desde_la_marca() {
    let original = tempfile::tempdir().unwrap();
    let copia = tempfile::tempdir().unwrap();
    let x = nid(0x30);
    let t = {
        let graph = Graph::open(original.path()).await.unwrap();
        let t = bulk_nodos(&graph, vec![receta(0x30, 1)]).await;
        graph.close().await.unwrap();
        t
    };
    // Instantánea del motor con solo el bulk.
    for entry in std::fs::read_dir(original.path()).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            std::fs::copy(entry.path(), copia.path().join(entry.file_name())).unwrap();
        }
    }
    let esperado = {
        let graph = Graph::open(original.path()).await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();
        tx.add_node(receta(0x30, 2)).await.unwrap();
        tx.commit().await.unwrap();
        let h: Vec<_> = graph.history(x).await.unwrap().iter().map(|v| (v.version, v.valid_from, v.valid_to)).collect();
        h
        // Drop sin close: el WAL conserva la transacción.
    };
    let u = esperado[0].1;
    assert_eq!(esperado, vec![(2, u, None), (1, t, Some(u))]);

    // El motor de la instantánea + el WAL de después = crash antes de que
    // el motor hiciera durable la transacción.
    std::fs::copy(original.path().join("nopal.wal"), copia.path().join("nopal.wal")).unwrap();
    {
        let graph = Graph::open(copia.path()).await.unwrap();
        let h: Vec<_> = graph.history(x).await.unwrap().iter().map(|v| (v.version, v.valid_from, v.valid_to)).collect();
        assert_eq!(h, esperado, "el redo debe materializar la v1 desde la marca con el ts del commit");
        assert_eq!(
            porciones(&graph.history(x).await.unwrap()[1].node_data),
            Some(&PropertyValue::Int(1))
        );
        graph.storage().delete_meta(nopaldb::storage::META_WAL_APPLIED_UPTO).await.unwrap();
    }
    let graph = Graph::open(copia.path()).await.unwrap();
    let h: Vec<_> = graph.history(x).await.unwrap().iter().map(|v| (v.version, v.valid_from, v.valid_to)).collect();
    assert_eq!(h, esperado, "repetir el redo duplicó versiones");
}

/// Una transacción RepeatableRead lee de su snapshot por la cadena MVCC: un
/// nodo del bulk (sin cadena) debe verse por su versión implícita.
#[cfg(feature = "full-isolation")]
#[tokio::test]
async fn repeatable_read_ve_un_nodo_del_bulk() {
    use nopaldb::IsolationLevel;
    let graph = Graph::in_memory().await.unwrap();
    bulk_nodos(&graph, vec![receta(0x40, 5)]).await;
    for level in [IsolationLevel::RepeatableRead, IsolationLevel::Serializable] {
        let tx = graph.begin_transaction().await.unwrap().with_isolation(level);
        let n = tx.get_node(nid(0x40)).await.unwrap();
        assert_eq!(porciones(&n), Some(&PropertyValue::Int(5)));
    }
}

/// El GC no recoge versiones implícitas, y una versión implícita válida en
/// `ts` se devuelve aunque `ts` quede bajo el horizonte del GC.
#[tokio::test]
async fn el_gc_no_toca_la_version_implicita() {
    let graph = Graph::in_memory().await.unwrap();
    let t = bulk_nodos(&graph, vec![receta(0x50, 1)]).await;
    // Historia transaccional en otro nodo, para que el GC mueva el horizonte.
    for p in 1..=3 {
        let mut tx = graph.begin_transaction().await.unwrap();
        tx.add_node(receta(0x51, p)).await.unwrap();
        tx.commit().await.unwrap();
    }
    graph.gc(GCConfig::before_timestamp(u64::MAX)).await.unwrap();
    assert!(graph.gc_horizon().unwrap() > t, "precondición: horizonte por encima de la marca");
    assert_eq!(graph.history(nid(0x50)).await.unwrap().len(), 1);
    assert!(graph.get_node_at(nid(0x50), t).await.is_ok());
    assert!(no_existe(graph.get_node_at(nid(0x50), t - 1).await), "antes de nacer: no existía");
}
