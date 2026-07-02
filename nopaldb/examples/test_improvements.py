#!/usr/bin/env python3
"""
Test NQL Edge Patterns - With Improvements
Tests both improvements:
1. Auto-generated variables for patterns without variables
2. Edge properties in add_edge()
SHAROP:PROBADO 26ENE26:Funcionando correctamente

"""

import nopaldb
from pathlib import Path
import shutil

DB_PATH = "data/edge_patterns_improved.db"

def setup_test_data():
    """Create test database with nodes and edges WITH properties"""
    print("🔧 Setting up test database...")

    # Remove existing
    if Path(DB_PATH).exists():
        shutil.rmtree(DB_PATH)

    # Create database
    graph = nopaldb.Graph.open(DB_PATH)

    # Create transaction
    tx = graph.begin_transaction()

    # Create nodes
    alice_id = tx.add_node("Person", {"name": "Alice", "age": 30, "city": "NYC"})
    bob_id = tx.add_node("Person", {"name": "Bob", "age": 25, "city": "LA"})
    charlie_id = tx.add_node("Person", {"name": "Charlie", "age": 35, "city": "SF"})
    techcorp_id = tx.add_node("Company", {"name": "TechCorp", "founded": 2010})
    datainc_id = tx.add_node("Company", {"name": "DataInc", "founded": 2015})

    print(f"   ✅ Created 5 nodes")

    # Create edges WITH properties (MEJORA 2)
    tx.add_edge(alice_id, bob_id, "KNOWS", {"since": 2019, "strength": "strong"})
    tx.add_edge(alice_id, charlie_id, "KNOWS", {"since": 2020, "strength": "medium"})
    tx.add_edge(bob_id, charlie_id, "KNOWS", {"since": 2018, "strength": "weak"})
    tx.add_edge(alice_id, techcorp_id, "WORKS_AT", {"since": 2020, "position": "Engineer"})
    tx.add_edge(bob_id, datainc_id, "WORKS_AT", {"since": 2021, "position": "Manager"})

    print(f"   ✅ Created 5 edges with properties")

    # Commit
    tx.commit()
    print(f"   ✅ Transaction committed\n")

    return graph


def run_test(graph, name, query, expect_success=True):
    """Run a single test"""
    print(f"\n{'='*70}")
    print(f"TEST: {name}")
    print(f"{'='*70}")
    print(f"Query: {query}\n")

    try:
        result = graph.execute_nql(query)

        if not expect_success:
            print(f"❌ FAIL: Expected to fail but succeeded")
            return False

        print(f"✅ SUCCESS: {len(result)} results")

        # Display first 3 rows
        for i, row in enumerate(result):
            print(f"\nRow {i+1}:")
            for key in sorted(row.keys()):
                value = row.get(key)
                print(f"  {key}: {value}")
            if i >= 2:
                break

        if len(result) > 3:
            print(f"\n... and {len(result) - 3} more rows")

        return True

    except Exception as e:
        if expect_success:
            print(f"❌ FAIL: {e}")
            import traceback
            traceback.print_exc()
            return False
        else:
            print(f"✅ EXPECTED FAIL: {str(e)[:80]}")
            return True


def main():
    """Run all tests"""
    print("="*70)
    print("NQL Edge Patterns - With Improvements Test Suite")
    print("="*70)

    # Setup
    graph = setup_test_data()

    results = []

    # ====================================================================
    # MEJORA 1: Patrones sin variables
    # ====================================================================
    print("\n" + "="*70)
    print("MEJORA 1: Auto-Generated Variables")
    print("="*70)

    results.append((
        "No variables: ()-[r]->()",
        run_test(graph, "No variables: ()-[r]->()",
                 "find * from ()-[r]->()"),
    ))

    results.append((
        "Partial variables: ()->(b)",
        run_test(graph, "Partial variables: ()->(b)",
                 "find * from ()->(b)"),
    ))

    results.append((
        "Partial variables: (a)->()",
        run_test(graph, "Partial variables: (a)->()",
                 "find * from (a)->()"),
    ))

    # ====================================================================
    # MEJORA 2: Edge Properties
    # ====================================================================
    print("\n" + "="*70)
    print("MEJORA 2: Edge Properties")
    print("="*70)

    results.append((
        "Edge property: r.since",
        run_test(graph, "Edge property: r.since",
                 "find r.since from (a)-[r:KNOWS]->(b)"),
    ))

    results.append((
        "Edge property: r.strength",
        run_test(graph, "Edge property: r.strength",
                 "find r.strength from (a)-[r:KNOWS]->(b)"),
    ))

    results.append((
        "Multiple edge properties",
        run_test(graph, "Multiple edge properties",
                 "find r.since, r.strength from (a)-[r:KNOWS]->(b)"),
    ))

    results.append((
        "Mixed properties: nodes + edge",
        run_test(graph, "Mixed properties: nodes + edge",
                 "find a.name, r.since, r.strength, b.name from (a:Person)-[r:KNOWS]->(b:Person)"),
    ))

    # ====================================================================
    # COMBINANDO AMBAS MEJORAS
    # ====================================================================
    print("\n" + "="*70)
    print("COMBINING BOTH IMPROVEMENTS")
    print("="*70)

    results.append((
        "No vars + edge props",
        run_test(graph, "No vars + edge props",
                 "find r.since from ()-[r:KNOWS]->()"),
    ))

    results.append((
        "Partial vars + edge props",
        run_test(graph, "Partial vars + edge props",
                 "find a.name, r.since from (a:Person)-[r:KNOWS]->()"),
    ))

    # ====================================================================
    # PATTERNS QUE YA FUNCIONABAN
    # ====================================================================
    print("\n" + "="*70)
    print("EXISTING PATTERNS (Should Still Work)")
    print("="*70)

    results.append((
        "Basic with vars: (a)->(b)",
        run_test(graph, "Basic with vars: (a)->(b)",
                 "find * from (a)->(b)"),
    ))

    results.append((
        "Edge variable: (a)-[r]->(b)",
        run_test(graph, "Edge variable: (a)-[r]->(b)",
                 "find * from (a)-[r]->(b)"),
    ))

    results.append((
        "Edge type: (a)-[:KNOWS]->(b)",
        run_test(graph, "Edge type: (a)-[:KNOWS]->(b)",
                 "find * from (a)-[:KNOWS]->(b)"),
    ))

    results.append((
        "Variable + type: (a)-[r:KNOWS]->(b)",
        run_test(graph, "Variable + type: (a)-[r:KNOWS]->(b)",
                 "find * from (a)-[r:KNOWS]->(b)"),
    ))

    results.append((
        "r.type property",
        run_test(graph, "r.type property",
                 "find r.type from (a)-[r]->(b)"),
    ))

    # ====================================================================
    # SUMMARY
    # ====================================================================
    print("\n" + "="*70)
    print("SUMMARY")
    print("="*70)

    passed_count = sum(1 for _, passed in results if passed)
    total_count = len(results)

    for name, passed in results:
        status = "✅ PASS" if passed else "❌ FAIL"
        print(f"{status}: {name}")

    print()
    print(f"Total: {passed_count}/{total_count} tests passed")

    if passed_count == total_count:
        print("\n🎉 ALL TESTS PASSED! 🎉")
        print("\n✅ MEJORA 1 Working:")
        print("   - ()-[r]->() - no variables")
        print("   - ()->(b) - partial variables")
        print("   - (a)->() - partial variables")
        print("\n✅ MEJORA 2 Working:")
        print("   - r.since - edge properties")
        print("   - r.strength - edge properties")
        print("   - Combined node + edge properties")
        print("\n✅ Backward Compatibility:")
        print("   - All existing patterns still work")
    else:
        print(f"\n⚠️  {total_count - passed_count} tests failed")
        print("   Check errors above for details")

    print("\n" + "="*70)


if __name__ == "__main__":
    main()