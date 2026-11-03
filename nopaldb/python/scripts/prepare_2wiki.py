#!/usr/bin/env python3
"""Prepare 2WikiMultiHopQA for the retrieval benches (#212).

Downloads the `dev` split of 2WikiMultiHopQA (the April 2021 version, with
the sentence segmentation fixed), deduplicates its paragraphs and computes
their embeddings, and those of the questions, with open models that run
locally. Writes everything to a directory OUTSIDE the repository: the text of
the paragraphs comes from Wikipedia (CC BY-SA), and neither it nor anything
derived from it (vectors included) is versioned. The repository keeps this
script, the benches and the numbers.

Not part of `make check-python`: it downloads 259 MB and needs
`sentence-transformers` (with torch), which the wheel does not depend on.

    python -m venv ~/.venvs/nopaldb-212 && ~/.venvs/nopaldb-212/bin/pip install sentence-transformers
    ~/.venvs/nopaldb-212/bin/python -I nopaldb/python/scripts/prepare_2wiki.py \\
        --cache ~/.cache/nopaldb-212 --out ~/nopaldb-212

Output (`--out`):

- `chunks.jsonl`: one paragraph per line, `{"title", "text"}`. The title is
  unique in `dev` and is the paragraph's identity. The row number is the row
  of its vector.
- `questions.jsonl`: `{"id", "question", "type", "gold", "evidences"}`; `gold`
  is the set of titles of the supporting facts (2 or 4), `evidences` the
  (subject, relation, object) triples of the dataset.
- `<model>.chunks.f32`, `<model>.questions.f32`: the vectors, normalised,
  float32 little-endian, one row after another (`dim` in the manifest).
- `manifest.json`: dataset URL and sha256, counts, seed, models with their
  pinned revision, versions of python/torch/sentence-transformers, device and
  date.

The embedded text of a paragraph is `"<title>. <sentences>"`. bge-large gets
its retrieval instruction in front of each question, as its model card says;
MiniLM gets the question as is.

Dataset: https://github.com/Alab-NII/2wikimultihop (repository Apache-2.0;
paragraph text from Wikipedia, CC BY-SA). Models: all-MiniLM-L6-v2
(Apache-2.0) and bge-large-en-v1.5 (MIT).
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import platform
import random
import sys
import urllib.request
import zipfile

DATASET_URL = "https://www.dropbox.com/s/ms2m13252h6xubs/data_ids_april7.zip?dl=1"
DATASET_SHA256 = "95df2bf56fdabe034e27aebc580e02264232203cf52552f9efe8a919e5529eef"
DATASET_ZIP = "data_ids_april7.zip"

# Pinned revisions: the same model name can change on the hub.
MODELS = {
    "minilm": {
        "name": "sentence-transformers/all-MiniLM-L6-v2",
        "revision": "1110a243fdf4706b3f48f1d95db1a4f5529b4d41",
        "query_prefix": "",
    },
    "bge": {
        "name": "BAAI/bge-large-en-v1.5",
        "revision": "d4aa6901d3a41ba39fb536a557fa166f842b0e09",
        "query_prefix": "Represent this sentence for searching relevant passages: ",
    },
}


def sha256_of(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def dataset_zip(cache: str) -> str:
    """The zip in `cache`, downloaded once and checked by its sha256."""
    os.makedirs(cache, exist_ok=True)
    path = os.path.join(cache, DATASET_ZIP)
    if not os.path.exists(path):
        print(f"downloading {DATASET_URL} …", flush=True)
        tmp = path + ".part"
        urllib.request.urlretrieve(DATASET_URL, tmp)
        os.replace(tmp, path)
    digest = sha256_of(path)
    if digest != DATASET_SHA256:
        sys.exit(f"{path}: sha256 {digest}, expected {DATASET_SHA256}; delete it and run again")
    return path


def load_dev(zip_path: str) -> list[dict]:
    with zipfile.ZipFile(zip_path) as z, z.open("dev.json") as f:
        return json.load(f)


def select(dev: list[dict], questions: int | None, corpus: str, seed: int):
    """The questions (all, or a seeded sample) and the paragraphs of the corpus.

    The corpus is every distinct paragraph of `dev` (`all`), so that each
    question competes with ~55k paragraphs, or only those of the sampled
    questions (`sampled`), for a small run.
    """
    chosen = list(dev)
    if questions is not None and questions < len(chosen):
        chosen = random.Random(seed).sample(chosen, questions)
    source = dev if corpus == "all" else chosen
    chunks: dict[str, str] = {}
    for q in source:
        for title, sentences in q["context"]:
            chunks.setdefault(title, " ".join(sentences))
    rows = []
    for q in chosen:
        gold = sorted({title for title, _ in q["supporting_facts"]})
        missing = [t for t in gold if t not in chunks]
        if missing:
            sys.exit(f"question {q['_id']}: supporting paragraph not in the corpus: {missing}")
        rows.append({
            "id": q["_id"],
            "question": q["question"],
            "type": q["type"],
            "gold": gold,
            "evidences": q["evidences"],
        })
    return rows, chunks


def write_vectors(path: str, vectors) -> None:
    import numpy as np

    np.ascontiguousarray(vectors, dtype="<f4").tofile(path)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--cache", required=True, help="where the dataset zip is kept")
    ap.add_argument("--out", required=True, help="output directory (outside the repository)")
    ap.add_argument("--model", action="append", choices=sorted(MODELS), help="repeatable; default both")
    ap.add_argument("--questions", type=int, help="seeded sample of questions (default: all)")
    ap.add_argument("--corpus", choices=["all", "sampled"], default="all")
    ap.add_argument("--seed", type=int, default=212)
    ap.add_argument("--device", default=None, help="torch device (default: what sentence-transformers picks)")
    ap.add_argument("--batch-size", type=int, default=64)
    args = ap.parse_args()

    repo = os.path.dirname(os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))))
    out = os.path.abspath(os.path.expanduser(args.out))
    if out == repo or out.startswith(repo + os.sep):
        sys.exit(f"--out {out} is inside the repository: the dataset is not versioned")

    zip_path = dataset_zip(os.path.expanduser(args.cache))
    questions, chunks = select(load_dev(zip_path), args.questions, args.corpus, args.seed)
    os.makedirs(out, exist_ok=True)
    titles = list(chunks)
    with open(os.path.join(out, "chunks.jsonl"), "w", encoding="utf-8") as f:
        for title in titles:
            f.write(json.dumps({"title": title, "text": chunks[title]}, ensure_ascii=False) + "\n")
    with open(os.path.join(out, "questions.jsonl"), "w", encoding="utf-8") as f:
        for q in questions:
            f.write(json.dumps(q, ensure_ascii=False) + "\n")
    print(f"{len(questions)} questions, {len(titles)} paragraphs", flush=True)

    import sentence_transformers
    import torch
    from sentence_transformers import SentenceTransformer

    models = {}
    for key in args.model or sorted(MODELS):
        spec = MODELS[key]
        model = SentenceTransformer(spec["name"], revision=spec["revision"], device=args.device)
        texts = [f"{t}. {chunks[t]}" for t in titles]
        vectors = model.encode(texts, batch_size=args.batch_size, normalize_embeddings=True,
                               show_progress_bar=True)
        write_vectors(os.path.join(out, f"{key}.chunks.f32"), vectors)
        queries = [spec["query_prefix"] + q["question"] for q in questions]
        qvectors = model.encode(queries, batch_size=args.batch_size, normalize_embeddings=True)
        write_vectors(os.path.join(out, f"{key}.questions.f32"), qvectors)
        models[key] = {**spec, "dim": int(vectors.shape[1]), "device": str(model.device)}
        print(f"{key}: {vectors.shape[0]} × {vectors.shape[1]}", flush=True)

    manifest = {
        "dataset": {"url": DATASET_URL, "sha256": DATASET_SHA256, "split": "dev"},
        "questions": len(questions),
        "chunks": len(titles),
        "corpus": args.corpus,
        "seed": args.seed,
        "sample": args.questions,
        "chunk_text": "<title>. <sentences>",
        "models": models,
        "versions": {
            "python": platform.python_version(),
            "torch": torch.__version__,
            "sentence_transformers": sentence_transformers.__version__,
        },
        "date": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
    }
    with open(os.path.join(out, "manifest.json"), "w") as f:
        json.dump(manifest, f, indent=2)
    print(f"written to {out}")


if __name__ == "__main__":
    main()
