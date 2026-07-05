#!/usr/bin/env python3
"""
Test NQL Queries from Python
SHAROP:PROBADO 26ENE26:Funcionando correctamente
"""

import nopaldb

def test_empty_query():
    print("=" * 50)
    print("TEST 1: Empty Query")
    print("=" * 50 + "\n")

    graph = nopaldb.Graph.in_memory()

    # Query empty database
    result = graph.execute_nql("""
        find p.name, p.age
        from (p:Person)
    """)

    print(f"✅ Query executed: {result}")
    print(f"   Rows: {len(result)}")
    print(f"   Columns: {result.columns}")

    # Try to iterate
    count = 0
    for row in result:
        count += 1
        print(f"   Row: {row}")

    print(f"   Iterated: {count} rows")

    if len(result) == 0:
        print("\n✅ Empty query test passed!\n")
    else:
        print("\n❌ Expected 0 rows\n")

def test_syntax_error():
    print("=" * 50)
    print("TEST 2: Syntax Error Handling")
    print("=" * 50 + "\n")

    graph = nopaldb.Graph.in_memory()

    try:
        result = graph.execute_nql("INVALID QUERY")
        print("❌ Should have raised an error")
    except Exception as e:
        print(f"✅ Caught error: {e}")
        print(f"   Type: {type(e).__name__}\n")

def test_wildcard_query():
    print("=" * 50)
    print("TEST 3: Wildcard Query")
    print("=" * 50 + "\n")

    graph = nopaldb.Graph.in_memory()

    # Query with wildcard
    result = graph.execute_nql("""
        find *
        from (p:Person)
    """)

    print(f"✅ Wildcard query executed: {result}")
    print(f"   Rows: {len(result)}\n")

def main():
    print("\n🔍 Testing NQL Queries from Python\n")

    test_empty_query()
    test_syntax_error()
    test_wildcard_query()

    print("=" * 50)
    print("✨ All NQL tests complete!")
    print("=" * 50 + "\n")

if __name__ == "__main__":
    main()