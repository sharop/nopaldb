// tests/mvcc_stamp_model_proptest.rs
//
// Prueba basada en modelo de la historia MVCC con marcas de nacimiento
// (0.10.1): secuencias aleatorias de bulk (nodos y aristas), updates por
// transacción, escrituras directas, borrados (con cascada), borrados y
// re-altas de aristas. Tras cada secuencia, en CADA timestamp desde 0 hasta
// el último, cada lector se compara contra un modelo de fuerza bruta
// construido con el registro de operaciones:
//
//   get_node_at, as_of().get_node, get_node_at_strict, history(),
//   get_class_nodes_at, edge_history(), get_versioned_edges_of_type_at,
//   get_all_versioned_nodes (contra history()).
//
// Se repite tras cerrar y reabrir la base (las marcas son persistentes).
// Dominio ficticio: ingredientes de un recetario (nodos) y maridajes
// (aristas, con extremos fijos por id).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use nopaldb::error::NopalError;
use nopaldb::types::NodeKind;
use nopaldb::{Edge, EdgeId, Graph, Node, NodeId, PropertyValue};
use proptest::prelude::*;
use uuid::Uuid;

const N: u8 = 5; // nodos
const M: u8 = 5; // aristas; la e une (e % N) → ((e + 1) % N)
const TIPO: &str = "marida";

fn nid(x: u8) -> NodeId {
    Uuid::from_u128(0x1a6e_0000_0000 + x as u128)
}

fn eid(x: u8) -> EdgeId {
    Uuid::from_u128(0x1a6e_e000_0000 + x as u128)
}

fn extremos(e: u8) -> (u8, u8) {
    (e % N, (e + 1) % N)
}

fn ingrediente(x: u8, v: i64) -> Node {
    let mut n = Node::with_id(nid(x), "Ingrediente").with_property("v", PropertyValue::Int(v));
    n.kind = NodeKind::Class; // para comparar también get_class_nodes_at
    n
}

fn maridaje(e: u8, v: i64) -> Edge {
    let (s, t) = extremos(e);
    Edge::with_id(eid(e), nid(s), nid(t), TIPO).with_property("v", PropertyValue::Int(v))
}

fn valor(props: &HashMap<String, PropertyValue>) -> Option<i64> {
    match props.get("v") {
        Some(PropertyValue::Int(v)) => Some(*v),
        _ => None,
    }
}

#[derive(Debug, Clone)]
enum Op {
    BulkNodes(Vec<(u8, i64)>),
    BulkEdges(Vec<(u8, i64)>),
    TxNode(u8, i64),
    DirectNode(u8, i64),
    DeleteNode(u8),
    DeleteEdge(u8),
    DirectEdge(u8, i64),
    TxEdge(u8, i64),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => prop::collection::vec((0..N, 0i64..50), 1..5).prop_map(Op::BulkNodes),
        3 => prop::collection::vec((0..M, 0i64..50), 1..5).prop_map(Op::BulkEdges),
        2 => (0..N, 0i64..50).prop_map(|(x, v)| Op::TxNode(x, v)),
        2 => (0..N, 0i64..50).prop_map(|(x, v)| Op::DirectNode(x, v)),
        1 => (0..N).prop_map(Op::DeleteNode),
        1 => (0..M).prop_map(Op::DeleteEdge),
        2 => (0..M, 0i64..50).prop_map(|(e, v)| Op::DirectEdge(e, v)),
        1 => (0..M, 0i64..50).prop_map(|(e, v)| Op::TxEdge(e, v)),
    ]
}

/// Por entidad: cambios `(ts, Some(valor) | None = borrada)`, en orden.
#[derive(Default)]
struct Modelo {
    nodos: BTreeMap<u8, Vec<(u64, Option<i64>)>>,
    aristas: BTreeMap<u8, Vec<(u64, Option<i64>)>>,
}

fn estado(cambios: Option<&Vec<(u64, Option<i64>)>>, ts: u64) -> Option<i64> {
    cambios?.iter().rev().find(|(t, _)| *t <= ts).and_then(|(_, v)| *v)
}

impl Modelo {
    fn nodo(&self, x: u8, ts: u64) -> Option<i64> {
        estado(self.nodos.get(&x), ts)
    }
    fn arista(&self, e: u8, ts: u64) -> Option<i64> {
        estado(self.aristas.get(&e), ts)
    }
    fn nodo_vivo(&self, x: u8) -> bool {
        self.nodo(x, u64::MAX).is_some()
    }
    fn arista_viva(&self, e: u8) -> bool {
        self.arista(e, u64::MAX).is_some()
    }
}

/// Aplica `op` al grafo y al modelo. Las operaciones que no aplican (borrar
/// algo que no existe, una arista sin extremos) se saltan en los dos.
async fn aplicar(graph: &Graph, modelo: &mut Modelo, op: &Op) {
    match op {
        Op::BulkNodes(items) => {
            let mut loader = graph.bulk_loader(10_000);
            for (x, v) in items {
                loader.add_node(ingrediente(*x, *v)).await.unwrap();
            }
            loader.finish().await.unwrap();
            let ts = graph.last_commit_timestamp();
            let mut ultimo: BTreeMap<u8, i64> = BTreeMap::new();
            for (x, v) in items {
                ultimo.insert(*x, *v);
            }
            for (x, v) in ultimo {
                modelo.nodos.entry(x).or_default().push((ts, Some(v)));
            }
        }
        Op::BulkEdges(items) => {
            let validos: Vec<(u8, i64)> = items
                .iter()
                .copied()
                .filter(|(e, _)| {
                    let (s, t) = extremos(*e);
                    modelo.nodo_vivo(s) && modelo.nodo_vivo(t)
                })
                .collect();
            if validos.is_empty() {
                return;
            }
            let mut loader = graph.bulk_loader(10_000);
            for (e, v) in &validos {
                loader.add_edge(maridaje(*e, *v)).await.unwrap();
            }
            loader.finish().await.unwrap();
            let ts = graph.last_commit_timestamp();
            let mut ultimo: BTreeMap<u8, i64> = BTreeMap::new();
            for (e, v) in validos {
                ultimo.insert(e, v);
            }
            for (e, v) in ultimo {
                modelo.aristas.entry(e).or_default().push((ts, Some(v)));
            }
        }
        Op::TxNode(x, v) => {
            let mut tx = graph.begin_transaction().await.unwrap();
            tx.add_node(ingrediente(*x, *v)).await.unwrap();
            tx.commit().await.unwrap();
            let ts = graph.last_commit_timestamp();
            modelo.nodos.entry(*x).or_default().push((ts, Some(*v)));
        }
        Op::DirectNode(x, v) => {
            graph.add_node(ingrediente(*x, *v)).await.unwrap();
            let ts = graph.last_commit_timestamp();
            modelo.nodos.entry(*x).or_default().push((ts, Some(*v)));
        }
        Op::DeleteNode(x) => {
            if !modelo.nodo_vivo(*x) {
                return;
            }
            graph.delete_node(nid(*x)).await.unwrap();
            let ts = graph.last_commit_timestamp();
            modelo.nodos.entry(*x).or_default().push((ts, None));
            for e in 0..M {
                let (s, t) = extremos(e);
                if (s == *x || t == *x) && modelo.arista_viva(e) {
                    modelo.aristas.entry(e).or_default().push((ts, None));
                }
            }
        }
        Op::DeleteEdge(e) => {
            if !modelo.arista_viva(*e) {
                return;
            }
            graph.delete_edge(eid(*e)).await.unwrap();
            let ts = graph.last_commit_timestamp();
            modelo.aristas.entry(*e).or_default().push((ts, None));
        }
        Op::DirectEdge(e, v) | Op::TxEdge(e, v) => {
            let (s, t) = extremos(*e);
            if !(modelo.nodo_vivo(s) && modelo.nodo_vivo(t)) {
                return;
            }
            if matches!(op, Op::DirectEdge(..)) {
                graph.add_edge(maridaje(*e, *v)).await.unwrap();
            } else {
                let mut tx = graph.begin_transaction().await.unwrap();
                tx.add_edge(maridaje(*e, *v)).unwrap();
                tx.commit().await.unwrap();
            }
            let ts = graph.last_commit_timestamp();
            modelo.aristas.entry(*e).or_default().push((ts, Some(*v)));
        }
    }
}

/// Compara cada lector con el modelo en todo timestamp `0..=hasta`.
async fn comparar(graph: &Graph, modelo: &Modelo, hasta: u64, fase: &str) {
    let storage = graph.storage();
    let mut historias = BTreeMap::new();
    for x in 0..N {
        historias.insert(x, graph.history(nid(x)).await.unwrap());
    }
    let mut historias_e = BTreeMap::new();
    for e in 0..M {
        historias_e.insert(e, graph.edge_history(eid(e)).await.unwrap());
    }

    for ts in 0..=hasta {
        for x in 0..N {
            let esperado = modelo.nodo(x, ts);
            let ctx = format!("[{fase}] nodo {x} en ts {ts}");

            match (graph.get_node_at(nid(x), ts).await, esperado) {
                (Ok(n), Some(v)) => assert_eq!(valor(&n.properties), Some(v), "{ctx}: get_node_at"),
                (Err(NopalError::NodeNotFound(_)), None) => {}
                (r, e) => panic!("{ctx}: get_node_at = {r:?}, modelo = {e:?}"),
            }
            let snap = graph.as_of(ts).get_node(nid(x)).await.ok().and_then(|n| valor(&n.properties));
            assert_eq!(snap, esperado, "{ctx}: as_of");
            let strict = graph.get_node_at_strict(nid(x), ts).await.ok().and_then(|n| valor(&n.properties));
            assert_eq!(strict, esperado, "{ctx}: get_node_at_strict");
            let por_historia = historias[&x]
                .iter()
                .find(|v| v.is_valid_at(ts))
                .and_then(|v| valor(&v.node_data.properties));
            assert_eq!(por_historia, esperado, "{ctx}: history()");
        }

        // Clases (parte de los nodos ACTUALES: límite declarado).
        #[cfg(feature = "reasoner")]
        {
            let clases: BTreeSet<NodeId> =
                graph.get_class_nodes_at(ts).await.unwrap().iter().map(|n| n.id).collect();
            let esperado: BTreeSet<NodeId> = (0..N)
                .filter(|x| modelo.nodo_vivo(*x) && modelo.nodo(*x, ts).is_some())
                .map(nid)
                .collect();
            assert_eq!(clases, esperado, "[{fase}] get_class_nodes_at({ts})");
        }

        let mut esperado_tipo = BTreeMap::new();
        for e in 0..M {
            let esperado = modelo.arista(e, ts);
            let por_historia = historias_e[&e]
                .iter()
                .find(|v| v.is_valid_at(ts))
                .and_then(|v| valor(&v.edge_data.properties));
            assert_eq!(por_historia, esperado, "[{fase}] arista {e} en ts {ts}: edge_history()");
            if let Some(v) = esperado {
                esperado_tipo.insert(eid(e), v);
            }
        }
        let por_tipo: BTreeMap<EdgeId, i64> = storage
            .get_versioned_edges_of_type_at(TIPO, ts)
            .await
            .unwrap()
            .into_iter()
            .map(|e| (e.id, valor(&e.properties).unwrap()))
            .collect();
        assert_eq!(por_tipo, esperado_tipo, "[{fase}] get_versioned_edges_of_type_at({ts})");
    }

    // La exportación de versiones = la unión de history() de cada nodo.
    let mut exportadas: BTreeMap<NodeId, BTreeSet<(u64, u64, Option<u64>)>> = BTreeMap::new();
    for v in storage.get_all_versioned_nodes().await.unwrap() {
        exportadas.entry(v.id).or_default().insert((v.version, v.valid_from, v.valid_to));
    }
    for x in 0..N {
        let de_history: BTreeSet<_> =
            historias[&x].iter().map(|v| (v.version, v.valid_from, v.valid_to)).collect();
        assert_eq!(
            exportadas.remove(&nid(x)).unwrap_or_default(),
            de_history,
            "[{fase}] get_all_versioned_nodes vs history() del nodo {x}"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, .. ProptestConfig::default() })]

    #[test]
    fn la_historia_coincide_con_el_modelo(ops in prop::collection::vec(op(), 1..25)) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let mut modelo = Modelo::default();
            let hasta = {
                let graph = Graph::open(dir.path()).await.unwrap();
                for op in &ops {
                    aplicar(&graph, &mut modelo, op).await;
                }
                let hasta = graph.last_commit_timestamp();
                comparar(&graph, &modelo, hasta, "abierta").await;
                graph.close().await.unwrap();
                hasta
            };
            let graph = Graph::open(dir.path()).await.unwrap();
            comparar(&graph, &modelo, hasta, "reabierta").await;
        });
    }
}
