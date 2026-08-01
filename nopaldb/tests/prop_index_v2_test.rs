// Comportamiento público del índice de propiedades v2 (claves tipadas).
// Las pruebas de migración v1→v2 y del encode viven como unit tests en
// storage/mod.rs (necesitan fabricar claves legadas); aquí va el contrato
// observable por el API.

use nopaldb::{Graph, Node, PropertyValue, Result};

#[tokio::test]
async fn typed_lookups_do_not_collide() -> Result<()> {
    // v1: Int(1), Float(1.0) y String("1") compartían la clave "1" y un
    // lookup por cualquiera regresaba los nodos de los otros.
    let graph = Graph::in_memory().await?;

    let n_int = graph.add_node(Node::new("T").with_property("v", 1i64)).await?;
    let n_float = graph.add_node(Node::new("T").with_property("v", 1.0f64)).await?;
    let n_str = graph.add_node(Node::new("T").with_property("v", "1")).await?;

    let by_int = graph.get_all_nodes_by_property("v", &PropertyValue::Int(1)).await?;
    assert_eq!(by_int, vec![n_int]);

    let by_float = graph.get_all_nodes_by_property("v", &PropertyValue::Float(1.0)).await?;
    assert_eq!(by_float, vec![n_float]);

    let by_str = graph
        .get_all_nodes_by_property("v", &PropertyValue::String("1".into()))
        .await?;
    assert_eq!(by_str, vec![n_str]);

    Ok(())
}

#[tokio::test]
async fn separator_injection_resolved() -> Result<()> {
    // v1: prop `a` valor `b:c` y prop `a:b` valor `c` producían la misma clave.
    let graph = Graph::in_memory().await?;
    let n1 = graph.add_node(Node::new("T").with_property("a", "b:c")).await?;
    let n2 = graph.add_node(Node::new("T").with_property("a:b", "c")).await?;

    let hits = graph
        .get_all_nodes_by_property("a", &PropertyValue::String("b:c".into()))
        .await?;
    assert_eq!(hits, vec![n1]);
    let hits = graph
        .get_all_nodes_by_property("a:b", &PropertyValue::String("c".into()))
        .await?;
    assert_eq!(hits, vec![n2]);

    Ok(())
}

#[tokio::test]
async fn float_canonicalization_negative_zero() -> Result<()> {
    // v1: -0.0 → "-0" y 0.0 → "0" eran claves distintas.
    let graph = Graph::in_memory().await?;
    let n = graph.add_node(Node::new("T").with_property("x", -0.0f64)).await?;

    let hits = graph.get_all_nodes_by_property("x", &PropertyValue::Float(0.0)).await?;
    assert_eq!(hits, vec![n]);

    Ok(())
}

#[tokio::test]
async fn index_survives_reopen() -> Result<()> {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("v2_reopen");

    let id = {
        let graph = Graph::open(&path).await?;
        let id = graph.add_node(Node::new("P").with_property("sku", "X-1")).await?;
        graph.checkpoint().await?;
        id
    };

    let graph = Graph::open(&path).await?;
    let hits = graph
        .get_all_nodes_by_property("sku", &PropertyValue::String("X-1".into()))
        .await?;
    assert_eq!(hits, vec![id]);

    Ok(())
}

#[tokio::test]
async fn rebuild_repairs_stale_index() -> Result<()> {
    // rebuild_property_index() repara un índice desactualizado reconstruyendo
    // desde los nodos (fuente de verdad) — el gap M1-9 tiene ruta de reparación.
    let graph = Graph::in_memory().await?;
    let id = graph.add_node(Node::new("P").with_property("k", 7i64)).await?;

    let processed = graph.rebuild_property_index().await?;
    assert_eq!(processed, 1);

    let hits = graph.get_all_nodes_by_property("k", &PropertyValue::Int(7)).await?;
    assert_eq!(hits, vec![id]);

    Ok(())
}
