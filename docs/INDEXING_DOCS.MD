# NopalDB Indexing System

**Version**: 0.3.0  
**Date**: February 2026

---

## Overview / Resumen

NopalDB provides a **hybrid indexing system** with three index types optimized for different use cases.

NopalDB proporciona un **sistema de indexación híbrido** con tres tipos de índices optimizados para diferentes casos de uso.

---

## Index Types / Tipos de Índices

### 1. Hash Index (O(1) lookups)

**Use for:** Exact equality searches  
**Usar para:** Búsquedas de igualdad exacta

```rust
// Rust
graph.create_index("Person", "email", IndexType::Hash).await?;

// Query
find n from (n:Person) where n.email = "alice@example.com"
```

```python
# Python
graph.create_index("Person", "email", "hash")

# Query
result = graph.execute_nql("""
    find n.name
    from (n:Person)
    where n.email = 'alice@example.com'
""")
```

**Performance:**
- **Without index**: O(N) - scan all nodes
- **With hash index**: O(1) - instant lookup
- **Speedup**: 1000x for 1M nodes

---

### 2. BTree Index (O(log N) range queries)

**Use for:** Range queries, sorting  
**Usar para:** Consultas de rangos, ordenamiento

```rust
// Rust
graph.create_index("Person", "age", IndexType::BTree).await?;

// Query
find n from (n:Person) where n.age > 30 and n.age < 50
```

```python
# Python
graph.create_index("Person", "age", "btree")

# Query
result = graph.execute_nql("""
    find n.name, n.age
    from (n:Person)
    where n.age >= 18 and n.age <= 65
    order by n.age
""")
```

**Performance:**
- **Without index**: O(N) - scan all nodes
- **With btree index**: O(log N) - binary search
- **Speedup**: 100x for 1M nodes

---

### 3. Full-Text Index (Tantivy-powered)

**Use for:** BM25 text search, alone or fused with vectors (`search_hybrid`)  
**Usar para:** Búsqueda de texto BM25, sola o fusionada con vectores (`search_hybrid`)

```rust
// Rust
graph.create_index("Document", "content", IndexType::FullText).await?;

// Per-index analyzer: language stemmer + stop words + accent folding
graph.create_index_with(
    "Nota", "cuerpo", IndexType::FullText,
    IndexOptions { analyzer: Some(FullTextAnalyzer::for_language("spanish")) },
).await?;

// Query: the text branch of hybrid search (BM25 ranking)
let mut q = HybridQuery::new();
q.text = Some("fraud detection".into());
let hits = graph.search_hybrid(q).await?;
```

```python
# Python
graph.create_index("Document", "content", "fulltext")
graph.create_index("Nota", "cuerpo", "fulltext", analyzer={"language": "spanish"})

hits = graph.search_hybrid(text="fraud detection", k=10)
graph.describe_index("Nota_cuerpo")["analyzer"]   # {'language': 'spanish', 'stemming': True, ...}
```

```nql
create index on Nota(cuerpo) type fulltext with (language = "spanish", ascii_folding = true)
find n.titulo from (n:Nota) where hybrid(n, "clasificacion de catalogos", "ref", "model")
```

**Features:**
- Boolean operators in the query text: AND, OR, NOT; phrase search: "exact phrase"
- Per-index analyzer (0.5.13): stemming and stop words for 18 / 13 languages, accent folding; the query is analyzed with the index's analyzer. Default = tantivy's `default` tokenizer (lowercase only). Changing it requires drop + create. Full option table in the NQL reference.
- There is no `fulltext(...)` predicate in NQL; text search goes through `hybrid(...)` / `search_hybrid`.

---

## API Reference / Referencia de API

### Creating Indexes / Crear Índices

**Rust:**
```rust
// Create index
let index_name = graph.create_index(
    "Person",           // label
    "name",            // property
    IndexType::Hash    // type
).await?;

// Drop index
graph.drop_index("Person_name").await?;

// List indexes
let indexes = graph.list_indexes().await;
for meta in indexes {
    println!("{}: {}.{}", meta.name, meta.label, meta.property);
}
```

**Python:**
```python
# Create index
index_name = graph.create_index("Person", "name", "hash")

# Drop index
graph.drop_index("Person_name")

# List indexes
indexes = graph.list_indexes()
for name, label, prop, type in indexes:
    print(f"{name}: {label}.{prop} [{type}]")
```

---

## Best Practices / Mejores Prácticas

### When to Index / Cuándo Indexar

✅ **DO Index:**
- Columns in WHERE clauses
- Foreign keys / relationships
- Frequently sorted columns
- Text fields for search

❌ **DON'T Index:**
- Low cardinality (< 10 unique values)
- Write-heavy columns
- Temporary data

### Performance Tips / Tips de Rendimiento

1. **Create indexes BEFORE bulk imports**
   ```python
   # ❌ Slow
   import_1M_nodes()
   graph.create_index("Person", "name", "hash")  # Slow backfill
   
   # ✅ Fast
   graph.create_index("Person", "name", "hash")
   import_1M_nodes()  # Indexed on insert
   ```

2. **Use appropriate index type**
    - Equality: Hash
    - Range: BTree
    - Text: FullText

3. **Monitor index size**
   ```python
   indexes = graph.list_indexes()
   for name, label, prop, type in indexes:
       print(f"{name}: {type}")
   ```

---

## Storage Overhead / Sobrecarga de Almacenamiento

| Index Type | Space Overhead | Example (1M nodes) |
|------------|----------------|-------------------|
| Hash       | ~50 bytes/node | 50 MB            |
| BTree      | ~80 bytes/node | 80 MB            |
| FullText   | ~200 bytes/node | 200 MB          |

**Total with all 3 indexes**: ~330 MB for 1M nodes

---

## Limitations / Limitaciones

- Max 100 indexes per database
- Max 200 character keys
- No composite indexes (yet)
- No partial indexes (yet)

---

## Troubleshooting / Solución de Problemas

### Slow queries despite index

**Problem:** Query still slow after creating index

**Solution:**
1. Verify index exists: `graph.list_indexes()`
2. Check query uses indexed column
3. Rebuild index: `drop + create`

### Out of memory

**Problem:** Index creation fails with OOM

**Solution:**
1. Increase available RAM
2. Use batch insertion with smaller batches
3. Consider full-text index only for essential fields

---

## Examples / Ejemplos

### Complete Example / Ejemplo Completo

```python
import nopaldb

# Open database
graph = nopaldb.Graph.open("my.db")

# Create indexes
graph.create_index("Person", "email", "hash")
graph.create_index("Person", "age", "btree")
graph.create_index("Person", "bio", "fulltext")

# Insert data
for i in range(10000):
    graph.execute_nql(f"""
        create (p:Person {{
            email: 'person_{i}@example.com',
            age: {20 + i % 60},
            bio: 'Expert in area {i % 10}'
        }})
    """)

# Query with index (age uses the btree; text goes through hybrid search)
result = graph.execute_nql("""
    find n.email
    from (n:Person)
    where n.age > 50
""")
hits = graph.search_hybrid(text="expert", k=10, label="Person")

# Close
graph.close()
```

---

## Performance Benchmarks / Benchmarks de Rendimiento

| Operation | No Index | Hash | BTree | Speedup |
|-----------|----------|------|-------|---------|
| Exact match (1M) | 1000ms | 1ms | 5ms | 1000x |
| Range query (1M) | 5000ms | N/A | 50ms | 100x |
| Text search (1M) | 10000ms | N/A | 100ms | 100x |

---

**Next**: Query Planner Documentation
**Siguiente**: Documentación del Query Planner