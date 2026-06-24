use nopaldb::{Edge, Graph, Node, PropertyValue};

#[tokio::main]

async fn main() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;

    let alice = graph
        .add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String("Alice".into()))
                .with_property("city", PropertyValue::String("CDMX".into()))
                .with_property("age", PropertyValue::Int(30))
                .with_property("interests", PropertyValue::String("Rust".into())),
        )
        .await?;

    let bob = graph
        .add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String("Bob".into()))
                .with_property("city", PropertyValue::String("CDMX".into()))
                .with_property("age", PropertyValue::Int(30))
                .with_property("interests", PropertyValue::String("Python,ML".into())),
        )
        .await?;

    let charlie = graph
        .add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String("Charlie".into()))
                .with_property("city", PropertyValue::String("GDL".into()))
                .with_property("age", PropertyValue::Int(35))
                .with_property("interests", PropertyValue::String("Rust,Systems".into())),
        )
        .await?;

    let diana = graph
        .add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String("Diana".into()))
                .with_property("city", PropertyValue::String("CDMX".into()))
                .with_property("age", PropertyValue::Int(28))
                .with_property("interests", PropertyValue::String("Databases".into())),
        )
        .await?;

    let eve = graph
        .add_node(
            Node::new("Person")
                .with_property("name", PropertyValue::String("Eve".into()))
                .with_property("city", PropertyValue::String("MTY".into()))
                .with_property("age", PropertyValue::Int(32))
                .with_property("interests", PropertyValue::String("Rust,Security".into())),
        )
        .await?;

    // === Crear lugares ===
    let cdmx = graph
        .add_node(Node::new("City").with_property("name", PropertyValue::String("CDMX".into())))
        .await?;

    let gdl = graph
        .add_node(
            Node::new("City").with_property("name", PropertyValue::String("Guadalajara".into())),
        )
        .await?;

    // === Relaciones sociales ===
    graph
        .add_edge(Edge::new(alice, bob, "KNOWS").with_property("since", PropertyValue::Int(2020)))
        .await?;

    graph
        .add_edge(
            Edge::new(alice, charlie, "KNOWS").with_property("since", PropertyValue::Int(2019)),
        )
        .await?;

    graph
        .add_edge(Edge::new(bob, diana, "KNOWS").with_property("since", PropertyValue::Int(2021)))
        .await?;

    graph
        .add_edge(Edge::new(charlie, eve, "KNOWS").with_property("since", PropertyValue::Int(2018)))
        .await?;

    graph
        .add_edge(Edge::new(alice, diana, "WORKS_WITH"))
        .await?;

    // === Relaciones con ciudades ===
    graph.add_edge(Edge::new(alice, cdmx, "LIVES_IN")).await?;
    graph.add_edge(Edge::new(bob, cdmx, "LIVES_IN")).await?;
    graph.add_edge(Edge::new(charlie, gdl, "LIVES_IN")).await?;
    graph.add_edge(Edge::new(diana, cdmx, "LIVES_IN")).await?;
    graph.add_edge(Edge::new(eve, gdl, "LIVES_IN")).await?;

    // === Query 1: Amigos directos de Alice ===
    println!("1. Amigos directos de Alice:");
    let friends = graph.traverse(alice).out_e("KNOWS").nodes().await?;

    for friend in &friends {
        if let Some(PropertyValue::String(name)) = friend.properties.get("name") {
            println!("   - {}", name);
        }
    }

    // === Query 2: Friends-of-friends (2 saltos) ===
    println!("\n2. Friends-of-friends de Alice:");
    let fof = graph
        .traverse(alice)
        .out_e("KNOWS") // Amigos directos
        .out_e("KNOWS") // Amigos de amigos
        .nodes()
        .await?;

    for person in &fof {
        if let Some(PropertyValue::String(name)) = person.properties.get("name") {
            println!("   - {}", name);
        }
    }

    // === Query 3: Personas que viven en CDMX ===
    println!("\n3. Amigos de Alice que viven en CDMX:");
    let cdmx_friends = graph
        .traverse(alice)
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

    for friend in &cdmx_friends {
        if let Some(PropertyValue::String(name)) = friend.properties.get("name") {
            println!("   - {}", name);
        }
    }

    // === Query 4: Personas mayores de 27 años ===
    println!("\n4. Amigos de Alice > 27 años:");
    let older_friends = graph
        .traverse(alice)
        .out_e("KNOWS")
        .filter(|node| {
            matches!(
                node.properties.get("age"),
                Some(PropertyValue::Int(age)) if *age > 27
            )
        })
        .nodes()
        .await?;

    for friend in &older_friends {
        if let Some(PropertyValue::String(name)) = friend.properties.get("name")
            && let Some(PropertyValue::Int(age)) = friend.properties.get("age") {
            println!("   - {} ({} años)", name, age);
        }
    }

    // === Query 5: Personas que les gusta Rust ===
    println!("\n5. Red extendida de Alice que le gusta Rust:");
    let rust_lovers = graph
        .traverse(alice)
        .out_e("KNOWS")
        .out_e("KNOWS")
        .has_label("Person")
        .filter(|node| {
            matches!(
                node.properties.get("interests"),
                Some(PropertyValue::String(interests)) if interests.contains("Rust")
            )
        })
        .nodes()
        .await?;

    for person in &rust_lovers {
        if let Some(PropertyValue::String(name)) = person.properties.get("name")
            && let Some(PropertyValue::String(interests)) = person.properties.get("interests") {
            println!("   - {} (intereses: {})", name, interests);
        }
    }

    // === Query 6: Conteo con limit ===
    println!("\n6. Primeras 2 personas en la red de Alice:");
    let limited = graph.traverse(alice).out().limit(2).nodes().await?;

    for person in &limited {
        if let Some(PropertyValue::String(name)) = person.properties.get("name") {
            println!("   - {}", name);
        }
    }

    // === Query 7: Solo contar ===
    println!("\n7. Total de conexiones directas de Alice:");
    let count = graph.traverse(alice).out().count().await?;
    println!("   {} conexiones", count);

    Ok(())
}
