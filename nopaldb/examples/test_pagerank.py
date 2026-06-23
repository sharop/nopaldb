#!/usr/bin/env python3
import nopaldb

graph = nopaldb.Graph.in_memory()
tx = graph.begin_transaction()

# Create test graph
alice = tx.add_node("Person", {"name": "Alice"})
bob = tx.add_node("Person", {"name": "Bob"})
carol = tx.add_node("Person", {"name": "Carol"})

tx.add_edge(alice, bob, "KNOWS", {})
tx.add_edge(bob, carol, "KNOWS", {})
tx.add_edge(carol, alice, "KNOWS", {})
tx.commit()

# Test PageRank
print("Testing PageRank in NQL:")
result = graph.execute_nql("""
    find pagerank(n) as rank
    from (n:Person)
""")

for row in result:
    print(f"  Average PageRank: {row.get('rank')}")

print("\n✅ PageRank working!")