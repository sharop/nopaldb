#!/usr/bin/env python3
"""Búsqueda híbrida con filtro selectivo desde Python (#213).

Guardia de la wheel: el único fragmento de un tenant que contiene el término
sale aunque más de 1000 fragmentos de otros tenants también lo contengan.
Antes de 0.6.13 la rama de texto pedía el top-1000 global y filtraba después:
`hits = 0`, `text.underfilled = True`, y una capa RAG se abstenía con
evidencia existente. `search_hybrid_explain` dice ahora por qué camino fue la
rama de texto (`text_path`). Corpus sintético. Se ejecuta en CI (job
python-stubs) tras `maturin develop`.
"""

import os
import tempfile

import nopaldb

with tempfile.TemporaryDirectory() as tmp:
    g = nopaldb.Graph.open(os.path.join(tmp, "tenants"))
    g.upsert_many(
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
    g.close()

print("hybrid_filter_sample: ok")
