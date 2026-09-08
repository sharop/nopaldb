#!/usr/bin/env python3
"""Round trip Turtle → NopalDB → Turtle desde Python.

Guardia del puente RDF en la wheel: `import_turtle`, `export_turtle`,
`export_owl_file` y `rdf_prefixes` existen, el export es fiel (`skipped`
vacío) y re-importarlo en una base limpia reproduce los mismos conteos.
Dominio ficticio (recetario). Se ejecuta en CI (job python-stubs) tras
`maturin develop`.
"""

import os
import tempfile

import nopaldb

TTL = """
@prefix owl:  <http://www.w3.org/2002/07/owl#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .
@prefix :     <http://cocina.example/> .

:Bebida   a owl:Class .
:Infusion a owl:Class ; rdfs:subClassOf :Bebida .
:Postre   a owl:Class .
:café_de_olla a :Infusion ; :nombre "Café de olla" ; :temperatura "85"^^xsd:integer ; :acompaña :pan .
:pan a :Postre .
"""


def main() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        g = nopaldb.Graph.open(os.path.join(tmp, "a.db"))
        imported = g.import_turtle(TTL)
        assert imported["edges_created"] == 3, imported  # 2 instanceOf + acompaña

        ttl, report = g.export_turtle()
        assert isinstance(ttl, str) and ":café_de_olla a :Infusion" in ttl, ttl
        assert report["skipped"] == [], report
        assert report["edges"] == 1 and report["classes"] == 3, report
        assert g.rdf_prefixes()[""] == "http://cocina.example/"

        path = os.path.join(tmp, "out.ttl")
        file_report = g.export_owl_file(path)
        assert file_report == report, (file_report, report)
        with open(path, encoding="utf-8") as fh:
            assert fh.read() == ttl
        g.close()

        h = nopaldb.Graph.open(os.path.join(tmp, "b.db"))
        again = h.import_turtle(ttl)
        assert again["classes_added"] == imported["classes_added"], (again, imported)
        assert again["instances_added"] == imported["instances_added"], (again, imported)
        assert again["edges_created"] == imported["edges_created"], (again, imported)
        assert again["warnings"] == [], again
        h.close()
    print("turtle round trip OK")


if __name__ == "__main__":
    main()
