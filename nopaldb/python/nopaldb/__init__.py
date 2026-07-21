"""
NopalDB — Embedded graph database with MVCC, NQL and Apache Arrow.

Clase principal:
    Graph       — abre/crea una base de datos, ejecuta NQL, gestiona embeddings.
    Transaction — escrituras ACID (add_node, add_edge, commit, rollback).
    QueryResult — resultado de execute_nql(); iterable de filas dict-like.

Semántica (feature `reasoner`):
    ELReasoner  — razonador OWL-EL (CR1/CR2/CR3).
    Inference   — inferencia individual generada por el razonador.

API rápida
----------
    import nopaldb

    graph = nopaldb.Graph.open("mi_db")          # abre o crea
    # graph = nopaldb.Graph.in_memory()           # solo en memoria

    tx = graph.begin_transaction()
    a = tx.add_node("Person", {"name": "Alice", "age": 30})
    b = tx.add_node("Person", {"name": "Bob",   "age": 25})
    e = tx.add_edge(a, b, "KNOWS", {"since": 2020})
    tx.commit()

    result = graph.execute_nql("find p.name from (p:Person) order by p.age")
    for row in result:
        print(row["p.name"])

NQL — cláusulas disponibles
---------------------------
    FIND p.name, p.age
    FROM (p:Person)-[:KNOWS]->(q:Person)
    WHERE p.age > 25
    GROUP BY p.name
    HAVING count(*) > 1
    ORDER BY p.name ASC
    LIMIT 10 OFFSET 0
    AT TIMESTAMP 1672531200
    EXPORT csv WITH path="out.csv", header=true

NQL — Path Queries (F1–F4, E-7–E-10)
--------------------------------------
    # F1: hops cuantificados
    find n.name, path.depth
    from (a:Account {name:"Alice"})-[:TX]->{1,4}(n:Account)

    # F2: metadatos del path
    find n.name, path.nodes, path.edges
    from (a:Account {name:"Alice"})-[:TX]->{1,3}(n:Account)

    # F3: reducers sobre propiedades de arista
    find n.name, path_sum(amount) as total
    from (a:Account {name:"Alice"})-[:TX]->{1,4}(n:Account)
    where path_sum(amount) > 10000

    # F4-B/C: mini-VM (init/gather/return) + PathObject
    find n.name, path.result as score, path.state as vm
    from (a:Company {name:"Alpha"})-[:OWNS]->{1,3}(n:Company)
    where path.result > 200
    init "risk = 0"
    gather "risk = risk + edge.risk_score"
    return "risk * path.depth"

    # E-7: filtrar paths con embeddings completos
    find n.name
    from (a:Account {name:"Alice"})-[:TX]->{1,3}(n:Account)
    where path_has_embeddings("node-minilm", "edge-relbert")

    # E-8: similitud coseno vs referencia persistida
    find n.name, path_embedding_similarity("baseline", "node-minilm", "edge-relbert") as sim
    from (a:Account {name:"Alice"})-[:TX]->{1,3}(n:Account)
    where path_embedding_similarity("baseline", "node-minilm", "edge-relbert") > 0.85

    # E-9: top-k referencias más similares
    find n.name, path_knn_references("node-minilm", "edge-relbert", 3, 0.5) as refs
    from (a:Account {name:"Alice"})-[:TX]->{1,3}(n:Account)
    where path_has_embeddings("node-minilm", "edge-relbert")

    # E-10: score de anomalía [0,1] respecto al centroide de referencias
    find n.name, path_anomaly_score("node-minilm", "edge-relbert") as anomaly
    from (a:Account {name:"Alice"})-[:TX]->{1,4}(n:Account)
    where path_has_embeddings("node-minilm", "edge-relbert")
      and path_anomaly_score("node-minilm", "edge-relbert") > 0.5
    order by anomaly desc

Embeddings API
--------------
    # Nodos
    graph.add_node_embedding(node_id, [0.1, 0.2, 0.3], "minilm")
    vec = graph.get_node_embedding(node_id, "minilm")

    # Aristas (requiere feature `embeddings`)
    graph.add_edge_embedding(edge_id, [0.4, 0.5, 0.6], "relbert")

    # Referencias de path para E-8/E-9/E-10 (requiere feature `embeddings`)
    # El vector tiene dim = dim_node + dim_edge (formato E-7: mean_nodes || mean_edges)
    graph.add_path_reference_embedding("baseline", "node-minilm", "edge-relbert", ref_vec)

    # ANN (requiere feature `embeddings-index`)
    hits = graph.knn_nodes([0.1, 0.2, 0.3], k=5, model="minilm")
    # hits: list[tuple[node_id_str, cosine_distance]]

Ejemplos
--------
    examples/path_queries_f1_f3.py       — F1/F2/F3 cuantificadores y reducers
    examples/path_queries_minivm.py      — F4-B/B.1/C mini-VM y PathObject
    examples/path_queries_embeddings.py  — E-7/E-8/E-9/E-10 embeddings de path
    examples/test_improvements.py        — patrones básicos y edge properties
    examples/test_transactions.py        — API transaccional
    examples/test_all_algorithms.py      — algoritmos de grafo
"""

from .nopaldb import (
    BulkLoader,
    Graph,
    NqlResult,
    ProfileResult,
    QueryResult,
    Transaction,
    __version__,
)

try:
    from .nopaldb import ELReasoner, Inference
except ImportError:
    # Compatibilidad con wheels legacy sin feature `reasoner`.
    ELReasoner = None
    Inference = None

__all__ = [
    "Graph",
    "Transaction",
    "QueryResult",
    "NqlResult",
    "ProfileResult",
    "BulkLoader",
    "ELReasoner",
    "Inference",
    "__version__",
]
