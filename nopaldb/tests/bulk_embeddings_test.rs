// #175: `Graph::add_node_embeddings` — un lote de embeddings de una vez.
//
// Lo que se afirma: el resultado es el mismo que de uno en uno; un lote
// inválido no escribe NADA; con el índice HNSW en caché el lote queda
// indexado (y los que ya tenían embedding se reemplazan); todo sobrevive a
// reabrir la base.

use nopaldb::embeddings::EXACT_SEARCH_THRESHOLD;
use nopaldb::types::{Node, NodeId};
use nopaldb::Graph;

const DIM: usize = 16;

fn vector(seed: u64) -> Vec<f32> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..DIM)
        .map(|_| {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            ((x >> 40) as f32 / (1u64 << 24) as f32) - 0.5
        })
        .collect()
}

async fn nodes(graph: &Graph, n: usize) -> Vec<NodeId> {
    let mut loader = graph.bulk_loader(10_000);
    let mut ids = Vec::with_capacity(n);
    for _ in 0..n {
        let node = Node::new("Chunk");
        ids.push(node.id);
        loader.add_node(node).await.unwrap();
    }
    loader.finish().await.unwrap();
    ids
}

async fn knn(graph: &Graph, q: &[f32], k: usize) -> Vec<NodeId> {
    let index = graph.get_or_build_embedding_index("m").await.unwrap();
    let guard = index.read().unwrap();
    guard.search_knn_with_ef(q, k, 512).unwrap().into_iter().map(|(id, _)| id).collect()
}

/// Bajo el umbral exacto la búsqueda es exacta: el lote y la carga de uno en
/// uno tienen que dar exactamente lo mismo.
#[tokio::test]
async fn a_batch_equals_loading_one_by_one() {
    let a = Graph::in_memory().await.unwrap();
    let b = Graph::in_memory().await.unwrap();
    let ids_a = nodes(&a, 500).await;
    let ids_b = nodes(&b, 500).await;
    for (i, id) in ids_a.iter().enumerate() {
        a.add_node_embedding(*id, vector(i as u64), "m").await.unwrap();
    }
    let items: Vec<_> = ids_b.iter().enumerate().map(|(i, id)| (*id, vector(i as u64))).collect();
    assert_eq!(b.add_node_embeddings("m", items).await.unwrap(), 500);

    for probe in [3u64, 77, 499] {
        let pos = |ids: &[NodeId], hits: Vec<NodeId>| hits.iter().map(|h| ids.iter().position(|x| x == h).unwrap()).collect::<Vec<_>>();
        let q = vector(probe);
        assert_eq!(pos(&ids_a, knn(&a, &q, 10).await), pos(&ids_b, knn(&b, &q, 10).await), "probe {probe}");
    }
    assert_eq!(b.get_node_embedding(ids_b[42], "m").await.unwrap().vector, vector(42));
}

/// Con el índice ya en caché (una búsqueda lo construyó) el lote se inserta
/// en él: por encima del umbral exacto, con `parallel_insert`. Se afirma lo
/// determinista: tamaño, pertenencia, y que cada punto nuevo es alcanzable
/// con una búsqueda ancha (no el primer puesto de un índice aproximado).
#[tokio::test]
async fn a_batch_lands_in_the_cached_index() {
    let graph = Graph::in_memory().await.unwrap();
    let n = EXACT_SEARCH_THRESHOLD + 200;
    let ids = nodes(&graph, n + 300).await;
    let first: Vec<_> = ids[..n].iter().enumerate().map(|(i, id)| (*id, vector(i as u64))).collect();
    graph.add_node_embeddings("m", first).await.unwrap();
    knn(&graph, &vector(0), 1).await; // construye y deja el índice en caché

    let batch: Vec<_> = ids[n..].iter().enumerate().map(|(i, id)| (*id, vector(10_000 + i as u64))).collect();
    graph.add_node_embeddings("m", batch).await.unwrap();
    let stats = graph.embedding_index_stats("m").await.unwrap();
    assert_eq!(stats.size, n + 300);
    assert_eq!(stats.tombstones, 0);
    let index = graph.get_or_build_embedding_index("m").await.unwrap();
    let total = index.read().unwrap().len();
    for (i, id) in ids[n..].iter().enumerate().step_by(37) {
        assert!(index.read().unwrap().contains(*id));
        let hits = index.read().unwrap().search_knn_with_ef(&vector(10_000 + i as u64), 50, total).unwrap();
        assert!(hits.iter().any(|(h, _)| h == id), "nodo {i} del lote alcanzable");
    }
}

/// Un lote sobre nodos que ya tenían embedding los reemplaza: en storage el
/// vector nuevo, en el índice el punto viejo queda como tombstone.
#[tokio::test]
async fn a_batch_replaces_existing_embeddings() {
    let graph = Graph::in_memory().await.unwrap();
    let ids = nodes(&graph, 50).await;
    let old: Vec<_> = ids.iter().enumerate().map(|(i, id)| (*id, vector(i as u64))).collect();
    graph.add_node_embeddings("m", old).await.unwrap();
    knn(&graph, &vector(0), 1).await;
    let new: Vec<_> = ids[..10].iter().enumerate().map(|(i, id)| (*id, vector(5_000 + i as u64))).collect();
    graph.add_node_embeddings("m", new).await.unwrap();

    assert_eq!(graph.get_node_embedding(ids[3], "m").await.unwrap().vector, vector(5_003));
    let stats = graph.embedding_index_stats("m").await.unwrap();
    assert_eq!((stats.size, stats.tombstones), (50, 10));
    assert_eq!(knn(&graph, &vector(5_003), 1).await, vec![ids[3]], "exacto bajo el umbral: el vector nuevo");
}

/// Un lote inválido es un error con nombre y no escribe ningún embedding.
#[tokio::test]
async fn an_invalid_batch_writes_nothing() {
    let graph = Graph::in_memory().await.unwrap();
    let ids = nodes(&graph, 4).await;
    let ghost = NodeId::new_v4();
    let short = vector(9)[..8].to_vec();
    let ghost_str = ghost.to_string();
    let cases: Vec<(Vec<(NodeId, Vec<f32>)>, &str)> = vec![
        (vec![(ids[0], vector(0)), (ids[1], short.clone())], "has dimension 8, the batch has 16"),
        (vec![(ids[0], vector(0)), (ids[0], vector(1))], "appears twice"),
        (vec![(ids[0], vector(0)), (ghost, vector(1))], ghost_str.as_str()),
        (vec![(ids[0], vec![])], "empty vector"),
    ];
    for (items, needle) in cases {
        let err = graph.add_node_embeddings("m", items).await.unwrap_err().to_string();
        assert!(err.contains(needle), "{err}\n  want: {needle}");
        assert!(graph.get_node_embedding(ids[0], "m").await.is_err(), "nada escrito tras: {err}");
    }

    // Con el índice en caché, una dimensión distinta a la suya también.
    graph.add_node_embeddings("m", vec![(ids[0], vector(0))]).await.unwrap();
    knn(&graph, &vector(0), 1).await;
    let err = graph.add_node_embeddings("m", vec![(ids[1], short)]).await.unwrap_err().to_string();
    assert!(err.contains("the index of model 'm' has 16"), "{err}");
    assert!(graph.get_node_embedding(ids[1], "m").await.is_err());
    assert_eq!(graph.add_node_embeddings("m", vec![]).await.unwrap(), 0);
}

/// Lo escrito por lote sobrevive a cerrar y reabrir.
#[tokio::test]
async fn a_batch_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let ids = {
        let graph = Graph::open(&path).await.unwrap();
        let ids = nodes(&graph, 1200).await;
        let items: Vec<_> = ids.iter().enumerate().map(|(i, id)| (*id, vector(i as u64))).collect();
        graph.add_node_embeddings("m", items).await.unwrap();
        graph.close().await.unwrap();
        ids
    };
    let graph = Graph::open(&path).await.unwrap();
    assert_eq!(graph.get_node_embedding(ids[1199], "m").await.unwrap().vector, vector(1199));
    let index = graph.get_or_build_embedding_index("m").await.unwrap();
    assert_eq!(index.read().unwrap().len(), 1200);
}
