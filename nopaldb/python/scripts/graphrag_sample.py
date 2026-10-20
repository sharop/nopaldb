"""Guard: the GraphRAG retrieval cycle from Python (0.6.8).

- get_node / get_nodes / get_edge / get_edges: point reads by id, input order
  kept, None for a missing id, ValueError for a non-UUID string.
- search_hybrid(..., hydrate=True) and knn_nodes(..., hydrate=True) return
  each hit with its node; without hydrate the shapes are the 0.6.x ones.
- neighborhood(ids, depth, ...): BFS with a global visited set, minimum
  depth, edge-type and label filters, max_nodes + truncated, "both".
- neighbors / degree.
- NQL: `where c.id = "…"` is a point read (EXPLAIN says ID LOOKUP, also on a
  miss), `in` / `not in` with and without an index.
"""
import os
import random
import tempfile
import uuid

import nopaldb

random.seed(3)
DIM = 8
N_CHUNK, N_ENT = 1_000, 200


def vec(seed):
    r = random.Random(seed)
    v = [r.gauss(0, 1) for _ in range(DIM)]
    n = sum(x * x for x in v) ** 0.5
    return [x / n for x in v]


with tempfile.TemporaryDirectory() as tmp:
    g = nopaldb.Graph.open(os.path.join(tmp, "rag"))
    with g.bulk_loader(500) as l:
        ents = [l.add_node("Entity", {"name": f"entity-{i}", "kind": ["persona", "lugar"][i % 2]}) for i in range(N_ENT)]
        chunks = [l.add_node("Chunk", {"doc": f"doc-{i // 10}", "text": f"chunk {i} menciona entity-{i % N_ENT}"}) for i in range(N_CHUNK)]
        for i, c in enumerate(chunks):
            l.add_edge(c, ents[i % N_ENT], "MENTIONS")
            l.add_edge(c, ents[(i * 7) % N_ENT], "MENTIONS")
        for i in range(N_ENT):
            l.add_edge(ents[i], ents[(i * 13 + 1) % N_ENT], "RELATED", {"w": 0.5})
    # Un lote en vez de uno por uno (0.6.10, #175): valida todo antes de
    # escribir y, con el índice ya en memoria, inserta en paralelo.
    assert g.add_node_embeddings("m", [(c, vec(i)) for i, c in enumerate(chunks)]) == len(chunks)
    g.create_index("Entity", "name", "hash")
    g.rebuild_indexes()

    # 1. Point reads.
    c0 = chunks[0]
    n = g.get_node(c0)
    assert n is not None and n["id"] == c0 and n["label"] == "Chunk" and n["properties"]["doc"] == "doc-0", n
    assert g.get_node(str(uuid.uuid4())) is None
    got = g.get_nodes([chunks[1], str(uuid.uuid4()), chunks[0], chunks[1]])
    assert [x and x["id"] for x in got] == [chunks[1], None, chunks[0], chunks[1]], got
    try:
        g.get_node("not-a-uuid")
        raise SystemExit("non-uuid must raise ValueError")
    except ValueError:
        pass
    edges = g.neighborhood([c0], depth=1)["edges"]
    e0 = g.get_edge(edges[0]["id"])
    assert e0 == edges[0] and e0["type"] == "MENTIONS" and e0["source"] == c0, e0
    assert g.get_edges([edges[0]["id"], str(uuid.uuid4())])[1] is None

    # 2. Search with hydration.
    q = vec(424242)
    hits = g.search_hybrid(vector=q, model="m", k=5, hydrate=True)
    assert len(hits) == 5 and hits[0]["node"]["id"] == hits[0]["node_id"] and "text" in hits[0]["node"]["properties"], hits[0]
    plain = g.search_hybrid(vector=q, model="m", k=5)
    assert "node" not in plain[0] and plain[0]["node_id"] == hits[0]["node_id"]
    tuples = g.knn_nodes(q, 3, "m")
    assert isinstance(tuples[0], tuple) and len(tuples[0]) == 2
    khits = g.knn_nodes(q, 3, "m", hydrate=True)
    assert khits[0]["node_id"] == tuples[0][0] and khits[0]["node"]["label"] == "Chunk" and abs(khits[0]["distance"] - tuples[0][1]) < 1e-6

    # 3. Neighborhood.
    nb = g.neighborhood([c0], depth=2)
    ids = [x["id"] for x in nb["nodes"]]
    assert len(ids) == len(set(ids)) and nb["depth"][c0] == 0 and nb["truncated"] is False
    assert nb["nodes"][0]["id"] == c0, "seeds first"
    d1 = {x["id"] for x in nb["nodes"] if nb["depth"][x["id"]] == 1}
    assert d1 == {ents[0]}, "chunk 0 mentions entity-0 twice: one neighbour, once"
    assert all(x["label"] == "Entity" for x in nb["nodes"] if nb["depth"][x["id"]] >= 1)
    assert all(e["source"] in nb["depth"] and e["target"] in nb["depth"] for e in nb["edges"])
    # Leiden acotado a las entidades y ponderado (0.6.11, #190): los chunks
    # no participan.
    comms = g.leiden(labels=["Entity"], edge_types=["RELATED"], weight="w")
    assert set(comms) == set(ents), "only entities get a community"
    assert all(isinstance(c, int) for c in comms.values())
    # Jerarquía (0.6.11, #190 b): nivel 0 = leiden(); cada nivel más fino
    # queda dentro del anterior.
    levels = g.leiden_hierarchy(labels=["Entity"], edge_types=["RELATED"], weight="w", max_cluster_size=1)
    assert levels[0] == comms and all(set(lv) == set(ents) for lv in levels)
    for coarse, fine in zip(levels, levels[1:]):
        parent: dict[int, int] = {}
        assert all(parent.setdefault(fine[n], coarse[n]) == coarse[n] for n in ents), "nested"
    # Comunidades persistidas con keys estables (0.6.11, #190 c).
    rep = g.materialize_communities(levels)
    assert rep["created"] == rep["communities"] == sum(len(set(lv.values())) for lv in levels)
    again = g.materialize_communities(g.leiden_hierarchy(labels=["Entity"], edge_types=["RELATED"],
                                                         weight="w", max_cluster_size=1))
    assert again["kept_keys"] == again["communities"] and again["created"] == again["deleted"] == 0
    assert again["memberships_added"] == again["memberships_removed"] == 0, "unchanged: no writes"
    found = list(g.execute_nql("find c.key, c.level, c.size from (c:Community) where c.level = 0"))
    assert len(found) == len(set(levels[0].values()))
    try:
        g.leiden_hierarchy(resolution_factor=1.0)
        raise AssertionError("resolution_factor <= 1 must fail")
    except (ValueError, RuntimeError) as e:
        assert "resolution_factor" in str(e)
    small = g.neighborhood([c0], depth=2, max_nodes=2)
    assert small["truncated"] is True and len(small["nodes"]) == 2
    assert small["score"] == {}, "BFS does not score"
    ranked = g.neighborhood([c0, chunks[1]], depth=2, direction="both", max_nodes=5,
                            rank="ppr", seed_weights={c0: 2.0, chunks[1]: 1.0})
    assert len(ranked["nodes"]) == 5 and [x["id"] for x in ranked["nodes"][:2]] == [c0, chunks[1]], "seeds first"
    rest = [ranked["score"][x["id"]] for x in ranked["nodes"][2:]]
    assert rest == sorted(rest, reverse=True), "then by PPR score"
    only_related = g.neighborhood([c0], depth=1, edge_types=["RELATED"])
    assert [x["id"] for x in only_related["nodes"]] == [c0], "a chunk has no RELATED edges"
    no_chunks = g.neighborhood([ents[0]], depth=2, direction="both", labels=["Entity"])
    assert all(x["label"] == "Entity" for x in no_chunks["nodes"])
    both = g.neighborhood([ents[0]], depth=1, direction="both")
    eids = [e["id"] for e in both["edges"]]
    assert len(eids) == len(set(eids))
    assert {x["id"] for x in g.neighbors(c0)} == d1
    assert g.degree(c0, "out") == 2 and g.degree(c0, "in") == 0
    try:
        g.neighborhood([c0], direction="sideways")
        raise SystemExit("bad direction must raise ValueError")
    except ValueError:
        pass

    # 4. NQL by id and IN.
    rows = list(g.execute_nql(f'find c.doc from (c:Chunk) where c.id = "{c0}"'))
    assert rows == [{"c.doc": "doc-0"}], rows
    assert list(g.execute_nql(f'find c.doc from (c:Chunk) where c.id = "{uuid.uuid4()}"')) == []
    plan = g.execute_nql(f'explain find c.doc from (c:Chunk) where c.id = "{uuid.uuid4()}"').explain
    assert "ID LOOKUP" in plan and "LABEL SCAN" not in plan, plan
    two = list(g.execute_nql(f'find c.doc from (c:Chunk) where c.id in ["{chunks[0]}", "{chunks[10]}"]'))
    assert sorted(r["c.doc"] for r in two) == ["doc-0", "doc-1"], two
    by_name = list(g.execute_nql('find e.kind from (e:Entity) where e.name in ["entity-1", "entity-2"]'))
    assert sorted(r["e.kind"] for r in by_name) == ["lugar", "persona"], by_name
    assert "INDEX SEEK (IN)" in g.execute_nql('explain find e.kind from (e:Entity) where e.name in ["entity-1"]').explain
    rest = list(g.execute_nql('find e.name from (e:Entity) where e.kind not in ["lugar"] limit 3'))
    assert len(rest) == 3 and all(r["e.name"].startswith("entity-") for r in rest)
    hop = list(g.execute_nql(f'find e.name from (c:Chunk)-[:MENTIONS]->(e:Entity) where c.id = "{c0}"'))
    assert {r["e.name"] for r in hop} == {x["properties"]["name"] for x in g.neighbors(c0)}

    # 5. Search and expand in ONE query: the question's vector inside NQL (0.6.9).
    lit = "[" + ", ".join(repr(x) for x in q) + "]"   # repr may print 1e-05: NQL parses it
    one = list(g.execute_nql(f'find c.text, e.name from (c:Chunk)-[:MENTIONS]->(e:Entity) where similar_to(c, vector = {lit}, model = "m", k = 3)'))
    top3 = [h["node_id"] for h in g.knn_nodes(q, 3, "m", hydrate=True)]
    expanded = {n["properties"]["name"] for n in g.neighborhood(top3, depth=1)["nodes"] if n["label"] == "Entity"}
    assert {r["e.name"] for r in one} == expanded and len(one) == sum(g.degree(c, "out") for c in top3), one
    plan = g.execute_nql(f'explain find e.name from (c:Chunk)-[:MENTIONS]->(e:Entity) where similar_to(c, vector = {lit}, model = "m", k = 3)').explain
    assert "PATTERN PIPELINE (seed: SIMILAR_TO)" in plan and "<literal dim=8>" in plan, plan
    ranked = list(g.execute_nql(f'find c.text from (c:Chunk) where hybrid(c, vector = {lit}, model = "m", k = 3)'))
    assert [r["c.text"] for r in ranked] == [h["node"]["properties"]["text"] for h in g.knn_nodes(q, 3, "m", hydrate=True)], "best first"
    try:
        g.execute_nql('find e.name from (c:Chunk)-[:MENTIONS]->(e:Entity) where similar_to(e, vector = [1, 0], model = "m")')
        raise SystemExit("searching the target variable must be a validation error")
    except Exception as exc:
        assert "must be the first node" in str(exc), exc
    g.close()


# 6. Recipe: end to end (#230). The four blocks between the `recipe:<name>`
#    markers are the ones docs/GRAPHRAG.md § "Recipe: end to end" shows, byte
#    for byte after dedent: scripts/check_graphrag_recipe.py fails when they
#    diverge, so the doc cannot age. Only `embed`, `path`, `question`, the
#    corpus and the asserts live outside the markers.
VOCAB = ["riego", "goteo", "nopal", "tuna", "plaga", "cochinilla", "cosecha", "mercado"]


def embed(text: str) -> list[float]:
    """Stand-in for an embedding model: bag of words over VOCAB, normalized."""
    words = text.lower().replace("¿", " ").replace("?", " ").replace(",", " ").replace(".", " ").split()
    v = [float(sum(1 for w in words if w.startswith(t))) for t in VOCAB]
    n = sum(x * x for x in v) ** 0.5
    return [x / n for x in v] if n else [1.0 / len(VOCAB) ** 0.5] * len(VOCAB)


CORPUS = [
    {"id": "riego", "chunks": [
        {"text": "El riego por goteo da al nopal un riego constante sin encharcar.", "entities": ["Riego por goteo", "Nopal"]},
        {"text": "Un nopal con riego por goteo produce tuna más uniforme.", "entities": ["Riego por goteo", "Nopal", "Tuna"]},
        {"text": "El goteo se programa de madrugada para que el riego no se evapore.", "entities": ["Riego por goteo"]},
        {"text": "Sin riego el nopal sobrevive, pero la cosecha de tuna cae.", "entities": ["Nopal", "Cosecha"]},
    ], "relations": [("Riego por goteo", "Nopal")]},
    {"id": "plagas", "chunks": [
        {"text": "La cochinilla es la plaga más común del nopal.", "entities": ["Cochinilla", "Nopal"]},
        {"text": "Una plaga de cochinilla se nota por el polvo blanco en las pencas.", "entities": ["Cochinilla"]},
        {"text": "Contra la plaga se poda la penca afectada del nopal.", "entities": ["Nopal"]},
        {"text": "La cochinilla baja la cosecha de tuna si no se controla.", "entities": ["Cochinilla", "Cosecha", "Tuna"]},
    ], "relations": [("Cochinilla", "Nopal"), ("Cochinilla", "Cosecha")]},
    {"id": "mercado", "chunks": [
        {"text": "La tuna se vende en el mercado por caja de cosecha.", "entities": ["Tuna", "Mercado", "Cosecha"]},
        {"text": "El mercado paga mejor la tuna roja que la verde.", "entities": ["Tuna", "Mercado"]},
        {"text": "La cosecha de tuna se lleva al mercado el mismo día.", "entities": ["Cosecha", "Mercado"]},
        {"text": "El nopal verdura también tiene mercado, por manojo.", "entities": ["Nopal", "Mercado"]},
    ], "relations": [("Cosecha", "Tuna"), ("Tuna", "Mercado")]},
]

with tempfile.TemporaryDirectory() as tmp:
    path = os.path.join(tmp, "kb")
    question = "¿Cómo riego el nopal?"

    # recipe:indexes:start
    g = nopaldb.Graph.open(path)
    g.create_index("Chunk", "text", "fulltext", analyzer={"language": "spanish"})  # BM25 branch of hybrid()
    g.create_index("Entity", "name", "hash")                                        # `where e.name in [...]` without a scan
    # Nothing to call for the vector index: HNSW is built on the first search and persisted.
    # recipe:indexes:end

    # recipe:ingest:start
    def ingest(doc: dict) -> dict[str, str]:
        """One document → chunks, entities and relations, in one idempotent batch."""
        rows = []
        for i, ch in enumerate(doc["chunks"]):
            rows.append({
                "label": "Chunk", "key": "key",
                "props": {"key": f'{doc["id"]}#{i}', "doc": doc["id"], "text": ch["text"]},
                "vector": embed(ch["text"]), "model": "m",
                "links": [{"type": "MENTIONS", "target_label": "Entity", "target_key": "name",
                           "target_key_value": e, "stub": True} for e in ch["entities"]],
            })
        for a, b in doc["relations"]:
            rows.append({
                "label": "Entity", "key": "name", "props": {"name": a},
                "links": [{"type": "RELATED", "target_label": "Entity", "target_key": "name",
                           "target_key_value": b, "stub": True}],
            })
        outcomes = g.upsert_many(rows)                    # [(outcome, node_id), ...], one per row
        return {row["props"]["key" if row["label"] == "Chunk" else "name"]: nid for row, (_, nid) in zip(rows, outcomes)}

    ids = {}
    for doc in CORPUS:
        ids.update(ingest(doc))
    # recipe:ingest:end
    first = [g.upsert_many([{"label": "Chunk", "key": "key", "props": {"key": "riego#0", "doc": "riego",
                                                                       "text": CORPUS[0]["chunks"][0]["text"]},
                              "vector": embed(CORPUS[0]["chunks"][0]["text"]), "model": "m",
                              "links": [{"type": "MENTIONS", "target_label": "Entity", "target_key": "name",
                                         "target_key_value": e, "stub": True} for e in ["Riego por goteo", "Nopal"]]}])]
    assert first[0][0][0] == "unchanged", first
    assert g.node_count() == 12 + 6, g.node_count()
    before = g.node_count()
    again = {}
    for doc in CORPUS:
        again.update(ingest(doc))
    assert again == ids and g.node_count() == before, "re-running the ingestion writes nothing"
    assert g.search_hybrid(text="cochinilla", k=3), "upserted chunks are in the full-text index"

    # recipe:retrieve:start
    q = embed(question)
    lit = "[" + ", ".join(repr(x) for x in q) + "]"      # the vector goes inside the query (0.6.9)
    rows = list(g.execute_nql(
        f"find c.text, e.name, rank(c), vector_score(c), text_score(c), score(c) "
        f"from (c:Chunk)-[:MENTIONS]->(e:Entity) "
        f'where hybrid(c, text = "{question}", vector = {lit}, model = "m", k = 4)'))
    # Cut by `vector_score` (a cosine in [0, 1]) or by `rank`, never by `score`: an RRF
    # score is at most 2/rrf_k (0.033 by default), so a cosine-style threshold keeps nothing.
    kept = [r for r in rows if r["rank(c)"] < 3 and (r["vector_score(c)"] or 0.0) >= 0.3]

    # When the caller fixes the universe (an ACL, the document the user selected),
    # pass it as `candidates`: ranking happens inside it, on both branches.
    allowed = [nid for key, nid in ids.items() if key.startswith("riego#")]
    hits = g.search_hybrid(text=question, vector=q, model="m", k=4, candidates=allowed, hydrate=True)
    # recipe:retrieve:end
    assert rows and rows[0]["rank(c)"] == 0 and rows[0]["c.text"].startswith("El riego por goteo"), rows[:2]
    assert all(0.0 <= r["vector_score(c)"] <= 1.0 for r in rows if r["vector_score(c)"] is not None), rows
    assert any(r["text_score(c)"] is not None for r in rows), "the text branch ranked the riego chunks"
    assert all(0 < r["score(c)"] <= 2 / 60 + 1e-6 for r in rows), "RRF, not cosine"
    assert kept and {r["c.text"] for r in kept} <= {r["c.text"] for r in rows}
    assert hits and {h["node_id"] for h in hits} <= set(allowed), hits
    assert all(h["node"]["properties"]["doc"] == "riego" for h in hits)

    # recipe:expand:start
    # (a) Two hops in the same query (0.6.19): chunk → entity → related entity, the chunk's
    #     score on every row. One row per path; `distinct` collapses them.
    two = list(g.execute_nql(
        f"find distinct c.text, f.name, score(c) "
        f"from (c:Chunk)-[:MENTIONS]->(e:Entity)-[:RELATED]->(f:Entity) "
        f'where hybrid(c, text = "{question}", vector = {lit}, model = "m", k = 4)'))

    # (b) The neighbourhood as a SET with a budget: each node once, at its minimum depth,
    #     ranked by Personalized PageRank from the hits, `max_nodes` as the cap.
    seeds = [h["node_id"] for h in hits]
    ctx = g.neighborhood(seeds, depth=2, direction="both", labels=["Entity"], max_nodes=20,
                         rank="ppr", seed_weights={h["node_id"]: h["score"] for h in hits})

    # Context budget: hits first (best first), then the ranked subgraph, until it fills.
    budget, context = 600, []
    for piece in [h["node"]["properties"]["text"] for h in hits] + \
                 [n["properties"]["name"] for n in ctx["nodes"] if n["label"] == "Entity"]:
        if budget - len(piece) < 0:
            break
        budget -= len(piece)
        context.append(piece)
    # recipe:expand:end
    assert {r["f.name"] for r in two} >= {"Nopal"}, two
    assert all(0 < r["score(c)"] <= 2 / 60 + 1e-6 for r in two)
    assert "hops: 2" in g.execute_nql(
        f"explain find c.text, f.name from (c:Chunk)-[:MENTIONS]->(e:Entity)-[:RELATED]->(f:Entity) "
        f'where hybrid(c, text = "{question}", vector = {lit}, model = "m", k = 4)').explain
    depths = {n["properties"].get("name"): ctx["depth"][n["id"]] for n in ctx["nodes"] if n["label"] == "Entity"}
    assert 2 in depths.values() and depths["Nopal"] == 1, depths
    assert all(n["label"] == "Entity" or n["id"] in seeds for n in ctx["nodes"])
    assert context and sum(len(p) for p in context) <= 600 and context[0] == hits[0]["node"]["properties"]["text"]
    g.close()

print("graphrag_sample: OK")
