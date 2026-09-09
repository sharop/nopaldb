#!/usr/bin/env python3
"""SHACL desde Python (#98): shapes en Turtle contra un grafo importado de Turtle.

Guardia de la wheel: `validate_shapes` existe, encuentra exactamente los
defectos plantados con componente/path/valor, y reporta lo que no comprueba
(`ignored`) en vez de callar. Dominio ficticio (recetario). Se ejecuta en CI
(job python-stubs) tras `maturin develop`.
"""

import nopaldb

DATA = """
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix :    <http://cocina.example/> .
:Receta a owl:Class .  :Ingrediente a owl:Class .
:canela a :Ingrediente .  :cafe a :Ingrediente .
:cafe_de_olla a :Receta ; :nombre "Café de olla" ; :tiempoMin "20"^^xsd:integer ; :usa :cafe, :canela .
:agua_fria    a :Receta ; :nombre "Agua fría" ; :tiempoMin "-5"^^xsd:integer ; :usa :cafe, :canela .
:te           a :Receta ; :nombre "Té" ; :tiempoMin "5"^^xsd:integer ; :usa :canela .
"""

SHAPES = """
@prefix sh:  <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix :    <http://cocina.example/> .
:RecetaShape a sh:NodeShape ; sh:name "Receta" ; sh:targetClass :Receta ;
  sh:property [ sh:path :tiempoMin ; sh:datatype xsd:integer ; sh:minInclusive 1 ] ,
              [ sh:path :usa ; sh:minCount 2 ; sh:class :Ingrediente ] ;
  sh:closed true .
"""


def main() -> None:
    g = nopaldb.Graph.in_memory()
    imported = g.import_turtle(DATA)
    assert imported["warnings"] == [], imported

    r = g.validate_shapes(SHAPES)
    assert r["conforms"] is False, r
    assert r["shapes"] == 1 and r["property_shapes"] == 2 and r["constraints"] == 4, r
    assert len(r["ignored"]) == 1 and "sh:closed" in r["ignored"][0], r["ignored"]
    assert r["warnings"] == [] and r["notes"] == [], r

    got = sorted((v["constraint"], v["path"], v["value"]) for v in r["violations"])
    assert got == [
        ("sh:MinCountConstraintComponent", "usa", None),
        ("sh:MinInclusiveConstraintComponent", "tiempoMin", -5),
    ], got
    assert all(v["shape"] == "Receta" and v["severity"] == "Violation" for v in r["violations"]), r

    # sh:or explains its branches (nested), and sh:severity Warning keeps conforms.
    r = g.validate_shapes("""
@prefix sh:  <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix :    <http://cocina.example/> .
:S sh:targetClass :Receta ; sh:severity sh:Warning ; sh:message "tiempo raro" ;
  sh:property [ sh:path :tiempoMin ; sh:or ( [ sh:datatype xsd:decimal ] [ sh:maxInclusive 10 ] ) ] .
""")
    assert r["conforms"] is True, r
    assert r["ignored"] == [], r
    bad = [v for v in r["violations"] if v["constraint"] == "sh:OrConstraintComponent"]
    assert len(bad) == 1 and bad[0]["severity"] == "Warning" and bad[0]["message"] == "tiempo raro", bad
    assert len(bad[0]["nested"]) == 2 and bad[0]["value"] == 20, bad[0]

    try:
        g.validate_shapes("@prefix sh: <http://www.w3.org/ns/shacl#> .\n:S sh:minCount .")
    except Exception as e:  # noqa: BLE001 — la excepción es la API
        assert "line 2" in str(e), e
    else:
        raise AssertionError("Turtle malformado debería levantar")
    g.close()
    print("shacl OK")


if __name__ == "__main__":
    main()
