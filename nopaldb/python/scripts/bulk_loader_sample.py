"""Guard: BulkLoader and the counts (W1 of the Python wrapper plan).

- `add_edge` takes properties and returns the edge id, like `Transaction`.
- Node and edge properties go through the one shared converter: None is
  null (it used to be ""), bytes/lists/dicts round-trip with exact types.
- The context manager flushes; a finished loader refuses more rows.
- `node_count()` / `edge_count()` are exact after a bulk load and after a
  later transactional write.
- `edges_to_arrow()` on a graph without edges is an empty IPC stream (it
  used to raise ValueError); `to_arrow(label=)` and `to_arrow_complete()`
  return bytes.
"""
import os
import tempfile
import uuid

import nopaldb

PROPS = {
    "flag": True,
    "n": 1,
    "x": 2.5,
    "s": "a",
    "none": None,
    "b": b"\x01\x02",
    "lst": [1, True, "x", None],
    "obj": {"k": "v", "m": 2, "inner": {"z": False}},
}


def check(row, key, expected):
    got = row[key]
    assert got == expected and type(got) is type(expected), (key, got, expected)


with tempfile.TemporaryDirectory() as tmp:
    g = nopaldb.Graph.open(os.path.join(tmp, "bl"))

    # 1. edges_to_arrow on an empty graph: bytes, not ValueError.
    ipc = g.edges_to_arrow()
    assert isinstance(ipc, bytes) and ipc, type(ipc)
    try:
        import pyarrow as pa  # optional locally; the CI job does not install it
    except ImportError:
        pa = None
    if pa is not None:
        rb = pa.ipc.open_stream(ipc).read_all()
        assert rb.num_rows == 0 and rb.schema.names == ["id", "source", "target", "edge_type"], rb.schema

    # 2. context manager; batch_size 2 forces a mid-load flush.
    with g.bulk_loader(2) as loader:
        assert repr(loader) == "<BulkLoader: active>"
        a = loader.add_node("Planta", PROPS)
        b = loader.add_node("Planta", {"s": "b"})
        c = loader.add_node("Planta", {"s": "c"})
        e1 = loader.add_edge(a, b, "Riego", {"since": 2020, "ok": True, "note": None, "w": 0.5})
        e2 = loader.add_edge(b, c, "Riego")  # properties omitted
        uuid.UUID(e1), uuid.UUID(e2)
        assert e1 != e2
    assert repr(loader) == "<BulkLoader: finished>"
    try:
        loader.add_node("Planta", {})
        raise SystemExit("a finished loader must refuse rows")
    except RuntimeError:
        pass
    try:
        loader.add_edge("not-a-uuid", b, "Riego")
        raise SystemExit("finished first, then invalid uuid")
    except RuntimeError:
        pass

    # 3. counts are exact after the load, and after a later transaction.
    assert (g.node_count(), g.edge_count()) == (3, 2), (g.node_count(), g.edge_count())
    tx = g.begin_transaction()
    tx.add_node("Planta", {"s": "d"})
    tx.commit()
    assert g.node_count() == 4, g.node_count()
    assert g.get_stats()["graph"]["total_edges"] == 2

    # 4. type-exact round trip of node properties (None is None, not "").
    rows = [r for r in g.execute_nql("find n.flag, n.n, n.x, n.s, n.none, n.b, n.lst, n.obj from (n:Planta)") if r["n.s"] == "a"]
    assert len(rows) == 1, rows
    row = rows[0]
    check(row, "n.flag", True)
    check(row, "n.n", 1)
    check(row, "n.x", 2.5)
    check(row, "n.s", "a")
    check(row, "n.none", None)
    check(row, "n.b", b"\x01\x02")
    check(row, "n.lst", [1, True, "x", None])
    check(row, "n.obj", {"k": "v", "m": 2, "inner": {"z": False}})

    # 5. edge properties travelled too.
    erows = list(g.execute_nql("find e.since, e.ok, e.note, e.w from (a:Planta)-[e:Riego]->(b:Planta)"))
    assert len(erows) == 2, erows
    # A property the edge does not have is absent from the row; one stored as
    # None is present with None.
    with_props = [r for r in erows if "e.since" in r]
    assert len(with_props) == 1 and any(r == {} for r in erows), erows
    check(with_props[0], "e.since", 2020)
    check(with_props[0], "e.ok", True)
    check(with_props[0], "e.note", None)
    check(with_props[0], "e.w", 0.5)

    # 6. finish() without `with` returns the stats dict.
    l2 = g.bulk_loader(1000)
    l2.add_node("Planta", {"s": "z"})
    st = l2.finish()
    assert set(st) == {"nodes", "edges", "duration_secs", "nodes_per_second"}, st
    assert st["nodes"] == 1 and st["edges"] == 0, st

    # 7. Arrow exports on a populated graph are bytes.
    assert isinstance(g.to_arrow(label="Planta"), bytes)
    nb, eb = g.to_arrow_complete()
    assert isinstance(nb, bytes) and isinstance(eb, bytes) and nb and eb
    if pa is not None:
        assert pa.ipc.open_stream(eb).read_all().num_rows == 2

    # 8. invalid UUID is a ValueError on a live loader.
    l3 = g.bulk_loader(10)
    try:
        l3.add_edge("not-a-uuid", b, "Riego")
        raise SystemExit("invalid uuid must raise ValueError")
    except ValueError:
        pass
    l3.finish()
    g.close()

print("bulk_loader_sample: OK")
