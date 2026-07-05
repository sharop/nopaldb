use nopaldb::{Graph, Node, Edge, TraversalConfig, Direction};

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;
    
    // Crear grafo lineal: A -> B -> C -> D -> E
    let a = graph.add_node(Node::new("Node")).await?;
    let b = graph.add_node(Node::new("Node")).await?;
    let c = graph.add_node(Node::new("Node")).await?;
    let d = graph.add_node(Node::new("Node")).await?;
    let e = graph.add_node(Node::new("Node")).await?;
    
    graph.add_edge(Edge::new(a, b, "NEXT")).await?;
    graph.add_edge(Edge::new(b, c, "NEXT")).await?;
    graph.add_edge(Edge::new(c, d, "NEXT")).await?;
    graph.add_edge(Edge::new(d, e, "NEXT")).await?;
    
    println!("=== BFS vs Traverse Builder ===\n");
    
    // Método 1: BFS tradicional
    println!("1. Con BFS (max_depth=2):");
    let bfs_result = graph.bfs(
        a,
        TraversalConfig::new()
            .direction(Direction::Outgoing)
            .max_depth(2)
    ).await?;
    
    println!("   Nodos visitados: {}", bfs_result.nodes.len());
    if let Some(distances) = bfs_result.distances {
        for (i, dist) in distances.iter().enumerate() {
            println!("   Nodo {} - distancia: {}", i, dist);
        }
    }
    
    // Método 2: Traverse builder
    println!("\n2. Con Traverse Builder (2 saltos):");
    let traverse_result = graph.traverse(a)
        .out()  // Salto 1
        .out()  // Salto 2
        .count()
        .await?;
    
    println!("   Nodos al final: {}", traverse_result);
    
    // Método 3: Traverse con limit
    println!("\n3. Traverse con limit(3):");
    let limited_result = graph.traverse(a)
        .out()
        .out()
        .out()
        .limit(3)
        .count()
        .await?;
    
    println!("   Nodos (limitado): {}", limited_result);
    
    Ok(())
}