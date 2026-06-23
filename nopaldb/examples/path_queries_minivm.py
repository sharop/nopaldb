#!/usr/bin/env python3
"""
Path Queries F4-B / F4-B.1 / F4-C — Mini-VM, PathObject y cláusula return

Cubre:
  F4-B   — mini-VM: cláusulas `init` y `gather` para acumular estado por hop
  F4-B.1 — composición booleana completa en expresiones quoted (and, or, not)
  F4-C   — PathObject: `path.start`, `path.end`, `path.state`, `path.result`
           + cláusula `return` evaluada una vez por path completo

Caso de uso: análisis de cadenas de propiedad corporativa (ownership chains).

Sintaxis del mini-VM:
    init   "var = valor_inicial"
    gather "var = expresion_por_hop"   -- ejecutado en cada arista
    return "expresion_final"           -- ejecutado una vez al terminar el path

Variables disponibles en gather/return:
    edge.PROPIEDAD   -- propiedad de la arista actual
    source.PROPIEDAD -- nodo origen del hop
    target.PROPIEDAD -- nodo destino del hop
    path.depth       -- profundidad actual
    <vars_definidas_en_init>

Variables de PathObject disponibles en FIND/WHERE:
    path.result  -- valor final de return (solo si hay return)
    path.state   -- objeto con todas las variables del mini-VM al final
    path.start   -- {id, label} del primer nodo
    path.end     -- {id, label} del último nodo
    path.depth   -- profundidad total
    path.nodes   -- lista de {id, label} en orden de recorrido
    path.edges   -- lista de {id, type, source, target}

"""

import nopaldb
import shutil
from pathlib import Path

DB_PATH = "data/path_minivm.db"


# ---------------------------------------------------------------------------
# Setup — cadena de propiedad corporativa con porcentajes y flags de riesgo
# ---------------------------------------------------------------------------

def setup(graph):
    tx = graph.begin_transaction()

    alpha   = tx.add_node("Company", {"name": "Alpha",   "country": "MX"})
    beta    = tx.add_node("Company", {"name": "Beta",    "country": "MX"})
    gamma   = tx.add_node("Company", {"name": "Gamma",   "country": "PA"})
    delta   = tx.add_node("Company", {"name": "Delta",   "country": "PA"})
    epsilon = tx.add_node("Company", {"name": "Epsilon", "country": "MX"})
    zeta    = tx.add_node("Company", {"name": "Zeta",    "country": "KY"})

    # pct = porcentaje de propiedad; risk_score = score de riesgo 0-100
    tx.add_edge(alpha,   beta,    "OWNS", {"pct": 80, "risk_score": 10, "audit_ok": True})
    tx.add_edge(beta,    gamma,   "OWNS", {"pct": 60, "risk_score": 75, "audit_ok": False})
    tx.add_edge(gamma,   delta,   "OWNS", {"pct": 90, "risk_score": 85, "audit_ok": False})
    tx.add_edge(alpha,   epsilon, "OWNS", {"pct": 51, "risk_score": 20, "audit_ok": True})
    tx.add_edge(epsilon, zeta,    "OWNS", {"pct": 45, "risk_score": 30, "audit_ok": True})

    tx.commit()


def run(graph, title, query):
    print(f"\n{'─'*70}")
    print(f"  {title}")
    print(f"{'─'*70}")
    # Mostrar query formateado
    for line in query.strip().splitlines():
        print(f"    {line.strip()}")
    print()
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

    print("=" * 70)
    print("  Path Queries F4-B / F4-B.1 / F4-C — Mini-VM y PathObject")
    print("=" * 70)

    # ── F4-B: init + gather ───────────────────────────────────────────────

    print("\n>>> F4-B: Acumulación de estado con init / gather")

    run(graph, "Suma de risk_score a lo largo del path",
        """
        find n.name, path_eval("total_risk")
        from (a:Company {name: "Alpha"})-[:OWNS]->{1,3}(n:Company)
        init "total_risk = 0"
        gather "total_risk = total_risk + edge.risk_score"
        """)

    run(graph, "Porcentaje de control acumulado (producto)",
        """
        find n.name, path.depth, path_eval("control")
        from (a:Company {name: "Alpha"})-[:OWNS]->{1,3}(n:Company)
        init "control = 100"
        gather "control = control * edge.pct / 100"
        """)

    # ── F4-B.1: Lógica booleana en gather ────────────────────────────────

    print("\n>>> F4-B.1: Composición booleana en expresiones quoted")

    run(graph, "Detectar si algún hop tiene risk_score > 70",
        """
        find n.name, path_eval("any_risky")
        from (a:Company {name: "Alpha"})-[:OWNS]->{1,3}(n:Company)
        init "any_risky = false"
        gather "any_risky = any_risky or edge.risk_score > 70"
        """)

    run(graph, "Verificar que todos los hops pasaron auditoría",
        """
        find n.name, path_eval("all_audited")
        from (a:Company {name: "Alpha"})-[:OWNS]->{1,3}(n:Company)
        init "all_audited = true"
        gather "all_audited = all_audited and edge.audit_ok = true"
        """)

    # ── F4-C: return + PathObject ─────────────────────────────────────────

    print("\n>>> F4-C: Cláusula return + path.result / path.state / path.start / path.end")

    run(graph, "Score final = risk_total * profundidad (return)",
        """
        find n.name, path.result as score, path.depth
        from (a:Company {name: "Alpha"})-[:OWNS]->{1,3}(n:Company)
        init "risk = 0"
        gather "risk = risk + edge.risk_score"
        return "risk * path.depth"
        """)

    run(graph, "Filtrar por path.result > 200 (return en WHERE)",
        """
        find n.name, path.result as score
        from (a:Company {name: "Alpha"})-[:OWNS]->{1,3}(n:Company)
        where path.result > 200
        init "risk = 0"
        gather "risk = risk + edge.risk_score"
        return "risk * path.depth"
        """)

    run(graph, "path.state — estado final del mini-VM como objeto",
        """
        find n.name, path.state as vm_final
        from (a:Company {name: "Alpha"})-[:OWNS]->{1,3}(n:Company)
        init "risk = 0"
        gather "risk = risk + edge.risk_score"
        return "risk"
        """)

    run(graph, "path.start y path.end — primer y último nodo",
        """
        find path.start as origen, path.end as destino, path.depth, path.result as total_risk
        from (a:Company {name: "Alpha"})-[:OWNS]->{1,3}(n:Company)
        init "risk = 0"
        gather "risk = risk + edge.risk_score"
        return "risk"
        """)

    run(graph, "path.nodes y path.edges — recorrido completo",
        """
        find n.name, path.nodes, path.edges
        from (a:Company {name: "Alpha"})-[:OWNS]->{1,2}(n:Company)
        """)

    # ── Caso completo: detección de cadenas de riesgo ────────────────────

    print("\n>>> Caso completo: cadenas con control > 40% Y algún hop riesgoso")

    run(graph, "Control efectivo > 40% y algún hop risk_score > 70",
        """
        find n.name, path.result as control_pct, path.depth
        from (a:Company {name: "Alpha"})-[:OWNS]->{1,3}(n:Company)
        where path.result > 40
        init "control = 100, any_risk = false"
        gather "control = control * edge.pct / 100, any_risk = any_risk or edge.risk_score > 70"
        return "control"
        """)

    print("\n" + "=" * 70)
    print("  Listo. Ver la guia de NQL en docs/python/NQL_GUIDE.md")
    print("=" * 70)


if __name__ == "__main__":
    main()
