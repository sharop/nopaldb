// examples/skill_tree.rs
//
// 🌳 Skill Tree System Demo
//
// Demonstrates:
// - Hierarchical skill dependencies
// - Prerequisite checking
// - Player progression tracking
// - Unlockable abilities

use nopaldb::{Graph, Node, Edge, PropertyValue, Result};

#[tokio::main]
async fn main() -> Result<()> {
    println!("\n╔═══════════════════════════════════════╗");
    println!("║  🌳 Skill Tree System Demo           ║");
    println!("║     NopalDB Graph Database            ║");
    println!("╚═══════════════════════════════════════╝\n");

    let graph = Graph::open("./examples/skill_tree.db").await?;

    println!("📊 Creating skill tree...\n");

    // ═════════════════════════════════════════════════════════
    // CREATE SKILLS - Warrior Tree
    // ═════════════════════════════════════════════════════════

    // Tier 1: Basic Skills
    let basic_attack = create_skill(
        &graph,
        "basic_attack",
        "Basic Attack",
        "Learn fundamental combat techniques",
        1,
        0,
        "physical"
    ).await?;

    let armor_training = create_skill(
        &graph,
        "armor_training",
        "Armor Training",
        "Wear heavier armor",
        1,
        0,
        "defense"
    ).await?;

    // Tier 2: Intermediate Skills
    let power_strike = create_skill(
        &graph,
        "power_strike",
        "Power Strike",
        "Deal massive damage with a single blow",
        2,
        1,
        "physical"
    ).await?;

    let shield_mastery = create_skill(
        &graph,
        "shield_mastery",
        "Shield Mastery",
        "Improved blocking and shield bash",
        2,
        1,
        "defense"
    ).await?;

    // Tier 3: Advanced Skills
    let whirlwind = create_skill(
        &graph,
        "whirlwind",
        "Whirlwind",
        "Spin and attack all nearby enemies",
        3,
        2,
        "physical"
    ).await?;

    let battle_cry = create_skill(
        &graph,
        "battle_cry",
        "Battle Cry",
        "Boost allies and intimidate enemies",
        3,
        2,
        "support"
    ).await?;

    // Tier 4: Master Skills
    let berserker_rage = create_skill(
        &graph,
        "berserker_rage",
        "Berserker Rage",
        "Enter a powerful rage state",
        4,
        3,
        "physical"
    ).await?;

    let last_stand = create_skill(
        &graph,
        "last_stand",
        "Last Stand",
        "Become invulnerable when near death",
        4,
        3,
        "defense"
    ).await?;

    println!("✅ Created 8 skills\n");

    // ═════════════════════════════════════════════════════════
    // CREATE SKILL DEPENDENCIES
    // ═════════════════════════════════════════════════════════

    let mut tx = graph.begin_transaction().await?;

    // Tier 1 → Tier 2
    tx.add_edge(Edge::new(power_strike, basic_attack, "REQUIRES"))?;
    tx.add_edge(Edge::new(shield_mastery, armor_training, "REQUIRES"))?;

    // Tier 2 → Tier 3
    tx.add_edge(Edge::new(whirlwind, power_strike, "REQUIRES"))?;
    tx.add_edge(Edge::new(battle_cry, power_strike, "REQUIRES"))?;
    tx.add_edge(Edge::new(battle_cry, shield_mastery, "REQUIRES"))?; // Hybrid

    // Tier 3 → Tier 4
    tx.add_edge(Edge::new(berserker_rage, whirlwind, "REQUIRES"))?;
    tx.add_edge(Edge::new(berserker_rage, battle_cry, "REQUIRES"))?;
    tx.add_edge(Edge::new(last_stand, shield_mastery, "REQUIRES"))?;

    tx.commit().await?;

    println!("✅ Created skill dependencies\n");

    // ═════════════════════════════════════════════════════════
    // CREATE PLAYER
    // ═════════════════════════════════════════════════════════

    let player = create_player(&graph, "Sharop", 1, 5).await?;

    // Player starts with basic skills
    let mut tx = graph.begin_transaction().await?;
    tx.add_edge(Edge::new(player, basic_attack, "LEARNED"))?;
    tx.add_edge(Edge::new(player, armor_training, "LEARNED"))?;
    tx.commit().await?;

    println!("✅ Created player with 2 basic skills\n");

    // ═════════════════════════════════════════════════════════
    // QUERY 1: Player's Current Skills
    // ═════════════════════════════════════════════════════════

    println!("═══════════════════════════════════════");
    println!("📚 LEARNED SKILLS");
    println!("═══════════════════════════════════════\n");

    let learned = graph.execute_nql(r#"
        find s.name, s.description, s.tier, s.category
        from (p:Player) -> [:LEARNED] -> (s:Skill)
        where p.name = "Sharop"
    "#).await?;

    for row in learned.rows() {
        let tier = row.get_int("s.tier").unwrap_or(0);
        let name = row.get_string("s.name").unwrap_or("?".into());
        let category = row.get_string("s.category").unwrap_or("?".into());
        let desc = row.get_string("s.description").unwrap_or("?".into());

        println!("  [Tier {}] {} ({})", tier, name, category);
        println!("           {}", desc);
        println!();
    }

    // ═════════════════════════════════════════════════════════
    // QUERY 2: Available Skills to Learn
    // ═════════════════════════════════════════════════════════

    println!("═══════════════════════════════════════");
    println!("🔓 AVAILABLE TO LEARN");
    println!("═══════════════════════════════════════\n");

    // Find skills that:
    // 1. Player hasn't learned
    // 2. Prerequisites are met
    let available = graph.execute_nql(r#"
        find s.name, s.tier, s.cost, req.name
        from (s:Skill) -> [:REQUIRES] -> (req:Skill),
             (p:Player) -> [:LEARNED] -> (req)
        where p.name = "Sharop"
    "#).await?;

    println!("  Skills you can unlock:\n");
    for row in available.rows() {
        let name = row.get_string("s.name").unwrap_or("?".into());
        let tier = row.get_int("s.tier").unwrap_or(0);
        let cost = row.get_int("s.cost").unwrap_or(0);
        let req = row.get_string("req.name").unwrap_or("?".into());

        println!("  🎯 {} (Tier {})", name, tier);
        println!("     💰 Cost: {} skill points", cost);
        println!("     ✓ Requires: {}", req);
        println!();
    }

    // ═════════════════════════════════════════════════════════
    // QUERY 3: Full Skill Tree Structure
    // ═════════════════════════════════════════════════════════

    println!("═══════════════════════════════════════");
    println!("🌳 SKILL TREE STRUCTURE");
    println!("═══════════════════════════════════════\n");

    let tree = graph.execute_nql(r#"
        find s1.name, s1.tier, s2.name
        from (s1:Skill) -> [:REQUIRES] -> (s2:Skill)
    "#).await?;

    let mut by_tier: std::collections::HashMap<i64, Vec<String>> =
        std::collections::HashMap::new();

    for row in tree.rows() {
        let s1 = row.get_string("s1.name").unwrap_or("?".into());
        let s2 = row.get_string("s2.name").unwrap_or("?".into());
        let tier = row.get_int("s1.tier").unwrap_or(0);

        by_tier.entry(tier)
            .or_insert_with(Vec::new)
            .push(format!("{} ← {}", s1, s2));
    }

    for tier in 1..=4 {
        if let Some(skills) = by_tier.get(&tier) {
            println!("  Tier {}:", tier);
            for skill in skills {
                println!("    • {}", skill);
            }
            println!();
        }
    }

    // ═════════════════════════════════════════════════════════
    // SIMULATE: Learn Power Strike
    // ═════════════════════════════════════════════════════════

    println!("═══════════════════════════════════════");
    println!("🎮 LEARNING NEW SKILL");
    println!("═══════════════════════════════════════\n");

    println!("▶️  Player learns: Power Strike");

    let mut tx = graph.begin_transaction().await?;
    tx.add_edge(Edge::new(player, power_strike, "LEARNED"))?;
    tx.commit().await?;

    println!("✅ Skill learned! -1 skill point\n");

    // Check what's newly available
    let newly_available = graph.execute_nql(r#"
        find s.name, s.tier
        from (s:Skill) -> [:REQUIRES] -> (req:Skill),
             (p:Player) -> [:LEARNED] -> (req)
        where p.name = "Sharop"
    "#).await?;

    println!("🔓 Newly unlocked:\n");
    for row in newly_available.rows() {
        let name = row.get_string("s.name").unwrap_or("?".into());
        let tier = row.get_int("s.tier").unwrap_or(0);
        println!("  • {} (Tier {})", name, tier);
    }

    println!("\n═══════════════════════════════════════");
    println!("✨ Demo Complete!");
    println!("═══════════════════════════════════════\n");

    Ok(())
}

// ═════════════════════════════════════════════════════════
// HELPER FUNCTIONS
// ═════════════════════════════════════════════════════════

async fn create_skill(
    graph: &Graph,
    id: &str,
    name: &str,
    description: &str,
    tier: i64,
    cost: i64,
    category: &str,
) -> Result<nopaldb::NodeId> {
    let mut tx = graph.begin_transaction().await?;

    let skill = Node::new("Skill")
        .with_property("id", PropertyValue::String(id.into()))
        .with_property("name", PropertyValue::String(name.into()))
        .with_property("description", PropertyValue::String(description.into()))
        .with_property("tier", PropertyValue::Int(tier))
        .with_property("cost", PropertyValue::Int(cost))
        .with_property("category", PropertyValue::String(category.into()));

    let node_id = tx.add_node(skill).await?;
    tx.commit().await?;

    Ok(node_id)
}

async fn create_player(
    graph: &Graph,
    name: &str,
    level: i64,
    skill_points: i64,
) -> Result<nopaldb::NodeId> {
    let mut tx = graph.begin_transaction().await?;

    let player = Node::new("Player")
        .with_property("name", PropertyValue::String(name.into()))
        .with_property("level", PropertyValue::Int(level))
        .with_property("skill_points", PropertyValue::Int(skill_points));

    let node_id = tx.add_node(player).await?;
    tx.commit().await?;

    Ok(node_id)
}