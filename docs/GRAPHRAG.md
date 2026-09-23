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

## In NQL

```sql
find c.text from (c:Chunk) where c.id in ["<uuid>", "<uuid>"]
find e.name, e.kind from (c:Chunk)-[:MENTIONS]->(e:Entity) where c.id = "<uuid>"
find e.name from (e:Entity) where e.name in ["ana", "beto"]     -- uses the hash index
```

`where var.id = "…"` and `where var.id in [...]` resolve by point read: no
scan, and a missing id is zero rows (`EXPLAIN` says `ID LOOKUP`). `in` /
`not in` take a list literal; with a user index on the property, `EXPLAIN`
says `INDEX SEEK (IN)`. Equality is strict, as with `=`: `1` is not `1.0`.
A root `AND` seeds the candidates with its indexed side and applies the rest
as a predicate.

## Measured

Python, 100k `Chunk` nodes with 64-dimension vectors, 20k `Entity`, 260k
edges, release wheel, same machine (`benches/retrieval.rs` reproduces it in
Rust; `python/scripts/graphrag_sample.py` is the functional guard).

| step | 0.6.7 | 0.6.8 |
|---|---|---|
| hybrid search k=10 | 0.2 ms | 0.2 ms |
| fetch one hit by id | 327 ms (NQL, label scan) | µs (`get_node`) |
| fetch the 10 hits | 3.3 s | one call |
| 1-hop expansion from a hit | 590 ms (NQL) | ms (`neighborhood`) |
| 2-hop expansion | 1.7 s | ms |

Fill in the exact numbers for your data with `make bench BENCH=retrieval`.

## Limits, and what 0.6.9 brings

- Edges are still read one by one from storage during expansion (the
  in-memory adjacency holds edge ids only). Typed adjacency (0.6.9) removes
  those reads; `neighborhood` keeps the same contract.
- `search_hybrid(label=...)` pre-filters by label with a scan of the label;
  omit the filter when your embeddings live on one label, or wait for the
  label index (0.6.9).
- The question's vector cannot yet be written inside an NQL query (`similar_to`
  and `hybrid()` take a node name); from NQL, search in Python or via
  `knn_nodes` and pass the ids with `in`. A vector literal in NQL is first
  in 0.6.9.

See also [HYBRID_SEARCH.md](HYBRID_SEARCH.md), [EMBEDDINGS.md](EMBEDDINGS.md)
and the Python [API reference](python/API_REFERENCE.md).
