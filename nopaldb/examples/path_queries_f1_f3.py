#!/usr/bin/env python3
"""
Path Queries F1 / F2 / F3 — Cuantificadores, Metadatos y Reducers

Cubre:
  F1  — hops cuantificados: -[:TIPO]->{n}  y  -[:TIPO]->{n,m}
  F2  — metadatos de path: path.depth, path.nodes, path.edges + PROFILE
  F3  — reducers escalares sobre aristas: path_sum, path_min, path_max, path_avg

Caso de uso: detección de cadenas de transferencias bancarias sospechosas.

"""

import nopaldb
import shutil
from pathlib import Path

DB_PATH = "data/path_f1_f3.db"


# ---------------------------------------------------------------------------
# Setup
# ---------------------------------------------------------------------------

def setup(graph):
    tx = graph.begin_transaction()

    alice  = tx.add_node("Account", {"name": "Alice",  "type": "personal"})
    bob    = tx.add_node("Account", {"name": "Bob",    "type": "personal"})
    carol  = tx.add_node("Account", {"name": "Carol",  "type": "personal"})
    shell  = tx.add_node("Account", {"name": "Shell",  "type": "offshore"})
    haven  = tx.add_node("Account", {"name": "Haven",  "type": "offshore"})
    diana  = tx.add_node("Account", {"name": "Diana",  "type": "personal"})

    tx.add_edge(alice, bob,   "TRANSFER", {"amount": 5000,  "fee": 50})
    tx.add_edge(bob,   carol, "TRANSFER", {"amount": 4800,  "fee": 48})
    tx.add_edge(carol, shell, "TRANSFER", {"amount": 4500,  "fee": 45})
    tx.add_edge(shell, haven, "TRANSFER", {"amount": 4200,  "fee": 42})
    tx.add_edge(alice, diana, "TRANSFER", {"amount": 200,   "fee": 2})

    tx.commit()
    return alice


def run(graph, title, query):
    print(f"\n{'─'*65}")
    print(f"  {title}")
    print(f"{'─'*65}")
    print(f"  NQL: {query.strip()}\n")
    try:
        result = graph.execute_nql(query)
        print(f"  Filas: {len(result)}")
        for row in result:
            print("  ", dict(row))
        return True
    except Exception as e:
        print(f"  ERROR: {e}")
        return False


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    if Path(DB_PATH).exists():
        shutil.rmtree(DB_PATH)
    graph = nopaldb.Graph.open(DB_PATH)
    setup(graph)

    print("=" * 65)
    print("  Path Queries F1 / F2 / F3")
    print("=" * 65)

    # ── F1: Cuantificadores ──────────────────────────────────────────────

    print("\n>>> F1: Cuantificadores de profundidad")

    run(graph, "Exactamente 2 hops desde Alice",
        'find n.name from (a:Account {name: "Alice"})-[:TRANSFER]->{2}(n:Account)')

    run(graph, "Entre 1 y 3 hops desde Alice",
        'find n.name, path.depth from (a:Account {name: "Alice"})-[:TRANSFER]->{1,3}(n:Account)')

    run(graph, "Hasta 4 hops, solo cuentas offshore",
        'find n.name, path.depth from (a:Account {name: "Alice"})-[:TRANSFER]->{1,4}(n:Account)'
        ' where n.type = "offshore"')

    # ── F2: Metadatos de path ────────────────────────────────────────────

    print("\n>>> F2: Metadatos de path")

    run(graph, "Profundidad y nodos del recorrido",
        'find n.name, path.depth, path.nodes'
        ' from (a:Account {name: "Alice"})-[:TRANSFER]->{1,4}(n:Account)')

    run(graph, "Aristas del recorrido (ids y tipos)",
        'find n.name, path.edges'
        ' from (a:Account {name: "Alice"})-[:TRANSFER]->{1,3}(n:Account)')

    run(graph, "PROFILE — métricas de ejecución",
        'profile find n.name, path.depth'
        ' from (a:Account {name: "Alice"})-[:TRANSFER]->{1,4}(n:Account)')

    # ── F3: Reducers ─────────────────────────────────────────────────────

    print("\n>>> F3: Reducers escalares sobre aristas")

    run(graph, "Suma acumulada de montos (path_sum)",
        'find n.name, path_sum(amount) as total'
        ' from (a:Account {name: "Alice"})-[:TRANSFER]->{1,4}(n:Account)')

    run(graph, "Monto mínimo en el camino (path_min)",
        'find n.name, path_min(amount) as min_tx'
        ' from (a:Account {name: "Alice"})-[:TRANSFER]->{1,4}(n:Account)')

    run(graph, "Monto máximo en el camino (path_max)",
        'find n.name, path_max(amount) as max_tx'
        ' from (a:Account {name: "Alice"})-[:TRANSFER]->{1,4}(n:Account)')

    run(graph, "Comisión media por hop (path_avg)",
        'find n.name, path.depth, path_avg(fee) as avg_fee'
        ' from (a:Account {name: "Alice"})-[:TRANSFER]->{1,4}(n:Account)')

    run(graph, "Solo paths donde el total supera 13000 (F3 en WHERE)",
        'find n.name, path_sum(amount) as total'
        ' from (a:Account {name: "Alice"})-[:TRANSFER]->{1,4}(n:Account)'
        ' where path_sum(amount) > 13000')

    print("\n" + "=" * 65)
    print("  Listo. Ver la guia de NQL en docs/python/NQL_GUIDE.md")
    print("=" * 65)


if __name__ == "__main__":
    main()
