# Upgrading to 0.9.3

> Español: [docs/es/ACTUALIZAR_0.9.md](es/ACTUALIZAR_0.9.md).

This page is for an application that runs NopalDB 0.6.x, 0.7.x or 0.8.x and
moves to **0.9.3**. Most upgrades need no code change: the on-disk format is
the same, and the Python API only gained methods. What deserves attention are
the fixes that make a query return **different rows** than before, and two
old bugs that may have **already written wrong data**. Each item links to its
[CHANGELOG](../CHANGELOG.md) entry.

Install:

```bash
pip install -U "nopaldb==0.9.3"          # Python 3.10+
pip install -U "nopaldb[networkx]==0.9.3" # with the NetworkX conversion
```

```toml
nopaldb = "0.9"   # Rust; 0.9.3 or later
```

## 1. What does not change

- **redb databases open as they are.** No migration and no rebuild. The
  layout, the WAL and the property index are the same since 0.6.
- **The HNSW index dump is the same format.** It is loaded on open as before.
  0.9.3 ships an internal copy of `hnsw_rs` 0.3.4 with a deadlock fix, and the
  dump code is unchanged.
- **The Python API only grew.** No Python method was removed or renamed
  between 0.6 and 0.9.3.

## 2. Check your data first: two bugs that wrote wrong data

Both are fixed, but upgrading does not repair what they already wrote. They
only affect writes made **through NQL**. Writes through the Python or Rust
API (`add_node`, `upsert_many`, dict properties, `delete_node`) were not
affected.

### Negative numbers lost their sign (fixed in 0.8.3)

Before 0.8.3, a negative number in NQL lost its `-` in `where`, `set` and
`delete` ([0.8.3](../CHANGELOG.md#083---2026-10-23)):

| NQL before 0.8.3 | What it did |
|---|---|
| `update … set m.temp = -7` | wrote **7** |
| `delete … where m.x = -5` | deleted the nodes with **x = 5** |
| `where m.temp < -1` | matched 0 as well |

Lists (`in [-5]`) and property maps (`{temp: -7}`) were not affected.

**How to check:** search your code for NQL strings that contain a negative
number literal (for example the regex `[=<>,(]\s*-\d`). For each `set` with a
negative value, the stored value is positive: write it again from your source
of truth. For each `delete … where … = -N`, the nodes with `N` were deleted
instead.

### DELETE and UPDATE with a pattern wrote too much (fixed in 0.8.4)

Before 0.8.4, `delete` and `update` with a pattern used only the labels and
the edge type ([0.8.4](../CHANGELOG.md#084---2026-10-23)). They ignored the
direction, the inline properties and every hop after the first:

- `delete (c:Chunk)<-[:MENTIONS]-(e:Entity)` deleted the edges `c -> e`, even
  with no edge in that direction;
- `delete …-[:MENTIONS {w: 1}]->…` also deleted the edges with `w = 2`;
- a two-hop pattern was cut to its first hop.

**How to check:** search your code for `delete (` and `update (` followed by a
pattern with `<-`, `<->`, `{…}` on an edge, or more than one hop. If you used
one, edges or properties outside the pattern may be gone or changed. In Rust,
`Graph::edge_history`, `Graph::history` and `Graph::as_of(timestamp)` show
earlier versions if garbage collection has not removed them. Python has no
time-travel API, so rebuild from your source of truth.

Since 0.8.4, a pattern means the same thing in reads and writes, and a write
pattern with more than one hop or a quantifier is an error.

## 3. Queries that return other rows with the same text

These are fixes: the new result is the correct one. If your application, a
test or a stored expectation depends on the old result, it will see the
difference.

| Since | Query | Before | Now |
|---|---|---|---|
| [0.7.0](../CHANGELOG.md#070---2026-10-21) | `where hybrid(c, …, k = 10) and c.tenant = "t7"` (also `similar_to`, `c.id in [...]`, inline props) | scored the whole label, then dropped other tenants: fewer than `k` rows, often none | the top 10 **within** the tenant; `rank(c)` / `score(c)` computed in that universe |
| 0.7.0 | `where hybrid(…) or c.x = "B"` | silently returned only the search hits | named error: a search must be a condition of the top-level `and` |
| [0.8.4](../CHANGELOG.md#084---2026-10-23) | `(e:Entity)<-[:MENTIONS]-(c:Chunk)` (one hop) | ignored `<-`: wrong or no rows | follows incoming edges, like multi-hop patterns |
| 0.8.4 | `-[:T {w: 1}]->` in a pattern of two or more hops | matched `w = 2` too | only `w = 1` |
| 0.8.4 | `find distinct r.region … limit 3` | limit before `distinct`: could return 1 row | 3 distinct rows |
| [0.8.3](../CHANGELOG.md#083---2026-10-23) | `where a and (b or c)` | parse error | groups |
| 0.8.3 | `r.p * 2`, `-expr`, `!x`, `not (…)` | silently ignored (`r.p * 2 = 6` ran as `r.p = 6`; `not (…)` let every row through) | **named error** |

Hybrid search with selective filters also returns more of the allowed
documents since 0.6.13 (`search_hybrid` and the NQL `hybrid()`): results that
used to be missing now appear. If you cut by a `score(c)` threshold with
`hybrid`, expect more rows above it.

The meaning of a query is written down in
[NQL_SEMANTICS.md](NQL_SEMANTICS.md): rows per path, the search universe and
the order of evaluation.

## 4. Arrow export: other columns and values (0.9.1)

`to_arrow_complete` (Rust `graph_to_arrow`), `nodes_to_arrow_with_properties`
and `edges_to_arrow_with_properties` gave wrong or incomplete data
([0.9.1](../CHANGELOG.md#091---2026-10-24)). Now:

- with a label, only the edges with both endpoints inside the label are
  exported: the induced subgraph. Before, every edge of the graph came out;
- property columns are sorted by name, so the same graph gives the same
  schema;
- lists, objects and bytes are JSON, not `<complex>` / `<bytes:N>`, and an
  explicit `Null` is an Arrow null, not the text `"null"`;
- a property named like a structural column (`id`, `label`; `source`,
  `target`, `edge_type` on edges) is exported as `property.<name>`;
- a column with mixed types is text instead of losing values;
- an empty export returns an empty batch with its schema instead of an error.

**Check:** code that reads columns by position, compares with `<complex>` or
`"null"`, or reads a property named `id` / `label` from the export.

## 5. Rust API changes

- **Removed in 0.9.0** ([0.9.0](../CHANGELOG.md#090---2026-10-24)):

  | Removed | Use instead |
  |---|---|
  | `Graph::update_node_with_tx` (did nothing) | `tx.add_node(node)` with the same id |
  | `Graph::delete_node_with_tx`, `Graph::add_node_with_label_and_props` (ignored the transaction) | `tx.delete_node(id)` / `tx.add_node(...)`, or `Graph::delete_node` / `Graph::add_node` |
  | `mvcc::VersionManager` (empty placeholder) | `Graph::get_node_at`, `Graph::history` |
  | `HnswIndex::repaired()` (always 0) | `HnswIndex::orphans()` |

- **`ExpandOptions` has a new field, `within`** (0.9.0). Code that builds it
  with `..Default::default()` does not change; a literal that names every
  field without it stops compiling.
- **`hnsw_rs` is no longer a dependency** (0.9.3). No NopalDB API exposed its
  types. If your crate used `hnsw_rs` without declaring it, because it came
  in through NopalDB, add it to your own `Cargo.toml`.
- **Do not enable `hnsw-simd`.** It does not build outside x86 (a bug in
  `anndists`), and NopalDB's cosine distance has no SIMD path, so it speeds
  nothing up.
- **`import_parquet`** (0.8.5) now imports every row in one transaction, and
  rejects a file with nulls, invalid or repeated ids, or ids that already
  exist. Before, it kept only the first 1 024 rows and replaced existing
  nodes with empty ones.

## 6. sled databases

A database created with 0.5.x (or with `engine="sled"` on 0.6–0.7) no longer
opens since 0.8.0: the error says so and gives the recipe. Migrate it once
with 0.7, then open the copy with 0.9.3. See
[MIGRATION_0.6.md](MIGRATION_0.6.md).

## 7. After upgrading: what to test

Run these against a copy of your real database. The checklist is also what
we ask you to report back:

1. **Open and count.** The database opens, and node and edge counts match the
   old version.
2. **Vector search after a restart.** Run `knn_nodes` or `search_hybrid`,
   close the database, reopen it and run the same query: same ids. This
   checks the HNSW dump, which is loaded on open.
3. **Hybrid search with your real filters** (tenant, document, ACL): the
   number of rows, and whether `k` rows come back where fewer did before.
   `search_hybrid_explain` / NQL `explain` show the path taken.
4. **Your NQL writes** (`update … set`, `delete` with a pattern) on a copy:
   they change exactly what you expect. Review sections 2 and 3 first.
5. **Exports** (Arrow, NetworkX) that feed another tool: the downstream step
   still reads the columns it needs.
6. **Times** for the operations you care about (ingest, index build, query),
   old version against new.

**What to report:** the version you upgraded from; the OS and CPU (Linux,
macOS or **Windows**, x86_64 or ARM); what failed, with the error text; and
the times from point 6. Windows reports are especially useful: the tests run
on Linux only, and Windows wheels are built but not tested.
