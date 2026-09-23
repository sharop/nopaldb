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
    for i, c in enumerate(chunks):
        g.add_node_embedding(c, vec(i), "m")
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
    small = g.neighborhood([c0], depth=2, max_nodes=2)
    assert small["truncated"] is True and len(small["nodes"]) == 2
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
    g.close()

print("graphrag_sample: OK")
