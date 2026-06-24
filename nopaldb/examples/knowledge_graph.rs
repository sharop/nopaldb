use nopaldb::{Edge, Graph, Node, PropertyValue};

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;

    // === Crear conceptos ===
    let rust = graph
        .add_node(
            Node::new("Language")
                .with_property("name", PropertyValue::String("Rust".into()))
                .with_property("year", PropertyValue::Int(2015)),
        )
        .await?;

    let python = graph
        .add_node(
            Node::new("Language")
                .with_property("name", PropertyValue::String("Python".into()))
                .with_property("year", PropertyValue::Int(1991)),
        )
        .await?;

    let systems_programming = graph
        .add_node(
            Node::new("Domain")
                .with_property("name", PropertyValue::String("Systems Programming".into())),
        )
        .await?;

    let web_dev = graph
        .add_node(
            Node::new("Domain")
                .with_property("name", PropertyValue::String("Web Development".into())),
        )
        .await?;

    let memory_safety = graph
        .add_node(
            Node::new("Concept")
                .with_property("name", PropertyValue::String("Memory Safety".into())),
        )
        .await?;

    // === Relaciones ===
    graph
        .add_edge(Edge::new(rust, systems_programming, "USED_IN"))
        .await?;
    graph.add_edge(Edge::new(rust, web_dev, "USED_IN")).await?;
    graph
        .add_edge(Edge::new(rust, memory_safety, "PROVIDES"))
        .await?;

    graph
        .add_edge(Edge::new(python, web_dev, "USED_IN"))
        .await?;

    // === Queries ===
    println!("=== Knowledge Graph Queries ===\n");

    // Query 1: ¿En qué dominios se usa Rust?
    println!("1. Dominios donde se usa Rust:");
    let domains = graph
        .traverse(rust)
        .out_e("USED_IN")
        .has_label("Domain")
        .nodes()
        .await?;

    for domain in &domains {
        if let Some(PropertyValue::String(name)) = domain.properties.get("name") {
            println!("   - {}", name);
        }
    }

    // Query 2: ¿Qué provee Rust?
    println!("\n2. Conceptos que provee Rust:");
    let concepts = graph.traverse(rust).out_e("PROVIDES").nodes().await?;

    for concept in &concepts {
        if let Some(PropertyValue::String(name)) = concept.properties.get("name") {
            println!("   - {}", name);
        }
    }

    // Query 3: Lenguajes para Web Development
    println!("\n3. Lenguajes para Web Development:");
    let web_langs = graph
        .traverse(web_dev)
        .in_e("USED_IN")
        .has_label("Language")
        .nodes()
        .await?;

    for lang in &web_langs {
        if let Some(PropertyValue::String(name)) = lang.properties.get("name")
            && let Some(PropertyValue::Int(year)) = lang.properties.get("year") {
            println!("   - {} (desde {})", name, year);
        }
    }

    Ok(())
}
