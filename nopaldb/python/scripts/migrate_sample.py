"""Guard: 0.6.0 engine flip from Python.

- `Graph.open` on a new directory uses redb; `storage_engine()` says so.
- A database created with engine="sled" reopens with `Graph.open` (auto) as sled.
- `Graph.migrate` copies it to redb with verification and the data is there.
"""
import tempfile
import os

import nopaldb

with tempfile.TemporaryDirectory() as tmp:
    new_dir = os.path.join(tmp, "new")
    g = nopaldb.Graph.open(new_dir)
    assert g.storage_engine() == "redb", g.storage_engine()
    g.close()
    del g

    sled_dir = os.path.join(tmp, "old_sled")
    g = nopaldb.Graph.open_with_options(sled_dir, engine="sled")
    assert g.storage_engine() == "sled"
    for n in ("nopal", "maguey", "biznaga"):
        g.upsert("Planta", "nombre", {"nombre": n})
    # A user index lives outside the KV (indexes/metadata.bin): it must travel
    # with the migration (#152).
    idx_name = g.create_index("Planta", "nombre", "hash")
    g.close()
    del g

    g = nopaldb.Graph.open(sled_dir)  # auto: keeps sled
    assert g.storage_engine() == "sled", g.storage_engine()
    assert g.node_count() == 3, g.node_count()
    g.close()
    del g

    redb_dir = os.path.join(tmp, "migrated")
    report = nopaldb.Graph.migrate(sled_dir, redb_dir)
    assert report["verified"] is True, report
    assert any(k["name"] == "entities" and k["pairs"] == 3 for k in report["keyspaces"]), report
    assert [ix["name"] for ix in report["indexes"]] == [idx_name], report
    assert report["indexes"][0]["type"] == "Hash" and report["indexes"][0]["analyzer"] is None, report
    assert any(sc["dir"] == "indexes" and sc["files"] >= 1 for sc in report["sidecars"]), report
    assert report["hnsw_copied"] is False, report  # no embeddings in this base

    g = nopaldb.Graph.open(redb_dir)
    assert g.storage_engine() == "redb"
    assert g.node_count() == 3, g.node_count()
    assert g.get_label_count("Planta") == 3
    names = [ix[0] for ix in g.list_indexes()]
    assert names == [idx_name], names
    assert g.rebuild_indexes() == 1
    g.close()
    del g

    try:
        nopaldb.Graph.open_with_options(sled_dir, engine="bogus")
        raise SystemExit("engine inválido debió fallar")
    except ValueError as e:
        assert "auto" in str(e) and "redb" in str(e), e

print("migrate_sample: OK (redb por defecto, sled detectado, migración verificada con índices)")
