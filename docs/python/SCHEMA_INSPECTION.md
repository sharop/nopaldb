# Schema Inspection API

**NopalDB v0.2.0+**

## Overview

NopalDB provides a comprehensive Schema Inspection API that allows you to explore the structure of your graph database without scanning all data manually.

## Features

- ✅ Get all node labels
- ✅ Get all edge types
- ✅ Get properties per label/type
- ✅ Get counts per label/type
- ✅ Complete schema metadata
- ✅ Efficient caching

---

## Quick Start
```python
import nopaldb

# Open database
graph = nopaldb.Graph.open("my_graph.db")

# Get all labels
labels = graph.get_labels()
print(f"Available labels: {labels}")

# Get all edge types
edge_types = graph.get_edge_types()
print(f"Available edge types: {edge_types}")

# Get properties for a specific label
props = graph.get_label_properties("Person")
print(f"Person properties: {props}")

# Get complete schema
schema = graph.get_schema()
print(f"Total nodes: {schema['total_nodes']}")
print(f"Total edges: {schema['total_edges']}")
```

---

## API Reference

### `get_labels() -> List[str]`

Get all unique node labels in the graph.

**Returns:** List of label names

**Example:**
```python
labels = graph.get_labels()
# ['Person', 'Entity', 'Address']
```

---

### `get_edge_types() -> List[str]`

Get all unique edge types in the graph.

**Returns:** List of edge type names

**Example:**
```python
types = graph.get_edge_types()
# ['KNOWS', 'OFFICER_OF', 'REGISTERED_AT']
```

---

### `get_label_properties(label: str) -> List[str]`

Get all properties used by nodes with a specific label.

**Parameters:**
- `label` (str): The node label to query

**Returns:** List of property names

**Example:**
```python
props = graph.get_label_properties("Person")
# ['name', 'age', 'email', 'country']
```

---

### `get_label_count(label: str) -> int`

Get the number of nodes with a specific label.

**Parameters:**
- `label` (str): The node label to count

**Returns:** Number of nodes

**Example:**
```python
count = graph.get_label_count("Person")
# 1523
```

---

### `get_edge_type_properties(edge_type: str) -> List[str]`

Get all properties used by edges of a specific type.

**Parameters:**
- `edge_type` (str): The edge type to query

**Returns:** List of property names

**Example:**
```python
props = graph.get_edge_type_properties("KNOWS")
# ['since', 'strength', 'context']
```

---

### `get_edge_type_count(edge_type: str) -> int`

Get the number of edges of a specific type.

**Parameters:**
- `edge_type` (str): The edge type to count

**Returns:** Number of edges

**Example:**
```python
count = graph.get_edge_type_count("KNOWS")
# 5432
```

---

### `get_schema() -> Dict`

Get complete schema information for the entire graph.

**Returns:** Dictionary with schema metadata

**Schema Dictionary Structure:**
```python
{
    'node_labels': ['Person', 'Entity'],
    'edge_types': ['KNOWS', 'OFFICER_OF'],
    'node_counts': {'Person': 100, 'Entity': 50},
    'edge_counts': {'KNOWS': 200, 'OFFICER_OF': 75},
    'total_nodes': 150,
    'total_edges': 275,
    'node_properties': {
        'Person': ['name', 'age', 'email'],
        'Entity': ['name', 'jurisdiction']
    },
    'edge_properties': {
        'KNOWS': ['since', 'strength'],
        'OFFICER_OF': ['position', 'start_date']
    }
}
```

**Example:**
```python
schema = graph.get_schema()

print(f"Node labels: {schema['node_labels']}")
print(f"Total nodes: {schema['total_nodes']}")

for label, count in schema['node_counts'].items():
    props = schema['node_properties'][label]
    print(f"{label}: {count} nodes, properties: {props}")
```

---

### `rebuild_schema()`

Force rebuild of the schema cache.

**Use when:**
- After bulk imports
- After major data changes
- When schema seems stale

**Example:**
```python
# After bulk import
loader = graph.bulk_loader(batch_size=1000)
# ... load lots of data ...
loader.commit()

# Rebuild schema cache
graph.rebuild_schema()

# Now schema is up to date
schema = graph.get_schema()
```

---

## Performance Considerations

### Caching

The schema is cached internally and only rebuilt when:
1. First accessed after graph opening
2. Explicitly rebuilt with `rebuild_schema()`
3. Marked as dirty by internal operations

### Cost

- **First call**: O(N + E) - scans all nodes and edges
- **Subsequent calls**: O(1) - returns cached data
- **Rebuild**: O(N + E) - full rescan

### Best Practices
```python
# ✅ GOOD: Call once, reuse results
schema = graph.get_schema()
labels = schema['node_labels']
counts = schema['node_counts']

# ❌ BAD: Multiple calls (triggers multiple rebuilds if dirty)
labels = graph.get_labels()
types = graph.get_edge_types()
counts = [graph.get_label_count(l) for l in labels]

# ✅ GOOD: Rebuild after bulk operations
loader = graph.bulk_loader(1000)
# ... bulk load ...
loader.commit()
graph.rebuild_schema()  # Explicit rebuild
```

---

## Complete Example: Synthetic Offshore Network Analysis
```python
import nopaldb

# Open Synthetic Offshore Network database
graph = nopaldb.Graph.open("synthetic_offshore.db")

# Get schema
schema = graph.get_schema()

print("="*60)
print("SYNTHETIC OFFSHORE NETWORK DATABASE SCHEMA")
print("="*60)

# Overall statistics
print(f"\n📊 Overall Statistics:")
print(f"   Total nodes: {schema['total_nodes']:,}")
print(f"   Total edges: {schema['total_edges']:,}")

# Node analysis
print(f"\n🏷️  Node Types:")
for label in schema['node_labels']:
    count = schema['node_counts'][label]
    props = schema['node_properties'][label]
    print(f"\n   {label}:")
    print(f"      Count: {count:,}")
    print(f"      Properties: {', '.join(props)}")

# Edge analysis
print(f"\n🔗 Edge Types:")
for edge_type in schema['edge_types']:
    count = schema['edge_counts'][edge_type]
    props = schema['edge_properties'].get(edge_type, [])
    print(f"\n   {edge_type}:")
    print(f"      Count: {count:,}")
    print(f"      Properties: {', '.join(props) if props else 'None'}")

# Find most common entity type
print(f"\n🔍 Analysis:")
max_label = max(schema['node_counts'], key=schema['node_counts'].get)
max_count = schema['node_counts'][max_label]
print(f"   Most common node type: {max_label} ({max_count:,} nodes)")

max_edge = max(schema['edge_counts'], key=schema['edge_counts'].get)
max_edge_count = schema['edge_counts'][max_edge]
print(f"   Most common relationship: {max_edge} ({max_edge_count:,} edges)")

print("="*60)
```

---

## Integration with NQL

You can use schema information to build dynamic queries:
```python
# Get all labels
labels = graph.get_labels()

# Query each label type
for label in labels:
    result = graph.execute_nql(f"""
        find n.name, count(n) as connections
        from (n:{label})-[r]->()
        group by n.name
        order by connections desc
        limit 10
    """)
    
    print(f"\nTop 10 most connected {label} nodes:")
    for row in result:
        print(f"  {row.get('n.name')}: {row.get('connections')} connections")
```

---

## Error Handling
```python
try:
    # Get properties for non-existent label
    props = graph.get_label_properties("NonExistent")
    # Returns empty list []
    
except Exception as e:
    print(f"Error: {e}")
```

Schema API methods are safe and return empty results for non-existent labels/types rather than throwing errors.

---

## See Also

- [NQL Reference](NQL_GUIDE.md)
- [API Reference](API_REFERENCE.md)
- [Arrow Export](ARROW_EXPORT.md)