# NopalDB Query Planner

**Vigente para**: 0.9.x (revisado 2026-10-26). El plan que imprime `EXPLAIN` es el que se ejecuta ([test de honestidad](../nopaldb/tests/nql_explain_honesty_test.rs)); no hay hints de query.

---

## What it decides / Qué decide

For a `find` over a single node pattern, the executor picks **how the first
nodes are fetched**. There are three strategies:

| Strategy | When | Cost |
|---|---|---|
| `ID LOOKUP` | `where n.id = "<uuid>"` or `n.id in [...]` | point reads by id |
| `INDEX SEEK` / `INDEX SEEK (IN)` | `where n.prop = <literal>` or `n.prop in [...]` (alone or inside an `AND`) **and** a `Hash` or `BTree` index exists on `Label.prop` | index lookup, then the rest of the `where` on those rows |
| `LABEL SCAN` | anything else | reads every node of the label |

What it does **not** do:

- **No range seeks.** `n.age > 50`, `<=`, and `order by` never use an index,
  `BTree` included: they scan the label.
- **No join reordering.** A pattern with relationships runs as a
  `PATTERN PIPELINE` from its first node, in the order written (seeded by an
  id lookup or by `similar_to`/`hybrid` when present).
- **No hints** (`USE INDEX` does not exist) and no SQL syntax: queries are NQL
  (`find … from (n:Label) where …`), see the [NQL reference](en/NQL_REFERENCE.md).

When an equality index exists it is always used; the cost model below only
adds an estimate (and a note when it would have preferred the scan).

---

## EXPLAIN

```python
graph.create_index("Person", "email", "hash")
print(graph.execute_nql('explain find n.name from (n:Person) where n.email = "p1@example.org"').explain)
```

Real output (0.9.3):

```
Query Execution Plan
====================
Strategy: INDEX SEEK
  Index: Person_email (tipo Hash)
  Label: Person
  Predicate: email = p1@example.org
  Runtime fallback: label scan si el índice regresa vacío o falla.
Planner estimate: ~2 rows, cost 10.2
```

Without the index, or with a range:

```
Strategy: LABEL SCAN
  Label: Person
  Predicate: email = p1@example.org
  Reason: no existe índice para Person.email (crear con CREATE INDEX).

Strategy: LABEL SCAN
  Reason: la condición no es `var.prop = literal` ni `var.prop in [...]` (ni un AND que los contenga)
```

On a very small label the estimate can favour the scan; EXPLAIN then adds
`Cost note: el modelo de costos preferiría LABEL SCAN (label chico); la
ejecución usa el índice de todas formas.`

Other strategies you may see: `ID LOOKUP`, `INDEX SEEK (IN)`,
`PATTERN PIPELINE` (with `seed: SIMILAR_TO`, `HYBRID` or `ID LOOKUP` and
the number of hops), and the vector/hybrid lines described in
[HYBRID_SEARCH.md](HYBRID_SEARCH.md).

---

## PROFILE

`profile <query>` runs the query and returns the same plan plus timing:

```python
r = graph.execute_nql('profile find n.name from (n:Person) where n.email = "p1@example.org"')
p = r.profile
print(p.plan)                    # same text as EXPLAIN
print(p.execution_ms, p.rows_returned, p.statement_type)   # e.g. 0.19 1 query
```

There is no slow-query log and no per-operator profile.

---

## Cost model / Modelo de costos

`QueryPlanner` (`nopaldb/src/planner/mod.rs`) compares an index seek with a
label scan using these constants:

| Constant | Value |
|---|---|
| `SCAN_COST_PER_NODE` | 1.0 |
| `INDEX_SEEK_BASE_COST` | 10.0 |
| `INDEX_SEEK_PER_ROW` | 0.1 |
| `FILTER_COST_PER_ROW` | 0.1 |
| `_EXPAND_COST_PER_EDGE` | 0.5 (unused) |

- label scan = `nodes_of_label × 1.0`
- index seek = `10 + 0.1 × (nodes_of_label / cardinality)`

The statistics come from `Graph::get_stats()` (Rust) → `GraphStats`:
`total_nodes`, `total_edges`, `nodes_per_label`, `edges_per_type`,
`avg_degree`, `property_cardinality`. The counts come from the schema
manager; `property_cardinality` is a heuristic (half the label's node count
per property), not a measured distinct count. `graph.create_planner()`
returns a planner built from them.

(Python's `graph.get_stats()` is a different thing: the operational state,
see [OPERATIONS.md](OPERATIONS.md).)

---

## Keeping it accurate / Mantenimiento

```python
graph.list_indexes()      # [(name, label, property, type), ...]
graph.rebuild_schema()    # recount labels / properties used by the statistics
graph.rebuild_indexes()   # rebuild user indexes from the nodes
```

### Query not using the index

1. `graph.list_indexes()`: is there an index on exactly that `Label.property`?
2. The predicate must be `n.prop = <literal>` or `n.prop in [...]` on the
   pattern variable. Ranges, functions and `or` scan.
3. `FullText` indexes are never used for equality; text goes through
   `hybrid(...)` / `search_hybrid`.

---

**See Also / Ver También:**
- [Indexing Documentation](INDEXING_DOCS.MD)
- [NQL Reference](en/NQL_REFERENCE.md) · [Referencia NQL](es/NQL_REFERENCIA.md)
