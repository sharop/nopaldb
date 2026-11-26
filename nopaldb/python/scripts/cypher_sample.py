#!/usr/bin/env python3
"""Cypher de solo lectura desde Python (CYP-01).

Guardia de la wheel: la wheel trae la feature `cypher`; `Graph.cypher_to_nql`
da el NQL sin ejecutarlo, `execute_cypher` devuelve las mismas filas que el
NQL equivalente, y lo que queda fuera del subconjunto (escrituras, parámetros)
es un RuntimeError que nombra la parte. Datos ficticios. Se ejecuta en CI
(job python-stubs) tras `maturin develop`.
"""

import nopaldb


def main() -> None:
    g = nopaldb.Graph.in_memory()
    g.execute_nql('add (c:Cuidador {nombre: "ana"})-[:CUIDA]->(a:Animal {nombre: "michi", edad: 3})')
    g.execute_nql('add (c:Cuidador {nombre: "beto"})-[:CUIDA]->(a:Animal {nombre: "tom", edad: 5})')

    q = "MATCH (c:Cuidador)-[:CUIDA]->(a) WHERE a.edad > 3 RETURN c.nombre, a.nombre"
    nql = nopaldb.Graph.cypher_to_nql(q)
    assert nql == "find c.nombre, a.nombre from (c:Cuidador)-[:CUIDA]->(a) where a.edad > 3", nql

    rows = [(r["c.nombre"], r["a.nombre"]) for r in g.execute_cypher(q)]
    assert rows == [("beto", "tom")], rows
    assert rows == [(r["c.nombre"], r["a.nombre"]) for r in g.execute_nql(nql)]

    n = list(g.execute_cypher("MATCH (a:Animal) RETURN count(*) AS n"))
    assert n[0]["n"] == 2, n

    for bad, part in [
        ("CREATE (a:Animal {nombre: 'lola'})", "read-only"),
        ("MATCH (a:Animal) WHERE a.edad = $edad RETURN a.nombre", "parameter"),
    ]:
        try:
            g.execute_cypher(bad)
            raise AssertionError(f"must be refused: {bad}")
        except RuntimeError as e:
            assert part in str(e), e
    assert [r["n"] for r in g.execute_nql("find count(*) as n from (a:Animal)")] == [2]

    print("cypher_sample: OK")


if __name__ == "__main__":
    main()
