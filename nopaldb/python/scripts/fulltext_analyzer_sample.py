#!/usr/bin/env python3
"""Analyzer full-text por índice desde Python (#74).

Guardia de la wheel: `create_index(..., analyzer={...})` y `describe_index`
existen, el analyzer español pliega acentos y hace stemming en documentos Y
consultas, y el índice sin analyzer se comporta como antes. Corpus ficticio
(fichas de una biblioteca de recetarios). Se ejecuta en CI (job python-stubs)
tras `maturin develop`.
"""

import os
import tempfile

import nopaldb

FICHAS = [
    ("f1", "Clasificación de los catálogos de recetas por región"),
    ("f2", "El catálogo de postres de la biblioteca"),
    ("f3", "Inventario de utensilios de cocina"),
]


def load(g):
    ids = {}
    for name, body in FICHAS:
        _status, node_id = g.upsert("Ficha", "name", {"name": name, "body": body})
        ids[node_id] = name
    return ids


def hits(g, ids, text):
    return sorted(ids[h["node_id"]] for h in g.search_hybrid(text=text, k=10) if h["node_id"] in ids)


def main() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        g = nopaldb.Graph.open(os.path.join(tmp, "es.db"))
        ids = load(g)
        name = g.create_index("Ficha", "body", "fulltext", analyzer={"language": "spanish"})
        assert name == "Ficha_body", name
        info = g.describe_index("Ficha_body")
        assert info["type"] == "FullText" and info["analyzer"] == {
            "language": "spanish", "stemming": True, "stopwords": True, "ascii_folding": True,
        }, info
        assert hits(g, ids, "clasificacion") == ["f1"], hits(g, ids, "clasificacion")
        assert hits(g, ids, "catalogos") == ["f1", "f2"], hits(g, ids, "catalogos")
        assert hits(g, ids, "de") == [], hits(g, ids, "de")

        try:
            g.create_index("Ficha", "body", "fulltext", analyzer={"language": "english"})
        except Exception as e:  # noqa: BLE001 — la excepción es la API
            assert "drop index Ficha_body" in str(e), e
        else:
            raise AssertionError("cambiar el analyzer sin drop debería fallar")

        try:
            g.create_index("Ficha", "name", "hash", analyzer={"language": "spanish"})
        except Exception as e:  # noqa: BLE001
            assert "only applies to full-text" in str(e), e
        else:
            raise AssertionError("analyzer en un índice hash debería fallar")

        try:
            g.create_index("Ficha", "name", "fulltext", analyzer={"lang": "spanish"})
        except ValueError as e:
            assert "unknown key 'lang'" in str(e), e
        else:
            raise AssertionError("clave desconocida debería fallar")
        g.close()

        h = nopaldb.Graph.open(os.path.join(tmp, "default.db"))
        ids = load(h)
        h.create_index("Ficha", "body", "fulltext")
        assert h.describe_index("Ficha_body")["analyzer"] == {
            "language": None, "stemming": False, "stopwords": False, "ascii_folding": False,
        }
        assert hits(h, ids, "clasificacion") == [], "el default no pliega acentos"
        assert hits(h, ids, "clasificación") == ["f1"]
        assert h.describe_index("no_existe") is None
        h.close()
    print("fulltext analyzer OK")


if __name__ == "__main__":
    main()
