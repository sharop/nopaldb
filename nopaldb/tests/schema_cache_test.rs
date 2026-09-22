//! El caché del esquema se invalida al escribir (0.6.6). Desde 0.4.27
//! `SchemaManager.dirty` nacía en `true` y nada lo volvía a marcar: la
//! primera lectura de `get_schema` / `get_labels` / `get_label_count` /
//! `get_stats` por handle quedaba congelada para siempre. Aquí se afirma que
//! cada camino de escritura (directo, transaccional, bulk, borrado y
//! sobrescritura) se refleja en la siguiente lectura, también en un clone.

use nopaldb::{Edge, Graph, Node, PropertyValue};

async fn labels(g: &Graph) -> Vec<String> {
    let mut l = g.get_labels().await.unwrap();
    l.sort();
    l
}

#[tokio::test]
async fn every_write_path_shows_up_in_the_next_schema_read() {
    let g = Graph::in_memory().await.unwrap();
    assert_eq!(g.get_stats().await.unwrap().total_nodes, 0, "first read on an empty graph");

    // Escritura directa.
    let a = g.add_node(Node::new("Planta").with_property("n", PropertyValue::Int(1))).await.unwrap();
    assert_eq!(g.get_label_count("Planta").await.unwrap(), 1);
    assert_eq!(labels(&g).await, vec!["Planta"]);

    // Commit transaccional, con arista.
    let mut tx = g.begin_transaction().await.unwrap();
    let b = tx.add_node(Node::new("Riego")).await.unwrap();
    tx.add_edge(Edge::new(a, b, "ALIMENTA")).unwrap();
    tx.commit().await.unwrap();
    let s = g.get_stats().await.unwrap();
    assert_eq!((s.total_nodes, s.total_edges), (2, 1));
    assert_eq!(labels(&g).await, vec!["Planta", "Riego"]);
    assert_eq!(g.get_edge_type_count("ALIMENTA").await.unwrap(), 1);

    // Bulk loader.
    let mut loader = g.bulk_loader(2);
    for i in 0..5 {
        loader.add_node(Node::new("Bulk").with_property("i", PropertyValue::Int(i))).await.unwrap();
    }
    loader.finish().await.unwrap();
    assert_eq!(g.get_label_count("Bulk").await.unwrap(), 5);
    assert_eq!(g.get_stats().await.unwrap().total_nodes, 7);

    // Borrado.
    g.delete_node(b).await.unwrap();
    let s = g.get_stats().await.unwrap();
    assert_eq!((s.total_nodes, s.total_edges), (6, 0), "the node and its edge are gone");
    assert_eq!(labels(&g).await, vec!["Bulk", "Planta"]);

    // Sobrescritura: mismo id, propiedad nueva → el esquema la lista.
    let mut overwrite = Node::new("Planta")
        .with_property("n", PropertyValue::Int(1))
        .with_property("nuevo", PropertyValue::Bool(true));
    overwrite.id = a;
    g.add_node(overwrite).await.unwrap();
    let props = g.get_label_properties("Planta").await.unwrap();
    assert!(props.contains(&"nuevo".to_string()), "{props:?}");

    // Un clone del handle ve lo mismo (el caché es compartido).
    let g2 = g.clone();
    g2.add_node(Node::new("Clon")).await.unwrap();
    assert_eq!(g.get_label_count("Clon").await.unwrap(), 1);
    assert_eq!(g2.get_stats().await.unwrap().total_nodes, 7);

    // Y node_count/edge_count (claves, sin esquema) coinciden.
    assert_eq!(g.node_count().await.unwrap(), 7);
    assert_eq!(g.edge_count().await.unwrap(), 0);
}
