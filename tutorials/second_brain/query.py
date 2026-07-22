#!/usr/bin/env python3
"""Query the second brain built by ingest.py.

Shows the two things NopalDB gives you that a plain vector store does not:
  1. hybrid search — full-text + vector fused with RRF, and
  2. the wikilink graph — traversable as edges, not a manual join.

    python query.py --db ./second_brain.db          # demo output
    python query.py --db ./second_brain.db --check  # CI assertions
"""
from __future__ import annotations

import argparse
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from shared.embeddings import encode_texts  # noqa: E402

import nopaldb  # noqa: E402


def id_to_key(graph) -> dict[str, str]:
    """Map every Note's UUID to its stable `key` (via NQL)."""
    rows = graph.execute_nql("find n.id, n.key from (n:Note)")
    return {r["n.id"]: r["n.key"] for r in rows if r.get("n.key")}


def mentions(graph, note: str) -> list[str]:
    """Notes that `note` links to, straight from the wikilink graph."""
    rows = graph.execute_nql(
        f'find t.key from (n:Note)-[:MENTIONS]->(t:Note) where n.key = "{note}"'
    )
    return sorted(r["t.key"] for r in rows)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", required=True)
    ap.add_argument("--check", action="store_true", help="assert expected results (CI)")
    args = ap.parse_args()

    graph = nopaldb.Graph.open(args.db)
    key = id_to_key(graph)

    print("Hybrid search for 'forgetting curve':")
    qvec = encode_texts(["forgetting curve"], cache_name="second_brain_query")[0].tolist()
    hits = graph.search_hybrid(text="forgetting curve", vector=qvec, model="minilm", k=3, label="Note")
    for h in hits:
        title = key.get(h["node_id"], h["node_id"][:8])
        print(f"  {title:18} score={h['score']:.4f} text_rank={h['text_rank']} vec_rank={h['vector_rank']}")

    print("\n'second-brain' mentions (wikilink graph):")
    linked = mentions(graph, "second-brain")
    for t in linked:
        print(f"  -> {t}")

    if args.check:
        # Deterministic assertions rely on the full-text path (the offline
        # embedding fallback is not semantic). "forgetting curve" appears only
        # in spaced-repetition.md, so it must top a text-driven search.
        text_hits = graph.search_hybrid(text="forgetting curve", k=1, label="Note")
        assert key.get(text_hits[0]["node_id"]) == "spaced-repetition", "unexpected top hit"
        assert linked == ["graphrag", "zettelkasten"], f"unexpected mentions: {linked}"
        print("\nCHECK: OK")

    graph.close()


if __name__ == "__main__":
    main()
