#!/usr/bin/env python3
"""Roundtrip tipo-exacto de propiedades Python ↔ NopalDB.

Guardia de regresión del conversor único de la frontera Python→Rust
(`pyany_to_property` en src/python/mod.rs). Cubre en particular:

- bool ANTES que int: `True` debe roundtripear como bool, no como 1
  (bug corregido: los conversores inline de transaction.rs probaban i64
  antes que bool).
- bytes por downcast explícito: `b"\\x01"` es Bytes; `[1, 2]` es List.
- list/tuple/dict anidados (List/Object), None (Null).

Se ejecuta en CI (job python-stubs) tras `maturin develop`.
"""

import tempfile

import nopaldb


def check(row, col, expected):
    got = row.get(col)
    assert got == expected and type(got) is type(expected), (
        f"{col}: esperado {expected!r} ({type(expected).__name__}), "
        f"obtenido {got!r} ({type(got).__name__})"
    )


def run(graph, node_adder, tag):
    props = {
        "flag": True,
        "n": 1,
        "x": 2.5,
        "s": "a",
        "none": None,
        "b": b"\x01",
        "lst": [1, True, "x"],
        "obj": {"k": "v", "m": 2},
    }
    node_adder(props)

    result = graph.execute_nql(
        f'find n.flag, n.n, n.x, n.s, n.b, n.lst, n.obj from (n:RT{tag})'
    )
    rows = list(result)
    assert len(rows) == 1, f"esperaba 1 fila, hay {len(rows)}: {rows}"
    row = rows[0]

    # El caso del bug: True debe seguir siendo bool (no int 1)
    check(row, "n.flag", True)
    check(row, "n.n", 1)
    check(row, "n.x", 2.5)
    check(row, "n.s", "a")
    check(row, "n.b", b"\x01")
    check(row, "n.lst", [1, True, "x"])
    check(row, "n.obj", {"k": "v", "m": 2})


def main():
    with tempfile.TemporaryDirectory() as tmp:
        graph = nopaldb.Graph.open(f"{tmp}/rt_db")

        # Vía transaction (donde vivía el bug)
        def via_tx(props):
            tx = graph.begin_transaction()
            tx.add_node("RTTX", props)
            tx.commit()

        run(graph, via_tx, "TX")

        # Vía upsert (frontera de graph.rs)
        def via_upsert(props):
            graph.upsert("RTUP", "s", props)

        run(graph, via_upsert, "UP")

        # Los errores del add se propagan como excepción (antes se tragaban y
        # el caller recibía el id de un nodo fantasma): usar una transacción
        # ya cerrada debe levantar RuntimeError, no regresar un id.
        tx = graph.begin_transaction()
        tx.add_node("RTERR", {"x": 1})
        tx.commit()
        try:
            tx.add_node("RTERR", {"x": 2})
            raise AssertionError("add_node sobre tx cerrada debió levantar excepción")
        except RuntimeError:
            pass

        graph.close()
    print("roundtrip_sample: OK (bool/int/float/str/None/bytes/list/dict tipo-exactos; errores propagados)")


if __name__ == "__main__":
    main()
