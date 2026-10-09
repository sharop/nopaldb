# GraphRAG on NopalDB: the retrieval cycle

A GraphRAG answers a question in four steps: search (vector, full-text or
both), hydrate the hits, expand their neighbourhood, and hand the resulting
subgraph to the model as context. Since 0.6.8 each step is one call, from
Python or from NQL, and none of them scans the graph.

## In Python

```python
import nopaldb

g = nopaldb.Graph.open("data/kb.db")
q = embed(question)                                   # your embedding model

# 1. Search: full-text + vector fused with RRF, each hit with its node.
hits = g.search_hybrid(text=question, vector=q, model="m", k=10, hydrate=True)
chunks = [h["node"] for h in hits if h["node"]]

# 2. Expand: entities mentioned by the hits and what they relate to.
ctx = g.neighborhood([h["node_id"] for h in hits], depth=2,
                     edge_types=["MENTIONS", "RELATED"], labels=["Entity"],
                     max_nodes=200, max_edges_per_node=50)

# 3. Context: chunks + entity subgraph.
context = render(chunks, ctx["nodes"], ctx["edges"])
```

- `search_hybrid(..., hydrate=True)` and `knn_nodes(..., hydrate=True)` read
  the hit nodes in the same call. Without `hydrate` the shapes are the 0.6.x
  ones (dicts with `node_id`/`score`, tuples `(id, distance)`).
- `get_node(id)`, `get_nodes(ids)`, `get_edge(id)`, `get_edges(ids)` are
  point reads: `{"id", "label", "properties"}` / `{"id", "source", "target",
  "type", "properties"}`, `None` where an id does not exist, input order kept.
- `neighborhood(ids, depth, direction, edge_types, labels, max_nodes,
  max_edges_per_node)` is a BFS by node with a global visited set: a node
  reachable by several paths appears once, at its minimum depth; edges are
  filtered by type before their target is read; a node whose label is
  filtered out is neither returned nor expanded; `max_nodes` caps the result
  (`truncated` tells you) and `max_edges_per_node` is the brake on a
  super-node. `neighbors(id)` and `degree(id)` are the one-hop shortcuts.
- **Ranking the expansion (0.6.10):** with `max_nodes` cutting, BFS keeps
  whatever comes first in level order. `rank="ppr"` keeps the most relevant
  nodes instead: the BFS gathers up to `candidate_factor × max_nodes`
  candidates (5× by default), a Personalized PageRank seeded at the hits runs
  over them in memory, and the best by score are kept (seeds always stay,
  ties break by node id). A node two hops away that several hits point to can
  then beat one hop away that only one hit points to. Pass the search score
  as `seed_weights` to favour the best hits; `ctx["score"]` has each node's
  score.

  ```python
  ctx = g.neighborhood([h["node_id"] for h in hits], depth=2, direction="both",
                       max_nodes=50, rank="ppr",
                       seed_weights={h["node_id"]: h["score"] for h in hits})
  ```

## In NQL

```sql
-- search and expand in ONE query: the question's vector is written in it (0.6.9)
find c.text, e.name from (c:Chunk)-[:MENTIONS]->(e:Entity)
where similar_to(c, vector = [0.12, -0.03, ...], model = "minilm", k = 10)

find c.text from (c:Chunk)
where hybrid(c, text = "drip irrigation", vector = [...], model = "minilm", k = 10)

find c.text from (c:Chunk) where c.id in ["<uuid>", "<uuid>"]
find e.name, e.kind from (c:Chunk)-[:MENTIONS]->(e:Entity) where c.id = "<uuid>"
find e.name from (e:Entity) where e.name in ["ana", "beto"]     -- uses the hash index
```

`similar_to(var, vector = [...], model = "…", k = N)` and
`hybrid(var, text = "…", vector = [...], model = "…", k = N)` take the
question's embedding as a list literal (integers and `1e-05` parse; the
length must match the model's index). The candidates come out best first and,
in a one-hop pattern, they **seed the pipeline**: only the top-K chunks are
expanded (`EXPLAIN` says `PATTERN PIPELINE (seed: SIMILAR_TO)`). Give `k`
explicitly in a pattern query: there `LIMIT` caps the expanded rows. The
search must be on the first node of a single one-hop pattern; other shapes
are validation errors (before 0.6.9 they silently returned the whole graph).
This is what an agent behind the MCP server uses: one round trip.

`where var.id = "…"` and `where var.id in [...]` resolve by point read: no
scan, and a missing id is zero rows (`EXPLAIN` says `ID LOOKUP`). In a
pattern query the same condition on the source node seeds the pipeline with
those nodes instead of scanning the label (`EXPLAIN` says `PATTERN PIPELINE
(seed: ID LOOKUP)`). `in` /
`not in` take a list literal; with a user index on the property, `EXPLAIN`
says `INDEX SEEK (IN)`. Equality is strict, as with `=`: `1` is not `1.0`.
A root `AND` seeds the candidates with its indexed side and applies the rest
as a predicate.

## Global search (0.6.11)

The local cycle answers questions about specific entities. A global question
("what are the main topics?", "how do we organize irrigation?") needs a view
of the whole corpus: GraphRAG answers it with **community reports**. You
detect communities, have the LLM summarize each one, and at query time run a
map-reduce over the relevant summaries. NopalDB does not call the LLM: it
stores the communities and the reports, tells you which reports are stale,
and searches them.

```python
import nopaldb

g = nopaldb.Graph.open("data/kb.db")

# 1. Communities: hierarchy (level 0 = coarsest) persisted with stable keys.
levels = g.leiden_hierarchy(labels=["Entity"], edge_types=["RELATED"], max_cluster_size=10)
g.materialize_communities(levels)          # (:Community {partition, level, key, size})

# 2. Reports: only for communities without one, or whose content changed.
for s in g.stale_reports():                # [{"status", "community_key", "level", ...}]
    if s["status"] == "orphan":            # its community no longer exists
        g.delete("Report", "community_key", s["community_key"])
        continue
    members = g.execute_nql(
        f'find e.name, e.description from (e:Entity)-[:IN_COMMUNITY]->(c:Community) '
        f'where c.key = "{s["community_key"]}"')
    title, summary, rating = llm_summarize(members)                 # your LLM
    g.upsert_community_report(s["community_key"], title, summary, rating=rating,
                              vector=embed(summary), model="m")    # your embedder

# 3. Global search over one level: retrieve, map, reduce.
q = embed(question)
hits = g.search_hybrid(vector=q, model="m", k=20, label="Report",
                       props={"level": 1}, hydrate=True)
partials = [llm_partial_answer(question, h["node"]["properties"]) for h in hits]   # map
answer = llm_combine(question, sorted(partials, key=lambda p: -p.score))          # reduce
```

The same retrieval in NQL, with the question's vector as a literal (an agent
behind the MCP server does this in one round trip):

```sql
find r.title, r.summary, r.rating, r.level from (r:Report)
where similar_to(r, vector = [0.12, -0.03, ...], model = "m", k = 20)

-- members of a community, to write its report
find e.name from (e:Entity)-[:IN_COMMUNITY]->(c:Community) where c.key = "leiden/L1/..."
```

`similar_to` takes the K nearest reports of every level; keep the level you
want on the client, or use `search_hybrid(..., props={"level": L})` to filter
before ranking.

**The schema** (a convention; `upsert_community_report` writes it for you):

```
(:Community {partition, level, key, size})
(member)-[:IN_COMMUNITY]->(:Community)                  one edge per level
(:Community level L)-[:PARENT_OF]->(:Community level L+1)
(:Report {community_key, partition, level, title, summary, rating,
          generated_at, source_version})-[:SUMMARIZES]->(:Community)
```

- **One report per community**, keyed by `community_key`. Rewriting replaces
  it; the report keeps its node id and its embedding is refreshed.
- **Stable keys:** recomputing the communities keeps the key (and node id) of
  every community that overlaps a previous one by Jaccard ≥ 0.5, so its
  report stays attached. See [ALGORITHMS.md](ALGORITHMS.md#persisted-communities-with-stable-keys-0611-190-c).
- **`source_version`** is the community's **fingerprint**
  (`community_fingerprint(key)`): a hash of its members (label and properties)
  and of the edges between them (type and properties). Not only the member
  set: a new edge inside a community may not move it to another partition,
  yet it changes what its report should say.
- **`stale_reports(partition, level)`** lists `missing` (no report), `stale`
  (fingerprint changed) and `orphan` (the report's community no longer
  exists), sorted by level and key. An edge inside community C is also inside
  every ancestor of C, so C and its ancestors become stale and its siblings
  do not.
- Higher levels can be summarized from the reports of their children (follow
  `PARENT_OF`), as the reference GraphRAG does for large communities.

`python/scripts/global_search_sample.py` runs this end to end with a fake LLM
and embedder in `make check-python`.

## Measured

Python, 100k `Chunk` nodes with 64-dimension vectors, 20k `Entity`, 260k
edges, release wheel, same machine (`benches/retrieval.rs` reproduces it in
Rust; `python/scripts/graphrag_sample.py` is the functional guard).

| step | 0.6.7 | 0.6.8 |
|---|---|---|
| hybrid search k=10 (text + vector) | 0.20 ms | 0.20 ms; 0.21 ms with `hydrate=True` |
| KNN k=10 | 0.11 ms | 0.11 ms |
| fetch one hit by id | 327 ms (NQL `where c.id`, label scan) | 0.002 ms (`get_node`); 0.013 ms (NQL) |
| fetch the 10 hits | 3.3 s (10 NQL queries) | 0.016 ms (`get_nodes`); 0.031 ms (NQL `in`) |
| 1-hop expansion from one hit | 590 ms (NQL pattern) | 0.007 ms (`neighborhood`); 0.01 ms (NQL pattern seeded by id) |
| 1-hop expansion from the 10 hits | 5.9 s | 0.056 ms |
| 2-hop expansion from the 10 hits, both directions | ~17 s | 0.41 ms |
| whole cycle: hybrid k=10 hydrated + 1 hop of the 10 hits | ~9 s | 0.27 ms |
| **search + expand in ONE NQL query** (`similar_to` with the vector literal, k=10, 1 hop) | not possible | 0.35 ms (0.6.9) |
| same with `hybrid(text, vector)` in NQL | not possible | 132 ms (0.6.9): the pattern's label reached the hybrid filter as a label scan. Since 0.6.10 the label is checked on each branch's candidates: 0.99 ms at 100k chunks since the full-text branch asks tantivy only for the hits it uses (2.42 ms before; 16.2 ms → 0.89 ms at 10k). The rest is BM25 over every matching chunk: the bench matches ~57k of 100k on purpose |

Fill in the exact numbers for your data with `make bench BENCH=retrieval`.

### Real embedding sizes: 384 and 1024 dimensions (0.6.11)

The table above uses 64 dimensions. Text embedders produce 384 (small
sentence models) or 1024 (large ones), so the vector index is measured
again at those sizes: `HnswIndex` alone, KNN k=10, 200 queries, recall@10
against an exact scan of the same vectors. Apple M3 Max (16 cores, 128 GB),
release build, 9 October 2026; `make bench BENCH=retrieval_dims` reproduces
it (`NOPALDB_BENCH_DIMS`, `NOPALDB_BENCH_SCALES` and `NOPALDB_BENCH_QUERIES`
change the grid).

The vectors are synthetic and clustered: 100 clusters with a 16-dimension
spread plus noise, normalized. That is how embeddings of real text behave
(nearby texts, nearby vectors); a real corpus may score somewhat lower.

| dims | vectors | build | orphans | index memory | p95, `ef_search` 30 | p95, `ef_search` 100 | exact scan | recall@10, ef 30 / 100 |
|---|---|---|---|---|---|---|---|---|
| 384 | 10k | 3.4 s | 78 (0.8%) | 35 MiB | 0.28 ms | 0.68 ms | 3.0 ms | 0.998 / 0.999 |
| 384 | 100k | 32 s | 864 (0.9%) | 317 MiB | 0.67 ms | 0.89 ms | 31 ms | 0.994 / 0.999 |
| 1024 | 10k | 9.9 s | 51 (0.5%) | 59 MiB | 0.72 ms | 1.9 ms | 8.9 ms | 0.997 / 0.997 |
| 1024 | 100k | 72 s | 810 (0.8%) | 565 MiB | 1.3 ms | 1.9 ms | 82 ms | 0.989 / 0.998 |

- At 100k, the index answers 35–60× faster than the exact scan and finds
  99% of the true top 10 or more. `ef_search` 100 buys the last point of
  recall for 1.3–1.5× the latency.
- *Orphans* are the points the graph cannot reach; every search compares
  them directly, so they are never lost (see
  [EMBEDDINGS.md](EMBEDDINGS.md)). They stay under 1% at both sizes.
- Memory is the index alone (vectors plus graph): 2.2× the raw vectors at
  384 dimensions (100k × 384 × 4 bytes = 147 MiB) and 1.45× at 1024
  (391 MiB), since the graph costs the same whatever the size of the
  vector. Budget it per embedding model loaded. Up to 0.6.11 a fresh build
  also held ~3.4 KB per point that the HNSW library reserved and never used
  (650 and 897 MiB at 100k; #206); an index reopened from disk did not.
- Building is the slow part: 72 s for 100k × 1024, paid once; the index is
  persisted, so reopening the database does not rebuild it.

**Uniform random vectors** are the worst case for any HNSW index: with no
structure, all points are almost equally far apart. The bench measures them
too, as a floor, not as a reference. At 100k, recall@10 is 0.11 / 0.24 at
384 dimensions and 0.08 / 0.18 at 1024, with 2.4% and 3.5% orphans and a
build of 119 s and 243 s. If your vectors look like that (hashes, random
projections), use the exact scan instead.

## Limits, and what comes next

- Edges are still read one by one from storage during expansion (the
  in-memory adjacency holds edge ids only). Typed adjacency (next) removes
  those reads; `neighborhood` keeps the same contract.
- Since 0.6.10, `search_hybrid(label=...)` and NQL `hybrid()` no longer scan
  the label: each branch checks the label on its candidates (see
  [HYBRID_SEARCH.md](HYBRID_SEARCH.md#filter)). A label that is very rare among
  the nearest neighbours, and a filter with properties (`props=`), build an
  allowed set from the label's nodes; since 0.6.12 that set comes from the
  label index and reads only the label's nodes (#207).
- Since 0.6.12 every lookup by label reads a label index instead of every
  node: `get_nodes_by_label`, NQL `from (n:Label)`, the `labels=` scope of
  Leiden and the community reports. At 100k nodes, a label with 1 000 nodes
  takes 0.64 ms instead of 41.7 ms, and one with 10 nodes 0.006 ms
  (`make bench BENCH=graph_ops`, `label_lookup_100k`).
- NQL `similar_to` does not scan either: it asks the index for 4·K neighbours
  and keeps the first K of the label, escalating ×4 while short and falling
  back to the label's own nodes when the label is rare among the neighbours.
  Since 0.6.10 it returns K rows whenever the label has K embedded nodes
  (before, embeddings spread over several labels could leave it short).
- Since 0.6.10 NQL rows can carry the score: `score(c)` in FIND (cosine
  similarity with `similar_to`, RRF with `hybrid`), to cut the context at a
  threshold or split a token budget. See the NQL reference.

See also [HYBRID_SEARCH.md](HYBRID_SEARCH.md), [EMBEDDINGS.md](EMBEDDINGS.md)
and the Python [API reference](python/API_REFERENCE.md).
