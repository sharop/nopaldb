// examples/test_index_usage.rs
//
// Direct Rust test to see if index is being used

use nopaldb::Graph;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Enable logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();

    println!("{}", "=".repeat(70));
    println!("🔍 Index Usage Test (Rust)");
    println!("{}", "=".repeat(70));
    println!();

    let graph = Graph::open("test_dbs/synthetic_character_network.db").await?;

    // Drop and recreate index
    println!("📇 Setting up index...");
    let _ = graph.execute_nql("drop index Character_house").await;
    graph
        .execute_nql("create index on Character(house) type hash")
        .await?;
    println!("✅ Index created: Character_house");
    println!();

    // Execute query that SHOULD use index
    println!("Executing query:");
    println!("  find c.name from (c:Character) where c.house = \"TeamA\" limit 10");
    println!();

    let start = Instant::now();
    let result = graph
        .execute_nql(
            r#"
        find c.name
        from (c:Character)
        where c.house = "TeamA"
        limit 10
    "#,
        )
        .await?;
    let elapsed = start.elapsed();

    println!("Results: {} rows in {:.2?}", result.rows.len(), elapsed);
    println!();

    println!("🔍 Check the logs above for:");
    println!("  - 'Checking if query can use index'");
    println!("  - '🚀 Attempting index lookup'");
    println!("  - '✅ Index returned X nodes'");
    println!();

    graph.close().await?;

    Ok(())
}
