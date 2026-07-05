// tests/easter_egg_test.rs

use nopaldb::Graph;

#[tokio::test]
#[ignore]  // Solo corre con: cargo test -- --ignored
async fn test_konami_code() {
    println!("\n🎮 Running Konami Code test...\n");

    let graph = Graph::in_memory().await.unwrap();

    // ↑↑↓↓←→←→BA
    graph.konami();

    println!("\n✅ Easter egg activated!\n");
}

#[tokio::test]
#[ignore]
async fn test_credits() {
    println!("\n🎬 Showing credits...\n");

    let graph = Graph::in_memory().await.unwrap();
    graph.credits();

    println!("\n✅ Credits displayed!\n");
}

#[tokio::test]
#[ignore]
async fn test_motivational() {
    let graph = Graph::in_memory().await.unwrap();

    println!("\n💪 Getting motivation...\n");

    for _ in 0..5 {
        let msg = graph.motivate();
        println!("  {}", msg);
    }

    println!("\n✅ Feeling motivated!\n");
}

#[tokio::test]
#[ignore]
async fn test_fun_facts() {
    let graph = Graph::in_memory().await.unwrap();

    println!("\n💡 Learning fun facts...\n");

    for _ in 0..3 {
        graph.fun_fact();
    }

    println!("✅ Knowledge increased!\n");
}

#[tokio::test]
#[ignore]
async fn test_all_easter_eggs() {
    println!("\n🎉 FULL EASTER EGG EXPERIENCE!\n");

    let graph = Graph::in_memory().await.unwrap();

    // Welcome
    nopaldb::easter_eggs::welcome_art();

    std::thread::sleep(std::time::Duration::from_secs(2));

    // Achievement
    nopaldb::easter_eggs::achievement_unlocked("Easter Egg Hunter 🥚");

    std::thread::sleep(std::time::Duration::from_secs(1));

    // Konami
    graph.konami();

    std::thread::sleep(std::time::Duration::from_secs(2));

    // Fun fact
    graph.fun_fact();

    std::thread::sleep(std::time::Duration::from_secs(1));

    // Motivation
    println!("{}", graph.motivate());

    std::thread::sleep(std::time::Duration::from_secs(2));

    // Credits
    graph.credits();

    println!("\n🎊 You found all the easter eggs! Legend! 🏆\n");
}