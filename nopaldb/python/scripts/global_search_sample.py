"""Guard: GraphRAG global search on NopalDB (0.6.11, #191).

End to end with a fake LLM and a fake embedder (no network):
1. communities: leiden_hierarchy -> materialize_communities;
2. reports: one per community, written with upsert_community_report;
3. global search: similar_to over the reports of one level (Python and NQL),
   map (partial answer + score per report), reduce;
4. staleness: an edge inside a community marks that community and its
   ancestors; rewriting their reports leaves nothing stale.

Fictional domain: a network of community gardens.
"""
import os
import random
import tempfile

import nopaldb

TOPICS = {
    "riego": ["goteo", "aspersor", "cisterna", "manguera", "turno de riego", "bomba",
              "sensor de humedad", "canal", "acolchado", "temporizador", "pozo", "filtro"],
    "plagas": ["pulgón", "mosca blanca", "caracol", "trampa amarilla", "jabón potásico", "mariquita",
               "neem", "rotación", "malla", "hormiga", "oídio", "cal"],
    "compostaje": ["compostera", "lombriz", "hojarasca", "volteo", "humus", "posos de café",
                   "cáscaras", "temperatura", "humedad del compost", "aireación", "tamiz", "bocashi"],
}
WORDS = sorted(TOPICS)
DIM = len(WORDS)


def embed(text):
    """Fake embedder: how much the text talks about each topic."""
    text = text.lower()
    v = [1e-3 + sum(text.count(w) for w in [t] + TOPICS[t]) for t in WORDS]
    n = sum(x * x for x in v) ** 0.5
    return [x / n for x in v]


def fake_llm_report(names, level):
    """Fake LLM: a report from the members of a community."""
    topic = max(WORDS, key=lambda t: sum(n in TOPICS[t] for n in names))
    return f"Tema {topic} (nivel {level})", f"Sobre {topic}: " + ", ".join(sorted(names)), float(len(names))


def fake_llm_partial(question, report):
    """Map step: a partial answer and how useful the report is (0-100)."""
    words = [w for w in TOPICS if w in question]
    score = 100 * sum(report["summary"].count(w) for w in words)
    return report["summary"], score


random.seed(5)
with tempfile.TemporaryDirectory() as tmp:
    g = nopaldb.Graph.open(os.path.join(tmp, "huertos"))
    ids = {}
    with g.bulk_loader(500) as l:
        for topic, names in TOPICS.items():
            for name in names:
                ids[name] = l.add_node("Entity", {"name": name, "topic": topic})
        for names in TOPICS.values():  # dense inside a topic
            for i, a in enumerate(names):
                for b in names[i + 1:]:
                    if random.random() < 0.6:
                        l.add_edge(ids[a], ids[b], "RELATED")
        l.add_edge(ids["goteo"], ids["pulgón"], "RELATED")  # sparse between topics
        l.add_edge(ids["lombriz"], ids["filtro"], "RELATED")

    # 1. Communities.
    levels = g.leiden_hierarchy(labels=["Entity"], max_cluster_size=5)
    assert len(set(levels[0].values())) == 3, "one community per topic"
    g.materialize_communities(levels)

    # 2. Reports, one per community.
    def write_reports(keys=None):
        rows = g.execute_nql("find c.key, c.level from (c:Community)")
        for key, level in [(r["c.key"], r["c.level"]) for r in rows]:
            if keys is not None and key not in keys:
                continue
            members = g.execute_nql(f'find e.name from (e:Entity)-[:IN_COMMUNITY]->(c:Community) where c.key = "{key}"')
            title, summary, rating = fake_llm_report([m["e.name"] for m in members], level)
            g.upsert_community_report(key, title, summary, rating=rating, vector=embed(summary), model="m")

    stale = g.stale_reports()
    assert stale and {s["status"] for s in stale} == {"missing"}
    write_reports()
    assert g.stale_reports() == [], "every report is fresh"

    # 3. Global search over level 0: retrieve reports, map, reduce.
    question = "¿cómo organizamos el riego del huerto?"
    q = embed(question)
    hits = g.search_hybrid(vector=q, model="m", k=3, label="Report", props={"level": 0}, hydrate=True)
    reports = [h["node"]["properties"] for h in hits]
    assert reports and all(r["level"] == 0 for r in reports)
    assert reports[0]["title"].startswith("Tema riego"), reports[0]
    partials = sorted((fake_llm_partial(question, r) for r in reports), key=lambda p: -p[1])
    answer = " | ".join(text for text, score in partials if score > 0)  # reduce
    assert "goteo" in answer and "lombriz" not in answer, answer

    # Same retrieval in NQL: the question's vector as a literal.
    literal = "[" + ", ".join(f"{x:.6f}" for x in q) + "]"
    rows = list(g.execute_nql(
        f'find r.title, r.level from (r:Report) where similar_to(r, vector = {literal}, model = "m", k = 10)'))
    top0 = [r for r in rows if r["r.level"] == 0]
    assert top0 and top0[0]["r.title"].startswith("Tema riego"), rows

    # 4. Staleness: a new edge inside the "plagas" community.
    tx = g.begin_transaction()
    tx.add_edge(ids["pulgón"], ids["neem"], "RELATED", {"source": "nota de campo"})
    tx.commit()
    stale = g.stale_reports()
    assert stale and {s["status"] for s in stale} == {"stale"}, stale
    by_level = {}
    for s in stale:
        by_level.setdefault(s["level"], []).append(s["community_key"])
    assert all(len(v) == 1 for v in by_level.values()), "one community per level: it and its ancestors"
    top = g.execute_nql(f'find r.title from (r:Report) where r.community_key = "{by_level[0][0]}"')
    assert list(top)[0]["r.title"].startswith("Tema plagas")
    write_reports({s["community_key"] for s in stale})
    assert g.stale_reports() == []
    g.close()
print("global_search_sample: OK")
