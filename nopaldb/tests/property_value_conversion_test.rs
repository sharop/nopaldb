// Pruebas de integración del API de conversión unificada de PropertyValue:
// literales en with_property de punta a punta, roundtrip por storage e
// índice de propiedades, y matriz de inferencia de Into<PropertyValue>.

use nopaldb::{Edge, Graph, Node, PropertyValue, Result};

/// Matriz de compilación/inferencia: si un tipo deja de implementar
/// Into<PropertyValue>, esto no compila.
fn assert_into<T: Into<PropertyValue>>(v: T) -> PropertyValue {
    v.into()
}

#[test]
fn inference_matrix_compiles() {
    assert_eq!(assert_into(true), PropertyValue::Bool(true));
    assert_eq!(assert_into(18), PropertyValue::Int(18)); // literal i32
    assert_eq!(assert_into(18i64), PropertyValue::Int(18));
    assert_eq!(assert_into(7u32), PropertyValue::Int(7));
    assert_eq!(assert_into(2.5), PropertyValue::Float(2.5)); // literal f64
    assert_eq!(assert_into(2.5f32), PropertyValue::Float(2.5));
    assert_eq!(assert_into("a"), PropertyValue::String("a".into()));
    assert_eq!(assert_into(String::from("a")), PropertyValue::String("a".into()));
    assert_eq!(assert_into(vec![1u8, 2]), PropertyValue::Bytes(vec![1, 2]));
    assert_eq!(assert_into(Some("x")), PropertyValue::String("x".into()));
    assert_eq!(assert_into(None::<i64>), PropertyValue::Null);
    // PropertyValue directo sigue pasando (Into reflexivo) — los 943 call
    // sites existentes dependen de esto.
    assert_eq!(assert_into(PropertyValue::Int(1)), PropertyValue::Int(1));
}

#[tokio::test]
async fn literals_roundtrip_through_storage() -> Result<()> {
    let graph = Graph::in_memory().await?;

    // Nodo construido 100% con literales
    let node = Node::new("Producto")
        .with_property("name", "Mesa")
        .with_property("cantidad", 18)
        .with_property("precio", 2.5)
        .with_property("activo", true);
    let id = graph.add_node(node).await?;

    // Los tipos exactos sobreviven el viaje por storage
    let stored = graph.get_node(id).await?;
    assert_eq!(stored.properties["name"], PropertyValue::String("Mesa".into()));
    assert_eq!(stored.properties["cantidad"], PropertyValue::Int(18));
    assert_eq!(stored.properties["precio"], PropertyValue::Float(2.5));
    assert_eq!(stored.properties["activo"], PropertyValue::Bool(true));

    // Aristas igual
    let other = graph.add_node(Node::new("Material").with_property("name", "Pata")).await?;
    let eid = graph
        .add_edge(Edge::new(id, other, "ContieneComponente").with_property("cantidad", 4))
        .await?;
    let edge = graph.get_edge(eid).await?;
    assert_eq!(edge.properties["cantidad"], PropertyValue::Int(4));

    Ok(())
}

#[tokio::test]
async fn property_index_lookup_hits_after_literal_insert() -> Result<()> {
    // El índice de propiedades en disco se alimenta en add_node; un nodo
    // insertado con literales debe ser localizable por la búsqueda tipada.
    let graph = Graph::in_memory().await?;

    let node = Node::new("Producto").with_property("sku", "MESA-01").with_property("stock", 7);
    let id = graph.add_node(node).await?;

    let hits = graph
        .get_all_nodes_by_property("sku", &PropertyValue::String("MESA-01".into()))
        .await?;
    assert_eq!(hits, vec![id]);

    let hits = graph
        .get_all_nodes_by_property("stock", &PropertyValue::Int(7))
        .await?;
    assert_eq!(hits, vec![id]);

    Ok(())
}

#[tokio::test]
async fn nql_sees_literal_properties() -> Result<()> {
    let graph = Graph::in_memory().await?;
    graph
        .add_node(Node::new("P").with_property("name", "Ana").with_property("edad", 30))
        .await?;

    let result = graph.execute_nql("find p.name, p.edad from (p:P) where p.edad = 30").await?;
    assert_eq!(result.rows().len(), 1);
    assert_eq!(result.rows()[0].get("p.name"), Some(&PropertyValue::String("Ana".into())));
    assert_eq!(result.rows()[0].get("p.edad"), Some(&PropertyValue::Int(30)));

    Ok(())
}
