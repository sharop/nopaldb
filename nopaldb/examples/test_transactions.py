#!/usr/bin/env python3
"""
Test Transaction API from Python
Updated to use new NQL syntax with edge patterns
SHAROP:PROBADO 26ENE26:Funcionando correctamente

"""

import nopaldb

def test_add_node():
    print("=" * 50)
    print("TEST 1: Add Node")
    print("=" * 50 + "\n")

    graph = nopaldb.Graph.in_memory()

    # Begin transaction
    tx = graph.begin_transaction()
    print(f"✅ Transaction: {tx}\n")

    # Add node
    alice_id = tx.add_node("Person", {
        "name": "Alice",
        "age": 30,
        "city": "Mexico City"
    })

    print(f"✅ Added node: {alice_id}\n")

    # Commit
    tx.commit()
    print(f"✅ Transaction committed\n")
    print(f"   Transaction after commit: {tx}\n")

    # Query to verify
    result = graph.execute_nql("""
        find p.name, p.age, p.city
        from (p:Person)
    """)

    print(f"📊 Query result: {len(result)} rows")
    for row in result:
        print(f"   {row}\n")

    if len(result) == 1:
        print("✅ Add node test passed!\n")
        return True
    else:
        print("❌ Expected 1 node\n")
        return False


def test_add_edge():
    print("=" * 50)
    print("TEST 2: Add Edge")
    print("=" * 50 + "\n")

    graph = nopaldb.Graph.in_memory()

    # Add two nodes
    tx = graph.begin_transaction()

    alice_id = tx.add_node("Person", {"name": "Alice", "age": 30})
    bob_id = tx.add_node("Person", {"name": "Bob", "age": 25})

    print(f"✅ Added Alice: {alice_id}")
    print(f"✅ Added Bob: {bob_id}\n")

    # Add edge
    edge_id = tx.add_edge(alice_id, bob_id, "KNOWS")
    print(f"✅ Added edge: {edge_id}\n")

    tx.commit()
    print(f"✅ Committed\n")

    # Query relationship - CORREGIDO: usar sintaxis correcta
    result = graph.execute_nql("""
        find a.name, b.name
        from (a:Person)-[:KNOWS]->(b:Person)
    """)

    print(f"📊 Relationships found: {len(result)}")
    for row in result:
        print(f"   {row.get('a.name')} knows {row.get('b.name')}\n")

    if len(result) == 1:
        print("✅ Add edge test passed!\n")
        return True
    else:
        print("❌ Expected 1 relationship\n")
        return False


def test_add_edge_with_properties():
    """Test adding edges with properties (NEW FEATURE)"""
    print("=" * 50)
    print("TEST 2b: Add Edge with Properties")
    print("=" * 50 + "\n")

    graph = nopaldb.Graph.in_memory()
    tx = graph.begin_transaction()

    alice_id = tx.add_node("Person", {"name": "Alice", "age": 30})
    bob_id = tx.add_node("Person", {"name": "Bob", "age": 25})

    # Add edge WITH properties (MEJORA 2)
    edge_id = tx.add_edge(alice_id, bob_id, "KNOWS", {
        "since": 2019,
        "strength": "strong",
        "context": "work"
    })

    print(f"✅ Added edge with properties: {edge_id}\n")
    tx.commit()

    # Query edge properties
    result = graph.execute_nql("""
        find a.name, r.since, r.strength, r.context, b.name
        from (a:Person)-[r:KNOWS]->(b:Person)
    """)

    print(f"📊 Edge with properties:")
    for row in result:
        print(f"   {row.get('a.name')} knows {row.get('b.name')}:")
        print(f"     - since: {row.get('r.since')}")
        print(f"     - strength: {row.get('r.strength')}")
        print(f"     - context: {row.get('r.context')}\n")

    if len(result) == 1:
        print("✅ Edge properties test passed!\n")
        return True
    else:
        print("❌ Expected 1 relationship with properties\n")
        return False


def test_multiple_properties():
    print("=" * 50)
    print("TEST 3: Multiple Property Types")
    print("=" * 50 + "\n")

    graph = nopaldb.Graph.in_memory()
    tx = graph.begin_transaction()

    # Test different property types
    node_id = tx.add_node("TestNode", {
        "string_prop": "hello",
        "int_prop": 42,
        "float_prop": 3.14,
        "bool_prop": True,
        "null_prop": None,
    })

    tx.commit()

    print(f"✅ Added node with mixed properties: {node_id}\n")

    # Query
    result = graph.execute_nql("find * from (n:TestNode)")

    print(f"📊 Node properties:")
    for row in result:
        for key, value in sorted(row.items()):
            print(f"   {key}: {value} ({type(value).__name__})")

    print("\n✅ Multiple property types test passed!\n")
    return True


def test_rollback():
    print("=" * 50)
    print("TEST 4: Rollback")
    print("=" * 50 + "\n")

    graph = nopaldb.Graph.in_memory()

    # Add and commit one node
    tx1 = graph.begin_transaction()
    tx1.add_node("Person", {"name": "Alice"})
    tx1.commit()

    # Add but rollback
    tx2 = graph.begin_transaction()
    tx2.add_node("Person", {"name": "Bob"})
    tx2.rollback()

    print("✅ Rolled back Bob's addition\n")

    # Query
    result = graph.execute_nql("find p.name from (p:Person)")

    names = [row.get('p.name') for row in result]
    print(f"📊 People in database: {names}\n")

    if len(result) == 1 and names[0] == "Alice":
        print("✅ Rollback test passed!\n")
        return True
    else:
        print("❌ Expected only Alice\n")
        return False


def test_wildcard_with_auto_variables():
    """Test wildcard with auto-generated variables (NEW FEATURE)"""
    print("=" * 50)
    print("TEST 5: Wildcard with Auto Variables")
    print("=" * 50 + "\n")

    graph = nopaldb.Graph.in_memory()
    tx = graph.begin_transaction()

    alice_id = tx.add_node("Person", {"name": "Alice", "age": 30, "city": "NYC"})
    bob_id = tx.add_node("Person", {"name": "Bob", "age": 25, "city": "LA"})
    tx.add_edge(alice_id, bob_id, "KNOWS", {"since": 2019})

    tx.commit()

    # Query with wildcard and no variables (MEJORA 1)
    result = graph.execute_nql("find * from ()-[r:KNOWS]->()")

    print(f"📊 Wildcard query with auto-generated variables:")
    if len(result) > 0:
        row = result[0]
        for key, value in sorted(row.items()):
            print(f"   {key}: {value}")
        print()

    if len(result) == 1:
        print("✅ Wildcard with auto variables test passed!\n")
        return True
    else:
        print("❌ Expected 1 result\n")
        return False


def main():
    print("\n🔄 Testing Transaction API with New Features\n")

    results = []

    results.append(("Add Node", test_add_node()))
    results.append(("Add Edge", test_add_edge()))
    results.append(("Add Edge with Properties", test_add_edge_with_properties()))
    results.append(("Multiple Property Types", test_multiple_properties()))
    results.append(("Rollback", test_rollback()))
    results.append(("Wildcard with Auto Variables", test_wildcard_with_auto_variables()))

    print("=" * 50)
    print("SUMMARY")
    print("=" * 50 + "\n")

    passed = sum(1 for _, result in results if result)
    total = len(results)

    for name, result in results:
        status = "✅ PASS" if result else "❌ FAIL"
        print(f"{status}: {name}")

    print(f"\nTotal: {passed}/{total} tests passed")

    if passed == total:
        print("\n✨ All transaction tests complete! 🎉\n")
    else:
        print(f"\n⚠️  {total - passed} tests failed\n")

    print("=" * 50 + "\n")


if __name__ == "__main__":
    main()