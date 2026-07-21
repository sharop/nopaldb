"""Sample exercised under `mypy --strict` to prove the stubs type-check.

    mypy --strict nopaldb/python/scripts/typecheck_sample.py
"""
from nopaldb import Graph


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
    graph.close()


if __name__ == "__main__":
    main()
