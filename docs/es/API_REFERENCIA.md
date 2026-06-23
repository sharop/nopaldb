# Referencia de API Python

Esta referencia resume la API Python real expuesta por el wrapper PyO3. La
referencia bilingue completa vive en [API_REFERENCE.md](../python/API_REFERENCE.md).

## Build

```bash
# Libreria Rust
cargo build -p nopaldb --release --features full

# Wrapper Python
make build-wheel

# Desarrollo local Python
cd nopaldb
maturin develop --release --features python-full
```

`cargo build --features full` no genera el wrapper Python; para Python usa
`maturin`.

## Graph

Constructores:

```python
graph = nopaldb.Graph.open("data/mi_grafo.db")
graph = nopaldb.Graph.open_with_profile("data/mi_grafo.db", "default")
graph = nopaldb.Graph.open_with_options(
    "data/mi_grafo.db",
    engine="sled",
    profile="default",
)

graph = nopaldb.Graph.in_memory()
graph = nopaldb.Graph.in_memory_with_profile("default")
graph = nopaldb.Graph.in_memory_with_options(engine="sled", profile="default")
```

Consultas:

```python
res = graph.execute_nql("""
    find p.nombre, p.edad
    from (p:Persona)
    where p.edad >= 18
""")

for fila in res.query:
    print(fila["p.nombre"])
```

Transacciones:

```python
tx = graph.begin_transaction()
ana_id = tx.add_node("Persona", {"nombre": "Ana"})
beto_id = tx.add_node("Persona", {"nombre": "Beto"})
tx.add_edge(ana_id, beto_id, "CONOCE", {"desde": 2024})
tx.commit()
```

La transaccion Python expone `add_node`, `add_edge`, `commit` y `rollback`.
Para borrar datos desde Python usa NQL `delete`.

## Arrow

```python
nodes_bytes = graph.to_arrow(label="Persona")
edges_bytes = graph.edges_to_arrow()
nodes_bytes, edges_bytes = graph.to_arrow_complete()
```

## Schema

```python
labels = graph.get_labels()
edge_types = graph.get_edge_types()
schema = graph.get_schema()

props = graph.get_label_properties("Persona")
count = graph.get_label_count("Persona")

edge_props = graph.get_edge_type_properties("CONOCE")
edge_count = graph.get_edge_type_count("CONOCE")

graph.rebuild_schema()
```

## Indices

```python
index_name = graph.create_index("Persona", "email", "hash")
graph.create_index("Persona", "edad", "btree")

for name, label, prop, index_type in graph.list_indexes():
    print(name, label, prop, index_type)

graph.drop_index(index_name)
```

## Stats y ciclo de vida

```python
stats = graph.get_stats()
count = graph.node_count()
graph.close()
```

Tambien se puede usar como context manager:

```python
with nopaldb.Graph.open("data/mi_grafo.db") as graph:
    print(graph.node_count())
```

## APIs bajo `python-full`

Embeddings:

```python
graph.add_node_embedding(node_id, [0.1, 0.2, 0.3], "modelo")
vector = graph.get_node_embedding(node_id, "modelo")
matches = graph.knn_nodes([0.1, 0.2, 0.3], 10, "modelo")
```

OWL:

```python
stats = graph.import_turtle("ontology.ttl")
```

Reasoner:

```python
reasoner = nopaldb.ELReasoner()

disease_id = "550e8400-e29b-41d4-a716-446655440000"
infection_id = "550e8400-e29b-41d4-a716-446655440001"

reasoner.register_class(disease_id, "Disease")
reasoner.register_class(infection_id, "Infection")
reasoner.assert_subclass(infection_id, disease_id)
reasoner.classify_all()
print(reasoner.is_subclass_of(infection_id, disease_id))
```

## Resultados

`NqlResult` expone `kind`, `query`, `write`, `explain`, `profile`, `message` y
`summary`.

`QueryResult` soporta `len(result)`, iteracion, acceso por indice y `columns`.

`ProfileResult` expone `plan`, `statement_type`, `execution_ms`,
`rows_returned`, `columns`, `path_query` y `path_metrics`.
