#!/usr/bin/env python3
"""Clases disjuntas y ⊥ en el ELReasoner desde Python (#235).

Guardia de la wheel: `ELReasoner.NOTHING`, `assert_disjoint`,
`unsatisfiable_classes`, `is_satisfiable`, `why_unsatisfiable` y
`ELReasoner.from_graph` sobre una ontología importada de Turtle con
`owl:disjointWith` y `owl:AllDisjointClasses`. Dominio ficticio (recetario).
Se ejecuta en CI (job python-stubs) tras `maturin develop`.
"""

import uuid

import nopaldb

ONTOLOGY = """
@prefix owl:  <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix :     <http://cocina.example/> .
:Cosa a owl:Class .
:Receta a owl:Class ; rdfs:subClassOf :Cosa .
:Utensilio a owl:Class ; rdfs:subClassOf :Cosa .
:Receta owl:disjointWith :Utensilio .
:Hibrido a owl:Class ; rdfs:subClassOf :Receta , :Utensilio .
:Sopa a owl:Class . :Postre a owl:Class . :Bebida a owl:Class .
[] a owl:AllDisjointClasses ; owl:members ( :Sopa :Postre :Bebida ) .
:Atole a owl:Class ; rdfs:subClassOf :Bebida .
"""


def in_memory() -> None:
    """Sin grafo: ids propios, la disyunción y ⊥ a mano."""
    r = nopaldb.ELReasoner()
    ids = {n: str(uuid.uuid4()) for n in ["Cosa", "Receta", "Utensilio", "Hibrido", "Nieto", "Plato"]}
    for name, i in ids.items():
        r.register_class(i, name)
    r.assert_subclass(ids["Receta"], ids["Cosa"])
    r.assert_subclass(ids["Utensilio"], ids["Cosa"])
    r.assert_subclass(ids["Hibrido"], ids["Receta"])
    r.assert_subclass(ids["Hibrido"], ids["Utensilio"])
    r.assert_disjoint([ids["Receta"], ids["Utensilio"]])
    r.assert_subclass(ids["Nieto"], ids["Hibrido"])
    r.assert_existential(ids["Plato"], "usa", ids["Hibrido"])

    unsat = set(r.unsatisfiable_classes())
    assert unsat == {ids["Hibrido"], ids["Nieto"], ids["Plato"]}, unsat
    assert r.is_satisfiable(ids["Receta"]) and not r.is_satisfiable(ids["Hibrido"])
    assert r.is_subclass_of(ids["Hibrido"], nopaldb.ELReasoner.NOTHING)

    why = r.why_unsatisfiable(ids["Hibrido"])
    assert why is not None and why["cause"] == "disjoint" and why["rule"] == "CR2", why
    assert set(why["classes"]) == {ids["Receta"], ids["Utensilio"]}, why
    plato = r.why_unsatisfiable(ids["Plato"])
    assert plato == {"cause": "existential", "rule": "CR4", "role": "usa", "filler": ids["Hibrido"]}, plato
    assert r.why_unsatisfiable(ids["Cosa"]) is None
    assert any(i.rule == "CR4" for i in r.derived_inferences())

    try:
        r.assert_disjoint([ids["Cosa"]])
    except ValueError as err:
        assert "at least two" in str(err), err
    else:
        raise AssertionError("one class is not a disjointness")


def class_id(g: nopaldb.Graph, label: str) -> str:
    rows = list(g.execute_nql(f"find c.id from (c:{label})"))
    assert len(rows) == 1, (label, rows)
    return str(rows[0]["c.id"])


def from_graph() -> None:
    """Turtle → grafo (aristas disjointWith) → razonador."""
    g = nopaldb.Graph.in_memory()
    report = g.import_turtle(ONTOLOGY)
    assert report["disjoint_pairs_added"] == 1 + 3, report
    assert report["triples_skipped"] == 0, report
    assert report["warnings"] == [], report

    r = nopaldb.ELReasoner.from_graph(g)
    unsat = r.unsatisfiable_classes()
    hibrido = class_id(g, "Hibrido")
    assert unsat == [hibrido], (unsat, hibrido)
    receta = class_id(g, "Receta")
    utensilio = class_id(g, "Utensilio")
    assert set(r.why_unsatisfiable(hibrido)["classes"]) == {receta, utensilio}

    # Una bebida que también es postre choca con AllDisjointClasses.
    atole = class_id(g, "Atole")
    postre = class_id(g, "Postre")
    assert r.is_satisfiable(atole)
    r.assert_subclass(atole, postre)
    assert not r.is_satisfiable(atole)


def main() -> None:
    in_memory()
    from_graph()
    print("reasoner OK")


if __name__ == "__main__":
    main()
