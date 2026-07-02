// examples/character_network.rs
//
// 🤝 Character Relationship Network Demo
//
// Demonstrates:
// - Social network analysis
// - Faction relationships
// - Alliance and rivalry tracking
// - Trade networks
// - Reputation system

use nopaldb::{Graph, Node, Edge, PropertyValue, Result};

#[tokio::main]
async fn main() -> Result<()> {
    println!("\n╔═══════════════════════════════════════╗");
    println!("║  🤝 Character Network Demo           ║");
    println!("║     NopalDB Graph Database            ║");
    println!("╚═══════════════════════════════════════╝\n");

    let graph = Graph::open("./examples/character_network.db").await?;

    println!("📊 Creating character network...\n");

    // ═════════════════════════════════════════════════════════
    // CREATE CHARACTERS
    // ═════════════════════════════════════════════════════════

    // Heroes
    let arthur = create_character(
        &graph,
        "Arthur",
        "Knight",
        "Kingdom",
        "lawful_good",
        100
    ).await?;

    let merlin = create_character(
        &graph,
        "Merlin",
        "Wizard",
        "Kingdom",
        "neutral_good",
        95
    ).await?;

    let lancelot = create_character(
        &graph,
        "Lancelot",
        "Knight",
        "Kingdom",
        "lawful_good",
        90
    ).await?;

    // Merchants
    let marcus = create_character(
        &graph,
        "Marcus",
        "Merchant",
        "Guild",
        "true_neutral",
        50
    ).await?;

    let elena = create_character(
        &graph,
        "Elena",
        "Trader",
        "Guild",
        "neutral_good",
        60
    ).await?;

    // Villains
    let mordred = create_character(
        &graph,
        "Mordred",
        "Dark Knight",
        "Empire",
        "lawful_evil",
        -80
    ).await?;

    let morgana = create_character(
        &graph,
        "Morgana",
        "Sorceress",
        "Empire",
        "chaotic_evil",
        -90
    ).await?;

    // Neutral
    let robin = create_character(
        &graph,
        "Robin",
        "Ranger",
        "Forest",
        "chaotic_good",
        70
    ).await?;

    println!("✅ Created 8 characters\n");

    // ═════════════════════════════════════════════════════════
    // CREATE RELATIONSHIPS
    // ═════════════════════════════════════════════════════════

    let mut tx = graph.begin_transaction().await?;

    // Kingdom Alliance
    tx.add_edge(create_relationship(arthur, lancelot, "ALLIES_WITH", 100))?;
    tx.add_edge(create_relationship(arthur, merlin, "ALLIES_WITH", 95))?;
    tx.add_edge(create_relationship(lancelot, merlin, "ALLIES_WITH", 85))?;

    // Kingdom <-> Empire Rivalry
    tx.add_edge(create_relationship(arthur, mordred, "ENEMIES_WITH", -100))?;
    tx.add_edge(create_relationship(merlin, morgana, "ENEMIES_WITH", -95))?;
    tx.add_edge(create_relationship(lancelot, mordred, "ENEMIES_WITH", -90))?;

    // Empire Alliance
    tx.add_edge(create_relationship(mordred, morgana, "ALLIES_WITH", 90))?;

    // Trade Networks
    tx.add_edge(create_relationship(arthur, marcus, "TRADES_WITH", 50))?;
    tx.add_edge(create_relationship(merlin, elena, "TRADES_WITH", 60))?;
    tx.add_edge(create_relationship(marcus, elena, "TRADES_WITH", 80))?;

    // Neutral Connections
    tx.add_edge(create_relationship(robin, arthur, "RESPECTS", 70))?;
    tx.add_edge(create_relationship(robin, marcus, "TRADES_WITH", 40))?;
    tx.add_edge(create_relationship(robin, mordred, "DISTRUSTS", -30))?;

    // Complex Relationship: Love Triangle Drama
    tx.add_edge(create_relationship(lancelot, arthur, "OWES_DEBT", 50))?;

    tx.commit().await?;

    println!("✅ Created character relationships\n");

    // ═════════════════════════════════════════════════════════
    // QUERY 1: Character's Allies
    // ═════════════════════════════════════════════════════════

    println!("═══════════════════════════════════════");
    println!("🤝 ARTHUR'S ALLIES");
    println!("═══════════════════════════════════════\n");

    let allies = graph.execute_nql(r#"
        find ally.name, ally.class, ally.faction
        from (c:Character) -> [:ALLIES_WITH] -> (ally:Character)
        where c.name = "Arthur"
    "#).await?;

    for row in allies.rows() {
        let name = row.get_string("ally.name").unwrap_or("?".into());
        let class = row.get_string("ally.class").unwrap_or("?".into());
        let faction = row.get_string("ally.faction").unwrap_or("?".into());

        println!("  ⚔️  {} the {} ({})", name, class, faction);
    }

    println!();

    // ═════════════════════════════════════════════════════════
    // QUERY 2: Character's Enemies
    // ═════════════════════════════════════════════════════════

    println!("═══════════════════════════════════════");
    println!("⚔️  ARTHUR'S ENEMIES");
    println!("═══════════════════════════════════════\n");

    let enemies = graph.execute_nql(r#"
        find enemy.name, enemy.class, enemy.alignment
        from (c:Character) -> [:ENEMIES_WITH] -> (enemy:Character)
        where c.name = "Arthur"
    "#).await?;

    for row in enemies.rows() {
        let name = row.get_string("enemy.name").unwrap_or("?".into());
        let class = row.get_string("enemy.class").unwrap_or("?".into());
        let alignment = row.get_string("enemy.alignment").unwrap_or("?".into());

        println!("  ⚡ {} the {} ({})", name, class, alignment);
    }

    println!();

    // ═════════════════════════════════════════════════════════
    // QUERY 3: Trade Network
    // ═════════════════════════════════════════════════════════

    println!("═══════════════════════════════════════");
    println!("💰 TRADE NETWORK");
    println!("═══════════════════════════════════════\n");

    let trades = graph.execute_nql(r#"
        find c1.name, c2.name
        from (c1:Character) -> [:TRADES_WITH] -> (c2:Character)
    "#).await?;

    for row in trades.rows() {
        let c1 = row.get_string("c1.name").unwrap_or("?".into());
        let c2 = row.get_string("c2.name").unwrap_or("?".into());

        println!("  💱 {} ⟷ {}", c1, c2);
    }

    println!();

    // ═════════════════════════════════════════════════════════
    // QUERY 4: Faction Analysis
    // ═════════════════════════════════════════════════════════

    println!("═══════════════════════════════════════");
    println!("🏰 FACTION MEMBERS");
    println!("═══════════════════════════════════════\n");

    let kingdom = graph.execute_nql(r#"
        find c.name, c.class, c.reputation
        from (c:Character)
        where c.faction = "Kingdom"
    "#).await?;

    println!("  Kingdom:");
    for row in kingdom.rows() {
        let name = row.get_string("c.name").unwrap_or("?".into());
        let class = row.get_string("c.class").unwrap_or("?".into());
        let rep = row.get_int("c.reputation").unwrap_or(0);

        println!("    • {} the {} (Rep: {})", name, class, rep);
    }

    println!();

    let empire = graph.execute_nql(r#"
        find c.name, c.class, c.reputation
        from (c:Character)
        where c.faction = "Empire"
    "#).await?;

    println!("  Empire:");
    for row in empire.rows() {
        let name = row.get_string("c.name").unwrap_or("?".into());
        let class = row.get_string("c.class").unwrap_or("?".into());
        let rep = row.get_int("c.reputation").unwrap_or(0);

        println!("    • {} the {} (Rep: {})", name, class, rep);
    }

    println!();

    // ═════════════════════════════════════════════════════════
    // QUERY 5: Friend of Friend (2-hop)
    // ═════════════════════════════════════════════════════════

    println!("═══════════════════════════════════════");
    println!("🔗 FRIENDS OF FRIENDS");
    println!("═══════════════════════════════════════\n");

    let fof = graph.execute_nql(r#"
        find friend.name, fof.name
        from (c:Character) -> [:ALLIES_WITH] -> (friend:Character)
             -> [:ALLIES_WITH] -> (fof:Character)
        where c.name = "Arthur"
    "#).await?;

    println!("  Arthur's extended network:\n");
    for row in fof.rows() {
        let friend = row.get_string("friend.name").unwrap_or("?".into());
        let fof = row.get_string("fof.name").unwrap_or("?".into());

        println!("    Arthur → {} → {}", friend, fof);
    }

    println!();

    // ═════════════════════════════════════════════════════════
    // QUERY 6: Most Connected Character
    // ═════════════════════════════════════════════════════════

    println!("═══════════════════════════════════════");
    println!("⭐ SOCIAL INFLUENCE");
    println!("═══════════════════════════════════════\n");

    // Find characters with most relationships
    let all_relationships = graph.execute_nql(r#"
        find c.name
        from (c:Character) -> [:ALLIES_WITH] -> (other:Character)
    "#).await?;

    let mut connection_count: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();

    for row in all_relationships.rows() {
        if let Some(PropertyValue::String(name)) = row.get("c.name") {
            *connection_count.entry(name.clone()).or_insert(0) += 1;
        }
    }

    let mut sorted: Vec<_> = connection_count.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));

    println!("  Most connected characters:\n");
    for (name, count) in sorted.iter().take(5) {
        println!("    {} - {} allies", name, count);
    }

    println!();

    // ═════════════════════════════════════════════════════════
    // QUERY 7: Alignment Distribution
    // ═════════════════════════════════════════════════════════

    println!("═══════════════════════════════════════");
    println!("⚖️  MORAL ALIGNMENT");
    println!("═══════════════════════════════════════\n");

    let good = graph.execute_nql(r#"
        find c.name, c.alignment
        from (c:Character)
        where c.reputation > 0
    "#).await?;

    println!("  Good Characters (Rep > 0): {}", good.len());

    let evil = graph.execute_nql(r#"
        find c.name, c.alignment
        from (c:Character)
        where c.reputation < 0
    "#).await?;

    println!("  Evil Characters (Rep < 0): {}", evil.len());

    println!("\n═══════════════════════════════════════");
    println!("✨ Demo Complete!");
    println!("═══════════════════════════════════════\n");

    Ok(())
}

// ═════════════════════════════════════════════════════════
// HELPER FUNCTIONS
// ═════════════════════════════════════════════════════════

async fn create_character(
    graph: &Graph,
    name: &str,
    class: &str,
    faction: &str,
    alignment: &str,
    reputation: i64,
) -> Result<nopaldb::NodeId> {
    let mut tx = graph.begin_transaction().await?;

    let character = Node::new("Character")
        .with_property("name", PropertyValue::String(name.into()))
        .with_property("class", PropertyValue::String(class.into()))
        .with_property("faction", PropertyValue::String(faction.into()))
        .with_property("alignment", PropertyValue::String(alignment.into()))
        .with_property("reputation", PropertyValue::Int(reputation));

    let node_id = tx.add_node(character).await?;
    tx.commit().await?;

    Ok(node_id)
}

fn create_relationship(
    source: nopaldb::NodeId,
    target: nopaldb::NodeId,
    rel_type: &str,
    strength: i64,
) -> Edge {
    let mut edge = Edge::new(source, target, rel_type);
    edge.properties.insert(
        "strength".to_string(),
        PropertyValue::Int(strength)
    );
    edge
}