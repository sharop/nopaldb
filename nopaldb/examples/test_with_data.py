#!/usr/bin/env python3
"""
Test NQL with actual data
"""

import nopaldb

def test_with_data():
    print("🎮 Testing NQL with Real Data\n")

    # For now, we need to add data via Rust
    # This will be our TODO: add Python API for transactions

    # Open a pre-populated database (from Rust examples)
    print("Opening RPG game database...")
    try:
        graph = nopaldb.Graph.open("./examples/rpg_game.db")
        print(f"✅ Opened: {graph}\n")

        # Count nodes
        count = graph.node_count()
        print(f"📊 Total nodes: {count}\n")

        # Query quests
        print("🎯 Querying quests...")
        result = graph.execute_nql("""
            find q.name, q.xp_reward, q.level_required
            from (q:Quest)
        """)

        print(f"   Found: {len(result)} quests")
        print(f"   Columns: {result.columns}\n")

        # Display quests
        print("📋 Quests:\n")
        for i, row in enumerate(result):
            name = row.get('q.name', 'Unknown')
            xp = row.get('q.xp_reward', 0)
            level = row.get('q.level_required', 0)
            print(f"   {i+1}. {name}")
            print(f"      XP: {xp}, Level: {level}\n")

        print("✅ Query with data successful!\n")

    except Exception as e:
        print(f"⚠️  Database not found: {e}")
        print("   Run: cargo run --example rpg_quest_system first\n")

if __name__ == "__main__":
    test_with_data()