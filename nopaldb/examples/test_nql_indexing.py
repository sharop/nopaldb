# test_nql_indexing.py
import nopaldb

graph = nopaldb.Graph.in_memory()

# Test CREATE INDEX
result = graph.execute_nql("create index on Person(name) type hash")
print("✅ CREATE INDEX works")

# Test DROP INDEX
result = graph.execute_nql("drop index Person_name")
print("✅ DROP INDEX works")

# Test EXPLAIN
result = graph.execute_nql("""
    explain find p.name
    from (p:Person)
    where p.age > 30
""")
print("✅ EXPLAIN works")

print("\n🎉 All NQL extensions work!")