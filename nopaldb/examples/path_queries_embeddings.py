#!/usr/bin/env python3
"""
Path Queries E-7 / E-8 / E-9 / E-10 — Embeddings de Path

Cubre:
  E-7  — path_has_embeddings(node_model, edge_model)
         path_embedding(node_model, edge_model)  → vector del path
  E-8  — path_embedding_similarity(ref_name, node_model, edge_model)
         Cosine similarity del path contra una referencia persistida.
  E-9  — path_knn_references(node_model, edge_model, k, min_score)
         Top-k referencias más similares al path actual.
  E-10 — path_anomaly_score(node_model, edge_model)
         Score de anomalía [0,1] respecto al centroide de referencias.

Requisitos del wheel:
    features = embeddings (incluido en tier core/semantic/full)

Formato del vector E-7 (PathEmbedding):
    [mean(node_vecs) || mean(edge_vecs)]
    dim_total = dim_node + dim_edge

    Ejemplo con dim_node=2, dim_edge=2:
        node_vecs = [v_A, v_B]   → mean = [(v_A+v_B)/2]   (dim 2)
        edge_vecs = [v_e_AB]     → mean = [v_e_AB]         (dim 2)
        path_vec  = [mean_node || mean_edge]                (dim 4)

"""

import nopaldb
import shutil
from pathlib import Path

DB_PATH = "data/path_embeddings.db"

NODE_MODEL = "node-minilm"   # nombre arbitrario — debe coincidir en todas las llamadas
EDGE_MODEL = "edge-relbert"


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def mean_vec(*vecs):
    """Media elemento a elemento de N vectores."""
    n = len(vecs)
    return [sum(v[i] for v in vecs) / n for i in range(len(vecs[0]))]

def concat(*vecs):
    """Concatenación de vectores."""
    result = []
    for v in vecs:
        result.extend(v)
    return result

def path_embedding_vector(node_vecs, edge_vecs):
    """Construye el vector E-7: [mean(node_vecs) || mean(edge_vecs)]."""
    return concat(mean_vec(*node_vecs), mean_vec(*edge_vecs))


# ---------------------------------------------------------------------------
# Setup
# ---------------------------------------------------------------------------

def setup(graph):
    """
    Grafo de transacciones:
      Alice -[50k]-> Bob -[48k]-> Carol   (cadena de 2 hops, típica)
      Alice -[200]-> Diana                (1 hop, pequeña, diferente)
      Bob   -[46k]-> Shell -[44k]-> Haven (cadena larga, sospechosa)
    """
    tx = graph.begin_transaction()

    alice  = tx.add_node("Account", {"name": "Alice",  "type": "personal"})
    bob    = tx.add_node("Account", {"name": "Bob",    "type": "personal"})
    carol  = tx.add_node("Account", {"name": "Carol",  "type": "personal"})
    diana  = tx.add_node("Account", {"name": "Diana",  "type": "personal"})
    shell  = tx.add_node("Account", {"name": "Shell",  "type": "offshore"})
    haven  = tx.add_node("Account", {"name": "Haven",  "type": "offshore"})

    e_ab  = tx.add_edge(alice, bob,   "TX", {"amount": 50000})
    e_bc  = tx.add_edge(bob,   carol, "TX", {"amount": 48000})
    e_ad  = tx.add_edge(alice, diana, "TX", {"amount": 200})
    e_bs  = tx.add_edge(bob,   shell, "TX", {"amount": 46000})
    e_sh  = tx.add_edge(shell, haven, "TX", {"amount": 44000})

    tx.commit()

    # ── Embeddings de nodos (dim=2, simulados) ───────────────────────────
    # Nodos "normales" agrupados cerca de (1, 0)
    # Nodos "offshore" agrupados cerca de (-1, 0)
    v_alice = [1.0, 0.1]
    v_bob   = [0.9, 0.2]
    v_carol = [0.8, 0.1]
    v_diana = [1.0, 0.0]
    v_shell = [-0.9, 0.1]
    v_haven = [-1.0, 0.0]

    graph.add_node_embedding(alice, v_alice, NODE_MODEL)
    graph.add_node_embedding(bob,   v_bob,   NODE_MODEL)
    graph.add_node_embedding(carol, v_carol, NODE_MODEL)
    graph.add_node_embedding(diana, v_diana, NODE_MODEL)
    graph.add_node_embedding(shell, v_shell, NODE_MODEL)
    graph.add_node_embedding(haven, v_haven, NODE_MODEL)

    # ── Embeddings de aristas (dim=2, simulados) ─────────────────────────
    # Transferencias grandes = (1, 1) normalizado
    # Transferencia pequeña  = (0, 1)
    v_e_ab = [0.7, 0.7]
    v_e_bc = [0.7, 0.7]
    v_e_ad = [0.0, 1.0]
    v_e_bs = [0.7, 0.7]
    v_e_sh = [0.7, 0.7]

    graph.add_edge_embedding(e_ab, v_e_ab, EDGE_MODEL)
    graph.add_edge_embedding(e_bc, v_e_bc, EDGE_MODEL)
    graph.add_edge_embedding(e_ad, v_e_ad, EDGE_MODEL)
    graph.add_edge_embedding(e_bs, v_e_bs, EDGE_MODEL)
    graph.add_edge_embedding(e_sh, v_e_sh, EDGE_MODEL)

    # ── Referencias E-8/E-9/E-10: paths de 2 hops "normales" ─────────────
    # Referencia 1: Alice→Bob→Carol (normal, montos altos entre personales)
    #   nodes=[alice, bob, carol], edges=[e_ab, e_bc]
    ref1 = path_embedding_vector(
        [v_alice, v_bob, v_carol],
        [v_e_ab, v_e_bc],
    )
    graph.add_path_reference_embedding("normal_2hop_personal", NODE_MODEL, EDGE_MODEL, ref1)

    # Referencia 2: variante ligeramente diferente
    ref2 = path_embedding_vector(
        [[0.85, 0.15], [0.90, 0.10], [0.80, 0.05]],
        [[0.68, 0.72], [0.72, 0.68]],
    )
    graph.add_path_reference_embedding("normal_2hop_variant", NODE_MODEL, EDGE_MODEL, ref2)

    return alice


def run(graph, title, query):
    print(f"\n{'─'*70}")
    print(f"  {title}")
    print(f"{'─'*70}")
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
    print("  Path Queries E-7 / E-8 / E-9 / E-10 — Embeddings de Path")
    print("=" * 70)

    # ── E-7: Filtrar y proyectar embeddings de path ───────────────────────

    print(f"\n>>> E-7: path_has_embeddings / path_embedding")
    print(f"    Modelos: node={NODE_MODEL!r}  edge={EDGE_MODEL!r}")

    run(graph, "Filtrar paths que tienen embeddings completos",
        f"""
        find n.name, path.depth
        from (a:Account {{name: "Alice"}})-[:TX]->{{1,3}}(n:Account)
        where path_has_embeddings("{NODE_MODEL}", "{EDGE_MODEL}")
        """)

    run(graph, "Proyectar el vector E-7 del path",
        f"""
        find n.name, path_embedding("{NODE_MODEL}", "{EDGE_MODEL}") as vec
        from (a:Account {{name: "Alice"}})-[:TX]->{{1,2}}(n:Account)
        where path_has_embeddings("{NODE_MODEL}", "{EDGE_MODEL}")
        """)

    # ── E-8: Similitud con referencia ────────────────────────────────────

    print("\n>>> E-8: path_embedding_similarity — cosine vs referencia")

    run(graph, "Similitud del path contra referencia 'normal_2hop_personal'",
        f"""
        find n.name, path_embedding_similarity("normal_2hop_personal", "{NODE_MODEL}", "{EDGE_MODEL}") as sim
        from (a:Account {{name: "Alice"}})-[:TX]->{{1,3}}(n:Account)
        where path_has_embeddings("{NODE_MODEL}", "{EDGE_MODEL}")
        """)

    run(graph, "Paths con similitud > 0.9 respecto al patrón normal",
        f"""
        find n.name, path_embedding_similarity("normal_2hop_personal", "{NODE_MODEL}", "{EDGE_MODEL}") as sim
        from (a:Account {{name: "Alice"}})-[:TX]->{{1,3}}(n:Account)
        where path_embedding_similarity("normal_2hop_personal", "{NODE_MODEL}", "{EDGE_MODEL}") > 0.9
        """)

    # ── E-9: KNN sobre referencias ───────────────────────────────────────

    print("\n>>> E-9: path_knn_references — top-k referencias más similares")

    run(graph, "Top-2 referencias más similares al path (k=2, min_score=0.5)",
        f"""
        find n.name, path_knn_references("{NODE_MODEL}", "{EDGE_MODEL}", 2, 0.5) as top_refs
        from (a:Account {{name: "Alice"}})-[:TX]->{{1,3}}(n:Account)
        where path_has_embeddings("{NODE_MODEL}", "{EDGE_MODEL}")
        """)

    # ── E-10: Score de anomalía ──────────────────────────────────────────

    print("\n>>> E-10: path_anomaly_score — distancia al centroide de referencias")
    print("    Score 0.0 = idéntico al centroide (típico)")
    print("    Score 1.0 = máxima anomalía")

    run(graph, "Score de anomalía para todos los paths",
        f"""
        find n.name, path_anomaly_score("{NODE_MODEL}", "{EDGE_MODEL}") as anomaly, path.depth
        from (a:Account {{name: "Alice"}})-[:TX]->{{1,3}}(n:Account)
        where path_has_embeddings("{NODE_MODEL}", "{EDGE_MODEL}")
        """)

    run(graph, "Alertar paths con anomalía > 0.5",
        f"""
        find n.name, path_anomaly_score("{NODE_MODEL}", "{EDGE_MODEL}") as anomaly
        from (a:Account {{name: "Alice"}})-[:TX]->{{1,4}}(n:Account)
        where path_has_embeddings("{NODE_MODEL}", "{EDGE_MODEL}")
          and path_anomaly_score("{NODE_MODEL}", "{EDGE_MODEL}") > 0.5
        """)

    run(graph, "Ordenar por anomalía descendente (alias proyectado)",
        f"""
        find n.name, path_anomaly_score("{NODE_MODEL}", "{EDGE_MODEL}") as anomaly
        from (a:Account {{name: "Alice"}})-[:TX]->{{1,4}}(n:Account)
        where path_has_embeddings("{NODE_MODEL}", "{EDGE_MODEL}")
        order by anomaly desc
        limit 5
        """)

    print("\n" + "=" * 70)
    print("  Listo. Ver docs/NQL_PATH_EMBEDDING_E7.md,")
    print("              docs/NQL_PATH_SIMILARITY_E8.md,")
    print("              docs/NQL_PATH_ANOMALY_E10.md")
    print("=" * 70)


if __name__ == "__main__":
    main()
