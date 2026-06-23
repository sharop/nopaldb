# NopalDB Query Planner

The NQL planner selects an execution strategy for a query before it reaches the
executor. The public contract is simple: write normal NQL, create useful
indexes, and inspect plans with `EXPLAIN` or `PROFILE` when you need to tune.

## Pipeline

```
NQL string
  -> parser
  -> AST
  -> planner
  -> executor
  -> NqlResult
```

## Planner Inputs

The planner can use:

- labels in node patterns, such as `(p:Person)`;
- edge types in relationship patterns, such as `-[r:KNOWS]->`;
- property predicates in `WHERE`;
- available secondary indexes;
- schema statistics maintained by the graph.

## Index Selection

Create indexes for fields that appear often in `WHERE` clauses.

```python
graph.create_index("Person", "email", "hash")

result = graph.execute_nql("""
    find p.name
    from (p:Person)
    where p.email = "alice@example.com"
""")
```

Use:

- `hash` for equality predicates;
- `btree` for range predicates and sortable values;
- `fulltext` for text search when full-text support is enabled.

## EXPLAIN

`EXPLAIN` returns the selected logical/physical plan without running the query.

```python
plan = graph.execute_nql("""
explain
find p.name
from (p:Person)
where p.age >= 18
""")

print(plan.summary)
```

## PROFILE

`PROFILE` runs the query and returns structured execution information.

```python
profile = graph.execute_nql("""
profile
find p.name
from (p:Person)
where p.age >= 18
""")

print(profile.profile.execution_ms)
print(profile.profile.rows_returned)
print(profile.profile.plan)
```

## Schema Statistics

Schema inspection APIs help you understand what the planner sees.

```python
print(graph.get_labels())
print(graph.get_edge_types())
print(graph.get_schema())

graph.rebuild_schema()
```

Use `rebuild_schema()` after bulk data changes performed by older tooling or
external migration scripts.

## Query Shape Tips

Prefer labels when you know them:

```nql
find p.name
from (p:Person)
where p.active = true
```

Keep predicates close to the entity they filter:

```nql
find p.name, c.name
from (p:Person)-[:WORKS_AT]->(c:Company)
where p.active = true and c.country = "MX"
```

Use `limit` while exploring:

```nql
find *
from (n)
limit 50
```

## Related Docs

- [Indexing](INDEXING_DOCS.MD)
- [NQL Guide](python/NQL_GUIDE.md)
- [Architecture](ARCHITECTURE.md)
