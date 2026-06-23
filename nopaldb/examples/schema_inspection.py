#!/usr/bin/env python3
"""
Schema Inspection Example
Demonstrates NopalDB Schema Inspection API
"""

import nopaldb
from datetime import datetime

def print_header(title):
    print("\n" + "="*60)
    print(title)
    print("="*60)

def main():
    print_header("NopalDB Schema Inspection Demo")

    # 1. Create sample graph
    print("\n1️⃣  Creating sample graph...")
    graph = nopaldb.Graph.in_memory()

    tx = graph.begin_transaction()

    # Add Person nodes
    alice = tx.add_node("Person", {
        "name": "Alice Smith",
        "age": 30,
        "email": "alice@example.com",
        "country": "USA"
    })

    bob = tx.add_node("Person", {
        "name": "Bob Jones",
        "age": 25,
        "email": "bob@example.com",
        "city": "NYC"
    })

    carol = tx.add_node("Person", {
        "name": "Carol White",
        "age": 28,
        "country": "UK"
    })

    # Add Entity nodes
    corp1 = tx.add_node("Entity", {
        "name": "Acme Corp",
        "jurisdiction": "Delaware",
        "founded": 2010,
        "status": "active"
    })

    corp2 = tx.add_node("Entity", {
        "name": "Beta LLC",
        "jurisdiction": "Harbor Cay",
        "founded": 2015
    })

    # Add Address nodes
    addr1 = tx.add_node("Address", {
        "street": "123 Main St",
        "city": "New York",
        "country": "USA"
    })

    # Add relationships with properties
    tx.add_edge(alice, bob, "KNOWS", {
        "since": 2019,
        "strength": "strong",
        "context": "work"
    })

    tx.add_edge(bob, carol, "KNOWS", {
        "since": 2020,
        "strength": "medium"
    })

    tx.add_edge(alice, corp1, "OFFICER_OF", {
        "position": "CEO",
        "start_date": "2018-01-01",
        "salary": 150000
    })

    tx.add_edge(bob, corp2, "OFFICER_OF", {
        "position": "CTO",
        "start_date": "2019-06-15"
    })

    tx.add_edge(corp1, addr1, "REGISTERED_AT", {
        "registration_date": "2010-03-20",
        "primary": True
    })

    tx.commit()
    print("   ✅ Sample graph created")

    # 2. Get all labels
    print_header("2️⃣  Node Labels")
    labels = graph.get_labels()
    print(f"\nAvailable labels: {labels}")
    print(f"Total label types: {len(labels)}")

    # 3. Get all edge types
    print_header("3️⃣  Edge Types")
    edge_types = graph.get_edge_types()
    print(f"\nAvailable edge types: {edge_types}")
    print(f"Total edge types: {len(edge_types)}")

    # 4. Analyze each label
    print_header("4️⃣  Label Analysis")

    for label in labels:
        print(f"\n📋 {label}:")

        # Get properties
        props = graph.get_label_properties(label)
        print(f"   Properties ({len(props)}): {', '.join(sorted(props))}")

        # Get count
        count = graph.get_label_count(label)
        print(f"   Count: {count} nodes")

    # 5. Analyze each edge type
    print_header("5️⃣  Edge Type Analysis")

    for edge_type in edge_types:
        print(f"\n🔗 {edge_type}:")

        # Get properties
        props = graph.get_edge_type_properties(edge_type)
        if props:
            print(f"   Properties ({len(props)}): {', '.join(sorted(props))}")
        else:
            print(f"   Properties: None")

        # Get count
        count = graph.get_edge_type_count(edge_type)
        print(f"   Count: {count} edges")

    # 6. Get complete schema
    print_header("6️⃣  Complete Schema")

    schema = graph.get_schema()

    print(f"\n📊 Overall Statistics:")
    print(f"   Total nodes: {schema['total_nodes']}")
    print(f"   Total edges: {schema['total_edges']}")
    print(f"   Node types: {len(schema['node_labels'])}")
    print(f"   Edge types: {len(schema['edge_types'])}")

    print(f"\n📈 Node Distribution:")
    for label, count in sorted(schema['node_counts'].items(),
                               key=lambda x: x[1],
                               reverse=True):
        percentage = (count / schema['total_nodes']) * 100
        print(f"   {label:15} {count:3} nodes ({percentage:5.1f}%)")

    print(f"\n📈 Edge Distribution:")
    for edge_type, count in sorted(schema['edge_counts'].items(),
                                   key=lambda x: x[1],
                                   reverse=True):
        percentage = (count / schema['total_edges']) * 100
        print(f"   {edge_type:15} {count:3} edges ({percentage:5.1f}%)")

    # 7. Property coverage analysis
    print_header("7️⃣  Property Coverage")

    for label in labels:
        props = schema['node_properties'][label]
        print(f"\n{label}:")
        print(f"   Total properties: {len(props)}")
        print(f"   Properties: {', '.join(sorted(props))}")

    # 8. Using schema for dynamic queries
    print_header("8️⃣  Dynamic Query Building")

    print("\nQuerying all node types dynamically:")
    for label in labels:
        result = graph.execute_nql(f"""
            find count(n) as total
            from (n:{label})
        """)

        total = list(result)[0].get('total')
        print(f"   {label}: {total} nodes")

    # 9. Rebuild schema demo
    print_header("9️⃣  Schema Rebuild")

    print("\nAdding more data...")
    tx = graph.begin_transaction()
    tx.add_node("Person", {"name": "David", "age": 35})
    tx.add_node("Person", {"name": "Eve", "age": 32})
    tx.commit()

    print("Rebuilding schema cache...")
    graph.rebuild_schema()

    new_person_count = graph.get_label_count("Person")
    print(f"Updated Person count: {new_person_count}")

    # 10. Summary
    print_header("✅ Demo Complete")

    final_schema = graph.get_schema()
    print(f"\nFinal Statistics:")
    print(f"   Total nodes: {final_schema['total_nodes']}")
    print(f"   Total edges: {final_schema['total_edges']}")
    print(f"   Node types: {len(final_schema['node_labels'])}")
    print(f"   Edge types: {len(final_schema['edge_types'])}")

    print("\n" + "="*60)
    print("Schema Inspection API is ready for production! 🚀")
    print("="*60 + "\n")

if __name__ == "__main__":
    main()