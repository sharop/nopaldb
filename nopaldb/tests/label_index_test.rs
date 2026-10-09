// #207: índice de etiquetas. Las búsquedas por etiqueta (`get_nodes_by_label`,
// `scan_nodes_batch(Some(label))`, NQL `from (c:Label)`) leen solo los
// nodos de esa etiqueta en vez de recorrer la base.
//
// El invariante: para cada etiqueta, lo que devuelve el índice es
// exactamente lo que daría un recorrido de todos los nodos, tras cualquier
// mezcla de escrituras (alta, upsert con cambio de etiqueta, borrado, lote
// con ids repetidos, transacción, NQL) y tras reabrir. Las pruebas de
// reconstrucción (una versión anterior escribió en la base, formato más
// nuevo) viven como unit tests en `storage/mod.rs`.

use std::collections::BTreeSet;

use nopaldb::{Graph, Node, NodeId, Result};

const LABELS: &[&str] = &["Doc", "Chunk", "Entity", "PER", "PERSON"];

/// Compara, etiqueta por etiqueta, el índice contra un recorrido completo,
/// por las tres lecturas públicas.
async fn assert_index_matches_scan(graph: &Graph, context: &str) -> Result<()> {
    let all = graph.get_all_nodes().await?;
    for label in LABELS {
        let want: BTreeSet<NodeId> = all.iter().filter(|n| n.label == *label).map(|n| n.id).collect();

        let by_label: BTreeSet<NodeId> =
            graph.get_nodes_by_label(label).await?.into_iter().map(|n| n.id).collect();
        assert_eq!(by_label, want, "{context}: get_nodes_by_label({label})");

        // Paginado con un límite chico para cruzar varias páginas.
        let storage = graph.storage();
        let mut paged = BTreeSet::new();
        let mut cursor: Option<String> = None;
        loop {
            let (nodes, next) = storage.scan_nodes_batch(Some(label), cursor.as_deref(), 3).await?;
            for node in nodes {
                assert_eq!(node.label, *label, "{context}: scan_nodes_batch devolvió otra etiqueta");
                assert!(paged.insert(node.id), "{context}: scan_nodes_batch repitió un nodo");
            }
            match next {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }
        assert_eq!(paged, want, "{context}: scan_nodes_batch({label})");

        let rows = graph
            .execute_nql(&format!("find n.i from (n:{label})"))
            .await?;
        assert_eq!(rows.len(), want.len(), "{context}: from (n:{label})");
    }
    Ok(())
}

/// Escrituras por todos los caminos, con cambios de etiqueta.
async fn write_everything(graph: &Graph) -> Result<()> {
    // Altas directas.
    let mut docs = Vec::new();
    for i in 0..10i64 {
        docs.push(graph.add_node(Node::new("Doc").with_property("i", i)).await?);
    }
    graph.add_node(Node::new("PER").with_property("i", 1i64)).await?;
    graph.add_node(Node::new("PERSON").with_property("i", 2i64)).await?;

    // Upsert directo con cambio de etiqueta: Doc → Chunk.
    graph.add_node(Node::with_id(docs[0], "Chunk").with_property("i", 0i64)).await?;
    // Upsert sin cambio de etiqueta.
    graph.add_node(Node::with_id(docs[1], "Doc").with_property("i", 100i64)).await?;

    // Borrados.
    graph.delete_node(docs[2]).await?;

    // Lote con ids nuevos, un id existente que cambia de etiqueta y un id
    // repetido dentro del lote (gana el último: Entity).
    let repeated = NodeId::new_v4();
    let batch = vec![
        Node::new("Chunk").with_property("i", 1i64),
        Node::with_id(docs[3], "Entity"),
        Node::with_id(repeated, "Doc"),
        Node::with_id(repeated, "Chunk"),
        Node::with_id(repeated, "Entity"),
    ];
    graph.add_nodes_batch(batch).await?;

    // Transacción: alta y cambio de etiqueta de un nodo existente.
    let mut tx = graph.begin_transaction().await?;
    tx.add_node(Node::new("Entity").with_property("i", 7i64)).await?;
    tx.add_node(Node::with_id(docs[4], "Entity")).await?;
    tx.commit().await?;

    // NQL.
    graph.execute_statement("add (n:Chunk {i: 42})").await?;
    Ok(())
}

#[tokio::test]
async fn the_label_index_matches_a_scan_after_every_kind_of_write() -> Result<()> {
    let graph = Graph::in_memory().await?;
    assert_index_matches_scan(&graph, "vacío").await?;
    write_everything(&graph).await?;
    assert_index_matches_scan(&graph, "tras escribir").await?;
    Ok(())
}

#[tokio::test]
async fn the_label_index_survives_a_reopen() -> Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    {
        let graph = Graph::open(&path).await?;
        write_everything(&graph).await?;
        graph.close().await?;
    }
    let graph = Graph::open(&path).await?;
    assert_index_matches_scan(&graph, "tras reabrir").await?;
    // Y sigue al día con escrituras de la sesión nueva.
    let id = graph.add_node(Node::new("Doc")).await?;
    graph.add_node(Node::with_id(id, "PER")).await?;
    assert_index_matches_scan(&graph, "tras reabrir y escribir").await?;
    Ok(())
}

#[tokio::test]
async fn a_label_is_not_a_prefix_of_another() -> Result<()> {
    let graph = Graph::in_memory().await?;
    let per = graph.add_node(Node::new("PER")).await?;
    let person = graph.add_node(Node::new("PERSON")).await?;
    let ids = |nodes: Vec<Node>| nodes.into_iter().map(|n| n.id).collect::<Vec<_>>();
    assert_eq!(ids(graph.get_nodes_by_label("PER").await?), vec![per]);
    assert_eq!(ids(graph.get_nodes_by_label("PERSON").await?), vec![person]);
    assert!(graph.get_nodes_by_label("PE").await?.is_empty());
    Ok(())
}
