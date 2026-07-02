// tests/edge_persistence_test.rs
//
// Test C2: Verify that edges survive graph close/reopen
// Bug: rebuild_indices was scanning the wrong sled tree,
// causing all edges to be lost after restart.

use nopaldb::{Graph, Node, Edge, PropertyValue, Result};
use tempfile::TempDir;

/// Core persistence test: create edges, reopen graph, verify they exist
#[tokio::test]
async fn test_edges_persist_after_reopen() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("test_edge_persist");

    let mut saved_edge_ids = Vec::new();
    let mut saved_alice_id = None;
    let mut saved_charlie_id = None;

    // === Phase 1: Create graph with nodes and edges ===
    {
        let graph = Graph::open(&path).await?;

        let alice = graph.add_node(
            Node::new("Person").with_property("name", PropertyValue::String("Alice".into()))
        ).await?;
        let bob = graph.add_node(
            Node::new("Person").with_property("name", PropertyValue::String("Bob".into()))
        ).await?;
        let charlie = graph.add_node(
            Node::new("Person").with_property("name", PropertyValue::String("Charlie".into()))
        ).await?;

        let e1 = graph.add_edge(Edge::new(alice, bob, "KNOWS")).await?;
        let e2 = graph.add_edge(Edge::new(bob, charlie, "KNOWS")).await?;
        let e3 = graph.add_edge(Edge::new(alice, charlie, "FRIENDS_WITH")).await?;

        saved_edge_ids.push(e1);
        saved_edge_ids.push(e2);
        saved_edge_ids.push(e3);
        saved_alice_id = Some(alice);
        saved_charlie_id = Some(charlie);

        // Verify edges exist before close
        assert!(graph.get_edge(e1).await.is_ok(), "Edge e1 should exist before close");
        assert!(graph.get_edge(e2).await.is_ok(), "Edge e2 should exist before close");
        assert!(graph.get_edge(e3).await.is_ok(), "Edge e3 should exist before close");

        // Verify adjacency works
        let alice_edges = graph.get_outgoing_edges(alice).await?;
        assert_eq!(alice_edges.len(), 2, "Alice should have 2 outgoing edges");

        let charlie_edges = graph.get_incoming_edges(charlie).await?;
        assert_eq!(charlie_edges.len(), 2, "Charlie should have 2 incoming edges");

        // Graph drops here, closing the DB
    }

    // === Phase 2: Reopen and verify everything survived ===
    {
        let graph = Graph::open(&path).await?;

        // Verify edges still exist in storage
        for eid in &saved_edge_ids {
            assert!(graph.get_edge(*eid).await.is_ok(),
                    "Edge {:?} should exist after reopen", eid);
        }

        // Verify adjacency indices were rebuilt correctly
        let alice_id = saved_alice_id.unwrap();
        let charlie_id = saved_charlie_id.unwrap();

        let alice_out = graph.get_outgoing_edges(alice_id).await?;
        assert_eq!(alice_out.len(), 2,
                   "Alice should still have 2 outgoing edges after reopen, got {}", alice_out.len());

        let charlie_in = graph.get_incoming_edges(charlie_id).await?;
        assert_eq!(charlie_in.len(), 2,
                   "Charlie should still have 2 incoming edges after reopen, got {}", charlie_in.len());
    }

    Ok(())
}

/// Test that rebuild_indices is triggered when adjacency indices are missing
#[tokio::test]
async fn test_rebuild_indices_from_edges_tree() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("test_rebuild_fallback");

    // Phase 1: Create data
    let alice_id;
    let bob_id;
    let edge_id;
    {
        let graph = Graph::open(&path).await?;
        alice_id = graph.add_node(
            Node::new("Person").with_property("name", PropertyValue::String("Alice".into()))
        ).await?;
        bob_id = graph.add_node(
            Node::new("Person").with_property("name", PropertyValue::String("Bob".into()))
        ).await?;
        edge_id = graph.add_edge(Edge::new(alice_id, bob_id, "KNOWS")).await?;
    }

    // Phase 2: Manually clear adjacency indices to force rebuild path
    {
        let db = sled::open(&path).unwrap();
        let mut keys_to_remove = Vec::new();
        for item in db.scan_prefix(b"idx:out:") {
            let (key, _) = item.unwrap();
            keys_to_remove.push(key);
        }
        for item in db.scan_prefix(b"idx:in:") {
            let (key, _) = item.unwrap();
            keys_to_remove.push(key);
        }
        for key in keys_to_remove {
            db.remove(key).unwrap();
        }
        db.flush().unwrap();
        drop(db);
    }

    // Phase 3: Reopen - should trigger rebuild_indices and recover edges
    {
        let graph = Graph::open(&path).await?;

        // Edge should still be retrievable from storage
        let edge = graph.get_edge(edge_id).await?;
        assert_eq!(edge.source, alice_id);
        assert_eq!(edge.target, bob_id);

        // Adjacency should be rebuilt from edges tree
        let alice_out = graph.get_outgoing_edges(alice_id).await?;
        assert_eq!(alice_out.len(), 1,
                   "rebuild_indices should recover Alice's outgoing edge, got {}", alice_out.len());

        let bob_in = graph.get_incoming_edges(bob_id).await?;
        assert_eq!(bob_in.len(), 1,
                   "rebuild_indices should recover Bob's incoming edge, got {}", bob_in.len());
    }

    Ok(())
}