// tests/property_model_test.rs
//
// Property tests (proptest): secuencias aleatorias de operaciones aplicadas al
// motor deben producir EXACTAMENTE el mismo estado que un modelo trivial en
// memoria — incluyendo errores esperados, adyacencia, y tras reabrir la base.
//
// Dos propiedades:
//   1. Secuencial: ops mixtas (tx commits, escrituras directas, deletes)
//      contra el modelo, comparación final + persistencia post-reopen.
//   2. Concurrente con particiones disjuntas: N tasks aplican sus propias
//      secuencias; el estado final debe ser la unión de los modelos.
//
// Casos por corrida: 24 por default (IO real de sled por caso); el nightly
// sube vía PROPTEST_CASES.

use nopaldb::{Direction, Edge, Graph, Node, NodeId, PropertyValue};
use proptest::prelude::*;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

const POOL: u128 = 12; // ids de nodo posibles por partición

#[derive(Debug, Clone)]
enum Op {
    UpsertNode { k: u128, v: i64 },
    DeleteNode { k: u128 },
    AddEdge { a: u128, b: u128 },
    DeleteEdge { a: u128, b: u128 },
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => (0..POOL, 0i64..1000).prop_map(|(k, v)| Op::UpsertNode { k, v }),
        1 => (0..POOL).prop_map(|k| Op::DeleteNode { k }),
        3 => (0..POOL, 0..POOL).prop_map(|(a, b)| Op::AddEdge { a, b }),
        1 => (0..POOL, 0..POOL).prop_map(|(a, b)| Op::DeleteEdge { a, b }),
    ]
}

/// Modelo de referencia trivial.
#[derive(Default)]
struct Model {
    nodes: HashMap<u128, i64>,
    /// (a, b) → id de la arista viva
    edges: HashMap<(u128, u128), NodeId>,
}

fn nid(partition: u128, k: u128) -> Uuid {
    Uuid::from_u128(0xA000_0000 + partition * 1000 + k)
}

/// Aplica una op al grafo Y al modelo; verifica que el resultado del motor
/// coincide con lo que el modelo espera (éxito o error).
async fn apply(graph: &Graph, model: &mut Model, partition: u128, op: &Op) {
    match op {
        Op::UpsertNode { k, v } => {
            // vía transacción: ejercita MVCC + batch atómico
            let mut tx = graph.begin_transaction().await.expect("begin");
            tx.add_node(
                Node::with_id(nid(partition, *k), "P").with_property("v", PropertyValue::Int(*v)),
            )
            .await
            .expect("tx add");
            tx.commit().await.expect("commit");
            model.nodes.insert(*k, *v);
        }
        Op::DeleteNode { k } => {
            let existed = model.nodes.remove(k).is_some();
            let result = graph.delete_node(nid(partition, *k)).await;
            assert_eq!(
                result.is_ok(),
                existed,
                "delete_node({k}) succeeded={} but model existed={existed}",
                result.is_ok()
            );
            if existed {
                // el motor elimina aristas incidentes; el modelo también
                model.edges.retain(|(a, b), _| a != k && b != k);
            }
        }
        Op::AddEdge { a, b } => {
            let endpoints = model.nodes.contains_key(a) && model.nodes.contains_key(b);
            let already = model.edges.contains_key(&(*a, *b));
            if !endpoints || already {
                // already: mantener una arista por par para un modelo simple
                if !endpoints {
                    let r = graph
                        .add_edge(Edge::new(nid(partition, *a), nid(partition, *b), "E"))
                        .await;
                    assert!(r.is_err(), "add_edge without endpoints must fail");
                }
                return;
            }
            let edge = Edge::new(nid(partition, *a), nid(partition, *b), "E");
            let id = graph.add_edge(edge).await.expect("add_edge");
            model.edges.insert((*a, *b), id);
        }
        Op::DeleteEdge { a, b } => {
            if let Some(id) = model.edges.remove(&(*a, *b)) {
                graph.delete_edge(id).await.expect("delete_edge");
            }
        }
    }
}

/// Compara el estado final del grafo contra el modelo.
async fn assert_matches(graph: &Graph, model: &Model, partition: u128) {
    // Nodos: presencia y valor
    for k in 0..POOL {
        let got = graph.get_node(nid(partition, k)).await;
        match model.nodes.get(&k) {
            Some(v) => {
                let node = got.unwrap_or_else(|_| panic!("node {k} missing (model has it)"));
                assert_eq!(
                    node.properties.get("v"),
                    Some(&PropertyValue::Int(*v)),
                    "node {k} value mismatch"
                );
            }
            None => assert!(got.is_err(), "node {k} exists but model deleted it"),
        }
    }

    // Aristas vivas del grafo (de esta partición) == modelo
    let live: HashSet<NodeId> = model.edges.values().cloned().collect();
    let all_edges = graph.get_all_edges().await.expect("all edges");
    let in_partition: HashSet<NodeId> = all_edges
        .iter()
        .filter(|e| {
            (0..POOL).any(|k| e.source == nid(partition, k))
                && (0..POOL).any(|k| e.target == nid(partition, k))
        })
        .map(|e| e.id)
        .collect();
    assert_eq!(in_partition, live, "edge set mismatch vs model");

    // Adyacencia saliente == modelo
    for k in 0..POOL {
        if !model.nodes.contains_key(&k) {
            continue;
        }
        let expected: HashSet<NodeId> = model
            .edges
            .iter()
            .filter(|((a, _), _)| *a == k)
            .map(|((_, b), _)| nid(partition, *b))
            .collect();
        let got: HashSet<NodeId> = graph
            .neighbors(nid(partition, k), Direction::Outgoing)
            .await
            .expect("neighbors")
            .into_iter()
            .collect();
        assert_eq!(got, expected, "outgoing adjacency mismatch for node {k}");
    }
}

fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(24)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), ..ProptestConfig::default() })]

    // 1. Secuencial + persistencia: motor ≡ modelo, también tras reopen.
    #[test]
    fn random_ops_match_model_and_survive_reopen(ops in proptest::collection::vec(op_strategy(), 1..40)) {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let mut model = Model::default();
            {
                let graph = Graph::open(dir.path()).await.expect("open");
                for op in &ops {
                    apply(&graph, &mut model, 0, op).await;
                }
                assert_matches(&graph, &model, 0).await;
            }
            // Persistencia: reabrir y volver a comparar
            let graph = Graph::open(dir.path()).await.expect("reopen");
            assert_matches(&graph, &model, 0).await;
        });
    }

    // 2. Concurrente con particiones disjuntas: unión de modelos.
    #[test]
    fn concurrent_disjoint_partitions_match_models(
        seqs in proptest::collection::vec(proptest::collection::vec(op_strategy(), 1..20), 4..=4)
    ) {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(4).enable_all().build().unwrap();
        rt.block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let graph = std::sync::Arc::new(Graph::open(dir.path()).await.expect("open"));

            let mut handles = Vec::new();
            for (p, ops) in seqs.into_iter().enumerate() {
                let g = std::sync::Arc::clone(&graph);
                handles.push(tokio::spawn(async move {
                    let mut model = Model::default();
                    for op in &ops {
                        apply(&g, &mut model, p as u128 + 1, op).await;
                    }
                    (p as u128 + 1, model)
                }));
            }
            for h in handles {
                let (partition, model) = h.await.expect("partition task");
                assert_matches(&graph, &model, partition).await;
            }
        });
    }
}
