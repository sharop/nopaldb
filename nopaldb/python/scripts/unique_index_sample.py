#!/usr/bin/env python3
"""Índices únicos desde Python (M1-8).

Guardia de la wheel: `create_index(..., unique=True)` existe, `describe_index`
y `get_stats()` reportan `"unique"`, una escritura (NQL o transacción) que
repite el valor falla y no escribe nada, `upsert` sobre la clave actualiza en
vez de fallar, y la marca sobrevive a reabrir. Datos ficticios. Se ejecuta en
CI (job python-stubs) tras `maturin develop`.
"""

import os
import tempfile

import nopaldb


def personas(g):
    return sorted(r["p.email"] for r in g.execute_nql("find p.email from (p:Persona)"))


def main() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        path = os.path.join(tmp, "unique.db")
        g = nopaldb.Graph.open(path)
        name = g.create_index("Persona", "email", unique=True)
        assert name == "Persona_email", name
        assert g.describe_index(name)["unique"] is True
        assert [ix["unique"] for ix in g.get_stats()["indexes"]] == [True]

        add = 'add (p:Persona {email: "s@ejemplo.org"})'
        g.execute_nql(add)
        try:
            g.execute_nql(add)
            raise AssertionError("a repeated email must be refused")
        except RuntimeError as e:
            assert "Unique index violation" in str(e), e

        # La transacción entera se rechaza.
        tx = g.begin_transaction()
        tx.add_node("Persona", {"email": "j@ejemplo.org"})
        tx.add_node("Persona", {"email": "s@ejemplo.org"})
        try:
            tx.commit()
            raise AssertionError("the commit must be refused")
        except RuntimeError as e:
            assert "Unique index violation" in str(e), e
        assert personas(g) == ["s@ejemplo.org"], personas(g)

        status, _ = g.upsert("Persona", "email", {"email": "s@ejemplo.org", "nombre": "Sergio"})
        assert status == "updated", status

        for bad in [
            lambda: g.create_index("Nota", "cuerpo", "fulltext", unique=True),
            lambda: g.create_index("Nota", "cuerpo", "hash", analyzer={"language": "spanish"}, unique=True),
        ]:
            try:
                bad()
                raise AssertionError("must be refused")
            except ValueError:
                pass

        # Un índice común sigue siendo común.
        g.create_index("Persona", "nombre")
        assert g.describe_index("Persona_nombre")["unique"] is False
        g.close()

        g = nopaldb.Graph.open(path)
        assert g.describe_index("Persona_email")["unique"] is True
        g.drop_index("Persona_email")
        g.execute_nql(add)
        g.close()
    print("unique_index: ok")


if __name__ == "__main__":
    main()
