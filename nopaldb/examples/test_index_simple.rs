// examples/test_index_simple.rs
//
// Simplified test without DROP INDEX

use nopaldb::Graph;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Enable logging for nopaldb only (skip sled spam)
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("nopaldb=debug"))
        .init();

    println!("{}", "=".repeat(70));
    println!("🔍 Index Usage Test - Simplified");
    println!("{}", "=".repeat(70));
    println!();

    let graph = Graph::open("test_dbs/synthetic_character_network.db").await?;

    println!("📇 Database opened");
    println!();

    // Just run the query (index already exists from Python script)
    println!("Executing query with WHERE clause:");
    println!("  find c.name from (c:Character) where c.house = \"TeamA\" limit 10");
    println!();
    println!("🔍 Watch for these logs:");
    println!("  [INFO nopaldb::query::nql::executor] Executing NQL query");
    println!("  [DEBUG nopaldb::query::nql::executor] Checking if query can use index");
    println!("  [INFO nopaldb::query::nql::executor] 🚀 Attempting index lookup");
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

    println!();
    println!("Results: {} rows in {:.2?}", result.rows.len(), elapsed);

    if result.rows.len() > 0 {
        println!("\nSample results:");
        for (i, row) in result.rows.iter().take(3).enumerate() {
            println!("  {}. {:?}", i + 1, row);
        }
    }

    println!();
    graph.close().await?;

    Ok(())
}
