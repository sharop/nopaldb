// examples/game_skill_tree.rs
//
// RPG Skill Tree with prerequisites

use nopaldb::{Direction, Edge, Graph, Node, PropertyValue};

#[tokio::main]
async fn main() -> nopaldb::Result<()> {
    let graph = Graph::in_memory().await?;

    println!("⚔️  Creating Skill Tree...\n");

    // Basic skills
    let basic_attack = create_skill(&graph, "Basic Attack", 1, 0, vec![]).await?;
    let basic_defense = create_skill(&graph, "Basic Defense", 1, 0, vec![]).await?;

    // Intermediate skills
    let power_strike = create_skill(&graph, "Power Strike", 5, 50, vec![basic_attack]).await?;
    let shield_bash = create_skill(&graph, "Shield Bash", 5, 50, vec![basic_defense]).await?;

    // Advanced skill (requires both)
    let _ultimate = create_skill(
        &graph,
        "Ultimate Combo",
        10,
        200,
        vec![power_strike, shield_bash]
    ).await?;

    // Check if player can learn skill
    let player_level = 6;
    let learned_skills = vec![basic_attack, basic_defense];

    println!("👤 Player Level: {}", player_level);
    println!("📚 Learned Skills: {}", learned_skills.len());

    can_learn_skill(&graph, power_strike, player_level, &learned_skills).await?;

    Ok(())
}

async fn create_skill(
    graph: &Graph,
    name: &str,
    level_required: i64,
    skill_points: i64,
    prerequisites: Vec<uuid::Uuid>,
) -> nopaldb::Result<uuid::Uuid> {
    let skill = graph.add_node(Node::new("Skill")
        .with_property("name", PropertyValue::String(name.into()))
        .with_property("level_required", PropertyValue::Int(level_required))
        .with_property("skill_points", PropertyValue::Int(skill_points))
    ).await?;

    for prereq in prerequisites {
        graph.add_edge(Edge::new(prereq, skill, "REQUIRES")).await?;
    }

    println!("  ✓ Created skill: {}", name);

    Ok(skill)
}

async fn can_learn_skill(
    graph: &Graph,
    skill_id: uuid::Uuid,
    player_level: i64,
    learned: &[uuid::Uuid],
) -> nopaldb::Result<bool> {
    let skill = graph.get_node(skill_id).await?;
    let name = skill.properties.get("name").unwrap();
    let required_level = skill.properties.get("level_required").unwrap();

    println!("\n🎯 Checking if can learn: {:?}", name);

    // Check level
    if let PropertyValue::Int(req) = required_level {
        if player_level < *req {
            println!("  ❌ Level too low (need {})", req);
            return Ok(false);
        }
    }

    // Check prerequisites
    let prereqs = graph.edges_of(skill_id, Direction::Incoming).await?;

    for edge in prereqs {
        if !learned.contains(&edge.source) {
            let prereq = graph.get_node(edge.source).await?;
            println!("  ❌ Missing prerequisite: {:?}",
                     prereq.properties.get("name").unwrap());
            return Ok(false);
        }
    }

    println!("  ✅ Can learn!");
    Ok(true)
}