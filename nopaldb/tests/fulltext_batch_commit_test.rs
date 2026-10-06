// #178: el índice full-text publica sus escrituras en un commit por lote, no
// en uno por documento. Lo que no puede cambiar con eso: quien escribe
// encuentra lo que escribió, una sobrescritura nunca deja el nodo invisible,
// y el índice sobrevive a cerrar, a reabrir y a morir sin `close`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use nopaldb::index::IndexType;
use nopaldb::types::{Node, NodeId, PropertyValue};
use nopaldb::{Graph, HybridQuery};

fn s(v: &str) -> PropertyValue {
    PropertyValue::String(v.to_string())
}

async fn fulltext(g: &Graph, text: &str) -> Vec<NodeId> {
    let mut q = HybridQuery::new();
    q.text = Some(text.to_string());
    q.k = 1000;
    let mut ids: Vec<NodeId> = g.search_hybrid(q).await.unwrap().into_iter().map(|h| h.node_id).collect();
    ids.sort();
    ids
}

/// Sobrescribir el texto de un nodo retira el documento viejo y añade el
/// nuevo. Antes eran dos commits y entre ellos el nodo no aparecía por
/// ninguno de los dos textos; ahora van en el mismo commit. Un lector
/// concurrente busca la palabra que comparten las dos versiones: tiene que
/// encontrar el nodo SIEMPRE.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn overwriting_the_text_never_leaves_the_node_invisible() {
    let dir = tempfile::tempdir().unwrap();
    let graph = Arc::new(Graph::open(dir.path()).await.unwrap());
    let id = NodeId::new_v4();
    graph.add_node(Node::with_id(id, "Doc").with_property("body", s("alfa comun"))).await.unwrap();
    graph.create_index("Doc", "body", IndexType::FullText).await.unwrap();

    let done = Arc::new(AtomicBool::new(false));
    let reader = {
        let (graph, done) = (graph.clone(), done.clone());
        tokio::spawn(async move {
            let mut reads = 0usize;
            while !done.load(Ordering::Acquire) {
                assert_eq!(fulltext(&graph, "comun").await, vec![id], "lectura {reads}: el nodo desapareció");
                reads += 1;
            }
            reads
        })
    };
    for i in 0..60 {
        let body = if i % 2 == 0 { "beta comun" } else { "alfa comun" };
        graph.add_node(Node::with_id(id, "Doc").with_property("body", s(body))).await.unwrap();
    }
    done.store(true, Ordering::Release);
    let reads = reader.await.unwrap();
    assert!(reads > 0, "el lector llegó a correr");
    // Última versión (i = 59): "alfa comun".
    assert_eq!(fulltext(&graph, "alfa").await, vec![id]);
    assert!(fulltext(&graph, "beta").await.is_empty(), "el texto viejo ya no matchea");
}

/// `create_index` sobre datos existentes publica la población entera, y
/// reabrir (que reconstruye el índice) deja el mismo contenido.
#[tokio::test]
async fn create_index_and_reopen_publish_every_document() {
    let dir = tempfile::tempdir().unwrap();
    let mut ids = Vec::new();
    {
        let graph = Graph::open(dir.path()).await.unwrap();
        let mut loader = graph.bulk_loader(1000);
        for i in 0..300 {
            let node = Node::new("Doc").with_property("body", s(&format!("nopal numero{i}")));
            ids.push(node.id);
            loader.add_node(node).await.unwrap();
        }
        loader.finish().await.unwrap();
        graph.create_index("Doc", "body", IndexType::FullText).await.unwrap();
        ids.sort();
        assert_eq!(fulltext(&graph, "nopal").await, ids, "todos tras create_index");
        graph.close().await.unwrap();
    }
    let graph = Graph::open(dir.path()).await.unwrap();
    assert_eq!(fulltext(&graph, "nopal").await, ids, "todos tras reabrir");
    assert_eq!(fulltext(&graph, "numero7").await.len(), 1);
}

/// Morir sin `close` con escrituras recientes: el índice se reconstruye al
/// abrir desde storage, así que lo escrito aparece aunque su commit de
/// tantivy no hubiera llegado a disco.
#[tokio::test]
async fn writes_without_close_are_found_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let id = NodeId::new_v4();
    {
        let graph = Graph::open(dir.path()).await.unwrap();
        graph.create_index("Doc", "body", IndexType::FullText).await.unwrap();
        graph.close().await.unwrap();
    }
    {
        let graph = Graph::open(dir.path()).await.unwrap();
        graph.add_node(Node::with_id(id, "Doc").with_property("body", s("biznaga"))).await.unwrap();
        // sin close()
    }
    let graph = Graph::open(dir.path()).await.unwrap();
    assert_eq!(fulltext(&graph, "biznaga").await, vec![id]);
}
