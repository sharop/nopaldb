import nopaldb

# Test 1: Manual close
print("Test 1: Manual close")
graph = nopaldb.Graph.in_memory()
graph.execute_nql("create (n:Test {value: 1})")
graph.close()
print("✅ Manual close OK")

# Test 2: Context manager
print("\nTest 2: Context manager")
with nopaldb.Graph.in_memory() as graph:
    graph.execute_nql("create (n:Test {value: 2})")
    print("  Inside context")
print("✅ Context manager OK (auto-closed)")

# Test 3: With file database
print("\nTest 3: File database")
with nopaldb.Graph.open("test.db") as graph:
    graph.execute_nql("create (n:Test {value: 3})")
print("✅ File database closed properly")

print("\n🎉 All tests passed!")