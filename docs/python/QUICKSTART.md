# NopalDB - Quick Start 🌵

Get started with NopalDB in 5 minutes.

---

## 📦 Installation

### From PyPI (Recommended)
```bash
pip install nopaldb
```

### From Source
```bash
git clone https://github.com/sharop/nopaldb.git
cd nopaldb
maturin develop --release --features python-full
```

### Verify Installation
```python
import nopaldb
print(f"NopalDB v{nopaldb.__version__}")
```

---

## 🚀 Your First Graph Database

### 1. Create Database

```python
import nopaldb

# Create in-memory database
graph = nopaldb.Graph.in_memory()

# Or persistent
graph = nopaldb.Graph.open("my_graph.db")
```

---

### 2. Add Nodes

```python
# Start transaction
tx = graph.begin_transaction()

# Add nodes with properties
alice_id = tx.add_node("Person", {
    "name": "Alice",
    "age": 30,
    "city": "New York"
})

bob_id = tx.add_node("Person", {
    "name": "Bob",
    "age": 25,
    "city": "San Francisco"
})

# Commit changes
tx.commit()

print(f"✅ Nodes created: {alice_id}, {bob_id}")
```

---

### 3. Add Relationships (Edges)

```python
tx = graph.begin_transaction()

# Add edge WITH properties
_edge_id = tx.add_edge(alice_id, bob_id, "KNOWS", {
    "since": 2019,
    "strength": "strong",
    "context": "work"
})

tx.commit()

print(f"✅ Relationship created: {_edge_id}")
```

---

### 4. Query with NQL

#### Basic Query
```python
# Find all people
result = graph.execute_nql("""
    find p.name, p.age, p.city
    from (p:Person)
""")

rows = result.query
print(f"People found: {len(rows)}")
for row in rows:
    print(f"  - {row.get('p.name')}, {row.get('p.age')} years old")
```

#### Query with Relationships
```python
# Find KNOWS relationships
result = graph.execute_nql("""
    find a.name, r.since, r.strength, b.name
    from (a:Person)-[r:KNOWS]->(b:Person)
""")

for row in result.query:
    print(f"{row.get('a.name')} knows {row.get('b.name')} since {row.get('r.since')}")
```

---

## 🎯 Common Usage Patterns

### Filter by Properties
```python
result = graph.execute_nql("""
    find p.name, p.age
    from (p:Person)
    where p.age > 25
""")
```

### Relationships without Explicit Variables
```python
# Auto-generated variables (_source, _target)
result = graph.execute_nql("""
    find *
    from ()-[r:KNOWS]->()
""")
```

### Wildcard (all properties)
```python
result = graph.execute_nql("""
    find *
    from (a:Person)-[r:KNOWS]->(b:Person)
""")

# Shows ALL properties from nodes and edges
for row in result.query:
    for key, value in row.items():
        print(f"  {key}: {value}")
```

---

## 📊 Export to Pandas/Arrow

### Complete Export
```python
import pyarrow as pa
import pandas as pd

# Export to Arrow
nodes_bytes, edges_bytes = graph.to_arrow_complete()

# Convert to Pandas
nodes_reader = pa.ipc.open_stream(nodes_bytes)
edges_reader = pa.ipc.open_stream(edges_bytes)

nodes_df = nodes_reader.read_next_batch().to_pandas()
edges_df = edges_reader.read_next_batch().to_pandas()

print(f"Nodes: {len(nodes_df)}")
print(f"Edges: {len(edges_df)}")

# Use with PyTorch, NetworkX, etc.
```

---

## 🔄 Transactions

### Commit and Rollback
```python
tx = graph.begin_transaction()

try:
    # Operations
    node_id = tx.add_node("Person", {"name": "Charlie"})
    tx.add_edge(alice_id, node_id, "KNOWS")
    
    # Commit
    tx.commit()
    print("✅ Transaction successful")
    
except Exception as e:
    # Rollback changes
    tx.rollback()
    print(f"❌ Error: {e}")
```

---

## 📚 Next Steps

1. **[API Reference](API_REFERENCE.md)** - Complete API
2. **[NQL Guide](NQL_GUIDE.md)** - Query language
3. **[Edge Patterns](EDGE_PATTERNS.md)** - Advanced patterns
4. **[Examples](EXAMPLES.md)** - Complete examples

---

## 💡 Quick Examples

### Social Network
```python
# Create social network
tx = graph.begin_transaction()

users = []
for name in ["Alice", "Bob", "Charlie", "Diana"]:
    uid = tx.add_node("User", {"name": name})
    users.append(uid)

# Connections
tx.add_edge(users[0], users[1], "FOLLOWS", {"since": 2020})
tx.add_edge(users[1], users[2], "FOLLOWS", {"since": 2021})
tx.add_edge(users[0], users[3], "FOLLOWS", {"since": 2019})

tx.commit()

# Analyze
result = graph.execute_nql("""
    find a.name, count(*) as followers
    from ()-[r:FOLLOWS]->(a:User)
    group by a.name
""")
```

### Fraud Detection (Synthetic Offshore Network)
```python
# Find offshore entities
result = graph.execute_nql("""
    find p.name, e.jurisdiction, e.incorporation_date
    from (p:Officer)-[r:OFFICER_OF]->(e:Entity)
    where e.jurisdiction = 'Harbor Cay'
""")
```

---

## ⚡ Performance Tips

1. **Use transactions** for batch operations
2. **Create indexes** for frequent equality or range filters
3. **Use Arrow export** when moving graph data into analytics tools
4. **Use bulk loading** for large imports

---

## 🆘 Help

- **Documentation**: [docs/python/](.)
- **Issues**: [GitHub Issues](https://github.com/sharop/nopaldb/issues)
- **Examples**: [nopaldb/examples/](../../nopaldb/examples/)

---

**Ready to explore!** 🚀
