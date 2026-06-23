#!/usr/bin/env python3
"""
NopalDB Python Bindings Test
"""

import nopaldb

def test_basic():
    print("🐍 Testing NopalDB Python Bindings\n")

    print(f"✅ Version: {nopaldb.__version__}")
    print(f"✅ Graph class: {nopaldb.Graph}")
    print(f"✅ QueryResult class: {nopaldb.QueryResult}\n")

    # Create in-memory database
    print("Creating in-memory graph...")
    graph = nopaldb.Graph.in_memory()
    print(f"✅ Graph created: {graph}\n")

    # Try to get node count
    print("Getting node count...")
    try:
        count = graph.node_count()
        print(f"✅ Node count: {count}\n")
    except Exception as e:
        print(f"⚠️  Error getting node count: {e}\n")

    print("✨ Basic tests complete!")

if __name__ == "__main__":
    test_basic()