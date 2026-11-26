# Read-only Cypher

> Spanish version: [es/CYPHER.md](es/CYPHER.md)

NopalDB's query language is NQL ([reference](en/NQL_REFERENCE.md)). With the
optional `cypher` feature (off by default; it is not part of `full`) it also
reads a small subset of openCypher: `MATCH … WHERE … RETURN … ORDER BY … SKIP
… LIMIT`. The Cypher query is parsed with
[`open-cypher`](https://crates.io/crates/open-cypher) 0.5.0 and translated to
NQL text, which then runs like any NQL query, through the same validator and
executor.

```toml
nopaldb = { version = "0.11", features = ["cypher"] }
```

```rust
// The NQL it becomes:
let nql = nopaldb::query::cypher::to_nql(
    "MATCH (c:Cuidador)-[:CUIDA]->(a) WHERE a.edad > 3 RETURN a.nombre ORDER BY a.nombre",
)?;
assert_eq!(nql, "find a.nombre from (c:Cuidador)-[:CUIDA]->(a) where a.edad > 3 order by a.nombre");

// Or run it directly:
let result = graph.execute_cypher("MATCH (a:Animal) RETURN a.zona, count(*) AS n").await?;
```

Anything outside the subset is an error that names the rejected part. Nothing
is approximated or dropped.

From Python (the wheel includes `cypher` since 0.11.6):

```python
nql = nopaldb.Graph.cypher_to_nql("MATCH (a:Animal) WHERE a.edad > 3 RETURN a.nombre")
for row in graph.execute_cypher("MATCH (a:Animal) RETURN a.zona, count(*) AS n"):
    print(row["a.zona"], row["n"])
```

In Python a rejected query raises `RuntimeError`. The MCP server
[nopaldb-mcp](https://github.com/Anxious-Mind-Group/nopaldb-mcp) exposes it
as the `cypher_query` tool, which also returns the NQL it ran.

## What is accepted

Every row below is checked by `nopaldb/tests/cypher_read_only_test.rs`: the
NQL is what `to_nql` returns, and the test runs it against a small zoo graph
(names in Spanish: `Cuidador` keeper, `CUIDA` feeds, `edad` age).

| Cypher | NQL |
| --- | --- |
| `MATCH (a:Animal) WHERE a.edad > 3 RETURN a.nombre` | `find a.nombre from (a:Animal) where a.edad > 3` |
| `MATCH (c:Cuidador)-[:CUIDA]->(a) RETURN a.nombre ORDER BY a.nombre` | `find a.nombre from (c:Cuidador)-[:CUIDA]->(a) order by a.nombre` |
| `MATCH (a:Animal) RETURN a.zona, count(*) AS n ORDER BY n DESC` | `find a.zona, count(*) as n from (a:Animal) order by n desc` |
| `MATCH (a:Animal {nombre: 'michi'})-[:JUEGA_CON*1..2]->(b) RETURN b.nombre` | `find b.nombre from (a:Animal {nombre: "michi"})-[:JUEGA_CON]->{1,2}(b)` |
| `MATCH (a:Animal {nombre: 'tom'})-[:JUEGA_CON]-(b) RETURN b.nombre` | `find b.nombre from (a:Animal {nombre: "tom"})-[:JUEGA_CON]-(b)` |
| `MATCH (a:Animal)<-[:CUIDA]-(c) RETURN DISTINCT c.nombre` | `find distinct c.nombre from (a:Animal)<-[:CUIDA]-(c)` |
| `MATCH (c:Cuidador) MATCH (c)-[:CUIDA]->(a) WHERE a.especie = 'ave' RETURN c.nombre, a.nombre` | `find c.nombre, a.nombre from (c:Cuidador), (c)-[:CUIDA]->(a) where a.especie = "ave"` |
| `MATCH (a:Animal) RETURN a.nombre ORDER BY a.edad DESC, a.nombre SKIP 1 LIMIT 2` | `find a.nombre from (a:Animal) order by a.edad desc, a.nombre limit 2 offset 1` |
| `MATCH (a:Animal) WHERE a.especie IN ['ave', 'reptil'] OR a.edad <> 3 RETURN a.nombre` | `find a.nombre from (a:Animal) where (a.especie in ["ave", "reptil"]) or (a.edad != 3)` |
| `MATCH (a:Animal) RETURN min(a.edad) AS menor, max(a.edad) AS mayor, sum(a.edad) AS total` | `find min(a.edad) as menor, max(a.edad) as mayor, sum(a.edad) as total from (a:Animal)` |
| `MATCH (a:Animal WHERE a.zona = 'norte')-[r:JUEGA_CON]->(b) RETURN type(r) AS t, b.nombre` | `find r.type as t, b.nombre from (a:Animal)-[r:JUEGA_CON]->(b) where a.zona = "norte"` |
| `MATCH (a:Animal) WHERE a.edad IS NULL RETURN a.nombre` | `find a.nombre from (a:Animal) where !(a.edad != null)` |
| `MATCH (a:Cuidador) RETURN a, id(a) AS i` | `find all(a), a.id as i from (a:Cuidador)` |

In detail:

- **Patterns:** a node with zero or one label and a map of literal properties;
  `->`, `<-` and undirected `-` relationships, with a type or a union
  `:A|B`; variable length with both bounds (`*2`, `*1..3`, `*..3`); several
  patterns and several `MATCH` clauses, which share variables. A `WHERE`
  inside a pattern joins the query's `WHERE`.
- **Expressions:** literals (strings, integers in any base, floats, booleans,
  `null`, lists), `n.prop`, `= <> < <= > >=`, `AND OR NOT`, `IN` with a
  literal list, `+ - * / %`, `IS [NOT] NULL`, and the functions `count`
  (also `count(*)`), `sum`, `avg`, `min`, `max`, `id`/`elementId` and `type`.
- **Projection:** `RETURN [DISTINCT]` with `AS` aliases or `*`; `ORDER BY`
  with `ASC`/`DESC`; `SKIP`/`OFFSET` with `LIMIT`. Aggregates group by the
  other columns, as in Cypher.

## What is rejected

| Cypher | Why |
| --- | --- |
| `CREATE`, `MERGE`, `SET`, `DELETE`, `REMOVE`, `FOREACH`, `LOAD CSV` | read-only; writes are NQL `add`, `upsert`, `update`, `delete` |
| `OPTIONAL MATCH`, `WITH`, `UNWIND`, `UNION`, `CALL` | no NQL equivalent yet |
| `$param` | no parameters; put the value in the query |
| `STARTS WITH`, `ENDS WITH`, `CONTAINS`, `=~`, `XOR`, `^`, `\|\|` | no NQL operator |
| `count(DISTINCT x)`, `collect`, `size`, `labels`, `coalesce`, `toUpper`, … | no NQL function |
| `(n:A:B)`, `(n:!A)`, `(n:%)` | a node has one label in NopalDB |
| `p = (a)-->(b)`, `shortestPath(…)` | no path values; for a shortest path NQL has `find path(a, b) from (a), (b) where …` |
| `-[*]->`, `-[*0..2]->`, `-[*2..]->`, `-[r*1..2]->` | NQL needs at least one hop and an upper bound, and does not bind the list of edges |
| `SKIP` without `LIMIT`, a non-literal `SKIP`/`LIMIT` | NQL `offset` goes with `limit` |
| `RETURN n AS m`, `RETURN n, count(*)` | a whole node has no alias in NQL; group by its properties |
| `` `a name` `` with spaces or symbols | NQL names are letters, digits and `_` |
| `EXPLAIN`, `PROFILE` | translate with `to_nql` and run `explain <nql>` |

## Where the result differs from Cypher

- **`RETURN n`** gives the columns of `all(n)`: `n.id`, `n.label` and one per
  property, not a node object.
- **Column names** of functions follow NQL: `type(r)` is the column `r.type`
  and `id(n)` is `n.id`. Use `AS` to name them.
- **Missing properties.** NQL uses two-valued logic: a comparison with a
  missing property is false, not `null`. So `NOT a.edad = 3` keeps the nodes
  without `edad`, where Cypher drops them. Write
  `a.edad IS NOT NULL AND NOT a.edad = 3` to get the Cypher result.
- **`sum` of integers** is a float (`8.0`, not `8`).
- **Class nodes.** `(n:Label)` does not match class nodes (`NodeKind::Class`,
  e.g. an imported `owl:Class`) since 0.11.5; add `is_class(n)` in NQL to get
  them.
- **Ids** are NopalDB ids (UUID strings), not integers.

Nulls sort as in Cypher: last with `ASC`, first with `DESC`.

## Coverage

Of the 994 read-only queries in the openCypher 2024.3 TCK that start with
`MATCH`, 237 translate and 231 of those run in NQL. The other six fail
with an NQL error, for example an aggregate in `ORDER BY` that is not a
column. The most common rejections are `WITH` (284 queries) and path
variables (193). This is a syntax corpus: it says which shapes of query get
through, not that the rows match Cypher's (the tests above check that).
