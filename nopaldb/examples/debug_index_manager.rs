// examples/debug_index_manager.rs
//
// Debug IndexManager to see what's happening

use nopaldb::Graph;
use nopaldb::index::IndexType;
use nopaldb::types::PropertyValue;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("nopaldb=debug"))
        .init();

    println!("{}", "=".repeat(70));
    println!("🔍 IndexManager Debug");
    println!("{}", "=".repeat(70));
    println!();

    let graph = Graph::open("test_dbs/synthetic_character_network.db").await?;

    // Step 1: Create index
    println!("Step 1: Creating index...");
    let index_name = graph
        .create_index("Character", "house", IndexType::Hash)
        .await?;
    println!("  ✅ Created: {}", index_name);
    println!();

    // Step 2: Try to find index
    println!("Step 2: Checking if index exists...");
    // We need to access index_manager internals
    // For now, just try to use it
    println!();

    // Step 3: Get some nodes and see their properties
    println!("Step 3: Sample nodes to understand data...");
    let nodes = graph.get_nodes_by_label("Character").await?;

    println!("  Total Character nodes: {}", nodes.len());

    if !nodes.is_empty() {
        println!("\n  Sample node properties:");
        for (i, node) in nodes.iter().take(5).enumerate() {
            println!("    Node {}:", i + 1);
            for (key, value) in node.properties.iter().take(5) {
                println!("      {}: {:?}", key, value);
            }
        }
    }
    println!();

    // Step 4: Try direct index query
    println!("Step 4: Try direct find_nodes_indexed()...");

    // Get a real value from first node
    if let Some(first_node) = nodes.first()
        && let Some(house_value) = first_node.properties.get("house") {
        println!("  Searching for house: {:?}", house_value);

        match graph
            .find_nodes_indexed("Character", "house", house_value.clone())
            .await
        {
            Ok(results) => {
                println!("  ✅ Found {} nodes", results.len());
                if !results.is_empty() {
                    println!("    Sample: {:?}", results[0].properties.get("name"));
                }
            }
            Err(e) => {
                println!("  ❌ Error: {}", e);
            }
        }
    }

    println!();

    // Step 5: Try with "TeamA" specifically
    println!("Step 5: Try with 'TeamA' value...");
    let team_value = PropertyValue::String("TeamA".to_string());

    match graph
        .find_nodes_indexed("Character", "house", team_value)
        .await
    {
        Ok(results) => {
            println!("  ✅ Found {} nodes", results.len());
            if !results.is_empty() {
                println!("    Sample: {:?}", results[0].properties.get("name"));
            }
        }
        Err(e) => {
            println!("  ❌ Error: {}", e);
        }
    }
    println!();

    graph.close().await?;

    Ok(())
}
