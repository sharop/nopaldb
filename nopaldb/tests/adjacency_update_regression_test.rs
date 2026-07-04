// tests/adjacency_update_regression_test.rs
//
// Regresión: actualizar un nodo (vía commit o replay del WAL) NO debe borrar
// su adyacencia. Bug histórico: el upsert re-inicializaba las listas en
// memoria con Vec::new() y persistía listas vacías, así que un update de un
// nodo con aristas vaciaba neighbors() — y tras reabrir, también en disco.

use nopaldb::{Direction, Edge, Graph, Node, PropertyValue};
use uuid::Uuid;

#[tokio::test]
async fn updating_a_node_preserves_its_adjacency() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();
    let hub_id = Uuid::new_v4();

    {
        let graph = Graph::open(dir.path()).await?;

        // hub con 3 aristas salientes y 1 entrante
        let mut tx = graph.begin_transaction().await?;
        tx.add_node(Node::with_id(hub_id, "Hub").with_property("v", PropertyValue::Int(1)))
            .await?;
        tx.commit().await?;

        let mut spokes = Vec::new();
        let mut tx = graph.begin_transaction().await?;
        for i in 0..3 {
            let s = tx
                .add_node(Node::new("Spoke").with_property("n", PropertyValue::Int(i)))
                .await?;
            spokes.push(s);
        }
        tx.commit().await?;

        for s in &spokes {
            graph.add_edge(Edge::new(hub_id, *s, "LINKS")).await?;
        }
        graph.add_edge(Edge::new(spokes[0], hub_id, "BACK")).await?;

        assert_eq!(graph.neighbors(hub_id, Direction::Outgoing).await?.len(), 3);
        assert_eq!(graph.neighbors(hub_id, Direction::Incoming).await?.len(), 1);

        // UPDATE del hub — antes esto vaciaba su adyacencia
        let mut tx = graph.begin_transaction().await?;
        tx.add_node(Node::with_id(hub_id, "Hub").with_property("v", PropertyValue::Int(2)))
            .await?;
        tx.commit().await?;

        assert_eq!(
            graph.neighbors(hub_id, Direction::Outgoing).await?.len(),
            3,
            "update wiped outgoing adjacency"
        );
        assert_eq!(
            graph.neighbors(hub_id, Direction::Incoming).await?.len(),
            1,
            "update wiped incoming adjacency"
        );
    }

    // Y la adyacencia PERSISTIDA también sobrevive el reopen
    let graph = Graph::open(dir.path()).await?;
    assert_eq!(
        graph.neighbors(hub_id, Direction::Outgoing).await?.len(),
        3,
        "persisted outgoing adjacency wiped by node update"
    );
    assert_eq!(
        graph.neighbors(hub_id, Direction::Incoming).await?.len(),
        1,
        "persisted incoming adjacency wiped by node update"
    );

    // La versión del nodo sí avanzó (el update ocurrió)
    let node = graph.get_node(hub_id).await?;
    assert_eq!(node.properties.get("v"), Some(&PropertyValue::Int(2)));

    Ok(())
}
