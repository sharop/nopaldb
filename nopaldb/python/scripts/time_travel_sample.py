"""Guard: NQL `find … at timestamp <ts>` from Python (#298).

- The query runs on the graph as it was at `ts` (a value of
  `last_commit_timestamp()`): an updated property reads its old value, a
  deleted node is still there, a node created later is not.
- At the latest commit it answers like the query without the clause.
- `explain` says the graph was materialized and how big it was.
- Searches over data without history (vectors) are a named error.
"""
import os
import tempfile

import nopaldb


def names(g, q):
    return sorted(r["r.nombre"] for r in g.execute_nql(q))


with tempfile.TemporaryDirectory() as tmp:
    g = nopaldb.Graph.open(os.path.join(tmp, "cocina"))
    g.execute_nql('add (r:Receta {nombre: "mole", paso: "v1"})')
    t1 = g.last_commit_timestamp()
    g.execute_nql('update (r:Receta) set r.paso = "v2" where r.nombre = "mole"')
    g.execute_nql('add (r:Receta {nombre: "pozole"})')
    t3 = g.last_commit_timestamp()
    g.execute_nql('delete (r:Receta) where r.nombre = "mole"')
    t4 = g.last_commit_timestamp()

    rows = list(g.execute_nql(f"find r.paso from (r:Receta) at timestamp {t1}"))
    assert rows == [{"r.paso": "v1"}], rows
    assert names(g, f"find r.nombre from (r:Receta) at timestamp {t1}") == ["mole"]
    assert names(g, f"find r.nombre from (r:Receta) at timestamp {t3}") == ["mole", "pozole"]
    assert names(g, f"find r.nombre from (r:Receta) at timestamp {t4}") == ["pozole"]
    assert names(g, f"find r.nombre from (r:Receta) at timestamp {t4}") == names(g, "find r.nombre from (r:Receta)")

    plan = g.execute_nql(f"explain find r.nombre from (r:Receta) at timestamp {t3}").explain
    assert plan is not None and f"materialized at timestamp {t3} (2 nodes, 0 edges)" in plan, plan

    try:
        g.execute_nql(f'find r.nombre from (r:Receta) where similar_to(r, "ref") at timestamp {t3}')
        raise AssertionError("expected an error")
    except Exception as err:  # noqa: BLE001 - the message is the contract
        assert "not available with AT TIMESTAMP" in str(err), err
    g.close()

print("time_travel_sample: ok")
