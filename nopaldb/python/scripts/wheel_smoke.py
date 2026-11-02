#!/usr/bin/env python3
"""Prueba de humo de una wheel instalada, en su sistema operativo (#370).

`release-python.yml` la corre con cada wheel recién construida, instalada en
un entorno limpio, antes de publicar: si falla, la wheel no sube a PyPI. Toca
lo que depende de la plataforma: crear la base en disco (en Windows se cierra
antes del rename), transacciones, el índice HNSW construido en paralelo y su
volcado, el índice full-text, cerrar, reabrir con los mismos resultados y
borrar el directorio (en Windows falla si queda un archivo abierto).

Dominio ficticio: fichas de recetas encadenadas.
"""

import os
import random
import shutil
import tempfile

import nopaldb

DOCS = 2000  # por encima del umbral de persistencia del índice HNSW (1024)
DIM = 16
WORDS = ["mole", "tamal", "pozole", "atole", "elote", "chile", "sopa", "salsa"]


def build(path):
    g = nopaldb.Graph.open(path)
    tx = g.begin_transaction()
    ids = []
    for i in range(DOCS):
        body = f"receta {WORDS[i % len(WORDS)]} numero {i}"
        ids.append(tx.add_node("Doc", {"i": i, "body": body}))
    for a, b in zip(ids, ids[1:]):
        tx.add_edge(a, b, "SIGUE")
    tx.commit()

    rng = random.Random(7)
    vectors = {i: [rng.uniform(-1.0, 1.0) for _ in range(DIM)] for i in ids}
    assert g.add_node_embeddings("m", list(vectors.items())) == DOCS
    g.create_index("Doc", "body", "fulltext")
    return g, ids, vectors


def answers(g, ids, vectors):
    sample = ids[::50]
    knn = [g.knn_nodes(vectors[i], 5, "m")[0][0] for i in sample]
    text = sorted(h["node_id"] for h in g.search_hybrid(text="pozole", k=DOCS))
    rows = list(g.execute_nql("find count(*) from (a:Doc)-[:SIGUE]->(b:Doc)"))
    return {"knn": knn, "text": text, "hops": rows, "nodes": g.node_count(), "edges": g.edge_count()}


def main() -> None:
    tmp = tempfile.mkdtemp(prefix="nopaldb-smoke-")
    path = os.path.join(tmp, "base.db")
    try:
        g, ids, vectors = build(path)
        before = answers(g, ids, vectors)
        sample = ids[::50]
        assert before["knn"] == sample, "cada vector es su propio vecino más cercano"
        assert len(before["text"]) == DOCS // len(WORDS), len(before["text"])
        assert before["hops"] == [{"count(*)": DOCS - 1}], before["hops"]
        g.close()

        g = nopaldb.Graph.open(path)
        after = answers(g, ids, vectors)
        assert after == before, (before, after)
        stats = g.embedding_index_stats("m")
        assert stats is not None and stats["size"] == DOCS, stats
        assert stats["loaded_from_disk_ms"] is not None, "el índice HNSW se recarga del volcado"
        g.close()
    finally:
        shutil.rmtree(tmp)
    assert not os.path.exists(tmp)
    print(f"wheel OK: nopaldb {nopaldb.__version__}")


if __name__ == "__main__":
    main()
