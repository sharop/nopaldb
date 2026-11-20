# NopalDB Indexing System

**Vigente para**: 0.9.x (revisado 2026-10-26). Los tres tipos (`Hash`, `BTree`, `FullText`) son los de `IndexType`; el indice de propiedades del layout v2 se describe en [PROPERTY_INDEXING.md](PROPERTY_INDEXING.md) y el analizador del indice de texto en [INDEXING_DOCS](#3-full-text-index-tantivy-powered) y [docs/en/NQL_REFERENCE.md](en/NQL_REFERENCE.md) (`CREATE INDEX ... WITH`).

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
- **Without index**: the query reads every node of the label (`LABEL SCAN`)
- **With an index**: `where n.prop = <literal>` and `where n.prop in [...]`
  (alone or inside an `AND`) are answered by an index seek (`INDEX SEEK`);
  `EXPLAIN` shows which one ran

---

### 2. BTree Index

**Use for:** Equality lookups today, like Hash (ordered structure internally)  
**Usar para:** Hoy, búsquedas de igualdad, igual que Hash (estructura ordenada por dentro)

```rust
// Rust
graph.create_index("Person", "age", IndexType::BTree).await?;
```

```python
# Python
graph.create_index("Person", "age", "btree")
```

**Ranges are not served by the index.** The query planner only seeks an
index for `= <literal>` and `in [...]`; a range such as `n.age > 50` (or
`order by`) runs as a label scan whatever the index type. Check it with
`EXPLAIN`:

```
explain find n.name from (n:Person) where n.age > 50
Strategy: LABEL SCAN
  Reason: la condición no es `var.prop = literal` ni `var.prop in [...]` (ni un AND que los contenga)
```

**Los rangos no usan el índice.** El planificador solo busca en un índice
con `= <literal>` e `in [...]`; un rango (`n.age > 50`) o un `order by` se
resuelven recorriendo la etiqueta, sea cual sea el tipo de índice.

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

### 4. Unique Index

**Use for:** business keys (`email`, an external id) that must not repeat within a label  
**Usar para:** claves de negocio (`email`, un id externo) que no deben repetirse dentro de una etiqueta

```rust
// Rust (0.11.0): a hash or btree index that refuses repeated values
graph.create_unique_index("Persona", "email", IndexType::Hash).await?;

graph.add_node(Node::new("Persona").with_property("email", "s@example.org")).await?;
let err = graph.add_node(Node::new("Persona").with_property("email", "s@example.org")).await;
assert!(matches!(err, Err(NopalError::UniqueViolation(_))));
```

- **Every write path is checked:** transactions, direct writes, NQL and the
  bulk loader. The check runs before the write reaches the WAL: a refused
  commit writes nothing. Concurrent commits of the same value leave exactly
  one node.
- **Scope:** one label and one property. Nodes without the property, or with
  `null`, are not constrained (as in SQL). The same value under another label
  is allowed.
- **Creating** it over data that already repeats a value fails, creates
  nothing, and lists the first repeated values with their nodes.
- **Updates:** a node may be rewritten with its own value; taking another
  node's value is refused. Swapping two values inside one transaction works,
  and deleting a node, or moving it to another label, frees its value.
- `list_indexes()` reports `unique: true`; `drop_index` removes the index and
  the constraint.
- **Older versions** (≤ 0.10.x) open the base and keep the index as a plain
  one, without enforcing it. If they wrote repeated values, opening again
  logs a warning and `graph.unique_index_duplicates("Persona_email")` lists
  them; new repetitions are still refused.
- NQL (`create unique index on Persona(email)`) and Python arrive in a
  following release.

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

1. **Either order works for bulk imports**
   ```python
   # An index created first is maintained on every write
   graph.create_index("Person", "name", "hash")
   import_nodes()

   # An index created afterwards backfills the existing nodes
   # (progress via graph.set_progress_callback)
   import_nodes()
   graph.create_index("Person", "name", "hash")
   ```

2. **Use appropriate index type**
    - Equality / `in [...]`: Hash (or BTree; neither speeds up ranges)
    - Text: FullText

3. **Inspect indexes**
   ```python
   for name, label, prop, type in graph.list_indexes():
       print(f"{name}: {type}")
   graph.describe_index("Person_name")   # details of one index
   ```

---

## Limitations / Limitaciones

- No range seeks: `<`, `>`, `between`-style filters and `order by` scan the label
- One property per index: no composite indexes
- No partial indexes

---

## Troubleshooting / Solución de Problemas

### Slow queries despite index

**Problem:** Query still slow after creating index

**Solution:**
1. Verify index exists: `graph.list_indexes()`
2. Run `explain <query>`: only `= literal` / `in [...]` on the indexed
   property produce `INDEX SEEK`; ranges always scan
3. Rebuild index: `drop + create` (or `graph.rebuild_indexes()`)

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
        add (p:Person {{
            email: 'person_{i}@example.com',
            age: {20 + i % 60},
            bio: 'Expert in area {i % 10}'
        }})
    """)

# Equality on an indexed property: INDEX SEEK
result = graph.execute_nql("""
    find n.email
    from (n:Person)
    where n.email = 'person_42@example.com'
""")

# A range is a label scan, with or without the btree index
older = graph.execute_nql("find n.email from (n:Person) where n.age > 50")

# Text goes through hybrid search
hits = graph.search_hybrid(text="expert", k=10, label="Person")

# Close
graph.close()
```

---

**Next**: [Query Planner Documentation](QUERY_PLANNER_DOCS.MD)
**Siguiente**: [Documentación del Query Planner](QUERY_PLANNER_DOCS.MD)