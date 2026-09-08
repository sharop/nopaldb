# NQL Query Language - Quick Reference

**NQL (NopalDB Query Language)** is an intuitive and powerful graph query language designed for developers, **data scientists, analysts, and sociologists**. Its Cypher-inspired syntax optimizes finding patterns and complex connections in your data.

---

## Table of Contents

- [Core Concepts](#core-concepts)
- [General Syntax](#general-syntax)
- [Pattern Matching (FROM)](#pattern-matching-from)
- [Data Selection (FIND)](#data-selection-find)
- [Filtering (WHERE)](#filtering-where)
- [Ontology Predicates](#ontology-predicates-instanceof--subclassof)
- [Write Operations (ADD/UPDATE/DELETE)](#write-operations-addupdatedelete)
- [Indexes (CREATE INDEX / DROP INDEX)](#indexes-create-index--drop-index)
- [Aggregations & Functions](#aggregations--functions)
- [Data Export (EXPORT)](#data-export-export)
- [Operators](#operators)
- [Examples by Discipline](#examples-by-discipline)

---

## Core Concepts

- **Nodes**: The entities in your data (People, Cities, Transactions). Represented in parentheses `(n:Person)`.
- **Relationships**: How nodes connect. Represented with arrows `->`.
- **Properties**: Details of nodes or relationships, like `{name: "Alice", age: 30}`.

---

## General Syntax

The basic structure of an NQL query is simple:

```nql
FIND <what_to_return>
FROM <connection_pattern>
WHERE <filter_conditions>
LIMIT <number_of_results>
```

**Simple Example:**
```nql
find p.name, p.age
from (p:Person)
where p.age > 25
limit 10
```

---

## Pattern Matching (FROM)

The `FROM` clause defines the "shape" or pattern you are looking for in the graph.

### Node Patterns
- `(p:Person)`: Matches nodes with label "Person" and calls them `p`.
- `(n)`: Matches any node.
- `(p:Person {city: "NYC"})`: **New!** Matches Person nodes having property `city` equal to "NYC".

### Relationship Patterns
Relationships connect nodes. The arrow direction matters.

- `->`: Outgoing relationship (any type).
- `<-`: Incoming relationship (any type).
- `<->`: Bidirectional relationship (any direction).
- `-[:FRIEND]->`: Outgoing relationship of specific type "FRIEND".
- `<-[:PURCHASED]-`: Incoming relationship of type "PURCHASED".

### Connection Examples
```nql
-- A knows B
from (a:Person) -> [:KNOWS] -> (b:Person)

-- Influence Chain: A influences B, B influences C
from (a:Person) -> [:INFLUENCES] -> (b:Person) -> [:INFLUENCES] -> (c:Person)

-- Mutual Collaboration
from (author1:Researcher) <-> [:COLLABORATES] <-> (author2:Researcher)
```

---

## Data Selection (FIND)

Specifies what information to retrieve from the matched patterns.

### Syntax
- `find p.name`: Returns property `name` of node `p`.
- `find *`: Returns **all** properties of matched elements.
- `find count(*)`: Counts how many matches were found.

### Examples
```nql
find p.name, p.email
from (p:Customer)
```

---

## Filtering (WHERE)

Refine your search with logical conditions.

### Operators
- **Comparison**: `=`, `!=`, `<`, `>`, `<=`, `>=`
- **Logical**: `and`, `or`, `not`

### Examples
```nql
-- High-value customers
where c.total_purchases > 50000

-- Geographic and demographic segmentation
where (p.city = "NYC" or p.city = "SF") and p.age < 30

-- Exclusion
where not p.status = "Inactive"
```

---

## Ontology Predicates (instanceOf / subClassOf)

When the graph carries a class hierarchy (imported from Turtle/OWL, or built
with `NodeKind::Class` nodes and `subClassOf` edges), two predicates in `where`
ask the taxonomy instead of comparing labels:

- `instanceOf(n, "C")` — `n` is an individual whose declared type is `C` **or a
  subclass of `C`**, at any depth. Every declared type counts: a node imported
  with two `rdf:type` matches either of them, not only the one shown as its
  label.
- `subClassOf(c, "C")` — `c` is a class node that is a strict subclass of `C`
  (a class is not a subclass of itself).

The class can be named three ways; the first form that resolves wins:

| Form | Example | Resolved through |
|---|---|---|
| Label | `"Arbol"` | the class node's label (the only form for graphs built by hand) |
| Prefixed name | `"flora:Arbol"` | the prefix catalog of the imported documents (`graph.rdf_prefixes()`); an unknown prefix falls back to the label, which is how a class whose local name collided is labelled (`fauna:Rosa`) |
| Full IRI | `"http://plantas.example/flora#Arbol"` | the class node's `iri` property |

```nql
-- Every plant: direct instances and instances of subclasses alike
find n.label from (n) where instanceOf(n, "Planta")

-- Same class, unambiguous across namespaces
find n.iri from (n) where instanceOf(n, "flora:Rosa")

-- Only the classes under Planta
find c.label from (c) where subClassOf(c, "Planta")
```

Path queries run the same test over the nodes of a path:
`path_start_instanceOf("C")`, `path_end_instanceOf("C")`,
`path_any_instanceOf("C")`, `path_all_instanceOf("C")`, and the `subClassOf`
counterparts. They take the class name only, in any of the three forms.

Without a taxonomy (no class nodes in the graph) both predicates are `false`
for every node.

---

## Path Metadata and PROFILE (F2)

`F2` adds metadata over the full linear match:

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

`PROFILE` is also available through `execute_statement()`:

```nql
profile
find b.name, path.depth
from (a:Person {name: "Alice"})-[:KNOWS]->{1,2}(b:Person)
```

---

## Write Operations (ADD/UPDATE/DELETE)

NQL supports CRUD for **graph content**:

- `ADD`: create nodes and relationships
- `UPDATE`: update node and edge properties
- `DELETE`: delete matched nodes or relationships
- `CREATE INDEX` / `DROP INDEX`: index management

Important:
- NQL does **not** create the storage/database container. That is done through the API with `Graph.open(...)` or `Graph.in_memory()`.
- NQL does **not** currently support `CREATE GRAPH`, `BEGIN`, `COMMIT`, `ROLLBACK`, or `MERGE`.

### ADD
```nql
add (a:Person {name: "Alice"})-[:KNOWS {since: 2020, strength: "high"}]->(b:Person {name: "Bob"})
```

### UPDATE
```nql
update (a:Person)-[r:KNOWS]->(b:Person)
set r.since = 2024, b.city = "Mexico City"
where a.name = "Alice" and b.name = "Bob"
```

### DELETE
```nql
-- Deletes only the matched relationship
delete (a:Person)-[:KNOWS]->(b:Person)
where a.name = "Alice" and b.name = "Bob"

-- Deletes nodes (and their incident relationships)
delete (p:Person)
where p.name = "Bob"
```

For a detailed status matrix and end-to-end examples:
- `docs/NQL_WRITE_CRUD_STATUS.md`
- `docs/NQL_WRITE_CRUD_HANDS_ON.md`

---

## Indexes (CREATE INDEX / DROP INDEX)

```nql
create index on Person(email)                      -- hash (default): equality lookups
create index on Person(age) type btree             -- range queries
create index on Article(body) type fulltext        -- BM25 text search, used by hybrid()
drop index Person_email                            -- the name is Label_property
```

### Full-text analyzer

A full-text index tokenizes its documents once, at indexing time, and every
query against it goes through the **same** analyzer. By default that is
tantivy's `default` tokenizer: split on non-alphanumerics, lowercase, nothing
else, so `clasificacion` does not find `clasificación` and `catálogos` does not
find `catálogo`. The `with (...)` clause configures the analyzer per index:

```nql
-- Everything on for Spanish: stemming, stop words, accent folding
create index on Nota(cuerpo) type fulltext with (language = "spanish")

-- Each switch can be turned off (or on without a language, for folding only)
create index on Nota(cuerpo) type fulltext with (language = "spanish", stopwords = false)
create index on Nota(cuerpo) type fulltext with (ascii_folding = true)
```

| Option | Values | Effect |
|---|---|---|
| `language` | `"arabic"`, `"danish"`, `"dutch"`, `"english"`, `"finnish"`, `"french"`, `"german"`, `"greek"`, `"hungarian"`, `"italian"`, `"norwegian"`, `"portuguese"`, `"romanian"`, `"russian"`, `"spanish"`, `"swedish"`, `"tamil"`, `"turkish"` | Language of the stemmer and the stop-word list. Alone, it turns the three switches below on. |
| `stemming` | `true` / `false` | Reduce words to their stem (`catálogos`, `catálogo` → `catalog`). Needs `language`. |
| `stopwords` | `true` / `false` | Drop the language's stop words (`de`, `la`, `el`). Needs `language`; lists exist for danish, dutch, english, finnish, french, german, hungarian, italian, norwegian, portuguese, russian, spanish, swedish. |
| `ascii_folding` | `true` / `false` | Fold accents to ASCII (`clasificación` → `clasificacion`), so a query typed without accents matches. Works without a language. |

`with (...)` is only valid with `type fulltext`; an unknown option or a value of
the wrong kind is a parse error, never a silently ignored setting.

**Changing the analyzer requires reindexing.** The tokens on disk were produced
by the old chain, so there is no in-place change: `drop index Nota_cuerpo` and
create it again (it repopulates from the nodes). Creating an index that already
exists tells you exactly that. The analyzer is stored next to the index and
survives reopening the database; indexes created before 0.5.13 keep the default.

Rust: `graph.create_index_with(label, property, IndexType::FullText, IndexOptions { analyzer: Some(FullTextAnalyzer::for_language("spanish")) })` and `graph.describe_index(name)`. Python: `graph.create_index("Nota", "cuerpo", "fulltext", analyzer={"language": "spanish"})` and `graph.describe_index("Nota_cuerpo")`.

---

## Aggregations & Functions

NQL supports functions for data summarization, ideal for statistical analysis.

- `count(*)`: Counts total matches.
- `sum(p.amount)`: Sums numeric property values.
- `avg(p.age)`: Calculates average.
- `min(p.score)`, `max(p.score)`: Finds minimum and maximum values.
- `degree(n)`, `pagerank(n)`, `betweenness(n)`, `clustering(n)`: Graph analytics aggregations.
- `community(n)`: Exact global community detection (Louvain, modularity-based).
- `community_fast(n)`: Approximate local community detection for low-latency exploration.
- `leiden(n)`: Exact global community detection using **Leiden / CPM** (Traag et al. 2019). Guarantees internally well-connected communities. Uses a separate cache from `community()` — both can coexist in the same query. Default gamma = 0.1; use `LeidenCommunity::with_gamma(γ)` for custom resolution.
- `shortestPath("source-uuid", "target-uuid")`: Shortest distance between two nodes (returns `-1.0` when no path exists).

`community(n)` and `leiden(n)` compute global partitions, so `LIMIT` is applied after aggregation and does not reduce first-run cost.
Both functions reuse topology-versioned caches independently — a topology change invalidates both.

**Analysis Example:**
```nql
find count(*), avg(p.age)
from (p:User)
where p.active = true
```

```nql
-- Exploration (fast/approximate)
find community_fast(n) as cluster_fast
from (n)
limit 1

-- Exact Louvain partition
find community(n) as cluster
from (n)
limit 1

-- Leiden (Traag et al. 2019) — well-connected communities, deterministic
find n.name, leiden(n) as leiden_cluster
from (n)
order by leiden_cluster
limit 20

-- Compare Louvain vs Leiden in one query (independent caches)
find n.name, community(n) as louvain, leiden(n) as leiden_comm
from (n)
limit 10
```

---

## Data Export (EXPORT)

Export results to CSV or JSON.
Best practice is to place `export` at the end of the query.
The export path is optional.
If no path is provided, the result is returned inline.

### Syntax
```nql
find ...
from ...
export csv with path="path/to/file.csv", header=true, separator=","
```

```nql
find ...
from ...
export json with path="path/to/file.json", pretty=true
```

```nql
find ...
from ...
export json with jsonl=true
```

```nql
-- Not supported:
export csv with header=true
find ...
```

### Supported Formats
- **CSV**: Optional path. Options: `header=true|false`, `delimiter=","`.
- **JSON**: Optional path. Options: `pretty=true|false`, `jsonl=true|false`.
- **ARROW/PARQUET**: Not exported via NQL. Use the Graph API (`to_arrow()`, `export_parquet()`).

**Example (CSV export):**
```nql
find p.name, p.age
from (p:Person)
order by p.age
export csv with path="./users.csv", header=true
```

---

## Comments

```nql
// Single-line comment
/* Block comment */
```

---

## Data Types

| Type | Example | Note |
|------|---------|------|
| String | `"Text"` or `'Text'` | Use double or single quotes. |
| Int | `42` | Integers. |
| Float | `3.14` | Decimals. |
| Bool | `true`, `false` | Boolean logic. |
| Null | `null` | Absence of value. |

---

## Examples by Discipline

### 🔬 For Sociologists: Network Analysis
Finding opinion leaders in a community.
```nql
-- Who is "followed" by people who are themselves highly followed?
find leader.name
from (leader:Person) <- [:FOLLOWS] <- (follower:Person) <- [:FOLLOWS] <- (fan:Person)
limit 20
```

### 📊 For Marketers: Segmentation
Identify young customers who bought tech products.
```nql
find c.name, c.email
from (c:Customer) -> [:BOUGHT] -> (p:Product)
where c.age < 25 and p.category = "Technology"
```

### 🛡️ For Security Analysts: Fraud Detection
Detect suspicious transaction chains between accounts.
```nql
find a.id, b.id, c.id
from (a:Account) -> [:TRANSFERS] -> (b:Account) -> [:TRANSFERS] -> (c:Account)
where a.risk_flag = true and c.risk_flag = true
```

### 💻 For Developers: Backend API
Retrieve user profile and permissions.
```nql
find u.username, r.role_name
from (u:User {id: "12345"}) -> [:HAS_ROLE] -> (r:Role)
```

---

## Need Help?

- **Python Integration:** See `NQL_TUTORIAL.md`.
- **Github:** [NopalDB Repository](https://github.com/sharop/nopaldb)

**NopalDB** - *Graph data for everyone.* 🌵
