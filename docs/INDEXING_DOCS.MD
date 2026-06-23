# NopalDB Indexing System

NopalDB exposes secondary indexes for common property-graph queries. Indexes are
created explicitly and are used by NQL when the planner can match a predicate to
an indexed label/property pair.

## Index Types

| Type | Use case | Query shape |
|------|----------|-------------|
| `hash` | Exact equality | `where n.email = "a@example.com"` |
| `btree` | Ordered/range predicates | `where n.age >= 18 and n.age <= 65` |
| `fulltext` | Text search, when compiled with full-text support | `where fulltext(d.body, "risk report")` |

## Create, List, Drop

Rust:

```rust
use nopaldb::{Graph, IndexType};

let graph = Graph::open("my.db").await?;

let index_name = graph
    .create_index("Person", "email", IndexType::Hash)
    .await?;

for index in graph.list_indexes().await {
    println!("{}: {}.{}", index.name, index.label, index.property);
}

graph.drop_index(&index_name).await?;
```

Python:

```python
import nopaldb

graph = nopaldb.Graph.open("my.db")

index_name = graph.create_index("Person", "email", "hash")

for name, label, prop, index_type in graph.list_indexes():
    print(f"{name}: {label}.{prop} [{index_type}]")

graph.drop_index(index_name)
```

## Persistence

Index definitions are persisted as metadata under the database directory. On
startup, NopalDB loads that metadata and rebuilds the in-memory index state from
the stored graph data.

This keeps the source of truth simple:

- node and edge data live in storage;
- index metadata records which indexes should exist;
- rebuild guarantees indexes match the current graph contents;
- `drop_index` removes the in-memory index and its metadata.

Rebuild work is proportional to the number of records that need to be indexed.
For large imports, create indexes before loading data when you want inserts to
maintain the index incrementally; create them after loading when you prefer one
explicit backfill step.

## Choosing An Index

Use `hash` for high-cardinality equality fields such as IDs, emails, slugs, or
external references.

Use `btree` for numeric or sortable values used in range predicates or ordering.

Use `fulltext` for natural-language fields. Builds that do not enable the
full-text feature should avoid creating this index type.

Avoid indexes on tiny-cardinality fields, write-heavy temporary fields, and
properties that are rarely queried.

## Query Planner Notes

NQL can use an index when the query includes:

- a label, such as `(p:Person)`;
- a property predicate on the same variable, such as `p.email = "a@example.com"`;
- an index whose label/property/type matches the predicate.

Use `EXPLAIN` or `PROFILE` to inspect the selected plan when tuning a query.

```python
plan = graph.execute_nql("""
explain
find p.name
from (p:Person)
where p.email = "alice@example.com"
""")

print(plan.summary)
```

## Troubleshooting

If a query still scans after creating an index:

1. Confirm the index exists with `graph.list_indexes()`.
2. Confirm the query uses the same label and property names.
3. Confirm the predicate shape matches the index type.
4. Rebuild schema metadata with `graph.rebuild_schema()` if the database was
   modified by older tooling.

## Related Docs

- [Architecture](ARCHITECTURE.md)
- [Query Planner](QUERY_PLANNER_DOCS.MD)
