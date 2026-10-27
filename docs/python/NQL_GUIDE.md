# NQL Guide (Nopal Query Language) 🔍

NQL is the query language for NopalDB, inspired by Cypher but optimized for Data Science and Python workflows.

---

## 📖 Basic Syntax

### Query Structure

```nql
find <projections>
from <pattern>
[where <conditions>]
[group by <columns>]
[having <conditions on groups>]
[order by <columns>]
[limit <number>]
```

Comments inside NQL are `// line` and `/* block */`; `--` and `#` are not
comments and fail to parse.

---

## 🎯 Node Patterns

**Syntax:** `(variable:Label {properties})`

### Basic Patterns
```nql
// Match any node
find * from (n)

// Match by Label
find * from (p:Person)

// Match with Inline Properties (New!)
find * from (p:Person {name: "Alice", city: "NYC"})
```

---

## 🔗 Relationship Patterns

**Syntax:** `(a)-[variable:TYPE]->(b)`

### Directionality
- `->`: Outgoing
- `<-`: Incoming
- `<->`: Bidirectional (matches both ways)

### Examples
```nql
// Simple connection
find * from (a)->(b)

// Typed connection
find * from (a)-[:KNOWS]->(b)

// Variable assignment
find r.since from (a)-[r:KNOWS]->(b)
```

---

## 📊 Projections & Aggregations

Define what data to return using the `FIND` clause.

### Property Selection
```nql
find p.name, p.age
from (p:Person)
```

### Aggregations (New!)
NopalDB now supports standard aggregation functions:

- `count(*)`: Count total matches
- `sum(x)`: Sum values
- `avg(x)`: Calculate average
- `min(x)` / `max(x)`: Find extremes

**Example:**
```nql
find count(*) as total_users, avg(u.age) as average_age
from (u:User)
where u.active = true
```

**Per group** (`having` filters the groups):
```nql
find u.name, count(*) as follows
from (u:User)-[:FOLLOWS]->(v:User)
group by u.name
having count(*) > 10
order by follows desc
```

---

## 🔍 Filtering (WHERE)

Filter results based on properties.

### Operators
- `=`, `!=`, `<`, `>`, `<=`, `>=`
- `in [...]`: `where p.city in ["London", "Paris"]`
- `AND`, `OR`, and parentheses for grouping

There is no `NOT` / `!` yet: `!(p.age > 25)` is an error that says so.
Write the opposite comparison instead (`p.age <= 25`, `p.status != "done"`).

Every condition is a comparison. A bare property such as `where p.active` is
an error since 0.9.4 that asks for `p.active = true` (until 0.9.3 it was
accepted and filtered nothing). A projection alias can be used in `where`
and `having` (since 0.9.4): `find a.name, avg(r.w) as m from (a)-[r:RATES]->(b) group by a.name having m > 0.5`.

### Examples
```nql
// Numeric comparison
where p.age > 25

// String matching
where p.city = "London"

// Boolean logic
where (p.age > 18 or p.student = true) and p.active = true
```

---

## 🛣️ Path Metadata & PROFILE (F2)

Path Queries `F2` adds metadata over the full linear match:

- `path.depth`
- `path.nodes`
- `path.edges`

Example:

```nql
find b.name, path.depth, path.nodes
from (a:Person {name: "Alice"})-[:KNOWS]->{1,2}(b:Person)
where path.depth >= 1
order by path.depth desc
```

Rules:
- `path.depth` works in `FIND`, `WHERE`, and `ORDER BY`
- `path.nodes` and `path.edges` work only in `FIND`
- `path.*` requires a single linear pattern with at least one relationship

`PROFILE` is also available:

```python
result = graph.execute_nql("""
profile
find b.name, path.depth
from (a:Person {name: "Alice"})-[:KNOWS]->{1,2}(b:Person)
""")

print(result.profile.execution_ms)
print(result.profile.path_metrics)
```

---

## 📝 Write Operations (ADD, UPDATE, DELETE)

NQL can modify the **contents** of a graph directly.

Important:
- You still create/open the graph itself with `nopaldb.Graph.open(...)` or `nopaldb.Graph.in_memory()`.
- NQL does not currently support `CREATE GRAPH`, `BEGIN`, `COMMIT`, `ROLLBACK`, or `MERGE`.

### ADD
Create new nodes or relationships.
```nql
// Add a node
add (p:Person {name: "Bob", age: 30})

// Add a relationship with edge properties
add (a:Person {name: "Alice"})-[:KNOWS {since: 2020, strength: "high"}]->(b:Person {name: "Bob"})
```

`add` always creates: both nodes of that pattern are new even if an `Alice`
already exists. For idempotent writes by key use `graph.upsert(...)` (see the
[API cheat sheet](API_CHEATSHEET.md)).

### UPDATE
Modify existing properties.
```nql
update (p:Person)
set p.verified = true
where p.email = "bob@example.com"

update (a:Person)-[r:KNOWS]->(b:Person)
set r.since = 2024
where a.name = "Alice" and b.name = "Bob"
```

### DELETE
Remove nodes or relationships.
```nql
// Delete only the matched relationship
delete (a:Person)-[:KNOWS]->(b:Person)
where a.name = "Alice" and b.name = "Bob"

// Delete nodes
delete (p:Person)
where p.last_login < 1600000000
```

Deleting a node also deletes the edges attached to it.

---

## ⚡ Performance Tips

1. **Use Labels:** `(p:Person)` is much faster than `(n)` because it uses specific indices.
2. **Filter Early:** Place restrictive conditions in `WHERE` to reduce the working set.
3. **Limit Results:** Always use `LIMIT` when exploring large datasets.

---

**Next Steps:**
- See [API Reference](API_REFERENCE.md) for Python integration.
- Check [Examples](EXAMPLES.md) for real-world use cases.
- See `../NQL_WRITE_CRUD_HANDS_ON.md` for a complete Rust + Python + NQL CRUD walkthrough.
