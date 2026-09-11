#!/usr/bin/env python3
"""Índice HNSW desde Python (#113): knn_nodes usa el índice cacheado y un
embedding nuevo se inserta en él sin rebuild.

Guardia de la wheel: `knn_nodes`, `embedding_index_stats` y `add_node_embedding`
existen; tras la primera búsqueda el índice existe (stats no es None); insertar
un vector nuevo lo hace visible y no cambia `tombstones`; actualizar uno deja
un tombstone. Se ejecuta en CI (job python-stubs) tras `maturin develop`.
"""

import nopaldb


def main() -> None:
    g = nopaldb.Graph.in_memory()
    ids = []
    for i in range(20):
        _, node_id = g.upsert("Doc", "i", {"i": i})
        v = [0.01] * 8
        v[i % 8] = 1.0
        g.add_node_embedding(node_id, v, "m")
        ids.append(node_id)

    assert g.embedding_index_stats("m") is None, "el índice es perezoso: nada hasta la primera búsqueda"
    hits = g.knn_nodes([1.0] + [0.01] * 7, 3, "m")
    assert len(hits) == 3 and hits[0][0] in ids, hits
    st = g.embedding_index_stats("m")
    assert st == {
    "model": "m", "size": 20, "tombstones": 0, "dimension": 8, "needs_rebuild": False,
    "persisted": False, "loaded_from_disk_ms": None,
}, st  # 20 puntos: bajo el umbral de persistencia (1024), nada en disco

    _, new_id = g.upsert("Doc", "i", {"i": 99})
    q = [0.0] * 8
    q[3] = 1.0
    q[5] = 1.0
    g.add_node_embedding(new_id, q, "m")
    st = g.embedding_index_stats("m")
    assert st["size"] == 21 and st["tombstones"] == 0, st
    assert g.knn_nodes(q, 1, "m")[0][0] == new_id, "el vector recién insertado es su propio vecino más cercano"

    g.add_node_embedding(new_id, [0.0] * 7 + [1.0], "m")
    st = g.embedding_index_stats("m")
    assert st["size"] == 21 and st["tombstones"] == 1, st
    assert g.knn_nodes([0.0] * 7 + [1.0], 1, "m")[0][0] == new_id
    g.close()
    print("hnsw incremental OK")


if __name__ == "__main__":
    main()
