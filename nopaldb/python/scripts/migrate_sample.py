"""Guard: storage engine and verified copy from Python.

- `Graph.open` on a new directory uses redb, the only engine since 0.8.0.
- A sled database from 0.5.x (sled was removed in 0.8.0, #141) is recognized
  and refused with the migration recipe; so is engine="sled".
- `Graph.migrate` copies a database with verification, user indexes included.
"""
import tempfile
import os

import nopaldb

with tempfile.TemporaryDirectory() as tmp:
    src_dir = os.path.join(tmp, "plantas")
    g = nopaldb.Graph.open(src_dir)
    assert g.storage_engine() == "redb", g.storage_engine()
    for n in ("nopal", "maguey", "biznaga"):
        g.upsert("Planta", "nombre", {"nombre": n})
    # A user index lives outside the KV (indexes/metadata.bin): it must travel
    # with the copy (#152).
    idx_name = g.create_index("Planta", "nombre", "hash")
    g.close()
    del g

    copy_dir = os.path.join(tmp, "copia")
    report = nopaldb.Graph.migrate(src_dir, copy_dir)
    assert report["verified"] is True, report
    assert any(k["name"] == "entities" and k["pairs"] == 3 for k in report["keyspaces"]), report
    assert [ix["name"] for ix in report["indexes"]] == [idx_name], report
    assert report["indexes"][0]["type"] == "Hash" and report["indexes"][0]["analyzer"] is None, report
    assert any(sc["dir"] == "indexes" and sc["files"] >= 1 for sc in report["sidecars"]), report
    assert report["hnsw_copied"] is False, report  # no embeddings in this base

    g = nopaldb.Graph.open(copy_dir)
    assert g.storage_engine() == "redb"
    assert g.node_count() == 3, g.node_count()
    assert g.get_label_count("Planta") == 3
    names = [ix[0] for ix in g.list_indexes()]
    assert names == [idx_name], names
    assert g.rebuild_indexes() == 1
    g.close()
    del g

    # What a sled database leaves on disk: enough to recognize it.
    sled_dir = os.path.join(tmp, "old_sled")
    os.makedirs(sled_dir)
    for f in ("conf", "db"):
        open(os.path.join(sled_dir, f), "wb").close()
    try:
        nopaldb.Graph.open(sled_dir)
        raise SystemExit("una base sled debió fallar")
    except Exception as e:  # noqa: BLE001 - the message is what matters
        assert "sled database" in str(e) and "nopaldb==0.7" in str(e), e
    assert not os.path.exists(os.path.join(sled_dir, "nopal.redb"))

    for engine, needle in (("sled", "nopaldb==0.7"), ("bogus", "auto, redb")):
        try:
            nopaldb.Graph.open_with_options(os.path.join(tmp, "x"), engine=engine)
            raise SystemExit(f"engine={engine!r} debió fallar")
        except ValueError as e:
            assert needle in str(e), e

print("migrate_sample: OK (redb, base sled rechazada con la receta, copia verificada con índices)")
