# Upgrading to 0.10

> Español: [docs/es/ACTUALIZAR_0.10.md](es/ACTUALIZAR_0.10.md).

This page is for an application that runs NopalDB **0.9.4** and moves to
**0.10**. Coming from 0.9.3 or earlier, read
[UPGRADING_0.9.md](UPGRADING_0.9.md) first: everything there still applies.
Already on 0.10.0? Section 8 lists what 0.10.1 changes.

0.10 fixes the MVCC history (the versions behind `history`, `as_of` and
`get_node_at`), the GC, and the durability of NQL `update`. It is a minor
release because two Rust APIs changed in incompatible ways (section 2). Each
item links to its issue.

Install:

```bash
pip install -U "nopaldb==0.10.1"   # Python 3.10+
```

```toml
nopaldb = "0.10"
```

## 1. What does not change

- **Databases written by 0.9.4 open with no step on your side.** The
  on-disk format of nodes, edges, indexes and the WAL is the same; a pending
  WAL left by a 0.9.4 crash is replayed correctly by 0.10. Since 0.10.1 the
  first open writes once to the database: it adopts the history of data
  written without versions (section 3).
- The Python API only grew (section 4).

## 2. Rust code that may stop compiling or behave differently

- **`GCConfig` has a new public field, `min_age_ms`**
  ([#310](https://github.com/sharop/nopaldb/issues/310)). A struct literal
  that names every field without `..Default::default()` stops compiling. So
  does building `GcAutoSummary` or `GcSection` by hand: both gained fields.
  Use the constructors (`GCConfig::older_than_days(7)`,
  `GCConfig::before_timestamp(ts)`) or `..Default::default()`.
- **`get_node_at` and `as_of(ts)` return errors instead of the current
  node** ([#311](https://github.com/sharop/nopaldb/issues/311)). Until 0.9.4
  they returned the node as it is now when they found no version valid at
  `ts`, which was a silent wrong answer.

  | `graph.get_node_at(id, ts)` / `graph.as_of(ts).get_node(id)` | 0.10 |
  |---|---|
  | a version valid at `ts` exists | that version |
  | the node did not exist at `ts` (created or bulk-loaded later, or already deleted) | `NopalError::NodeNotFound` |
  | `ts` is below the node's history floor (data written before 0.10.1, adopted on the first open; section 3), or below the GC horizon | `NopalError::HistoryUnavailable` (new) |

  `GraphSnapshot::get_nodes` skips only `NodeNotFound` and returns any other
  error; it used to swallow every error. `NopalError` is `#[non_exhaustive]`,
  so a `match` already has a `_` arm.

## 3. Behaviour that changed

### GC by age keeps recent history ([#310](https://github.com/sharop/nopaldb/issues/310))

Until 0.9.4, `GCConfig::older_than_ms/hours/days` and `Graph::gc_default`
deleted **all** closed history: they compared Unix milliseconds with the
logical timestamps of the versions. History already deleted that way cannot
come back.

0.10 translates the age to the logical clock with wall-clock marks the
database now stores, at most one per second. The translation is
conservative: it only deletes what the database can prove was closed at
least that long ago.

**Consequence for existing databases: GC by age reclaims nothing at first.**
Nothing is reclaimed until the requested age has passed since the first write
made with 0.10, because there are no marks before that. If you rely on GC to
free space, expect no versions removed during that first period.
`GCConfig::before_timestamp(ts)` deletes by logical timestamp right away.

### Deleting a node closes its history ([#309](https://github.com/sharop/nopaldb/issues/309))

Every delete path closes the node's version, and the versions of the edges
deleted with it, at the delete's timestamp: direct `delete_node`,
transactions, NQL `delete`, Python `delete(label, key, value)` and WAL
replay. Re-creating a node with the same id continues its version chain.

WAL replay now re-applies every write with the timestamp recorded in the WAL.
Until 0.9.4, edges and node deletes got new timestamps on replay.

**Nodes deleted before 0.10 have no recorded delete timestamp.** Their
history was left open. Since 0.10.1 the first open closes it at the adoption
timestamp F and gives the node a history floor at F (next section): from F on
the node is `NodeNotFound`, and before F `HistoryUnavailable`, because the
real delete date is unknown. With 0.10.0, `as_of(ts)` after their delete
still finds them.

### NQL `update … set` is a transaction ([#312](https://github.com/sharop/nopaldb/issues/312))

Like `add` and `delete`, each `update` statement is now one transaction:
- it is written to the WAL, so an acknowledged update survives a crash (until
  0.9.4 it could be lost before the next checkpoint);
- it leaves an MVCC version;
- it keeps the hash/btree/full-text indexes created with `create_index` up to
  date (a hash index used to keep finding the node by its old value);
- it is atomic: a failure in the middle writes nothing.

Under `Serializable`, an `update` now takes the write lock of each node it
changes. Since 0.10.1, rewriting an edge's properties chains a new version
(0.10.0 still replaced it).

### Every write leaves history (0.10.1, [#319](https://github.com/sharop/nopaldb/issues/319))

0.10.0 still wrote some data with no version, so time travel over it
answered `HistoryUnavailable`. Since 0.10.1 every write leaves history:

- **Direct `add_node`** and the **Turtle/OWL import** write a version, like
  a transaction.
- **The BulkLoader** does not copy each new node or edge into a version
  (that cost +62 % disk). It stores an 8-byte **birth mark** with the load's
  timestamp, one per flush: the record is valid from that timestamp, and
  before it the node did not exist (`NodeNotFound`). The first later write
  or delete of that node turns the mark into a real first version. An id
  that already exists continues its chain as usual.
- **Edges keep a version chain.** Rewriting an edge writes the next version,
  and re-creating a deleted edge continues from its last version. In 0.10.0
  both overwrote version 1.

**History adoption on the first open.** A database written before 0.10.1
has nodes and edges with no version (direct writes, BulkLoader, Turtle
import) and chains that no longer match the current record. The first open
with 0.10.1 takes one logical timestamp F from the clock and:

- gives each node or edge with no version a **history floor** at F, with no
  copy: from F on, reads return the current record; before F they return
  `HistoryUnavailable`, because what happened before is unknown;
- adds a version at F to a chain whose last version differs from the record
  (and gives it the floor F);
- closes at F the open history of nodes and edges already deleted (and gives
  them the floor F).

History that was written by transactions and matches the record gets no
floor and keeps its whole past. For a node with no chain, `history(id)`
shows one version that starts at F. `last_commit_timestamp()` reaches F.

The pass runs inside `open`, after WAL replay and before `open` returns, also
with `open_read_only`. It reads every node and edge once: about 260 ms for
100k nodes and 100k edges, about 30 ms on later opens. It runs in chunks, so
an open killed during the pass resumes it with the same F next time, and it
does not run again once it has finished.

**BulkLoader cost.** Measured with 200k nodes × 5 properties: nodes +6 %
time and +6 % disk; edges +23 % time, from two extra reads per edge to tell a
new edge from an existing one.

## 4. New

- `Graph::last_commit_timestamp()` (Rust and Python): the logical timestamp
  of the latest applied write, the value to pass to `as_of` / `get_node_at`.
  `begin_transaction`, rollbacks and empty commits do not move it.
- `Graph::gc_horizon()` (Rust and Python; also `stats().gc.horizon`,
  `get_stats()["gc"]["horizon"]`): how far the GC collected. History from
  that timestamp on is complete.
- `GCConfig::before_timestamp(ts)`: a GC cutoff in logical time.

## 5. Still limited

- **History before the adoption is not recovered.** Data written before
  0.10.1 without versions answers `HistoryUnavailable` before the floor F
  (section 3). There is no record of that past to rebuild it from.
- **The low-level `Storage::insert_node` / `insert_edge` primitives still
  write no version.** Overwriting a node or edge that only has a birth mark
  or a floor with them removes the mark, so that entity reads
  `HistoryUnavailable` from then on. Use the `Graph` API.
- **NQL `find … at timestamp <n>`** was a named error until 0.10.1. Since
  0.10.2 it reads the graph as it was at that timestamp, also from Python,
  except for vector, full-text and community searches
  ([#298](https://github.com/sharop/nopaldb/issues/298),
  [NQL reference § Time travel](en/NQL_REFERENCE.md#time-travel-at-timestamp)).

## 6. Going back to 0.10.0 or 0.9.4

Both open a database written by 0.10.1, because the format is the same: the
wall-clock marks, the birth marks, the floors and the adoption marker are
new keys they ignore. Ignoring them has costs, so **go back to the copy you
took before upgrading** rather than to the upgraded database:

- **They do not see birth marks or floors.** On 0.10.0, bulk-loaded and
  adopted data answers `HistoryUnavailable` again; on 0.9.4 `get_node_at`
  falls back to the current node.
- **Their writes break the implicit history.** A write by 0.10.0 or 0.9.4
  to a node or edge that only has a birth mark or a floor starts a chain
  without first saving the record the mark stood for. Back on 0.10.1, that
  entity reads as not existing between the mark and that write.
- **Their unversioned writes are not adopted.** The adoption runs once; a
  direct `add_node` made by 0.10.0 afterwards has no history on 0.10.1.
- **0.9.4 only:** re-creating a deleted id re-dates the closed version, and
  GC by age deletes all closed history (section 3).

## 7. What to test

On a copy of your database:
1. it opens and the counts match;
2. a GC by age you run regularly removes what you expect, given the warm-up
   above;
3. code that calls `get_node_at` / `as_of` handles `NodeNotFound` and
   `HistoryUnavailable`;
4. your NQL `update` statements still change exactly what you expect;
5. the first open with 0.10.1 takes the time you can afford (section 3: the
   adoption reads every node and edge once), and the next open is fast
   again.

## 8. From 0.10.0 to 0.10.1

No incompatible API changes. New in Rust: `Graph::history_floor(id)` and
`Graph::edge_history_floor(id)` return the floor of a node or edge (`None`
if it has none). What changes:
- every write leaves history, the BulkLoader with a birth mark, and edges
  keep a version chain (section 3);
- the first open adopts the data 0.10.0 wrote without versions: it writes
  once, takes time proportional to the database, and puts a history floor
  on that data (section 3);
- reads below a floor answer `HistoryUnavailable`; a node bulk-loaded after
  `ts` answers `NodeNotFound` (section 2);
- `ELReasoner::from_graph_at(ts)` below a floor answers
  `HistoryUnavailable` instead of using the current taxonomy;
- do not write to the database with 0.10.0 after opening it with 0.10.1
  (section 6).

## 9. From 0.10.2 to 0.10.3

No API or disk format changes. NQL stops dropping things without an
error, and some queries return different rows or columns:

| Query | 0.10.2 | 0.10.3 | What to do |
|---|---|---|---|
| `find degree(n) as d from (n:L)` (also `pagerank`, `betweenness`, `clustering`, `community`, `leiden`) | one row with the average | one row per node ([#336](https://github.com/sharop/nopaldb/issues/336)) | for the average write `find avg(degree(n)) as d` |
| `find degree(n) as d, n.name from (n:L)` | grouped by `n.name`: nodes with the same name merged | one row per node | add `group by n.name` and `avg(...)` if you meant the group |
| `find n.name as j …` (Rust) | the row had `j` and `n.name` | `j` only ([#331](https://github.com/sharop/nopaldb/issues/331)) | read `j`; Python and `export` already gave `j` only |
| `find n.name, count(*) as c … group by n.name` | columns `n.id`, `n.name`, `c` | `n.name`, `c` | ask for `n.id` if you used it |
| `find foo(n) …`, `id(n)`, `COUNT(*)` | column dropped, `where` passed every row, empty row | `SemanticError`; `COUNT(*)` is `count(*)` ([#328](https://github.com/sharop/nopaldb/issues/328)) | use `n.id` instead of `id(n)` |
| `update … set n.x = n.name` | OK, nothing updated | `SemanticError` ([#330](https://github.com/sharop/nopaldb/issues/330)) | only literals in `set` for now |
| `find all(n)`, `find *` with an indexed `where` | empty rows; only `n.id`, `n.label` in Python | every property | — |

## 10. From 0.10.3 to 0.10.4: the columns of a result

The columns of a result come from `find`, the same whatever path runs the
query ([#335](https://github.com/sharop/nopaldb/issues/335)). No API or
disk format changes; names and order change in Rust, Python and `export`,
and some queries that answered are now errors:

| Query | 0.10.3 | 0.10.4 | What to do |
|---|---|---|---|
| `find count(*), sum(n.age) …` | columns `count`, `sum` | `count(*)`, `sum(n.age)` | give an alias (`count(*) as c`) if you read the column by name |
| `find degree(n), n.name …` | column `degree` | `degree(n)` | same |
| `find count(*) as c, n.city … group by n.city` | `n.city`, `c` | `c`, `n.city`: the order of `find` | read by name, or write `find` in the order you want |
| `find *` (Rust; Python already gave every key) | columns `["*"]` | `n.id`, `n.label`, then the properties sorted | — |
| `find n.a as x, n.b as x` | one `x`, the other value lost | `SemanticError` | give different aliases |
| `find n.age, count(*) … group by n.city` | `n.city` and `count`, `n.age` lost | `SemanticError` | add `n.age` to `group by` or aggregate it (`avg(n.age)`) |
| `find n.city, pagerank(n) … group by n.city` | average of the group | `SemanticError` | write `avg(pagerank(n))` |
| `find n.name … order by degree(n)` | not sorted | `SemanticError` | project it (`degree(n) as d`) and `order by d` |
| `find … order by count(*)` with `count(*)` in `find` | not sorted | sorted | — |
| `find n.name, 1 …` | column dropped | constant column `1` | — |

## 11. From 0.10.4 to 0.10.5: arithmetic, negation and several patterns

NQL evaluates `+ - * / %`, unary minus, `not` and `!`
([#269](https://github.com/sharop/nopaldb/issues/269)). Queries that were a
parse error now answer; a few that answered change:

| Query | 0.10.4 | 0.10.5 | What to do |
|---|---|---|---|
| `where r.servings * 2 = 6`, `find r.servings * 2 as d`, `order by -r.servings`, `having count(*) * 2 > 3`, `where not r.region = "south"`, `where !(…)` | parse error | evaluated | — |
| `set r.servings = r.servings + 1` | `SemanticError` | writes the new value | — |
| `path_eval` / `gather "x = a / b"` with two integers | float (`7 / 2` = `3.5`) | integer (`3`) | write one operand as a float: `a / 100.0`, or `init "x = 100.0"` |
| `where n.x = 3.0` on an integer property (also `in [3.0]`) | no match | match: numbers compare by value | — |
| integer overflow, `/ 0`, `% 0` in a `gather` | panic in debug, wrapped value in release / named error | named error | — |
| a variable named `not` | allowed | `not` is a keyword | rename the variable |
| `where not similar_to(…)` (also `hybrid`, `in_community`) | parse error | `SemanticError`: a search picks candidates, it cannot be negated | — |
| `where r.servings + 1` (arithmetic as a condition) | parse error | `SemanticError`: compare it | `where r.servings + 1 > 0` |
| `find distinct r.servings % 2`, or `distinct` with `order by` an expression | parse error | error: not supported yet | — |
| `delete (n:L) where true` (also `update`) | deleted nothing: the write WHERE read anything but a comparison as `false` | deletes every `L`, as `find` reads it | check scripts that relied on it |

Several patterns in `from` that are all single nodes are joined
([#345](https://github.com/sharop/nopaldb/issues/345)), as they already were
when a pattern had an edge:

| Query | 0.10.4 | 0.10.5 | What to do |
|---|---|---|---|
| `find a.name, b.name from (a:Recipe), (b:Recipe)` | the rows of `a`, `b` empty | the cross product (N·M rows) | relate the patterns in `where` if you expect fewer rows |
| `… where a.servings < b.servings` (a condition on both) | no rows | the pairs that pass | — |
| `find count(*) from (a:Recipe), (b:Recipe)` | counted `a` only | named error: aggregations over several patterns are not supported yet | count over one pattern |

## 12. From 0.10.5 to 0.10.6: the join of several patterns

The join of several patterns in `from` filters while it combines and has a
row limit ([#349](https://github.com/sharop/nopaldb/issues/349)). The rows do
not change; what changes:

| Situation | 0.10.5 | 0.10.6 | What to do |
|---|---|---|---|
| `from (a:X), (b:X) where … limit 10` over large labels | the whole product in memory (2000 × 2000: 10 GB, 4 s) | only the rows that pass (18 MB, 10 ms) | — |
| more than `Graph::max_join_rows()` rows pass the `where` in a join (400 000; 100 000 with `Mobile`) | ran out of memory | named error | relate each pattern to the ones before it in `where`, or `set_max_join_rows(n)` |
| `from (a:X), (b:Y) order by a.x limit 10` with more than the limit of pairs (`order by` and `distinct` need every row) | answered, with memory to spare | named error | relate the patterns, or `set_max_join_rows(n)` |
| `from (a), (b), (c) where a.k = c.k` (the condition skips a pattern) | answered, with memory to spare | the join of `a` and `b` is the whole product and counts | write `(a), (c), (b)` |
| `explain` of several single-node patterns | `Strategy: LABEL SCAN` | `Strategy: PATTERN PIPELINE` and a `Join: …` line | update parsers of the plan text |
