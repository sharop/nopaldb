//! Persistencia del índice HNSW (#114): reabrir una base carga el índice de
//! `<data_dir>/hnsw/` en vez de reconstruirlo, y cualquier desfase entre el
//! dump y los embeddings de storage —o un dump dañado— vuelve al rebuild.
//!
//! Los tamaños están justo por encima de `EXACT_SEARCH_THRESHOLD` (1024):
//! por debajo el índice no se persiste a propósito (ver el test
//! `small_index_is_not_persisted`).

use nopaldb::embeddings::EXACT_SEARCH_THRESHOLD;
use nopaldb::types::{Node, NodeId, PropertyValue};
use nopaldb::{Graph, StorageEngine, StorageOptions};
use std::path::Path;

const MODEL: &str = "plantas/minilm";
const DIM: usize = 16;

fn vector(seed: u64) -> Vec<f32> {
    let mut s = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..DIM)
        .map(|_| {
            s ^= s >> 12;
            s ^= s << 25;
            s ^= s >> 27;
            ((s.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
        })
        .collect()
}

fn node_id(i: u128) -> NodeId {
    NodeId::from_u128(0x5A00_0000_0000_0000_0000_0000_0000_0000 + i)
}

fn options(engine: StorageEngine) -> StorageOptions {
    let mut o = StorageOptions::default();
    o.engine = engine;
    o
}

async fn populate(graph: &Graph, n: usize) {
    let mut loader = graph.bulk_loader(512);
    for i in 0..n {
        let mut node = Node::new("Planta").with_property("i", PropertyValue::Int(i as i64));
        node.id = node_id(i as u128);
        loader.add_node(node).await.unwrap();
    }
    loader.finish().await.unwrap();
    for i in 0..n {
        graph.add_node_embedding(node_id(i as u128), vector(i as u64), MODEL).await.unwrap();
    }
}

async fn search(graph: &Graph, query: &[f32]) -> Vec<NodeId> {
    let index = graph.get_or_build_embedding_index(MODEL).await.unwrap();
    let guard = index.read().unwrap();
    guard.search_knn(query, 5).unwrap().into_iter().map(|(id, _)| id).collect()
}

fn dump_files(path: &Path) -> Vec<String> {
    let dir = path.join("hnsw");
    if !dir.exists() {
        return vec![];
    }
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    v.sort();
    v
}

/// Construir, buscar, cerrar: el dump queda en disco y la reapertura lo
/// carga (mismos resultados, `loaded_from_disk_ms` presente).
async fn reopen_loads_from_disk(engine: StorageEngine) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let n = EXACT_SEARCH_THRESHOLD + 300;
    let query = vector(777);

    let before = {
        let graph = Graph::open_with_options(&path, options(engine)).await.unwrap();
        populate(&graph, n).await;
        let stats = graph.embedding_index_stats(MODEL).await;
        assert!(stats.is_none(), "sin búsqueda no hay índice cacheado");
        let hits = search(&graph, &query).await;
        let stats = graph.embedding_index_stats(MODEL).await.unwrap();
        assert_eq!(stats.size, n);
        assert!(stats.persisted, "el build completo escribe el dump: {stats:?}");
        assert_eq!(stats.loaded_from_disk_ms, None, "se construyó, no se cargó");
        graph.close().await.unwrap();
        hits
    };
    let files = dump_files(&path);
    assert_eq!(files.len(), 3, "graph + data + meta: {files:?}");
    assert!(files.iter().all(|f| f.starts_with("plantas_minilm-")), "{files:?}");

    let graph = Graph::open_with_options(&path, options(engine)).await.unwrap();
    let after = search(&graph, &query).await;
    assert_eq!(after, before);
    let stats = graph.embedding_index_stats(MODEL).await.unwrap();
    assert!(stats.loaded_from_disk_ms.is_some(), "debió cargarse de disco: {stats:?}");
    assert!(stats.persisted);
    assert_eq!(stats.size, n);
    graph.close().await.unwrap();
}

#[tokio::test]
async fn reopen_loads_from_disk_sled() {
    reopen_loads_from_disk(StorageEngine::Sled).await;
}

#[cfg(feature = "storage-redb")]
#[tokio::test]
async fn reopen_loads_from_disk_redb() {
    reopen_loads_from_disk(StorageEngine::Redb).await;
}

/// Un insert tras cargar deja el índice sucio; `close` lo reescribe y la
/// siguiente apertura carga el dump nuevo, con el nodo nuevo dentro.
#[tokio::test]
async fn dirty_index_is_rewritten_on_close() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let n = EXACT_SEARCH_THRESHOLD + 50;
    {
        let graph = Graph::open(&path).await.unwrap();
        populate(&graph, n).await;
        search(&graph, &vector(1)).await;
        graph.close().await.unwrap();
    }
    let extra = node_id(9_999);
    let extra_vec = vector(424_242);
    {
        let graph = Graph::open(&path).await.unwrap();
        search(&graph, &vector(1)).await; // carga el dump
        assert!(graph.embedding_index_stats(MODEL).await.unwrap().loaded_from_disk_ms.is_some());
        let mut node = Node::new("Planta");
        node.id = extra;
        graph.add_node(node).await.unwrap();
        graph.add_node_embedding(extra, extra_vec.clone(), MODEL).await.unwrap();
        let stats = graph.embedding_index_stats(MODEL).await.unwrap();
        assert!(!stats.persisted, "con un insert pendiente ya no está en disco");
        assert_eq!(stats.size, n + 1);
        graph.close().await.unwrap(); // reescribe
    }
    let graph = Graph::open(&path).await.unwrap();
    let hits = search(&graph, &extra_vec).await;
    assert_eq!(hits[0], extra);
    let stats = graph.embedding_index_stats(MODEL).await.unwrap();
    assert!(stats.loaded_from_disk_ms.is_some(), "debió cargar el dump reescrito: {stats:?}");
    assert_eq!(stats.size, n + 1);
    assert!(stats.persisted);
}

/// Escribir embeddings y morir sin `close` (aquí: soltar el `Graph`): al
/// reabrir la huella no coincide, se reconstruye y el nodo nuevo está.
#[tokio::test]
async fn writes_without_close_invalidate_the_dump() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let n = EXACT_SEARCH_THRESHOLD + 20;
    {
        let graph = Graph::open(&path).await.unwrap();
        populate(&graph, n).await;
        search(&graph, &vector(1)).await;
        graph.close().await.unwrap();
    }
    let extra = node_id(8_888);
    let extra_vec = vector(31_337);
    {
        let graph = Graph::open(&path).await.unwrap();
        let mut node = Node::new("Planta");
        node.id = extra;
        graph.add_node(node).await.unwrap();
        graph.add_node_embedding(extra, extra_vec.clone(), MODEL).await.unwrap();
        // sin close(): drop suelta el lock y deja el dump viejo en disco
    }
    let graph = Graph::open(&path).await.unwrap();
    let hits = search(&graph, &extra_vec).await;
    assert_eq!(hits[0], extra, "el índice reconstruido incluye lo escrito sin close");
    let stats = graph.embedding_index_stats(MODEL).await.unwrap();
    assert_eq!(stats.loaded_from_disk_ms, None, "dump desfasado ⇒ rebuild: {stats:?}");
    assert_eq!(stats.size, n + 1);
    assert!(stats.persisted, "el rebuild vuelve a escribir el dump");
}

/// Un dump con bytes cambiados o truncado nunca llega a `hnsw_rs`: se
/// detecta y se reconstruye.
#[tokio::test]
async fn corrupt_or_truncated_dump_falls_back_to_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let n = EXACT_SEARCH_THRESHOLD + 10;
    let query = vector(5);
    let before = {
        let graph = Graph::open(&path).await.unwrap();
        populate(&graph, n).await;
        let hits = search(&graph, &query).await;
        graph.close().await.unwrap();
        hits
    };
    let graph_file = path.join("hnsw").join(
        dump_files(&path).into_iter().find(|f| f.ends_with(".hnsw.graph")).unwrap(),
    );
    // 1. bytes volteados en medio
    let mut bytes = std::fs::read(&graph_file).unwrap();
    let mid = bytes.len() / 2;
    for b in &mut bytes[mid..mid + 64] {
        *b ^= 0xFF;
    }
    std::fs::write(&graph_file, &bytes).unwrap();
    {
        let graph = Graph::open(&path).await.unwrap();
        assert_eq!(search(&graph, &query).await, before);
        let stats = graph.embedding_index_stats(MODEL).await.unwrap();
        assert_eq!(stats.loaded_from_disk_ms, None, "corrupto ⇒ rebuild: {stats:?}");
        assert!(stats.persisted, "y el rebuild reescribe un dump sano");
        graph.close().await.unwrap();
    }
    // 2. truncado (escritura interrumpida)
    let f = std::fs::OpenOptions::new().write(true).open(&graph_file).unwrap();
    f.set_len(200).unwrap();
    drop(f);
    {
        let graph = Graph::open(&path).await.unwrap();
        assert_eq!(search(&graph, &query).await, before);
        assert_eq!(graph.embedding_index_stats(MODEL).await.unwrap().loaded_from_disk_ms, None);
        graph.close().await.unwrap();
    }
    // 3. sano otra vez
    let graph = Graph::open(&path).await.unwrap();
    assert_eq!(search(&graph, &query).await, before);
    assert!(graph.embedding_index_stats(MODEL).await.unwrap().loaded_from_disk_ms.is_some());
}

/// Borrar un nodo deja un tombstone; el dump lo conserva y al recargar el
/// nodo no vuelve a aparecer.
#[tokio::test]
async fn tombstones_survive_the_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let n = EXACT_SEARCH_THRESHOLD + 5;
    let victim = node_id(3);
    {
        let graph = Graph::open(&path).await.unwrap();
        populate(&graph, n).await;
        search(&graph, &vector(1)).await;
        graph.delete_node(victim).await.unwrap();
        let stats = graph.embedding_index_stats(MODEL).await.unwrap();
        assert_eq!(stats.tombstones, 1);
        graph.close().await.unwrap();
    }
    let graph = Graph::open(&path).await.unwrap();
    let hits = search(&graph, &vector(3)).await;
    assert!(!hits.contains(&victim), "el nodo borrado no vuelve: {hits:?}");
    let stats = graph.embedding_index_stats(MODEL).await.unwrap();
    assert!(stats.loaded_from_disk_ms.is_some());
    assert_eq!(stats.tombstones, 1);
    assert_eq!(stats.size, n - 1);
}

/// Bajo el umbral del camino exacto no se persiste: la reconstrucción
/// cuesta milisegundos y `persisted` lo dice.
#[tokio::test]
async fn small_index_is_not_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let graph = Graph::open(&path).await.unwrap();
    populate(&graph, 50).await;
    search(&graph, &vector(1)).await;
    let stats = graph.embedding_index_stats(MODEL).await.unwrap();
    assert!(!stats.persisted);
    graph.close().await.unwrap();
    assert!(dump_files(&path).is_empty(), "{:?}", dump_files(&path));
}

/// En memoria no hay directorio: nada se escribe y las stats lo reflejan.
#[tokio::test]
async fn in_memory_graph_never_persists() {
    let graph = Graph::in_memory().await.unwrap();
    populate(&graph, EXACT_SEARCH_THRESHOLD + 2).await;
    search(&graph, &vector(1)).await;
    let stats = graph.embedding_index_stats(MODEL).await.unwrap();
    assert!(!stats.persisted);
    assert_eq!(stats.loaded_from_disk_ms, None);
    assert!(graph.persist_embedding_indices().await.unwrap().is_empty());
}

/// `persist_embedding_indices` a mano: escribe solo los índices sucios y
/// devuelve sus modelos.
#[tokio::test]
async fn persist_embedding_indices_reports_what_it_wrote() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let graph = Graph::open(&path).await.unwrap();
    populate(&graph, EXACT_SEARCH_THRESHOLD + 1).await;
    search(&graph, &vector(1)).await; // build ⇒ ya escrito
    assert!(graph.persist_embedding_indices().await.unwrap().is_empty(), "nada sucio");
    let extra = node_id(7_777);
    let mut node = Node::new("Planta");
    node.id = extra;
    graph.add_node(node).await.unwrap();
    graph.add_node_embedding(extra, vector(1_234), MODEL).await.unwrap();
    assert_eq!(graph.persist_embedding_indices().await.unwrap(), vec![MODEL.to_string()]);
    assert!(graph.embedding_index_stats(MODEL).await.unwrap().persisted);
    graph.close().await.unwrap();
}
