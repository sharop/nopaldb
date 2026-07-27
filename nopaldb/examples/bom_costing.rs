// Explosión de materiales (BOM) con acumulador por traverser.
//
// Costear una mesa de taller: la cantidad pedida baja por el árbol
// multiplicándose con la `cantidad` de cada arista. El mismo material puede
// entrar por varias ramas y cada camino conserva su propia cantidad.
//
//   cargo run -p nopaldb --example bom_costing

use nopaldb::{Edge, Graph, Node, PropertyValue};

fn nombre(n: &str) -> PropertyValue {
    PropertyValue::String(n.into())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let graph = Graph::in_memory().await?;

    let mesa = graph.add_node(Node::new("Producto").with_property("name", nombre("Mesa"))).await?;
    let tornillo = graph.add_node(Node::new("Material").with_property("name", nombre("Tornillo"))).await?;
    let barniz = graph.add_node(Node::new("Material").with_property("name", nombre("Barniz"))).await?;
    let pata = graph.add_node(Node::new("Material").with_property("name", nombre("Pata"))).await?;
    let regaton = graph.add_node(Node::new("Material").with_property("name", nombre("Regatón"))).await?;

    // Mesa ──ContieneComponente{8}──▶ Tornillo   (ensamble)
    //      ──ContieneComponente{2}──▶ Tornillo   (niveladores)
    //      ──ContieneComponente{1}──▶ Barniz
    //      ──ContieneComponente{4}──▶ Pata ──ContieneComponente{1}──▶ Regatón
    graph.add_edge(Edge::new(mesa, tornillo, "ContieneComponente").with_property("cantidad", PropertyValue::Int(8))).await?;
    graph.add_edge(Edge::new(mesa, tornillo, "ContieneComponente").with_property("cantidad", PropertyValue::Int(2))).await?;
    graph.add_edge(Edge::new(mesa, barniz, "ContieneComponente").with_property("cantidad", PropertyValue::Int(1))).await?;
    graph.add_edge(Edge::new(mesa, pata, "ContieneComponente").with_property("cantidad", PropertyValue::Int(4))).await?;
    graph.add_edge(Edge::new(pata, regaton, "ContieneComponente").with_property("cantidad", PropertyValue::Int(1))).await?;

    let lote = 10.0;

    let r = graph
        .traverse(mesa)
        .sack(lote)
        .repeat(|b| b.out_e("ContieneComponente").sack_mul_by("cantidad"))
        .emit()
        .await?;

    println!("Materiales para un lote de {lote} mesas:\n");
    for item in &r.items {
        let material = graph.get_node(item.node).await?;
        let name = material
            .properties
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("<sin nombre>");
        println!("  {:<10} {:>8}", name, item.sack);
    }

    // El recorrido distingue «terminé» de «me detuve por un tope»:
    assert!(r.is_complete());
    println!("\nRecorrido completo: {}", r.is_complete());

    Ok(())
}