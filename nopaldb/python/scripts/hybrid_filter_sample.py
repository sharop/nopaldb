#!/usr/bin/env python3
"""Búsqueda híbrida con filtro selectivo desde Python (#213).

Guardia de la wheel: el único fragmento de un tenant que contiene el término
sale aunque más de 1000 fragmentos de otros tenants también lo contengan.
Antes de 0.6.13 la rama de texto pedía el top-1000 global y filtraba después:
`hits = 0`, `text.underfilled = True`, y una capa RAG se abstenía con
evidencia existente. `search_hybrid_explain` dice ahora por qué camino fue la
rama de texto (`text_path`), si la rama vectorial fue exacta (`vector_exact`,
#218: con un índice chico el filtro se resolvía exacto pero se informaba
`hnsw_filtered`) y de dónde salió el universo (`candidate_set`, #214: el
caller puede pasar `candidates=[ids]` y el filtro solo lo reduce). Corpus
sintético. Se ejecuta en CI (job python-stubs) tras
`maturin develop`.
"""

import os
import tempfile

import nopaldb

with tempfile.TemporaryDirectory() as tmp:
    g = nopaldb.Graph.open(os.path.join(tmp, "tenants"))
    upserted = g.upsert_many(
        [
            {
                "label": "Fragment",
                "key": "key",
                "props": {"key": "allowed", "tenant_id": "B", "case_id": "b", "body": "fragmento unico"},
            },
            *[
                {
                    "label": "Fragment",
                    "key": "key",
                    "props": {"key": f"noise-{i}", "tenant_id": "A", "case_id": "a", "body": "fragmento"},
                }
                for i in range(1023)
            ],
        ]
    )
    g.create_index("Fragment", "body", "fulltext", analyzer={"language": "spanish"})

    query = {"text": "fragmento", "label": "Fragment", "props": {"tenant_id": "B", "case_id": "b"}, "k": 8}
    hits = g.search_hybrid(**query)
    assert len(hits) == 1, hits
    e = g.search_hybrid_explain(**query)
    assert e["allowed_set_size"] == 1, e
    assert e["text_path"] == "term_set_over_allowed", e
    assert e["text"]["returned"] == 1 and e["text"]["underfilled"], e["text"]
    assert e["hits"][0]["node_id"] == hits[0]["node_id"]
    assert e["hits"][0]["text_score"] is not None

    # Sin filtro, el camino es el global de siempre.
    assert g.search_hybrid_explain(text="fragmento", k=8)["text_path"] == "global"

    # Rama vectorial sobre un índice chico (1 permitido + 64 distractores más
    # cercanos): el índice lo resuelve exacto y explain lo dice.
    allowed_id = hits[0]["node_id"]
    noise_ids = [node_id for (_outcome, node_id) in upserted[1:65]]
    g.add_node_embeddings(
        "m",
        [(allowed_id, [0.0, 1.0, 0.0])] + [(nid, [1.0, 0.0, 0.0]) for nid in noise_ids],
    )
    e = g.search_hybrid_explain(vector=[1.0, 0.0, 0.0], model="m", k=8, label="Fragment", props={"tenant_id": "B", "case_id": "b"})
    assert e["vector_path"] == "exact_index" and e["vector_exact"] is True, e
    assert [h["node_id"] for h in e["hits"]] == [allowed_id], e["hits"]
    assert e["vector"]["returned"] == 1 and e["vector"]["underfilled"], e["vector"]
    e = g.search_hybrid_explain(vector=[1.0, 0.0, 0.0], model="m", k=8)
    assert e["vector_path"] == "unfiltered" and e["vector_exact"] is True, e

    # Universo explícito (#214): el fragmento del documento seleccionado está
    # detrás de 64 distractores más cercanos; pedir 32 dentro del tenant y
    # filtrar el documento después lo pierde. Darlo como `candidates` lo
    # encuentra con rango en ambas ramas, y `label`/`props` solo pueden
    # reducir el conjunto, nunca ampliarlo.
    g.add_node_embeddings("m", [(allowed_id, [0.6, 0.8, 0.0])])
    e = g.search_hybrid_explain(
        text="fragmento", vector=[1.0, 0.0, 0.0], model="m", k=8,
        label="Fragment", props={"tenant_id": "B"},
        candidates=[allowed_id, *noise_ids[:4]],
    )
    assert [h["node_id"] for h in e["hits"]] == [allowed_id], e["hits"]
    assert e["hits"][0]["text_rank"] == 0 and e["hits"][0]["vector_rank"] == 0, e["hits"][0]
    assert e["candidate_set"] == {"source": "ids_and_filter", "caller_size": 5, "size": 1}, e["candidate_set"]
    assert e["allowed_set_size"] == 1
    hits = g.search_hybrid(text="fragmento", k=8, candidates=[allowed_id, *noise_ids[:4]])
    assert {h["node_id"] for h in hits} == {allowed_id, *noise_ids[:4]}, hits
    g.close()

print("hybrid_filter_sample: ok")
