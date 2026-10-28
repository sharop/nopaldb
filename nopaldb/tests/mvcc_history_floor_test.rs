// tests/mvcc_history_floor_test.rs
//
// API del suelo de historia (0.10.1): `Graph::history_floor` /
// `Graph::edge_history_floor` y las metas de la adopción. El comportamiento
// de la adopción lo cubre `mvcc_version_every_write_test.rs` con API previa;
// aquí, lo que solo existe desde 0.10.1. Dominio ficticio: un huerto.

use nopaldb::storage::{META_HISTORY_ADOPTED, META_HISTORY_ADOPTION_TS};
use nopaldb::{Edge, Graph, Node, PropertyValue};

#[tokio::test]
async fn el_suelo_solo_lo_tiene_lo_adoptado() {
    let dir = tempfile::tempdir().unwrap();
    let (sana, suelta, arista_sana, arista_suelta) = {
        let graph = Graph::open(dir.path()).await.unwrap();
        // Una base nueva hace la pasada vacía al crearse: marca sí, F no.
        assert!(graph.storage().get_meta_u64(META_HISTORY_ADOPTED).await.unwrap().is_some());
        assert_eq!(graph.storage().get_meta_u64(META_HISTORY_ADOPTION_TS).await.unwrap(), None);

        let sana = graph.add_node(Node::new("Planta").with_property("nombre", "acelga")).await.unwrap();
        let otra = graph.add_node(Node::new("Planta").with_property("nombre", "betabel")).await.unwrap();
        let arista_sana = graph.add_edge(Edge::new(sana, otra, "vecina")).await.unwrap();

        // Lo que dejaba una versión ≤ 0.10.0: registros sin versión.
        let suelta = Node::new("Planta").with_property("nombre", "chile");
        graph.storage().insert_node(&suelta).await.unwrap();
        let arista = Edge::new(suelta.id, sana, "vecina").with_property("m", PropertyValue::Int(1));
        graph.storage().insert_edges_batch(std::slice::from_ref(&arista)).await.unwrap();
        graph.storage().delete_meta(META_HISTORY_ADOPTED).await.unwrap();
        graph.close().await.unwrap();
        (sana, suelta.id, arista_sana, arista.id)
    };

    let graph = Graph::open(dir.path()).await.unwrap();
    let f = graph.storage().get_meta_u64(META_HISTORY_ADOPTION_TS).await.unwrap().expect("F persistido");
    assert_eq!(graph.history_floor(suelta).await.unwrap(), Some(f));
    assert_eq!(graph.edge_history_floor(arista_suelta).await.unwrap(), Some(f));
    assert_eq!(graph.history_floor(sana).await.unwrap(), None, "la historia sana no recibe suelo");
    assert_eq!(graph.edge_history_floor(arista_sana).await.unwrap(), None);
    assert_eq!(graph.history(suelta).await.unwrap()[0].valid_from, f);
}
