//! `Graph::export_snapshot`: one consistent read of the graph.
//!
//! The export holds the write gate while it reads, so a write is never seen
//! halfway. A writer keeps adding, in one transaction each, a node and an
//! edge to it, while exports run in a loop. If the export read nodes and
//! edges at two different moments it could see an edge whose target was
//! not read yet; with the gate held, every exported edge has both endpoints.

use std::collections::HashSet;
use std::sync::Arc;

use nopaldb::types::{Edge, Node};
use nopaldb::Graph;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_export_never_sees_a_write_halfway() {
    let graph = Arc::new(Graph::in_memory().await.unwrap());
    let root = graph.add_node(Node::new("Raiz")).await.unwrap();

    let writer = {
        let graph = graph.clone();
        tokio::spawn(async move {
            for i in 0..300 {
                let mut tx = graph.begin_transaction().await.unwrap();
                let node = Node::new("Hoja").with_property("i", nopaldb::types::PropertyValue::Int(i));
                let id = tx.add_node(node).await.unwrap();
                tx.add_edge(Edge::new(root, id, "TIENE")).unwrap();
                tx.commit().await.unwrap();
            }
        })
    };

    let mut exports = 0;
    while !writer.is_finished() {
        let (nodes, edges) = graph.export_snapshot(None).await.unwrap();
        let ids: HashSet<_> = nodes.iter().map(|n| n.id).collect();
        for e in &edges {
            assert!(ids.contains(&e.source) && ids.contains(&e.target), "an edge without its endpoints");
        }
        // Each leaf comes with exactly one edge from the root.
        assert_eq!(edges.len(), nodes.len() - 1, "nodes and edges from the same state");
        exports += 1;
    }
    writer.await.unwrap();
    assert!(exports > 0);

    let (nodes, edges) = graph.export_snapshot(None).await.unwrap();
    assert_eq!((nodes.len(), edges.len()), (301, 300));

    // With a label: the induced subgraph (the root is not a leaf, so no edge).
    let (leaves, leaf_edges) = graph.export_snapshot(Some("Hoja")).await.unwrap();
    assert_eq!((leaves.len(), leaf_edges.len()), (300, 0));
}
