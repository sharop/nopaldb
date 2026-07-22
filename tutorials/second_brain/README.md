# Second brain in ~20 lines

Ingest a folder of Obsidian-style markdown into NopalDB and query it two ways at
once: **hybrid search** (full-text + vector) and the **wikilink graph**. It's the
smallest end-to-end example of using NopalDB as an AI second brain.

## Run it

```bash
pip install nopaldb                      # or: make -C tutorials install-nopaldb
pip install numpy                        # sentence-transformers optional (see below)

python tutorials/second_brain/ingest.py --db ./second_brain.db --reset
python tutorials/second_brain/query.py  --db ./second_brain.db
```

By default the example runs **fully offline** with a deterministic (non-semantic)
embedding fallback, so it works without downloading a model — handy for CI. For
real semantic search install `sentence-transformers` (`make -C tutorials install`)
and the `all-MiniLM-L6-v2` model is used automatically.

## The whole ingestion, in one function

Every note becomes a `Note` keyed by its filename; `[[wikilinks]]` become
`MENTIONS` edges (creating stub targets when the note doesn't exist yet); and a
full-text index over the body enables hybrid search. Re-running is idempotent —
unchanged notes cost **zero writes** (`upsert`).

```python
def ingest(graph):
    notes = sorted(VAULT.glob("*.md"))
    texts = [p.read_text() for p in notes]
    vectors = encode_texts(texts, cache_name="second_brain")
    for path, text, vec in zip(notes, texts, vectors):
        links = [{"type": "MENTIONS", "target_label": "Note", "target_key": "key",
                  "target_key_value": t, "stub": True} for t in WIKILINK.findall(text)]
        graph.upsert("Note", "key",
                     {"key": path.stem, "title": path.stem, "body": text},
                     vector=vec.tolist(), model="minilm", links=links)
    graph.create_index("Note", "body", "fulltext")
```

Querying is just as short:

```python
# 1. hybrid: full-text relevance + vector similarity, fused with RRF
graph.search_hybrid(text="forgetting curve", vector=qvec, model="minilm", k=3, label="Note")

# 2. the wikilink graph, as a query — not a manual join
graph.execute_nql('find t.key from (n:Note)-[:MENTIONS]->(t:Note) where n.key = "second-brain"')
```

## Why not just sqlite-vec (or any vector store)?

A vector store finds notes that *read* similar. But your wikilinks are a **graph**,
and a second brain lives on those connections. With NopalDB the same store answers
both questions:

- *"notes about forgetting"* → hybrid search (text **and** meaning, one ranked list), and
- *"what does this note connect to"* / *"what bridges two topics"* → a graph query over the wikilink edges.

No second system, no manual join between a vector index and a graph — one embedded
database, one file.

## Files

- `vault/` — six tiny sample notes with `##` sections and `[[wikilinks]]`.
- `ingest.py` — walk the vault → `upsert` (with links) → full-text index.
- `query.py` — `search_hybrid` + a wikilink graph query. `--check` runs assertions.
