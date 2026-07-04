# NopalDB Query Planner

**Version**: 0.3.0  
**Date**: February 2026

---

## What is the Query Planner? / ¿Qué es el Query Planner?

The **Query Planner** is a **cost-based optimizer** that automatically chooses the fastest execution strategy for your queries.

El **Query Planner** es un **optimizador basado en costos** que elige automáticamente la estrategia de ejecución más rápida para tus queries.

---

## How It Works / Cómo Funciona

```
Query → Parser → Planner → Optimizer → Executor → Results
                    ↓
            [Choose Best Plan]
            - Index Seek: O(log N)
            - Label Scan: O(N)
            - Filter pushdown
            - Join reordering
```

---

## Cost Model / Modelo de Costos

### Operation Costs / Costos de Operaciones

| Operation | Cost Formula | Example (1M nodes) |
|-----------|--------------|-------------------|
| **Index Seek** | 10 + 0.1 * rows | ~210 (for 2K results) |
| **Label Scan** | 1.0 * rows | ~1,000,000 |
| **Filter** | 0.1 * rows | ~100,000 |
| **Expand** | 0.5 * edges | ~500,000 |

### Cost Estimation / Estimación de Costos

```python
# Without index
SELECT n FROM Person WHERE name = "Alice"
→ LabelScan: 1M nodes * 1.0 = 1,000,000 cost

# With index
SELECT n FROM Person WHERE name = "Alice"
→ IndexSeek: 10 + (0.1 * 1) = 10.1 cost

# SPEEDUP: 99,990x faster!
```

---

## EXPLAIN Command / Comando EXPLAIN

### Usage / Uso

```sql
EXPLAIN find n.name
from (n:Person)
where n.age > 30
```

### Output / Salida

```
Query Execution Plan:
┌─────────────────────────────────────┐
│ → IndexSeek (Person_age)            │
│   Rows: ~2,000                      │
│   Cost: 20.5                        │
│   ├─ Index: BTree                   │
│   └─ Predicate: age > 30            │
│                                     │
│ → Filter                            │
│   Rows: ~2,000                      │
│   Cost: 200.0                       │
│   └─ Additional conditions          │
│                                     │
│ Total Cost: 220.5                   │
│ Estimated Time: 0.22ms              │
└─────────────────────────────────────┘
```

---

## Optimization Strategies / Estrategias de Optimización

### 1. Index Selection / Selección de Índices

**Rule:** Use index if `index_cost < scan_cost`

```python
# Example decision
if has_index and (10 + 0.1*results) < (1.0 * total_nodes):
    use_index()
else:
    full_scan()
```

### 2. Filter Pushdown / Empuje de Filtros

Push filters as early as possible:

```sql
# ❌ Bad: Filter after join
SELECT n.name
FROM (n:Person)-[:KNOWS]->(m:Person)
WHERE n.age > 30

# ✅ Good: Filter before join
SELECT n.name
FROM (n:Person WHERE n.age > 30)-[:KNOWS]->(m:Person)
```

### 3. Join Reordering / Reordenamiento de Joins

Start with smallest result set:

```sql
# ❌ Bad: Start with 1M nodes
FROM (n:Person)-[:WORKS_AT]->(c:Company WHERE c.name = "Google")

# ✅ Good: Start with 1 node
FROM (c:Company WHERE c.name = "Google")<-[:WORKS_AT]-(n:Person)
```

---

## Statistics / Estadísticas

### Graph Statistics / Estadísticas del Grafo

The planner uses these statistics:

El planner usa estas estadísticas:

```rust
struct GraphStats {
    total_nodes: usize,           // Total nodes
    nodes_per_label: HashMap,     // Nodes per label
    total_edges: usize,           // Total edges
    edges_per_type: HashMap,      // Edges per type
    avg_degree: f64,              // Average connections
    property_cardinality: HashMap // Unique values per property
}
```

### Updating Statistics / Actualizar Estadísticas

```python
# Automatic after bulk operations
graph.bulk_loader(1000).load_nodes(nodes)
# → Statistics updated automatically

# Manual update
graph.rebuild_schema()  # Rebuilds statistics
```

---

## Query Hints / Hints de Query

### Force Index / Forzar Índice

```sql
-- Use specific index
find n.name
from (n:Person USE INDEX Person_name)
where n.name = "Alice"
```

### Force Scan / Forzar Scan

```sql
-- Disable index for this query
find n.name
from (n:Person IGNORE INDEX)
where n.name = "Alice"
```

---

## Performance Monitoring / Monitoreo de Rendimiento

### Query Profiling / Perfilado de Queries

```python
# Enable profiling
graph.enable_profiling()

# Run query
result = graph.execute_nql("find n from (n:Person)")

# Get profile
profile = graph.get_last_profile()
print(f"Execution time: {profile['time_ms']}ms")
print(f"Rows scanned: {profile['rows_scanned']}")
print(f"Rows returned: {profile['rows_returned']}")
print(f"Index used: {profile['index_used']}")
```

---

## Best Practices / Mejores Prácticas

### 1. Create Indexes for WHERE Clauses

```python
# ✅ Good
graph.create_index("Person", "email", "hash")
result = graph.execute_nql("find n from (n:Person) where n.email = 'alice@x.com'")
# → Uses index

# ❌ Bad
result = graph.execute_nql("find n from (n:Person) where n.email = 'alice@x.com'")
# → Full scan
```

### 2. Use EXPLAIN Before Production

```python
# Test query performance
explain = graph.execute_nql("EXPLAIN find n from (n:Person) where n.age > 30")
print(explain)

# If cost > 10000, create index
if explain['cost'] > 10000:
    graph.create_index("Person", "age", "btree")
```

### 3. Monitor Slow Queries

```python
# Log slow queries (> 100ms)
graph.set_slow_query_threshold(100)

# Check slow query log
slow_queries = graph.get_slow_queries()
for query in slow_queries:
    print(f"Query: {query['sql']}")
    print(f"Time: {query['time_ms']}ms")
    print(f"Plan: {query['plan']}")
```

---

## Examples / Ejemplos

### Example 1: Simple Query

```python
# Query
result = graph.execute_nql("""
    find n.name
    from (n:Person)
    where n.age > 30
""")

# Planner decisions:
# 1. Check for index on Person.age
# 2. If index exists → IndexSeek
# 3. If no index → LabelScan
# 4. Apply filter
```

### Example 2: Complex Query

```python
# Query with multiple filters
result = graph.execute_nql("""
    find n.name, c.name
    from (n:Person)-[:WORKS_AT]->(c:Company)
    where n.age > 30 and c.revenue > 1000000
""")

# Planner decisions:
# 1. Estimate: Person with age>30 = 10K nodes
# 2. Estimate: Company with revenue>1M = 100 nodes
# 3. Start with Company (smaller)
# 4. Expand to Person
# 5. Filter by age
```

---

## Troubleshooting / Solución de Problemas

### Query Not Using Index

**Problem:** Index exists but not used

**Solution:**
```python
# Check plan
explain = graph.execute_nql("EXPLAIN your_query")
print(explain)

# Verify index name matches
indexes = graph.list_indexes()
print(indexes)

# Force index usage
query_with_hint = "find n from (n:Person USE INDEX Person_age) where n.age > 30"
```

### High Cost Estimates

**Problem:** Planner estimates high cost

**Solution:**
```python
# Update statistics
graph.rebuild_schema()

# Create appropriate index
graph.create_index("Person", "age", "btree")

# Verify improvement
explain_after = graph.execute_nql("EXPLAIN your_query")
```

---

## Comparison / Comparación

### NopalDB vs Others

| Feature | NopalDB | Neo4j | SQLite |
|---------|---------|-------|--------|
| Cost-based | ✅ | ✅ | ✅ |
| Index hints | ✅ | ✅ | ✅ |
| Statistics | ✅ | ✅ | ✅ |
| EXPLAIN | ✅ | ✅ | ✅ |
| Auto-tuning | 🔜 | ✅ | ❌ |

---

## Future Improvements / Mejoras Futuras

- [ ] Histogram statistics
- [ ] Join cost estimation
- [ ] Query result caching
- [ ] Adaptive query execution
- [ ] Machine learning cost models

---

**See Also:**
- [Indexing Documentation](INDEXING_DOCS.md)
- [NQL Reference](NQL_REFERENCE.md)

**Ver También:**
- [Documentación de Indexación](INDEXING_DOCS.md)
- [Referencia NQL](NQL_REFERENCE.md)