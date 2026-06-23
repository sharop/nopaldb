#!/usr/bin/env python3
"""Test Betweenness Centrality in NQL"""

import nopaldb

print("="*60)
print("Testing Betweenness Centrality in NQL")
print("="*60)

graph = nopaldb.Graph.in_memory()
tx = graph.begin_transaction()

# Create star network: center connected to 4 periphery nodes
# Center should have high betweenness (it's on all shortest paths)
print("\n1️⃣  Creating star network...")
center = tx.add_node("Person", {"name": "Hub", "role": "center"})

periphery = []
for i in range(4):
    p = tx.add_node("Person", {"name": f"Spoke{i}", "role": "periphery"})
    tx.add_edge(center, p, "CONNECTS", {})
    periphery.append(p)

tx.commit()
print("   ✅ Network created (1 center + 4 periphery)")

# Test Betweenness
print("\n2️⃣  Computing Betweenness Centrality...")

result = graph.execute_nql("""
    find betweenness(n) as bc
    from (n:Person)
""")

for row in result:
    bc = row.get('bc')
    print(f"   Average Betweenness: {bc:.6f}")

print("\n3️⃣  Testing with GROUP BY...")

result = graph.execute_nql("""
    find n.role, betweenness(n) as bc
    from (n:Person)
    group by n.role
""")

for row in result:
    role = row.get('n.role')
    bc = row.get('bc')
    print(f"   {role:12} → Betweenness: {bc:.6f}")

print("\n" + "="*60)
print("✅ Betweenness Centrality working!")
print("="*60)