# Incremental ingestion

NopalDB is a good target for pipelines that re-run over a changing source (RAG
indexers, note/second-brain ingestion, any incremental indexer). The store
should end each run reflecting the *current* state of the source — nothing
stale, and no wasted writes for unchanged records. Three primitives cover that:

| Operation | Method | Meaning |
|-----------|--------|---------|
| **Upsert** | `upsert` / `Graph::upsert_node` | create if absent, update if changed, **no-op if identical** |
| **Delete** | `delete` / `Graph::delete_node_by_key` | drop the node whose source record disappeared |
| **Reconcile** | recipe below | full-sync: upsert what's present, delete what's gone |

Every operation is keyed by a **business key** `(label, key, value)` — a stable
identifier from your source (a file path, a chunk id, a record primary key) —
so a pipeline never has to track NopalDB's internal `NodeId`.

## Delete by key

The counterpart of upsert. Idempotent: deleting a missing key is a no-op;
deleting a key that matches more than one node is an error (deduplicate first).
Edges and index entries of the node are cleaned up.

```python
graph.delete("Note", "key", "note:intro")   # -> deleted node id, or None
```

Rust: `Graph::delete_node_by_key(label, key, &value) -> Option<NodeId>`.
MCP: the `delete_node` tool (requires the server not be in `--readonly`).

## Reconcile (full-sync)

To make the store match a source exactly, upsert everything present and delete
the keys that are no longer there. This is a recipe over the two primitives —
no extra API:

```python
def reconcile(graph, label, records):
    """`records` is the full desired set: each a dict with a 'key' + fields."""
    desired = set()
    for rec in records:
        graph.upsert(label, "key", rec)          # present → create/update/no-op
        desired.add(rec["key"])
    existing = {r["n.key"] for r in graph.execute_nql(f"find n.key from (n:{label})")}
    for key in existing - desired:
        graph.delete(label, "key", key)          # absent → delete
```

Re-running `reconcile` over an unchanged source performs **zero writes** (every
upsert is a no-op and nothing is deleted).

## Notes & limits

- **No cascade.** Deleting a node removes the node and its edges, but not its
  neighbors. A wikilink-style stub target that becomes orphaned stays until you
  delete it (or run a separate graph GC). This is intentional.
- **Concurrency.** `upsert` and `delete_node_by_key` share a per-key lock, so
  operations on the *same* business key are serialized; a transactional unique
  index (so the guarantee holds across processes) is a tracked follow-up.
- Building a thin adapter that maps a specific pipeline framework's target
  interface onto these primitives is straightforward and lives outside this repo.
