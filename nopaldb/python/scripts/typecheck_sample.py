"""Sample exercised under `mypy --strict` to prove the stubs type-check.

    mypy --strict nopaldb/python/scripts/typecheck_sample.py

It is also executed by `make check-python`, so what it types must also
run; the parts that need a feature not exercised here (embeddings) are
typed inside a function that is never called.
"""
from __future__ import annotations

from typing import TYPE_CHECKING

from nopaldb import Graph

if TYPE_CHECKING:  # type-only names: they exist in the stub, not at runtime
    from nopaldb.nopaldb import BulkLoadStats, IndexStats, Stats


def main() -> None:
    graph = Graph.in_memory()
    outcome, node_id = graph.upsert(
        label="Chunk",
        key="key",
        props={"key": "note:a", "path": "a.md"},
    )
    reveal_outcome: str = outcome
    reveal_id: str = node_id
    print(reveal_outcome, reveal_id)

    results = graph.upsert_many(
        [
            {"label": "Note", "key": "key", "props": {"key": "note:b"}},
        ]
    )
    for oc, nid in results:
        print(oc, nid)

    labels: list[str] = graph.get_labels()
    print(labels)

    # BulkLoader: nested property values, edge properties, the edge id.
    with graph.bulk_loader(1000) as loader:
        a: str = loader.add_node("Chunk", {"key": "note:c", "tags": ["x", "y"], "meta": {"k": None}})
        b: str = loader.add_node("Chunk", {"key": "note:d", "raw": b"\x00"})
        eid: str = loader.add_edge(a, b, "LINKS", properties={"since": 2020, "weight": 0.5})
        print(eid)
    loader2 = graph.bulk_loader(10)
    loader2.add_node("Chunk", {"key": "note:e"})
    stats: BulkLoadStats = loader2.finish()
    print(stats["nodes"], stats["duration_secs"])

    # Counts and stats.
    n: int = graph.node_count()
    e: int = graph.edge_count()
    report: Stats = graph.get_stats()
    indexes: list[IndexStats] = report["indexes"]
    print(n, e, report["graph"]["total_nodes"], len(indexes))

    # Arrow: bytes in, bytes out.
    nodes_ipc: bytes = graph.to_arrow(label="Chunk")
    edges_ipc: bytes = graph.edges_to_arrow()
    both: tuple[bytes, bytes] = graph.to_arrow_complete()
    print(len(nodes_ipc), len(edges_ipc), len(both[0]))

    # NQL result surface.
    res = graph.execute_nql("find n from (n:Chunk)")
    kind: str = res.kind
    for row in res:
        print(kind, row)
    graph.close()

    with Graph.in_memory() as g2:
        print(g2.node_count())


def typing_only_embeddings(graph: Graph) -> None:
    """Never called: types the embedding surface without needing vectors."""
    hits: list[tuple[str, float]] = graph.knn_nodes([0.1, 0.2], k=1, model="m", ef_search=64)
    print(hits)


if __name__ == "__main__":
    main()
