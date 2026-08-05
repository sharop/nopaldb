// Migración automática del layout v1→v2 (F5.5): bases v1 FABRICADAS byte a
// byte (claves string de la sopa del tree default, exactamente lo que
// escribía ≤0.5.2) que deben abrir con el código nuevo y quedar 100% v2 —
// datos idénticos, derivados reconstruidos, default limpio salvo la marca
// de diagnóstico.
//
// La fabricación usa las seams crudas #[doc(hidden)] de Storage
// (`debug_raw_*`): el runtime ya no escribe el layout v1, así que no hay
// API pública capaz de producir estas bases. Dominio ficticio (vivero).

use nopaldb::mvcc::{VersionedEdge, VersionedNode};
use nopaldb::wal::{WalManager, WalRecord};
use nopaldb::{Direction, Edge, Graph, Node, NodeId, PropertyValue, Storage};

const DEFAULT: &str = "default";
const CATALOG: &str = "catalog";
const ENTITIES: &str = "entities";
const ADJACENCY: &str = "adjacency";

/// Marca de diagnóstico que la migración deja en el tree default.
const DIAG_MARK: &[u8] = b"meta:layout_migrated_to";

fn ser<T: serde::Serialize>(v: &T) -> Vec<u8> {
    rmp_serde::to_vec(v).expect("msgpack")
}

/// Clave `m|{nombre}` del catalog (codec v2 trivial, pinneado por units).
fn catalog_meta(name: &str) -> Vec<u8> {
    let mut k = vec![b'm'];
    k.extend_from_slice(name.as_bytes());
    k
}

/// Clave `n|{uuid16}` de entities (codec v2 trivial).
fn entity_key(id: NodeId) -> Vec<u8> {
    let mut k = vec![b'n'];
    k.extend_from_slice(id.as_bytes());
    k
}

struct V1Fixture {
    a: NodeId,
    b: NodeId,
    edge_id: uuid::Uuid,
}

/// Fabrica en `path` una base v1 realista: nodo `a` con 3 versiones MVCC
/// (t=100/200/300), nodo `b` con 1 (t=150), una arista tipada a→b con su
/// historial, blobs de adyacencia y de índice ts, y las metas de relojes.
/// SIN `meta:prop_idx_format`: como una base pre-0.4.36, para que la
/// sub-migración prop-idx también corra (el orden entre ambas se prueba
/// aparte con claves `idx:prop:*`).
async fn fabricate_v1_base(path: &std::path::Path) -> V1Fixture {
    let storage = Storage::new(path).await.expect("storage");

    let a = uuid::Uuid::new_v4();
    let b = uuid::Uuid::new_v4();

    let node_a = |riego: i64| {
        Node::with_id(a, "Planta")
            .with_property("especie", PropertyValue::String("cactus".into()))
            .with_property("riego", PropertyValue::Int(riego))
    };
    let node_b = Node::with_id(b, "Jardinero")
        .with_property("nombre", PropertyValue::String("Valentina".into()));

    // Cadena MVCC de `a`: v1 (100→200), v2 (200→300), v3 (300→∞).
    let mut va1 = VersionedNode::new(node_a(1), 100);
    let mut va2 = VersionedNode::new_version(&va1, node_a(2), 200);
    let va3 = VersionedNode::new_version(&va2, node_a(3), 300);
    va1.invalidate(200);
    va2.invalidate(300);
    let vb1 = VersionedNode::new(node_b.clone(), 150);

    let raw = |k: String, v: Vec<u8>| (k, v);
    let mut default_pairs = vec![
        raw(format!("node:{a}"), ser(&node_a(3))),
        raw(format!("node:{a}:v1"), ser(&va1)),
        raw(format!("node:{a}:v2"), ser(&va2)),
        raw(format!("node:{a}:v3"), ser(&va3)),
        raw(format!("node:{a}:current"), 3u64.to_le_bytes().to_vec()),
        raw(format!("node:{a}:versions"), ser(&vec![3u64, 2, 1])),
        raw(format!("node:{b}"), ser(&node_b)),
        raw(format!("node:{b}:v1"), ser(&vb1)),
        raw(format!("node:{b}:current"), 1u64.to_le_bytes().to_vec()),
        raw(format!("node:{b}:versions"), ser(&vec![1u64])),
        // Índice ts v1: blobs Vec<NodeId> por timestamp (sin versión).
        raw("ts:100".into(), ser(&vec![a])),
        raw("ts:150".into(), ser(&vec![b])),
        raw("ts:200".into(), ser(&vec![a])),
        raw("ts:300".into(), ser(&vec![a])),
        // Relojes v1 (u64 BE, cotas superiores).
        raw("meta:next_timestamp".into(), 301u64.to_be_bytes().to_vec()),
        raw("meta:next_tx_id".into(), 7u64.to_be_bytes().to_vec()),
    ];

    // Arista tipada a→b, con historial MVCC y blobs de adyacencia v1.
    let edge = Edge::new(a, b, "riega");
    let edge_id = edge.id;
    let ve = VersionedEdge::new(edge.clone(), 160);
    storage
        .debug_raw_insert("edges", edge_id.to_string().as_bytes(), &ser(&edge))
        .unwrap();
    storage
        .debug_raw_insert(
            "versioned_edges",
            format!("{edge_id}:v{:020}", ve.version).as_bytes(),
            &ser(&ve),
        )
        .unwrap();
    storage
        .debug_raw_insert("versioned_edges_current", edge_id.to_string().as_bytes(), &ser(&ve))
        .unwrap();
    default_pairs.push(raw(format!("idx:out:{a}"), ser(&vec![edge_id])));
    default_pairs.push(raw(format!("idx:in:{b}"), ser(&vec![edge_id])));
    // v1 persistía listas vacías para el otro lado.
    default_pairs.push(raw(format!("idx:out:{b}"), ser(&Vec::<uuid::Uuid>::new())));
    default_pairs.push(raw(format!("idx:in:{a}"), ser(&Vec::<uuid::Uuid>::new())));

    for (k, v) in &default_pairs {
        storage.debug_raw_insert(DEFAULT, k.as_bytes(), v).unwrap();
    }
    storage.flush().await.unwrap();

    V1Fixture { a, b, edge_id }
}

async fn assert_migrated_and_intact(graph: &Graph, fx: &V1Fixture) {
    let storage = graph.storage();

    // Datos current desde entities.
    let na = graph.get_node(fx.a).await.expect("nodo a migrado");
    assert_eq!(na.properties.get("riego"), Some(&PropertyValue::Int(3)));
    let nb = graph.get_node(fx.b).await.expect("nodo b migrado");
    assert_eq!(nb.label, "Jardinero");

    // Historia completa + time-travel con los timestamps originales.
    let history = storage.get_node_history(fx.a).await.unwrap();
    assert_eq!(history.len(), 3, "las 3 versiones de `a` migran");
    let at_150 = storage.get_node_at_timestamp(fx.a, 150).await.unwrap();
    assert_eq!(at_150.node_data.properties.get("riego"), Some(&PropertyValue::Int(1)));
    let at_250 = storage.get_node_at_timestamp(fx.a, 250).await.unwrap();
    assert_eq!(at_250.node_data.properties.get("riego"), Some(&PropertyValue::Int(2)));
    assert_eq!(storage.get_current_version(fx.a).await.unwrap(), 3);
    assert_eq!(storage.get_node_history(fx.b).await.unwrap().len(), 1);

    // Adyacencia reconstruida desde edges (con tipo internado en la clave).
    assert_eq!(graph.neighbors(fx.a, Direction::Outgoing).await.unwrap(), vec![fx.b]);
    assert_eq!(graph.neighbors(fx.b, Direction::Incoming).await.unwrap(), vec![fx.a]);
    assert_eq!(graph.degree(fx.a, Direction::Both).await.unwrap(), 1);
    let edge = storage.get_edge(fx.edge_id).await.unwrap();
    assert_eq!(edge.edge_type, "riega");

    // Conteos idénticos.
    assert_eq!(graph.get_all_nodes().await.unwrap().len(), 2);

    // Índice de propiedades reconstruido (la sub-migración corre después).
    assert_eq!(
        graph
            .find_nodes_by_property("especie", &PropertyValue::String("cactus".into()))
            .await
            .unwrap(),
        vec![fx.a]
    );

    // Relojes: la cota migró y no retrocede respecto del máximo persistido.
    let clock = storage.get_meta_u64("next_timestamp").await.unwrap().unwrap();
    assert!(clock >= 301, "reloj migrado y sin retroceso: {clock}");
    assert_eq!(storage.get_meta_u64("next_tx_id").await.unwrap(), Some(7));

    // Sentinels de la migración.
    assert_eq!(storage.get_meta_u64("layout_format").await.unwrap(), Some(2));
    assert_eq!(storage.get_meta_u64("legacy_cleanup_done").await.unwrap(), Some(1));

    // El default quedó LIMPIO salvo la marca de diagnóstico.
    assert_eq!(
        storage.debug_raw_keys(DEFAULT).unwrap(),
        vec![DIAG_MARK.to_vec()],
        "default limpio salvo meta:layout_migrated_to"
    );
}

#[tokio::test]
async fn migra_base_v1_completa_y_es_idempotente_al_reabrir() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v1_db");
    let fx = fabricate_v1_base(&path).await;

    let graph = Graph::open(&path).await.expect("open migra la base v1");
    assert_migrated_and_intact(&graph, &fx).await;
    drop(graph);

    // Reopen: no-op (sentinel presente), todo sigue accesible.
    let graph = Graph::open(&path).await.expect("reopen idempotente");
    assert_migrated_and_intact(&graph, &fx).await;

    // Escribir POST-migración (commit transaccional → crea versión MVCC):
    // los timestamps nuevos van después de los migrados (el reloj no
    // retrocede → time-travel fiable).
    let mut tx = graph.begin_transaction().await.unwrap();
    let nuevo = tx.add_node(Node::new("Maceta")).await.unwrap();
    tx.commit().await.unwrap();
    let h = graph.storage().get_node_history(nuevo).await.unwrap();
    assert!(!h.is_empty(), "el commit crea historia MVCC");
    assert!(h[0].timestamp >= 301, "ts nuevo {} tras el máximo migrado", h[0].timestamp);
}

#[tokio::test]
async fn crash_en_fase_copying_reanuda_y_completa() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("crash_copying");
    let fx = fabricate_v1_base(&path).await;

    // Simular crash a media copia: estado `copying` persistido + una copia
    // PARCIAL y además CORRUPTA de `a` en entities (la re-copia sobrescribe
    // con los bytes del legacy — que quedó intacto).
    {
        let storage = Storage::new(&path).await.unwrap();
        storage
            .debug_raw_insert(CATALOG, &catalog_meta("layout_migration_state"), b"copying")
            .unwrap();
        storage
            .debug_raw_insert(ENTITIES, &entity_key(fx.a), b"bytes-basura-de-copia-a-medias")
            .unwrap();
        storage.flush().await.unwrap();
    }

    let graph = Graph::open(&path).await.expect("reopen completa la migración");
    assert_migrated_and_intact(&graph, &fx).await;
}

#[tokio::test]
async fn crash_en_fase_verified_solo_activa_y_limpia() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("crash_verified");
    let fx = fabricate_v1_base(&path).await;

    // Migración completa una vez…
    drop(Graph::open(&path).await.expect("primera migración"));

    // …y se fabrica el estado "crash tras rebuild, antes de activate":
    // sentinel y marca de cleanup fuera, estado `verified`, y legacy residual
    // en el default (lo que un crash a media limpieza habría dejado).
    {
        let storage = Storage::new(&path).await.unwrap();
        storage.debug_raw_remove(CATALOG, &catalog_meta("layout_format")).unwrap();
        storage.debug_raw_remove(CATALOG, &catalog_meta("legacy_cleanup_done")).unwrap();
        storage
            .debug_raw_insert(CATALOG, &catalog_meta("layout_migration_state"), b"verified")
            .unwrap();
        storage
            .debug_raw_insert(DEFAULT, format!("node:{}", fx.a).as_bytes(), b"residuo-legacy")
            .unwrap();
        storage
            .debug_raw_insert(DEFAULT, format!("idx:out:{}", fx.a).as_bytes(), b"residuo")
            .unwrap();
        storage.debug_raw_insert(DEFAULT, b"ts:100", b"residuo").unwrap();
        storage
            .debug_raw_insert(DEFAULT, b"meta:next_timestamp", &1u64.to_be_bytes())
            .unwrap();
        storage.flush().await.unwrap();
    }

    // Reopen: NO re-copia (los datos v2 ya verificados mandan — el residuo
    // legacy con bytes basura se borra sin tocarse), solo activa y limpia.
    let graph = Graph::open(&path).await.expect("reanuda en verified");
    assert_migrated_and_intact(&graph, &fx).await;
}

#[tokio::test]
async fn verificacion_fallida_es_error_fuerte_y_legacy_queda_intacto() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("verify_fail");
    let fx = fabricate_v1_base(&path).await;

    let legacy_keys_before = {
        let storage = Storage::new(&path).await.unwrap();
        // Un nodo FANTASMA pre-existente en entities que el legacy no tiene:
        // la copia no lo toca y la verificación de identidad (conteo+digest)
        // debe reventar en vez de activar un layout con datos de más.
        let ghost = uuid::Uuid::new_v4();
        storage
            .debug_raw_insert(ENTITIES, &entity_key(ghost), &ser(&Node::with_id(ghost, "Fantasma")))
            .unwrap();
        storage.flush().await.unwrap();
        storage.debug_raw_keys(DEFAULT).unwrap()
    };

    // Error fuerte, determinista en cada intento.
    assert!(Graph::open(&path).await.is_err(), "open debe fallar la verificación");
    assert!(Graph::open(&path).await.is_err(), "sigue fallando (no activa a medias)");

    // El legacy quedó INTACTO (recuperable con un binario v1) y el sentinel
    // jamás se escribió.
    let storage = Storage::new(&path).await.unwrap();
    assert_eq!(storage.debug_raw_keys(DEFAULT).unwrap(), legacy_keys_before);
    assert_eq!(storage.get_meta_u64("layout_format").await.unwrap(), None);
    let _ = fx;
}

#[tokio::test]
async fn adyacencia_huerfana_v1_no_migra() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("huerfanas");
    let fx = fabricate_v1_base(&path).await;

    // Blobs de adyacencia de un nodo que ya no existe (el bug histórico de
    // v1: delete_node jamás los borraba).
    {
        let storage = Storage::new(&path).await.unwrap();
        let ghost = uuid::Uuid::new_v4();
        let fake_edge = uuid::Uuid::new_v4();
        storage
            .debug_raw_insert(DEFAULT, format!("idx:out:{ghost}").as_bytes(), &ser(&vec![fake_edge]))
            .unwrap();
        storage
            .debug_raw_insert(DEFAULT, format!("idx:in:{ghost}").as_bytes(), &ser(&vec![fake_edge]))
            .unwrap();
        storage.flush().await.unwrap();
    }

    let graph = Graph::open(&path).await.expect("migra ignorando huérfanas");
    assert_migrated_and_intact(&graph, &fx).await;

    // La adyacencia v2 se reconstruyó desde edges: EXACTAMENTE las 2 claves
    // de la única arista real (O + espejo I). Las huérfanas murieron.
    assert_eq!(
        graph.storage().debug_raw_keys(ADJACENCY).unwrap().len(),
        2,
        "solo O+I de la arista real; cero huérfanas migradas"
    );
}

#[tokio::test]
async fn interaccion_con_migracion_prop_idx_legacy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prop_idx");
    let fx = fabricate_v1_base(&path).await;

    // Índice de propiedades LEGADO v1 (`idx:prop:*` en el default, la
    // colisión clásica de tipos incluida) y sin sentinel prop_idx_format:
    // una base pre-0.4.36. Ambas migraciones deben correr en orden.
    {
        let storage = Storage::new(&path).await.unwrap();
        storage
            .debug_raw_insert(DEFAULT, b"idx:prop:especie:cactus", &ser(&vec![fx.a]))
            .unwrap();
        storage.debug_raw_insert(DEFAULT, b"idx:prop:riego:3", &ser(&vec![fx.a])).unwrap();
        storage.flush().await.unwrap();
    }

    let graph = Graph::open(&path).await.expect("ambas migraciones corren");
    // El layout migró TODO y el prop-idx v2 quedó reconstruido y tipado:
    // buscar el string "3" ya no encuentra al Int(3).
    assert_migrated_and_intact(&graph, &fx).await;
    assert_eq!(
        graph.find_nodes_by_property("riego", &PropertyValue::Int(3)).await.unwrap(),
        vec![fx.a]
    );
    assert!(graph
        .find_nodes_by_property("riego", &PropertyValue::String("3".into()))
        .await
        .unwrap()
        .is_empty());
    assert_eq!(graph.storage().get_meta_u64("prop_idx_format").await.unwrap(), Some(2));
}

#[tokio::test]
async fn wal_commiteado_se_replaya_sobre_el_estado_migrado() {
    // El caso que fija el ORDEN migración→replay: una base v1 que crasheó
    // con transacciones commiteadas en el WAL pero no aplicadas. El redo
    // debe correr sobre el estado YA migrado (como lo habría hecho 0.5.2
    // sobre el legacy): la cadena MVCC de `a` se EXTIENDE a v4, no se
    // re-crea desde el sufijo del WAL.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wal_replay");
    let fx = fabricate_v1_base(&path).await;

    let c = uuid::Uuid::new_v4();
    {
        let wal = WalManager::new(path.join("nopal.wal")).await.unwrap();
        // Tx 8: nodo nuevo `c` (commiteado, jamás aplicado al storage).
        wal.append(WalRecord::Begin { tx_id: 8, timestamp: 400 }).await.unwrap();
        wal.append(WalRecord::InsertNode {
            tx_id: 8,
            node: Node::with_id(c, "Invernadero"),
        })
        .await
        .unwrap();
        wal.append(WalRecord::Commit { tx_id: 8, timestamp: 401 }).await.unwrap();
        // Tx 9: update de `a` (riego=4) commiteado y no aplicado.
        let old = Node::with_id(fx.a, "Planta");
        let new = Node::with_id(fx.a, "Planta")
            .with_property("especie", PropertyValue::String("cactus".into()))
            .with_property("riego", PropertyValue::Int(4));
        wal.append(WalRecord::Begin { tx_id: 9, timestamp: 410 }).await.unwrap();
        wal.append(WalRecord::UpdateNode {
            tx_id: 9,
            node_id: fx.a,
            old_node: old,
            new_node: new,
        })
        .await
        .unwrap();
        wal.append(WalRecord::Commit { tx_id: 9, timestamp: 411 }).await.unwrap();
        wal.flush().await.unwrap();
    }

    let graph = Graph::open(&path).await.expect("migra y luego replaya");
    let storage = graph.storage();

    // El nodo del WAL existe con su timestamp de commit.
    let hc = storage.get_node_history(c).await.unwrap();
    assert_eq!(hc.len(), 1);
    assert_eq!(hc[0].timestamp, 401);

    // La cadena migrada de `a` se EXTENDIÓ: 4 versiones, current = v4 con
    // el update del WAL, y el time-travel pre-crash sigue intacto.
    let ha = storage.get_node_history(fx.a).await.unwrap();
    assert_eq!(ha.len(), 4, "3 versiones migradas + 1 del redo del WAL");
    assert_eq!(storage.get_current_version(fx.a).await.unwrap(), 4);
    let na = graph.get_node(fx.a).await.unwrap();
    assert_eq!(na.properties.get("riego"), Some(&PropertyValue::Int(4)));
    let at_250 = storage.get_node_at_timestamp(fx.a, 250).await.unwrap();
    assert_eq!(at_250.node_data.properties.get("riego"), Some(&PropertyValue::Int(2)));

    // Y el replay idempotente del siguiente open no duplica nada.
    drop(storage); // el Arc<Storage> retenido mantendría el lock del engine
    drop(graph);
    let graph = Graph::open(&path).await.unwrap();
    assert_eq!(graph.storage().get_node_history(fx.a).await.unwrap().len(), 4);
}

#[tokio::test]
async fn base_nueva_recibe_sentinel_sin_migrar() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nueva");

    {
        let graph = Graph::open(&path).await.unwrap();
        graph.add_node(Node::new("Planta")).await.unwrap();
        drop(graph);
    }

    let graph = Graph::open(&path).await.unwrap();
    let storage = graph.storage();
    assert_eq!(storage.get_meta_u64("layout_format").await.unwrap(), Some(2));
    assert_eq!(storage.get_meta_u64("legacy_cleanup_done").await.unwrap(), Some(1));
    // Base nueva: sin marca de diagnóstico (no hubo migración) y default vacío.
    assert!(storage.debug_raw_keys(DEFAULT).unwrap().is_empty());
}
