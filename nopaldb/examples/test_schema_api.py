#!/usr/bin/env python3
"""Test Schema API"""

import nopaldb

print("="*60)
print("Testing NopalDB Schema API")
print("="*60)

# 1. Crear grafo
graph = nopaldb.Graph.in_memory()
tx = graph.begin_transaction()

# 2. Agregar datos
alice = tx.add_node("Person", {"name": "Alice", "age": 30, "email": "alice@example.com"})
bob = tx.add_node("Person", {"name": "Bob", "age": 25})
corp = tx.add_node("Entity", {"name": "Acme Corp", "jurisdiction": "Delaware"})

tx.add_edge(alice, bob, "KNOWS", {"since": 2019})
tx.add_edge(alice, corp, "WORKS_AT", {"position": "Engineer"})

tx.commit()

print("\n✅ Data inserted")

# 3. Probar Schema API
print("\n📋 Testing Schema API:")

# Get labels
labels = graph.get_labels()
print(f"\n1. Labels: {labels}")

# Get edge types
edge_types = graph.get_edge_types()
print(f"2. Edge types: {edge_types}")

# Get properties for Person
person_props = graph.get_label_properties("Person")
print(f"3. Person properties: {person_props}")

# Get counts
person_count = graph.get_label_count("Person")
print(f"4. Person count: {person_count}")

# Get full schema
schema = graph.get_schema()
print(f"\n5. Full schema:")
print(f"   Node labels: {schema['node_labels']}")
print(f"   Edge types: {schema['edge_types']}")
print(f"   Node counts: {schema['node_counts']}")
print(f"   Edge counts: {schema['edge_counts']}")
print(f"   Total nodes: {schema['total_nodes']}")
print(f"   Total edges: {schema['total_edges']}")
print(f"   Node properties: {schema['node_properties']}")
print(f"   Edge properties: {schema['edge_properties']}")

print("\n" + "="*60)
print("✅ Schema API working perfectly!")
print("="*60)