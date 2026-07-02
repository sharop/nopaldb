use nopaldb::{Graph, Node, Edge, TraversalConfig, Direction};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let graph = Graph::in_memory().await?;

    let node1 = graph.add_node(Node::new("Rosa")).await?;
    let node2 = graph.add_node(Node::new("Juan")).await?;
    let node3 = graph.add_node(Node::new("Alfredo")).await?;
    let node4 = graph.add_node(Node::new("Cristina")).await?;
    let node5 = graph.add_node(Node::new("Josefa")).await?;
    let node6 = graph.add_node(Node::new("Mario")).await.unwrap();
    
    graph.add_edge(Edge::new(node1, node2, "KNOWS")).await.unwrap();
    graph.add_edge(Edge::new(node1, node3, "KNOWS")).await.unwrap();
    graph.add_edge(Edge::new(node2, node4, "KNOWS")).await.unwrap();
    graph.add_edge(Edge::new(node2, node5, "KNOWS")).await.unwrap();
    graph.add_edge(Edge::new(node3, node6, "KNOWS")).await.unwrap();

    let result = graph.bfs(
        node3,
        TraversalConfig::new()
        .direction(Direction::Outgoing)
        .max_depth(2)
        .max_nodes(10)
    ).await?;

    println!("Nodos visitados: {:#?}", result.nodes.len());
    if let Some(distances) = &result.distances {
        for (node, dist) in result.nodes.iter().zip(distances) {
            let node_data = graph.get_node(*node).await?;
            println!("  {} (distancia: {})", node_data.label, dist);
        }
    }

    match graph.shortest_path(
        node3, 
        node6, 
        TraversalConfig::new().direction(Direction::Outgoing)
    ).await? {
            Some(path_result) => {
            println!("Camino encontrado ({} saltos):", path_result.nodes.len() - 1);
            for node_id in &path_result.nodes {
                let node = graph.get_node(*node_id).await?;
                println!("  -> {}", node.label);
            }
            },
        None => {
            println!("No se encontró camino entre Alfredo y Mario");
        }
    }
    Ok(())
}


    
    
    
