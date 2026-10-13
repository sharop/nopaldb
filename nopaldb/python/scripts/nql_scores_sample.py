#!/usr/bin/env python3
"""Guard: `rank(var)`, `text_score(var)` and `vector_score(var)` in the NQL
FIND (#228), next to `score(var)`.

`score` with `hybrid` is RRF (`Σ 1/(rrf_k + rank)`, at most `2/rrf_k`), so a
cosine-style threshold keeps nothing; the raw columns are what to cut on:
`vector_score` (cosine, [0, 1]) or `rank`. Values are the ones
`search_hybrid_explain` reports. Synthetic corpus; runs in CI (job
python-stubs) after `maturin develop`.
"""

import os
import tempfile

import nopaldb

CHUNKS = [
    ("c0", "riego por goteo", [1.0, 0.0, 0.0]),
    ("c1", "riego y drenaje", [0.95, 0.31, 0.0]),
    ("c2", "plagas del nopal", [0.80, 0.60, 0.0]),
    ("c3", "cosecha de tuna", [0.0, 1.0, 0.0]),
]

with tempfile.TemporaryDirectory() as tmp:
    g = nopaldb.Graph.open(os.path.join(tmp, "scores"))
    ids = {}
    for name, text, vec in CHUNKS:
        _, node_id = g.upsert("Chunk", "name", {"name": name, "text": text}, vector=vec, model="m")
        ids[name] = node_id
    g.create_index("Chunk", "text", "fulltext")

    q = 'find c.name, score(c), rank(c), text_score(c) as bm25, vector_score(c) as cos from (c:Chunk) where hybrid(c, text = "riego", vector = [1, 0, 0], model = "m", k = 4)'
    rows = list(g.execute_nql(q))
    assert len(rows) == 4, rows
    assert [r["rank(c)"] for r in rows] == [0, 1, 2, 3], rows
    # RRF: tiny numbers, bounded by 2/rrf_k — a cosine threshold would drop everything.
    # f32 rounds 1/60 + 1/60 up a hair past 2/60 in f64: allow float32 slack.
    assert all(0 < r["score(c)"] <= 2 / 60 + 1e-6 for r in rows), rows
    assert all(r["cos"] is None or -1.0 <= r["cos"] <= 1.0 for r in rows), rows
    # "cosecha de tuna" does not contain "riego": its text branch is null.
    by_name = {r["c.name"]: r for r in rows}
    assert by_name["c3"]["bm25"] is None, by_name["c3"]
    assert by_name["c0"]["bm25"] is not None and by_name["c0"]["cos"] > 0.99, by_name["c0"]

    # The same numbers search_hybrid_explain reports, hit by hit.
    e = g.search_hybrid_explain(text="riego", vector=[1.0, 0.0, 0.0], model="m", k=4, label="Chunk")
    for i, hit in enumerate(e["hits"]):
        row = rows[i]
        assert ids[row["c.name"]] == hit["node_id"], (row, hit)
        assert abs(row["score(c)"] - hit["score"]) < 1e-6
        assert (row["bm25"] is None) == (hit["text_score"] is None)
        if hit["vector_distance"] is not None:
            assert abs(row["cos"] - (1.0 - hit["vector_distance"])) < 1e-6

    # Cutting the context the right way: by cosine, or by rank — never by RRF.
    kept = [r for r in rows if r["cos"] is not None and r["cos"] >= 0.75]
    assert {r["c.name"] for r in kept} == {"c0", "c1", "c2"}, kept
    top2 = [r["c.name"] for r in rows if r["rank(c)"] < 2]
    assert len(top2) == 2

    # With similar_to: vector_score == score, text_score is null.
    rows = list(g.execute_nql('find c.name, score(c), vector_score(c), text_score(c) from (c:Chunk) where similar_to(c, vector = [1, 0, 0], model = "m", k = 2)'))
    assert len(rows) == 2 and all(r["vector_score(c)"] == r["score(c)"] and r["text_score(c)"] is None for r in rows), rows

    # Outside the FIND it is a named error, like score.
    try:
        g.execute_nql('find c.name from (c:Chunk) where similar_to(c, vector = [1, 0, 0], model = "m", k = 2) and vector_score(c) > 0.5')
        raise AssertionError("expected an error")
    except Exception as err:  # noqa: BLE001 - the message is the contract
        assert "only supported as a FIND column" in str(err), err
    g.close()

print("nql_scores_sample: ok")
