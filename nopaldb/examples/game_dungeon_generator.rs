// examples/game_dungeon_generator.rs
//
// Procedurally generate dungeons using graph structure
// Rooms as nodes, corridors as edges

use nopaldb::{Edge, Graph, Node, PropertyValue};
use rand::Rng;

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;

    println!("🏰 Generating Procedural Dungeon...\n");

    // Generate dungeon
    let entrance = generate_dungeon(&graph, 10).await?;

    // Visualize
    println!("📍 Entrance Room:");
    print_room(&graph, entrance).await?;

    // Find treasure room (furthest from entrance)
    let treasure = find_treasure_room(&graph, entrance).await?;
    println!("\n💎 Treasure Room:");
    print_room(&graph, treasure).await?;

    // Find path
    println!("\n🗺️  Path to Treasure:");
    let path = graph
        .shortest_path(entrance, treasure, nopaldb::TraversalConfig::new())
        .await?
        .unwrap();

    println!("  {} rooms to traverse", path.nodes.len());

    Ok(())
}

async fn generate_dungeon(graph: &Graph, num_rooms: usize) -> nopaldb::Result<uuid::Uuid> {
    let mut rng = rand::thread_rng();
    let mut rooms = Vec::new();

    // Create rooms
    for i in 0..num_rooms {
        let room_type = match rng.gen_range(0..100) {
            0..=60 => "Normal",
            61..=80 => "Monster",
            81..=95 => "Loot",
            _ => "Boss",
        };

        let room = graph
            .add_node(
                Node::new("Room")
                    .with_property("id", PropertyValue::Int(i as i64))
                    .with_property("type", PropertyValue::String(room_type.into()))
                    .with_property("danger", PropertyValue::Int(rng.gen_range(1..10))),
            )
            .await?;

        rooms.push(room);
    }

    // Connect rooms randomly
    for i in 0..num_rooms - 1 {
        // Connect to next room
        graph
            .add_edge(Edge::new(rooms[i], rooms[i + 1], "CORRIDOR"))
            .await?;

        // Random extra connections
        if rng.gen_bool(0.3) && i + 2 < num_rooms {
            graph
                .add_edge(Edge::new(rooms[i], rooms[i + 2], "CORRIDOR"))
                .await?;
        }
    }

    println!("  ✓ Generated {} rooms", num_rooms);
    println!("  ✓ Connected with corridors");

    Ok(rooms[0]) // Return entrance
}

async fn print_room(graph: &Graph, room_id: uuid::Uuid) -> nopaldb::Result<()> {
    let room = graph.get_node(room_id).await?;
    let room_type = room.properties.get("type").unwrap();
    let danger = room.properties.get("danger").unwrap();

    println!("  Type: {:?}", room_type);
    println!("  Danger Level: {:?}", danger);

    Ok(())
}

async fn find_treasure_room(graph: &Graph, entrance: uuid::Uuid) -> nopaldb::Result<uuid::Uuid> {
    // Use BFS to find furthest loot room
    let result = graph.bfs(entrance, nopaldb::TraversalConfig::new()).await?;

    // Find last loot room in traversal
    for &node_id in result.nodes.iter().rev() {
        let node = graph.get_node(node_id).await?;
        if let Some(PropertyValue::String(t)) = node.properties.get("type")
            && (t == "Loot" || t == "Boss") {
            return Ok(node_id);
        }
    }

    Ok(result.nodes.last().copied().unwrap())
}
