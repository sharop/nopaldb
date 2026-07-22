#!/usr/bin/env python3
"""Ingest a folder of Obsidian-style markdown into NopalDB.

Idempotent: re-running over unchanged notes performs zero writes. Wikilinks
`[[note]]` become MENTIONS edges (creating stub Note targets when absent), and a
full-text index over the note body enables hybrid search (see query.py).

    python ingest.py --db ./second_brain.db --reset
"""
from __future__ import annotations

import argparse
import re
import shutil
import sys
from pathlib import Path

# Make `shared` importable and pull in the offline-capable embedder.
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from shared.embeddings import encode_texts  # noqa: E402

import nopaldb  # noqa: E402

VAULT = Path(__file__).parent / "vault"
WIKILINK = re.compile(r"\[\[([^\]]+)\]\]")


def ingest(graph) -> int:
    notes = sorted(VAULT.glob("*.md"))
    texts = [p.read_text(encoding="utf-8") for p in notes]
    vectors = encode_texts(texts, cache_name="second_brain")  # offline-safe
    for path, text, vec in zip(notes, texts, vectors):
        links = [
            {"type": "MENTIONS", "target_label": "Note", "target_key": "key",
             "target_key_value": target, "stub": True}
            for target in WIKILINK.findall(text)
        ]
        graph.upsert(
            "Note", "key",
            {"key": path.stem, "title": path.stem, "body": text},
            vector=vec.tolist(), model="minilm", links=links,
        )
    graph.create_index("Note", "body", "fulltext")
    return len(notes)


def main() -> None:
    ap = argparse.ArgumentParser(description="Ingest a markdown vault into NopalDB.")
    ap.add_argument("--db", required=True)
    ap.add_argument("--reset", action="store_true", help="delete the db first")
    args = ap.parse_args()

    if args.reset and Path(args.db).exists():
        shutil.rmtree(args.db)

    graph = nopaldb.Graph.open(args.db)
    n = ingest(graph)
    graph.close()
    print(f"Ingested {n} notes from {VAULT} into {args.db}")


if __name__ == "__main__":
    main()
