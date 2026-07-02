use nopaldb::{Graph, Node, Edge, PropertyValue, Direction};

#[tokio::main]
async fn main() {
    // Crear un nuevo grafo en memoria
    let graph = Graph::in_memory().await.unwrap();

    // Crear usuarios
    let alice = Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()))
        .with_property("age", PropertyValue::Int(30));
    
    let bob = Node::new("Person")
        .with_property("name", PropertyValue::String("Bob".into()))
        .with_property("age", PropertyValue::Int(25));
    
    let charlie = Node::new("Person")
        .with_property("name", PropertyValue::String("Charlie".into()))
        .with_property("age", PropertyValue::Int(35));
    
    let alice_id = graph.add_node(alice).await.unwrap();
    let bob_id = graph.add_node(bob).await.unwrap();
    let charlie_id = graph.add_node(charlie).await.unwrap();
    
    // Crear relaciones
    graph.add_edge(Edge::new(alice_id, bob_id, "KNOWS")
        .with_property("since", PropertyValue::Int(2020))
    ).await.unwrap();
    
    graph.add_edge(Edge::new(bob_id, charlie_id, "KNOWS")
        .with_property("since", PropertyValue::Int(2019))
    ).await.unwrap();
    
    graph.add_edge(Edge::new(alice_id, charlie_id, "WORKS_WITH")
    ).await.unwrap();
    
    // Queries
    println!("Amigos de Alice:");
    let friends = graph.neighbors(alice_id, Direction::Outgoing).await.unwrap();
    for friend_id in friends {
        let friend = graph.get_node(friend_id).await.unwrap();
        let name = friend.properties.get("name").unwrap();
        println!("  - {:?}", name);
    }
    
    println!("\nGrado de Bob: {}", graph.degree(bob_id, Direction::Both).await.unwrap());
}