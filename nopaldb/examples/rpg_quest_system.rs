// examples/rpg_quest_system.rs
//
// 🎮 RPG Quest System Demo
//
// Demonstrates:
// - Quest dependencies (prerequisites)
// - Player progression tracking
// - Dynamic quest availability
// - Pattern matching queries

use nopaldb::{Edge, Graph, Node, PropertyValue, Result};

#[tokio::main]
async fn main() -> Result<()> {
    println!("\n╔═══════════════════════════════════════╗");
    println!("║  🎮 RPG Quest System Demo            ║");
    println!("║     NopalDB Graph Database            ║");
    println!("╚═══════════════════════════════════════╝\n");

    // Open/create database
    let graph = Graph::open("./examples/rpg_game.db").await?;

    println!("📊 Creating game world...\n");

    // ═════════════════════════════════════════════════════════
    // CREATE QUESTS
    // ═════════════════════════════════════════════════════════

    let tutorial = create_quest(
        &graph,
        "tutorial",
        "Welcome to the Adventure",
        "Learn the basic controls and mechanics",
        10,
        1,
        "available",
    )
    .await?;

    let goblin_camp = create_quest(
        &graph,
        "goblin_camp",
        "Clear the Goblin Camp",
        "Defeat the goblins threatening the village",
        50,
        3,
        "locked",
    )
    .await?;

    let forest_guardian = create_quest(
        &graph,
        "forest_guardian",
        "Defeat the Forest Guardian",
        "Prove your worth against the ancient protector",
        100,
        5,
        "locked",
    )
    .await?;

    let ancient_ruins = create_quest(
        &graph,
        "ancient_ruins",
        "Explore the Ancient Ruins",
        "Uncover the secrets of the lost civilization",
        200,
        8,
        "locked",
    )
    .await?;

    let dragon_slayer = create_quest(
        &graph,
        "dragon_slayer",
        "Slay the Ancient Dragon",
        "Face the ultimate challenge and save the kingdom",
        1000,
        15,
        "locked",
    )
    .await?;

    println!("✅ Created 5 quests\n");

    // ═════════════════════════════════════════════════════════
    // CREATE QUEST DEPENDENCIES
    // ═════════════════════════════════════════════════════════

    let mut tx = graph.begin_transaction().await?;

    // Linear progression
    tx.add_edge(Edge::new(goblin_camp, tutorial, "REQUIRES"))?;
    tx.add_edge(Edge::new(forest_guardian, goblin_camp, "REQUIRES"))?;
    tx.add_edge(Edge::new(ancient_ruins, forest_guardian, "REQUIRES"))?;
    tx.add_edge(Edge::new(dragon_slayer, ancient_ruins, "REQUIRES"))?;

    // Unlock relationships
    tx.add_edge(Edge::new(tutorial, goblin_camp, "UNLOCKS"))?;
    tx.add_edge(Edge::new(goblin_camp, forest_guardian, "UNLOCKS"))?;
    tx.add_edge(Edge::new(forest_guardian, ancient_ruins, "UNLOCKS"))?;
    tx.add_edge(Edge::new(ancient_ruins, dragon_slayer, "UNLOCKS"))?;

    tx.commit().await?;

    println!("✅ Created quest dependencies\n");

    // ═════════════════════════════════════════════════════════
    // CREATE PLAYER
    // ═════════════════════════════════════════════════════════

    let player = create_player(&graph, "Demo Player", 1, 0, "Village Square").await?;

    println!("✅ Created player: Demo Player\n");

    // ═════════════════════════════════════════════════════════
    // QUERY 1: Available Quests
    // ═════════════════════════════════════════════════════════

    println!("═══════════════════════════════════════");
    println!("📋 AVAILABLE QUESTS (Level 1)");
    println!("═══════════════════════════════════════\n");

    let available = graph
        .execute_nql(
            r#"
        find q.name, q.description, q.xp_reward, q.level_required
        from (q:Quest)
        where q.status = "available"
          and q.level_required <= 1
        "#,
        )
        .await?;

    for row in available.rows() {
        println!("🎯 {}", row.get_string("q.name").unwrap_or("?".into()));
        println!(
            "   📖 {}",
            row.get_string("q.description").unwrap_or("?".into())
        );
        println!(
            "   ⭐ Reward: {} XP",
            row.get_int("q.xp_reward").unwrap_or(0)
        );
        println!(
            "   📊 Required Level: {}",
            row.get_int("q.level_required").unwrap_or(0)
        );
        println!();
    }

    // ═════════════════════════════════════════════════════════
    // QUERY 2: Quest Chain
    // ═════════════════════════════════════════════════════════

    println!("═══════════════════════════════════════");
    println!("🔗 QUEST CHAIN (Prerequisites)");
    println!("═══════════════════════════════════════\n");

    let chain = graph
        .execute_nql(
            r#"
        find q1.name, q2.name
        from (q1:Quest) -> [:REQUIRES] -> (q2:Quest)
    "#,
        )
        .await?;

    for row in chain.rows() {
        let q1 = row.get_string("q1.name").unwrap_or("?".into());
        let q2 = row.get_string("q2.name").unwrap_or("?".into());
        println!("  ➡️  {} requires {}", q1, q2);
    }

    println!();

    // ═════════════════════════════════════════════════════════
    // QUERY 3: Quest Progression Tree
    // ═════════════════════════════════════════════════════════

    println!("═══════════════════════════════════════");
    println!("🌳 PROGRESSION TREE");
    println!("═══════════════════════════════════════\n");

    let progression = graph
        .execute_nql(
            r#"
        find q1.name, q2.name, q1.level_required
        from (q1:Quest) -> [:UNLOCKS] -> (q2:Quest)
    "#,
        )
        .await?;

    for row in progression.rows() {
        let q1 = row.get_string("q1.name").unwrap_or("?".into());
        let q2 = row.get_string("q2.name").unwrap_or("?".into());
        let lvl = row.get_int("q1.level_required").unwrap_or(0);
        println!("  [Lvl {}] {} → unlocks → {}", lvl, q1, q2);
    }

    println!();

    // ═════════════════════════════════════════════════════════
    // SIMULATE: Complete Tutorial Quest
    // ═════════════════════════════════════════════════════════

    println!("═══════════════════════════════════════");
    println!("🎮 SIMULATING GAMEPLAY");
    println!("═══════════════════════════════════════\n");

    println!("▶️  Player completes: Welcome to the Adventure");

    // Mark tutorial as completed
    let mut tx = graph.begin_transaction().await?;
    tx.add_edge(Edge::new(player, tutorial, "COMPLETED"))?;
    tx.commit().await?;

    // Update goblin quest to available
    // TODO: This would be done via UPDATE query once implemented
    println!("✅ Quest completed! +10 XP");
    println!("🔓 New quest unlocked: Clear the Goblin Camp\n");

    // ═════════════════════════════════════════════════════════
    // QUERY 4: Player's Completed Quests
    // ═════════════════════════════════════════════════════════

    println!("═══════════════════════════════════════");
    println!("🏆 PLAYER ACHIEVEMENTS");
    println!("═══════════════════════════════════════\n");

    let completed = graph
        .execute_nql(
            r#"
        find q.name, q.xp_reward
        from (p:Player) -> [:COMPLETED] -> (q:Quest)
        where p.name = "Demo Player"
    "#,
        )
        .await?;

    let mut total_xp = 0;
    for row in completed.rows() {
        let name = row.get_string("q.name").unwrap_or("?".into());
        let xp = row.get_int("q.xp_reward").unwrap_or(0);
        total_xp += xp;
        println!("  ✅ {} (+{} XP)", name, xp);
    }

    println!("\n  📊 Total XP: {}", total_xp);

    println!("\n═══════════════════════════════════════");
    println!("✨ Demo Complete!");
    println!("═══════════════════════════════════════\n");

    Ok(())
}

// ═════════════════════════════════════════════════════════
// HELPER FUNCTIONS
// ═════════════════════════════════════════════════════════

async fn create_quest(
    graph: &Graph,
    id: &str,
    name: &str,
    description: &str,
    xp: i64,
    level: i64,
    status: &str,
) -> Result<nopaldb::NodeId> {
    let mut tx = graph.begin_transaction().await?;

    let quest = Node::new("Quest")
        .with_property("id", PropertyValue::String(id.into()))
        .with_property("name", PropertyValue::String(name.into()))
        .with_property("description", PropertyValue::String(description.into()))
        .with_property("xp_reward", PropertyValue::Int(xp))
        .with_property("level_required", PropertyValue::Int(level))
        .with_property("status", PropertyValue::String(status.into()));

    let node_id = tx.add_node(quest).await?;
    tx.commit().await?;

    Ok(node_id)
}

async fn create_player(
    graph: &Graph,
    name: &str,
    level: i64,
    xp: i64,
    location: &str,
) -> Result<nopaldb::NodeId> {
    let mut tx = graph.begin_transaction().await?;

    let player = Node::new("Player")
        .with_property("name", PropertyValue::String(name.into()))
        .with_property("level", PropertyValue::Int(level))
        .with_property("xp", PropertyValue::Int(xp))
        .with_property("location", PropertyValue::String(location.into()));

    let node_id = tx.add_node(player).await?;
    tx.commit().await?;

    Ok(node_id)
}
