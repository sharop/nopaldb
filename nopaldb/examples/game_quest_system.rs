// examples/game_quest_system.rs
//
// Graph-based Quest System for RPG
// Quests as nodes, dependencies as edges

use nopaldb::{Direction, Edge, Graph, Node, PropertyValue};

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;

    // ════════════════════════════════════════════════════
    // CREATE QUEST GRAPH
    // ════════════════════════════════════════════════════

    // Main Quest: Save the Kingdom
    let main_quest = graph
        .add_node(
            Node::new("Quest")
                .with_property("name", PropertyValue::String("Save the Kingdom".into()))
                .with_property("type", PropertyValue::String("Main".into()))
                .with_property("reward_xp", PropertyValue::Int(1000))
                .with_property("status", PropertyValue::String("locked".into())),
        )
        .await?;

    // Side Quest 1: Gather Herbs
    let gather_herbs = graph
        .add_node(
            Node::new("Quest")
                .with_property("name", PropertyValue::String("Gather Herbs".into()))
                .with_property("type", PropertyValue::String("Side".into()))
                .with_property("reward_xp", PropertyValue::Int(50))
                .with_property("status", PropertyValue::String("available".into())),
        )
        .await?;

    // Side Quest 2: Defeat Bandits
    let defeat_bandits = graph
        .add_node(
            Node::new("Quest")
                .with_property("name", PropertyValue::String("Defeat Bandits".into()))
                .with_property("type", PropertyValue::String("Side".into()))
                .with_property("reward_xp", PropertyValue::Int(100))
                .with_property("status", PropertyValue::String("locked".into())),
        )
        .await?;

    // Dependencies: Main quest requires side quests
    graph
        .add_edge(
            Edge::new(gather_herbs, main_quest, "UNLOCKS")
                .with_property("progress_required", PropertyValue::Int(100)),
        )
        .await?;

    graph
        .add_edge(
            Edge::new(defeat_bandits, main_quest, "UNLOCKS")
                .with_property("progress_required", PropertyValue::Int(100)),
        )
        .await?;

    // Chain: Gathering herbs unlocks bandit quest
    graph
        .add_edge(Edge::new(gather_herbs, defeat_bandits, "UNLOCKS"))
        .await?;

    println!("🎮 Quest System Initialized!");
    println!("\n📜 Available Quests:");

    // Find available quests
    let available = graph
        .find_nodes_by_property("status", &PropertyValue::String("available".into()))
        .await?;

    for quest_id in available {
        let quest = graph.get_node(quest_id).await?;
        println!("  ✓ {:?}", quest.properties.get("name").unwrap());
    }

    println!("\n🔒 Locked Quests:");
    let locked = graph
        .find_nodes_by_property("status", &PropertyValue::String("locked".into()))
        .await?;

    for quest_id in locked {
        let quest = graph.get_node(quest_id).await?;
        let name = quest.properties.get("name").unwrap();

        // Find dependencies

        let deps = graph.edges_of(quest_id, Direction::Incoming).await?;
        println!("  🔒 {:?}", name);
        println!("     Requires:");

        for edge in deps {
            let dep_quest = graph.get_node(edge.source).await?;
            if let Some(PropertyValue::String(dep_name)) = dep_quest.properties.get("name") {
                println!("       → {}", dep_name);
            }
        }
    }

    Ok(())
}
