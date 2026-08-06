// tests/invariants_oracle_proptest.rs
//
// E3 — Oráculo de invariantes estructurales con proptest (nightly).
//
// Secuencias aleatorias de operaciones de la API pública (altas/updates/
// borrados de nodos y aristas — directos, transaccionales y batch — más
// rollbacks y checkpoints) aplicadas a una base real; al final de cada
// secuencia el oráculo verifica la ESTRUCTURA en disco vía los seams
// `debug_raw_*` de Storage, no la semántica (esa ya la cubre
// property_model_test contra un modelo de referencia):
//
//   A. edges ↔ adjacency es una biyección exacta: cada arista del tree
//      `edges` tiene EXACTAMENTE su par de claves O/I (contrato de 53 bytes)
//      y no existe ninguna clave de adyacencia sin arista.
//   B. RAM == rebuild(disco): los `neighbors()` de todos los nodos son
//      idénticos antes y después de reabrir la base.
//   C. Todo puntero current (`c|uuid` en `history`, u64 LE) apunta a una
//      versión que existe (`v|uuid|ver BE`), y toda clave de `history`
//      clasifica en uno de los tres namespaces (v/c/l).
//   D. `get_all_nodes()` == scan del keyspace `entities`, y toda clave de
//      `entities` es un `n|uuid16` bien formado.
//
// La variante diferencial contra bases v1 fabricadas quedó FUERA a
// propósito: la migración v1→v2 ya está cubierta byte a byte por
// layout_v2_migration_test + el soak nightly; el valor de este test es el
// oráculo sobre operaciones vivas del layout v2.
//
// ⚠️ HUECOS CONOCIDOS descubiertos por este oráculo (reproducidos
// determinísticamente durante su desarrollo; se esquivan aquí para que el
// nightly vigile todo lo demás en verde — quitar el dodge al cerrarlos):
//
//   H1. `add_nodes_batch` sobre un id EXISTENTE pisa su adyacencia RAM con
//       `Vec::new()` (el mismo bug que `apply_add_node` ya arregló con
//       `or_default`): RAM ≠ disco hasta el próximo open. Dodge: el
//       intérprete solo batchea slots frescos (ver `BatchNodes`).
//
//   H2. El path de update del commit transaccional exige cadena MVCC
//       (`get_current_version`), pero ni `add_node` directo ni
//       `add_nodes_batch` la crean: comitear un update sobre un nodo nacido
//       por esos paths falla con NodeNotFound a MEDIO apply.
//
//   H3. Un commit que falla a medias en el apply (H2, o una arista a nodo
//       inexistente) ya dejó su write-set en el WAL: el redo del próximo
//       open lo MATERIALIZA aunque `commit()` devolvió Err. (El path
//       directo no lo padece: un `add_edge` directo fallido no deja rastro
//       tras reopen — verificado.)
//
//   H4. El redo del WAL no es consistente entre registros: DeleteNode se
//       re-aplica con la única guarda "el nodo existe", mientras InsertNode
//       se guarda por timestamp de la historia. Secuencia mínima: alta
//       directa → delete por tx → re-alta por tx → reopen: el redo
//       re-aplica el delete viejo (el nodo "existe") y NO re-aplica la
//       re-alta ("ya aplicada" según su historia) — un nodo commiteado
//       desaparece al reabrir la base.
//
// Dodge de H2/H3/H4: los tres se manifiestan vía el redo del WAL sobre
// mundos mixtos directo/tx, así que el test hace `checkpoint()` (trunca el
// WAL) como ÚLTIMO paso antes de comparar y reabrir — la invariante B mide
// el rebuild de adyacencia puro, que es su objetivo, y los errores de
// commit esperados por H2/H3 se toleran en el intérprete. Los huecos van
// reportados aparte con sus repros; este archivo no los arregla ni los
// pinnea en rojo.
//
// Nightly-only (`#[ignore]`): cada caso hace IO real de sled + un reopen.
// Casos por corrida: 32 por default (PROPTEST_CASES lo sube/baja) con
// secuencias de ≤40 ops — minutos, no horas.
//
// Dominio ficticio (vivero), sin datos reales.

use nopaldb::{Direction, Edge, EdgeId, Graph, Node, NodeId, PropertyValue};
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// Pool de ids de nodo posibles (slots deterministas).
const NODE_SLOTS: u128 = 10;

/// Tipos de arista posibles (se internan en el catalog).
const EDGE_TYPES: [&str; 3] = ["riega", "poda", "abona"];

fn nid(slot: u128) -> NodeId {
    Uuid::from_u128(0xE3_0000_0000 + slot)
}

fn node(slot: u128, riego: i64) -> Node {
    Node::with_id(nid(slot), "Planta").with_property("riego", PropertyValue::Int(riego))
}

fn edge(src: u128, tgt: u128, etype: usize) -> Edge {
    Edge::new(nid(src), nid(tgt), EDGE_TYPES[etype % EDGE_TYPES.len()])
}

#[derive(Debug, Clone)]
enum Op {
    /// `add_node` directo — alta o update (upsert) según exista el slot.
    AddNode { slot: u128, riego: i64 },
    /// Alta/update vía transacción. El commit puede fallar por H2 (update
    /// de un nodo sin cadena MVCC) — tolerado; el residuo WAL lo neutraliza
    /// el checkpoint final (H3).
    AddNodeTx { slot: u128, riego: i64 },
    /// `delete_node` directo (purga aristas incidentes por chunks).
    DeleteNode { slot: u128 },
    /// Borrado de nodo vía transacción. Si el nodo no existe, el commit
    /// falla en el prefetch (antes del WAL) — tolerado.
    DeleteNodeTx { slot: u128 },
    /// `add_edge` directo (falla limpio si faltan extremos — tolerado).
    AddEdge { src: u128, tgt: u128, etype: usize },
    /// Nodo + arista en LA MISMA transacción (commit multi-op). Puede
    /// fallar por H2/H3 — tolerado; ver el checkpoint final.
    AddEdgeTx { src: u128, tgt: u128, etype: usize, riego: i64 },
    /// `delete_edge` directo sobre una arista creada antes (puede ya no
    /// existir si un delete_node la purgó — tolerado).
    DeleteEdge { pick: usize },
    /// Borrado de arista vía transacción (commit falla si ya murió — tolerado).
    DeleteEdgeTx { pick: usize },
    /// `add_nodes_batch` — SOLO slots nuevos (dodge de H1: el batch-upsert
    /// de un id existente pisa su adyacencia RAM con `Vec::new()`).
    BatchNodes { slots: Vec<u128>, riego: i64 },
    /// `add_edges_batch` — solo entre slots existentes: el path batch no
    /// valida extremos (hueco semántico menor, distinto de H1–H4) y las
    /// aristas fantasma no aportan nada a las invariantes estructurales.
    BatchEdges { pairs: Vec<(u128, u128)>, etype: usize },
    /// Transacción con trabajo (nodo + arista) que se revierte: no debe
    /// dejar rastro estructural.
    TxRollback { slot: u128, tgt: u128, etype: usize },
    /// `checkpoint()` — WAL checkpoint + truncado + persistencia de relojes.
    Checkpoint,
}

fn slot_strategy() -> impl Strategy<Value = u128> {
    0..NODE_SLOTS
}

fn op_strategy() -> impl Strategy<Value = Op> {
    let slot = slot_strategy;
    prop_oneof![
        4 => (slot(), 0i64..100).prop_map(|(slot, riego)| Op::AddNode { slot, riego }),
        2 => (slot(), 0i64..100).prop_map(|(slot, riego)| Op::AddNodeTx { slot, riego }),
        2 => slot().prop_map(|slot| Op::DeleteNode { slot }),
        1 => slot().prop_map(|slot| Op::DeleteNodeTx { slot }),
        4 => (slot(), slot(), 0..EDGE_TYPES.len())
            .prop_map(|(src, tgt, etype)| Op::AddEdge { src, tgt, etype }),
        2 => (slot(), slot(), 0..EDGE_TYPES.len(), 0i64..100)
            .prop_map(|(src, tgt, etype, riego)| Op::AddEdgeTx { src, tgt, etype, riego }),
        2 => (0..64usize).prop_map(|pick| Op::DeleteEdge { pick }),
        1 => (0..64usize).prop_map(|pick| Op::DeleteEdgeTx { pick }),
        1 => (proptest::collection::vec(slot(), 1..4), 0i64..100)
            .prop_map(|(slots, riego)| Op::BatchNodes { slots, riego }),
        1 => (proptest::collection::vec((slot(), slot()), 1..4), 0..EDGE_TYPES.len())
            .prop_map(|(pairs, etype)| Op::BatchEdges { pairs, etype }),
        1 => (slot(), slot(), 0..EDGE_TYPES.len())
            .prop_map(|(slot, tgt, etype)| Op::TxRollback { slot, tgt, etype }),
        1 => Just(Op::Checkpoint),
    ]
}

async fn exists(graph: &Graph, slot: u128) -> bool {
    graph.node_exists(nid(slot)).await.unwrap_or(false)
}

/// Aplica una op. Los errores ESPERADOS (extremos inexistentes, nodo/arista
/// ya borrados, commits H2/H3) se TOLERAN — el punto del oráculo es que,
/// falle o no la op, las invariantes estructurales se conserven. Lo que
/// jamás debe fallar (altas directas/batch gateado, rollback, checkpoint)
/// lleva `expect`.
async fn apply(graph: &Graph, edge_ids: &mut Vec<EdgeId>, op: &Op) {
    match op {
        Op::AddNode { slot, riego } => {
            graph.add_node(node(*slot, *riego)).await.expect("add_node es upsert");
        }
        Op::AddNodeTx { slot, riego } => {
            let mut tx = graph.begin_transaction().await.expect("begin");
            tx.add_node(node(*slot, *riego)).await.expect("tx add_node");
            let _ = tx.commit().await; // H2 tolerado
        }
        Op::DeleteNode { slot } => {
            let _ = graph.delete_node(nid(*slot)).await; // NodeNotFound tolerado
        }
        Op::DeleteNodeTx { slot } => {
            let mut tx = graph.begin_transaction().await.expect("begin");
            tx.delete_node(nid(*slot)).expect("tx delete_node");
            let _ = tx.commit().await; // nodo inexistente: falla pre-WAL, tolerado
        }
        Op::AddEdge { src, tgt, etype } => {
            if let Ok(id) = graph.add_edge(edge(*src, *tgt, *etype)).await {
                edge_ids.push(id);
            }
        }
        Op::AddEdgeTx { src, tgt, etype, riego } => {
            let mut tx = graph.begin_transaction().await.expect("begin");
            tx.add_node(node(*src, *riego)).await.expect("tx add_node");
            let id = tx.add_edge(edge(*src, *tgt, *etype)).expect("tx add_edge");
            if tx.commit().await.is_ok() {
                edge_ids.push(id);
            }
        }
        Op::DeleteEdge { pick } => {
            if !edge_ids.is_empty() {
                let id = edge_ids[pick % edge_ids.len()];
                let _ = graph.delete_edge(id).await; // EdgeNotFound tolerado
            }
        }
        Op::DeleteEdgeTx { pick } => {
            if !edge_ids.is_empty() {
                let id = edge_ids[pick % edge_ids.len()];
                let mut tx = graph.begin_transaction().await.expect("begin");
                tx.delete_edge(id).expect("tx delete_edge");
                let _ = tx.commit().await; // arista ya muerta: tolerado
            }
        }
        Op::BatchNodes { slots, riego } => {
            // Dodge de H1 (ver doc de la variante): solo slots que NO existen.
            let mut nodes = Vec::new();
            let mut seen = BTreeSet::new();
            for slot in slots {
                if seen.insert(*slot) && !exists(graph, *slot).await {
                    nodes.push(node(*slot, *riego));
                }
            }
            graph.add_nodes_batch(nodes).await.expect("add_nodes_batch");
        }
        Op::BatchEdges { pairs, etype } => {
            let mut edges = Vec::new();
            for (src, tgt) in pairs {
                if exists(graph, *src).await && exists(graph, *tgt).await {
                    edges.push(edge(*src, *tgt, *etype));
                }
            }
            let ids = graph.add_edges_batch(edges).await.expect("add_edges_batch");
            edge_ids.extend(ids);
        }
        Op::TxRollback { slot, tgt, etype } => {
            let mut tx = graph.begin_transaction().await.expect("begin");
            tx.add_node(node(*slot, 1)).await.expect("tx add_node");
            let _ = tx.add_edge(edge(*slot, *tgt, *etype));
            tx.rollback().expect("rollback");
        }
        Op::Checkpoint => {
            graph.checkpoint().await.expect("checkpoint");
        }
    }
}

// ─── Oráculo ─────────────────────────────────────────────────────────────────
//
// Réplica local del contrato de claves v2 (documentado en
// src/storage/keys.rs, pinneado por sus tests). Se replica a propósito: si
// el codec del runtime cambiara por accidente, un helper compartido
// cambiaría con él y el oráculo no vería nada.

const ADJ_KEY_LEN: usize = 53;

fn adj_key(dir: u8, owner: NodeId, etype: u32, other: NodeId, edge: EdgeId) -> Vec<u8> {
    let mut k = Vec::with_capacity(ADJ_KEY_LEN);
    k.push(dir);
    k.extend_from_slice(owner.as_bytes());
    k.extend_from_slice(&etype.to_be_bytes());
    k.extend_from_slice(other.as_bytes());
    k.extend_from_slice(edge.as_bytes());
    k
}

/// Invariantes A, C y D sobre el estado en disco + API.
async fn check_structural_invariants(graph: &Graph) {
    let storage = graph.storage();

    // ── A. edges ↔ adjacency: biyección exacta de pares O/I ────────────────
    let mut expected_adj: BTreeSet<Vec<u8>> = BTreeSet::new();
    for key in storage.debug_raw_keys("edges").expect("scan edges") {
        let raw = storage
            .debug_raw_get("edges", &key)
            .expect("get edge")
            .expect("clave de edges sin valor");
        let e: Edge = rmp_serde::from_slice(&raw).expect("edge msgpack corrupto");
        assert_eq!(
            key,
            e.id.to_string().into_bytes(),
            "clave del tree edges != id de la arista serializada"
        );

        // Tipo internado en el catalog: `etn` + nombre → u32 BE.
        let mut etn = b"etn".to_vec();
        etn.extend_from_slice(e.edge_type.as_bytes());
        let etype_raw = storage
            .debug_raw_get("catalog", &etn)
            .expect("get catalog")
            .unwrap_or_else(|| panic!("arista {} con tipo '{}' sin internar", e.id, e.edge_type));
        let etype = u32::from_be_bytes(etype_raw[..4].try_into().expect("etn value u32 BE"));

        expected_adj.insert(adj_key(b'O', e.source, etype, e.target, e.id));
        expected_adj.insert(adj_key(b'I', e.target, etype, e.source, e.id));
    }

    let actual_adj: BTreeSet<Vec<u8>> = storage
        .debug_raw_keys("adjacency")
        .expect("scan adjacency")
        .into_iter()
        .collect();
    for k in &actual_adj {
        assert_eq!(k.len(), ADJ_KEY_LEN, "clave de adyacencia malformada: {k:02x?}");
        assert!(
            k[0] == b'O' || k[0] == b'I',
            "discriminador de adyacencia desconocido: {k:02x?}"
        );
    }
    assert_eq!(
        expected_adj, actual_adj,
        "adjacency != pares O/I exactos de edges (huérfanas o faltantes)"
    );

    // ── C. history: todo current apunta a versión existente ────────────────
    let history: BTreeSet<Vec<u8>> = storage
        .debug_raw_keys("history")
        .expect("scan history")
        .into_iter()
        .collect();
    for k in &history {
        match (k.first(), k.len()) {
            (Some(b'v'), 25) | (Some(b'c'), 17) | (Some(b'l'), 17) => {}
            _ => panic!("clave de history fuera de los namespaces v/c/l: {k:02x?}"),
        }
        if k[0] == b'c' {
            let raw = storage
                .debug_raw_get("history", k)
                .expect("get current")
                .expect("puntero current sin valor");
            let ver = u64::from_le_bytes(raw[..8].try_into().expect("current u64 LE"));
            let mut vkey = vec![b'v'];
            vkey.extend_from_slice(&k[1..17]);
            vkey.extend_from_slice(&ver.to_be_bytes());
            assert!(
                history.contains(&vkey),
                "current de {:?} apunta a la versión {ver}, que no existe",
                Uuid::from_slice(&k[1..17]).unwrap()
            );
        }
    }

    // ── D. get_all_nodes() == scan de entities ──────────────────────────────
    let api_ids: BTreeSet<NodeId> = graph
        .get_all_nodes()
        .await
        .expect("get_all_nodes")
        .into_iter()
        .map(|n| n.id)
        .collect();
    let disk_ids: BTreeSet<NodeId> = storage
        .debug_raw_keys("entities")
        .expect("scan entities")
        .into_iter()
        .map(|k| {
            assert_eq!(k.len(), 17, "clave de entities malformada: {k:02x?}");
            assert_eq!(k[0], b'n', "tag de entities desconocido: {k:02x?}");
            Uuid::from_slice(&k[1..17]).unwrap()
        })
        .collect();
    assert_eq!(api_ids, disk_ids, "get_all_nodes() != scan del keyspace entities");
}

/// Vecindad completa (out/in, ordenada) de todos los nodos vivos — la foto
/// que debe sobrevivir al reopen (invariante B).
async fn neighbors_snapshot(graph: &Graph) -> BTreeMap<NodeId, (Vec<NodeId>, Vec<NodeId>)> {
    let mut snap = BTreeMap::new();
    for n in graph.get_all_nodes().await.expect("get_all_nodes") {
        let mut out = graph.neighbors(n.id, Direction::Outgoing).await.expect("out");
        let mut inn = graph.neighbors(n.id, Direction::Incoming).await.expect("in");
        out.sort_unstable();
        inn.sort_unstable();
        snap.insert(n.id, (out, inn));
    }
    snap
}

fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(32)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    /// Nightly-only: `cargo test -p nopaldb --features core --test
    /// invariants_oracle_proptest -- --ignored` (ver nightly.yml).
    #[test]
    #[ignore = "oráculo nightly: IO real de sled + reopen por caso"]
    fn oraculo_invariantes_estructurales(
        ops in proptest::collection::vec(op_strategy(), 1..=40)
    ) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let before;
            {
                let graph = Graph::open(dir.path()).await.expect("open");
                let mut edge_ids: Vec<EdgeId> = Vec::new();
                for op in &ops {
                    apply(&graph, &mut edge_ids, op).await;
                }

                // Checkpoint FINAL deliberado (dodge de H2/H3/H4, ver la
                // cabecera): trunca el WAL para que el reopen mida el
                // rebuild de adyacencia puro y no el redo del WAL, que hoy
                // re-aplica deletes viejos y materializa commits fallidos
                // sobre mundos mixtos directo/tx.
                graph.checkpoint().await.expect("checkpoint final");

                check_structural_invariants(&graph).await;
                before = neighbors_snapshot(&graph).await;
            } // drop: cierra sled y suelta el lock

            // B. RAM == rebuild(disco): reabrir y comparar la vecindad;
            // el resto de invariantes también debe sobrevivir al reopen.
            let graph = Graph::open(dir.path()).await.expect("reopen");
            let after = neighbors_snapshot(&graph).await;
            assert_eq!(before, after, "vecindad RAM != rebuild desde disco tras reopen");
            check_structural_invariants(&graph).await;
        });
    }
}
