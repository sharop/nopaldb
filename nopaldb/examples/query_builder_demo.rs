use nopaldb::{Graph, Node, Edge, PropertyValue};

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;
    
    // Crear red social
    let alice = graph.add_node(Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()))
        .with_property("city", PropertyValue::String("CDMX".into()))
        .with_property("age", PropertyValue::Int(30))
    ).await?;
    
    let bob = graph.add_node(Node::new("Person")
        .with_property("name", PropertyValue::String("Bob".into()))
        .with_property("city", PropertyValue::String("CDMX".into()))
        .with_property("age", PropertyValue::Int(25))
    ).await?;
    
    let charlie = graph.add_node(Node::new("Person")
        .with_property("name", PropertyValue::String("Charlie".into()))
        .with_property("city", PropertyValue::String("GDL".into()))
        .with_property("age", PropertyValue::Int(35))
    ).await?;
    
    graph.add_edge(Edge::new(alice, bob, "KNOWS")).await?;
    graph.add_edge(Edge::new(alice, charlie, "KNOWS")).await?;
    
    // Query: Amigos de Alice que viven en CDMX
    let cdmx_friends = graph.traverse(alice)
        .out_e("KNOWS")
        .has_label("Person")
        .filter(|node| {
            matches!(
                node.properties.get("city"),
                Some(PropertyValue::String(city)) if city == "CDMX"
            )
        })
        .nodes()
        .await?;
    
    println!("Amigos de Alice en CDMX:");
    for friend in cdmx_friends {
        if let Some(PropertyValue::String(name)) = friend.properties.get("name") {
            println!("  - {}", name);
        }
    }
    
    // Query: Contar amigos de Alice mayores de 26
    let count = graph.traverse(alice)
        .out_e("KNOWS")
        .filter(|node| {
            matches!(
                node.properties.get("age"),
                Some(PropertyValue::Int(age)) if *age > 26
            )
        })
        .count()
        .await?;
    
    println!("\nAmigos de Alice > 26 años: {}", count);
    
    Ok(())
}