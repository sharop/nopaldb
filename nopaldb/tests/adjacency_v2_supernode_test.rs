// tests/adjacency_v2_supernode_test.rs
//
// F5.3: borrar un SUPERNODO purga sus aristas por chunks idempotentes
// (≈3_333 aristas ≈ 10k mutaciones por apply_multi; 25k aristas ⇒ varios
// chunks) sin dejar residuos de ningún tipo:
//   - cero registros en el keyspace `edges`,
//   - cero claves O/I en `adjacency` de AMBOS lados (propias y espejos),
//   - RAM limpia inmediatamente.
// El reopen es el detector de residuos en disco: la RAM se reconstruye
// escaneando el keyspace `adjacency`, así que cualquier clave O/I residual
// se materializaría como grado > 0 en algún spoke.

use nopaldb::{Direction, Edge, Graph, Node};

const SPOKES: usize = 25_000;

#[tokio::test(flavor = "multi_thread")]
async fn delete_supernode_purga_edges_y_adyacencia_en_chunks() -> nopaldb::Result<()> {
    let dir = tempfile::tempdir().unwrap();

    let hub;
    let spokes: Vec<_>;
    {
        let graph = Graph::open(dir.path()).await?;
        hub = graph.add_node(Node::new("Hub")).await?;

        let nodes: Vec<Node> = (0..SPOKES).map(|_| Node::new("Spoke")).collect();
        spokes = graph.add_nodes_batch(nodes).await?;

        // Mitad salientes y mitad entrantes: la purga debe limpiar la clave
        // propia Y el espejo del otro extremo en ambos sentidos.
        let edges: Vec<Edge> = spokes
            .iter()
            .enumerate()
            .map(|(i, s)| {
                if i % 2 == 0 {
                    Edge::new(hub, *s, "LINKS")
                } else {
                    Edge::new(*s, hub, "LINKS")
                }
            })
            .collect();
        graph.add_edges_batch(edges).await?;

        assert_eq!(graph.degree(hub, Direction::Both).await?, SPOKES);

        graph.delete_node(hub).await?;

        // RAM limpia inmediatamente: hub sin grado ni entidad, edges tree
        // vacío y CERO residuos en las listas de los spokes (ambos lados).
        assert_eq!(graph.degree(hub, Direction::Both).await?, 0);
        assert!(graph.get_node(hub).await.is_err(), "hub should be gone");
        assert!(
            graph.get_all_edges().await?.is_empty(),
            "edges tree must be empty after supernode delete"
        );
        for s in &spokes {
            assert_eq!(
                graph.degree(*s, Direction::Both).await?,
                0,
                "spoke {s} keeps a stale edge in RAM"
            );
        }

        graph.close().await?;
    }

    // Reopen: la RAM sale del scan del keyspace `adjacency` — una clave O/I
    // residual de cualquier lado aparecería aquí como grado > 0.
    let graph = Graph::open(dir.path()).await?;
    assert!(
        graph.get_all_edges().await?.is_empty(),
        "edges tree must be empty after reopen"
    );
    assert_eq!(graph.degree(hub, Direction::Both).await?, 0);
    for s in &spokes {
        assert_eq!(
            graph.degree(*s, Direction::Both).await?,
            0,
            "spoke {s} has residual adjacency keys on disk"
        );
    }

    Ok(())
}
