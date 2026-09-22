"""Guard: `get_stats()` and the progress callback (#158).

- get_stats() is a nested dict with native types and keeps the 0.6.x flat
  string keys one more minor.
- After 30 commits and no close(), reopening reports the WAL it read and a
  crash recovery; after checkpoint() + close() it reports one record and none.
- `indexes` carries size and analyzer; `hnsw` and `gc` are present.
- The progress callback sees `upsert_batch` and `index_build` start to end,
  and an open with `on_progress` sees `wal_replay`.
"""
import os
import tempfile

import nopaldb

with tempfile.TemporaryDirectory() as tmp:
    d = os.path.join(tmp, "obs")
    g = nopaldb.Graph.open(d)
    for i in range(30):
        tx = g.begin_transaction()
        tx.add_node("Planta", {"nombre": f"planta-{i}", "n": i})
        tx.commit()

    s = g.get_stats()
    assert set(s) >= {"graph", "storage", "wal", "recovery", "indexes", "hnsw", "gc"}, sorted(s)
    assert s["graph"]["total_nodes"] == 30 and s["graph"]["nodes_per_label"] == {"Planta": 30}, s["graph"]
    assert s["storage"]["engine"] == g.storage_engine() and s["storage"]["data_dir"] == d, s["storage"]
    assert s["storage"]["read_only"] is False
    assert s["wal"]["checkpoints_this_session"] == 0 and s["wal"]["last_checkpoint_unix_ms"] is None, s["wal"]
    assert s["wal"]["bytes"] > 1000 and s["wal"]["checkpoint_threshold_bytes"] == 16 * 1024 * 1024, s["wal"]
    assert s["wal"]["direct_write_durability"] == "process_crash"
    assert s["recovery"]["operations_replayed"] == 0 and s["recovery"]["crash_recovery"] is False, s["recovery"]
    assert s["recovery"]["open_ms"]["total"] >= s["recovery"]["open_ms"]["wal_replay"]
    assert s["indexes"] == [] and s["hnsw"] == [], (s["indexes"], s["hnsw"])
    assert s["gc"] == {"auto_running": False, "auto": None, "last_run": None}, s["gc"]
    # Flat 0.6.x keys: still strings.
    assert s["total_nodes"] == "30" and s["wal_bytes"] == str(s["wal"]["bytes"]), (s["total_nodes"], s["wal_bytes"])
    assert s["storage_engine"] == s["storage"]["engine"] and s["avg_degree"] == "0.00"

    events: list[dict] = []
    g.set_progress_callback(events.append)
    g.upsert_many([{"label": "Fila", "key": "k", "props": {"k": i}} for i in range(1500)])
    name = g.create_index("Planta", "nombre", "hash")
    g.set_progress_callback(None)
    g.upsert("Fila", "k", {"k": 99999})  # no callback: no event
    up = [e for e in events if e["phase"] == "upsert_batch"]
    assert up[0] == {"phase": "upsert_batch", "done": 0, "total": 1500}, up[0]
    assert up[-1] == {"phase": "upsert_batch", "done": 1500, "total": 1500}, up[-1]
    build = [e for e in events if e["phase"] == "index_build"]
    assert build[-1] == {"phase": "index_build", "done": 30, "total": 30}, build
    assert all(e["done"] <= 1500 for e in events), events

    ix = g.get_stats()["indexes"]
    assert ix == [{"name": name, "label": "Planta", "property": "nombre", "type": "Hash", "size": 30, "analyzer": None}], ix

    g.checkpoint()
    s = g.get_stats()
    assert s["wal"]["checkpoints_this_session"] == 1 and s["wal"]["last_checkpoint_unix_ms"] > 1_600_000_000_000, s["wal"]
    assert s["wal"]["bytes"] < 256, s["wal"]
    g.close()  # another checkpoint; the WAL keeps the Checkpoint record only
    del g

    # Reopen with progress: nothing to replay, but the phases are announced.
    events = []
    g = nopaldb.Graph.open_with_options(d, on_progress=events.append)
    r = g.get_stats()["recovery"]
    assert r["wal_records_read"] == 1 and r["operations_replayed"] == 0 and r["crash_recovery"] is False, r
    phases = {e["phase"] for e in events}
    assert {"index_load", "wal_replay"} <= phases, phases
    assert g.get_stats()["indexes"][0]["size"] == 30
    g.close()
    del g

    # Reopen after commits without checkpoint: the WAL is read in full and
    # the open is a crash recovery.
    g = nopaldb.Graph.open(d)
    for i in range(10):
        tx = g.begin_transaction()
        tx.add_node("Planta", {"nombre": f"extra-{i}"})
        tx.commit()
    del g
    g = nopaldb.Graph.open(d)
    r = g.get_stats()["recovery"]
    assert r["wal_records_read"] >= 30 and r["crash_recovery"] is True and r["adjacency_rebuilt"] is True, r
    assert g.get_stats()["graph"]["nodes_per_label"]["Planta"] == 40
    g.close()

print("stats_sample: OK")
