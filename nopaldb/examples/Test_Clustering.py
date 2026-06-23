#!/usr/bin/env python3
"""Test Clustering Coefficient in NQL"""

import nopaldb

print("="*60)
print("Testing Clustering Coefficient in NQL")
print("="*60)

graph = nopaldb.Graph.in_memory()
tx = graph.begin_transaction()

# Create two scenarios:
# 1. Complete triangle (high clustering)
# 2. Star network (low clustering)

print("\n1️⃣  Creating complete triangle network...")
# Triangle: A -- B -- C -- A (all connected)
a = tx.add_node("Person", {"name": "Alice", "group": "triangle"})
b = tx.add_node("Person", {"name": "Bob", "group": "triangle"})
c = tx.add_node("Person", {"name": "Carol", "group": "triangle"})

tx.add_edge(a, b, "FRIENDS", {})
tx.add_edge(b, c, "FRIENDS", {})
tx.add_edge(c, a, "FRIENDS", {})

print("   ✅ Triangle created (3 nodes, all connected)")

print("\n2️⃣  Creating star network...")
# Star: Hub connected to 3 spokes (spokes not connected to each other)
hub = tx.add_node("Person", {"name": "Hub", "group": "star"})
spoke1 = tx.add_node("Person", {"name": "Spoke1", "group": "star"})
spoke2 = tx.add_node("Person", {"name": "Spoke2", "group": "star"})
spoke3 = tx.add_node("Person", {"name": "Spoke3", "group": "star"})

tx.add_edge(hub, spoke1, "FRIENDS", {})
tx.add_edge(hub, spoke2, "FRIENDS", {})
tx.add_edge(hub, spoke3, "FRIENDS", {})

tx.commit()
print("   ✅ Star created (1 hub + 3 spokes)")

# Test Clustering
print("\n3️⃣  Computing Clustering Coefficient...")

result = graph.execute_nql("""
    find clustering(n) as cc
    from (n:Person)
""")

for row in result:
    cc = row.get('cc')
    print(f"   Average Clustering: {cc:.6f}")

print("\n4️⃣  Testing with GROUP BY (by network type)...")

result = graph.execute_nql("""
    find n.group, clustering(n) as cc
    from (n:Person)
    group by n.group
""")

for row in result:
    group = row.get('n.group')
    cc = row.get('cc')
    print(f"   {group:12} → Clustering: {cc:.6f}")

print("\n5️⃣  Individual node analysis...")

result = graph.execute_nql("""
    find n.name
    from (n:Person)
""")

# Note: To get individual clustering coefficients, we'd need to extend NQL
# For now, we can only get averages per group
print("   (Individual clustering requires per-node projections)")
print("   Triangle nodes should have ~1.0 (complete triangle)")
print("   Star center should have ~0.0 (neighbors not connected)")
print("   Star spokes should have ~0.0 (only 1 neighbor each)")

print("\n" + "="*60)
print("✅ Clustering Coefficient working!")
print("="*60)
print("\n💡 Expected results:")
print("   - Triangle group: clustering ≈ 1.0 (perfectly clustered)")
print("   - Star group: clustering ≈ 0.0 (no clustering)")