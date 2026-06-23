
"""
This example demonstrates how to create indexes in a NopalDB graph database
and how the query planner utilizes these indexes for efficient query execution.
"""

import nopaldb

# Create database with data
graph = nopaldb.Graph.in_memory()

# Insert 10K nodes
print("Inserting nodes...")
for i in range(10000):
    graph.execute_nql(f"create (p:Person {{email: 'person{i}@test.com', age: {20 + i % 50}}})")

print("Creating indexes...")
graph.create_index("Person", "email", "hash")
graph.create_index("Person", "age", "btree")

# Get statistics
stats = graph.get_stats()
print(f"\nGraph Statistics:")
print(f"  Total nodes: {stats['total_nodes']}")
print(f"  Total edges: {stats['total_edges']}")
print(f"  Avg degree: {stats['avg_degree']}")

# Query with index (should be fast)
print("\nQuery 1: Exact match (uses hash index)")
result = graph.execute_nql("find n.email from (n:Person) where n.email = 'person5000@test.com'")
print(f"  Found: {len(list(result))} results")

# Query with range (should use btree index)
print("\nQuery 2: Range query (uses btree index)")
result = graph.execute_nql("find count(n) as total from (n:Person) where n.age > 50")
for row in result:
    print(f"  Total: {row.get('total')}")

print("\n✅ Planner test complete!")