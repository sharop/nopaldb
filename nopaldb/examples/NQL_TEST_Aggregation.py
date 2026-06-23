#!/usr/bin/env python3
"""Debug NQL aggregation"""

import nopaldb

graph = nopaldb.Graph.in_memory()
tx = graph.begin_transaction()

# Add test data
tx.add_node("Person", {"name": "Alice"})
tx.add_node("Person", {"name": "Bob"})
tx.add_node("Entity", {"name": "Corp"})
tx.add_node("Location", {"name": "México City"})
tx.add_node("Address", {"name": "123 Main St"})
tx.add_node("Person", {"name": "Charlie"})
tx.commit()

print("="*60)
print("Testing NQL Aggregation")
print("="*60)

# Test 1: Simple count
print("\n1. Simple Person count:")
result = graph.execute_nql("""
    find count(n) as total
    from (n:Person)
""")
for row in result:
    print(f"   Row: {dict(row)}")
    print(f"   Keys: {list(row.keys())}")
    print(f"   total value: {row.get('total')}")

# Test 2: Count without alias
print("\n2. Count without alias:")
result = graph.execute_nql("""
    find count(n)
    from (n:Person)
""")
for row in result:
    print(f"   Row: {dict(row)}")
    print(f"   Keys: {list(row.keys())}")

# Test 3: Group by with count
print("\n3. Group by with count:")
result = graph.execute_nql("""
    find n.label, count(n) as total
    from (n)
    group by n.label
""")
for row in result:
    print(f"   Row: {dict(row)}")

# Test 4: Just properties
print("\n4. Just properties (should work):")
result = graph.execute_nql("""
    find n.name
    from (n:Person)
""")
for row in result:
    print(f"   Row: {dict(row)}")

print("="*60)