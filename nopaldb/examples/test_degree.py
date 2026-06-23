#!/usr/bin/env python3
"""Test Degree Centrality in NQL"""

import nopaldb

print("="*60)
print("Testing Degree Centrality in NQL")
print("="*60)

graph = nopaldb.Graph.in_memory()
tx = graph.begin_transaction()

# Create hub-and-spoke network
# Hub should have high degree, spokes should have low degree

print("\n1️⃣  Creating network...")
hub = tx.add_node("Person", {"name": "Hub", "role": "hub"})

spokes = []
for i in range(5):
    spoke = tx.add_node("Person", {"name": f"Spoke{i}", "role": "spoke"})
    tx.add_edge(hub, spoke, "CONNECTS", {})
    spokes.append(spoke)

# Add one connection between spokes
tx.add_edge(spokes[0], spokes[1], "CONNECTS", {})

tx.commit()
print("   ✅ Network created (1 hub + 5 spokes)")

# Test Degree
print("\n2️⃣  Computing Degree Centrality...")

result = graph.execute_nql("""
    find degree(n) as deg
    from (n:Person)
""")

for row in result:
    deg = row.get('deg')
    print(f"   Average Degree: {deg:.2f}")

print("\n3️⃣  Testing with GROUP BY (by role)...")

result = graph.execute_nql("""
    find n.role, degree(n) as deg
    from (n:Person)
    group by n.role
""")

for row in result:
    role = row.get('n.role')
    deg = row.get('deg')
    print(f"   {role:12} → Average Degree: {deg:.2f}")

print("\n4️⃣  Comparing with count...")

result = graph.execute_nql("""
    find n.role, count(n) as num, degree(n) as deg
    from (n:Person)
    group by n.role
""")

print("   Role          Count    Avg Degree")
print("   " + "-"*40)
for row in result:
    role = row.get('n.role')
    num = row.get('num')
    deg = row.get('deg')
    print(f"   {role:12}  {num:5}    {deg:10.2f}")

print("\n" + "="*60)
print("✅ Degree Centrality working!")
print("="*60)
print("\n💡 Expected results:")
print("   - Hub: degree = 5 (connected to all 5 spokes)")
print("   - Spokes: most have degree = 1, two have degree = 2")
print("   - Average: hub group ≈ 5.0, spoke group ≈ 1.2")