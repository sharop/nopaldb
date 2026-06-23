# NopalDB Python API Reference

This reference covers the public Python wrapper exposed by `nopaldb`.

## Build Model

Rust builds and Python builds are separate:

```bash
# Rust library
cargo build -p nopaldb --release --features full

# Python wheel
make build-wheel

# Local Python development
cd nopaldb
maturin develop --release --features python-full
```

`cargo build --features full` does not build the Python extension. PyO3 needs
`maturin` so the extension is linked against the active Python interpreter.

## Graph

### Constructors

```python
import nopaldb

graph = nopaldb.Graph.open("data/my_graph.db")
graph = nopaldb.Graph.open_with_profile("data/my_graph.db", "default")
graph = nopaldb.Graph.open_with_options(
    "data/my_graph.db",
    engine="sled",
    profile="default",
)

memory_graph = nopaldb.Graph.in_memory()
memory_graph = nopaldb.Graph.in_memory_with_profile("default")
memory_graph = nopaldb.Graph.in_memory_with_options(
    engine="sled",
    profile="default",
)
```

The public storage engine in the community build is `sled`. Profiles are
`default`, `mobile`, and `server`.

### Queries

```python
result = graph.execute_nql("""
    find p.name, p.age
    from (p:Person)
    where p.age >= 18
""")

for row in result.query:
    print(row["p.name"])
```

`execute_nql(query: str) -> NqlResult` supports read queries, writes, `EXPLAIN`,
`PROFILE`, index commands, exports, and message results.

### Transactions

```python
tx = graph.begin_transaction()

alice_id = tx.add_node("Person", {"name": "Alice", "age": 30})
bob_id = tx.add_node("Person", {"name": "Bob"})
edge_id = tx.add_edge(alice_id, bob_id, "KNOWS", {"since": 2024})

tx.commit()
```

Use `rollback()` to discard pending transaction changes.

The Python `Transaction` API exposes:

- `add_node(label: str, properties: dict | None = None) -> str`
- `add_edge(source: str, target: str, edge_type: str, properties: dict | None = None) -> str`
- `commit() -> None`
- `rollback() -> None`

Deletes are available through NQL `delete` statements or the Rust API, not
through Python `Transaction` methods.

### Arrow Export

```python
nodes_bytes = graph.to_arrow(label="Person")
edges_bytes = graph.edges_to_arrow()
nodes_bytes, edges_bytes = graph.to_arrow_complete()
```

The return values are Arrow IPC stream bytes that can be opened with PyArrow.

### Bulk Loading

```python
loader = graph.bulk_loader(batch_size=1000)
```

`bulk_loader` returns the Python bulk loader object exposed by the extension.

### Schema Inspection

```python
labels = graph.get_labels()
edge_types = graph.get_edge_types()
schema = graph.get_schema()

person_props = graph.get_label_properties("Person")
person_count = graph.get_label_count("Person")

knows_props = graph.get_edge_type_properties("KNOWS")
knows_count = graph.get_edge_type_count("KNOWS")

graph.rebuild_schema()
```

### Indexes

```python
index_name = graph.create_index("Person", "email", "hash")
graph.create_index("Person", "age", "btree")

for name, label, prop, index_type in graph.list_indexes():
    print(name, label, prop, index_type)

graph.drop_index(index_name)
```

`index_type` is one of `hash`, `btree`, or `fulltext` when full-text support is
available in the build.

### Stats And Lifecycle

```python
stats = graph.get_stats()
count = graph.node_count()
graph.close()
```

`Graph` also supports Python context manager usage.

```python
with nopaldb.Graph.open("data/my_graph.db") as graph:
    print(graph.node_count())
```

## Embeddings APIs

Available when the Python extension is built with `python-full`:

```python
graph.add_node_embedding(node_id, [0.1, 0.2, 0.3], "model-name")
graph.add_edge_embedding(edge_id, [0.1, 0.2, 0.3], "model-name")
graph.add_path_reference_embedding("reference-name", "node-model", "edge-model", [0.1, 0.2])

vector = graph.get_node_embedding(node_id, "model-name")
matches = graph.knn_nodes([0.1, 0.2, 0.3], 10, "model-name")
```

## OWL Import

Available when the Python extension includes OWL support:

```python
stats = graph.import_turtle("ontology.ttl")
```

## Reasoner API

Available when the Python extension includes reasoner support:

```python
reasoner = nopaldb.ELReasoner()

disease_id = "550e8400-e29b-41d4-a716-446655440000"
infection_id = "550e8400-e29b-41d4-a716-446655440001"

reasoner.register_class(disease_id, "Disease")
reasoner.register_class(infection_id, "Infection")
reasoner.assert_subclass(infection_id, disease_id)

reasoner.classify_all()

assert reasoner.is_subclass_of(infection_id, disease_id)
print(reasoner.superclasses(infection_id))
print(reasoner.subclasses(disease_id))
print(reasoner.axiom_count())
print(reasoner.derived_count())

for inference in reasoner.derived_inferences():
    print(inference.to_dict())
```

Reasoner methods:

- `register_class(node_id, label)`
- `assert_subclass(sub, sup)`
- `assert_conjunction(left, right, sup)`
- `assert_existential(sub, property, filler)`
- `assert_existential_domain(property, filler, sup)`
- `classify_all()`
- `is_subclass_of(sub, sup)`
- `superclasses(class_name)`
- `subclasses(class_name)`
- `axiom_count()`
- `derived_count()`
- `derived_inferences()`

## Result Types

### NqlResult

`NqlResult` exposes:

- `kind`
- `query`
- `write`
- `explain`
- `profile`
- `message`
- `summary`

### QueryResult

Read results support:

- `len(result)`
- iteration
- index access with `result[i]`
- `columns`

### ProfileResult

Profile results expose:

- `plan`
- `statement_type`
- `execution_ms`
- `rows_returned`
- `columns`
- `path_query`
- `path_metrics`

---

# Referencia en espanol

## Graph

Constructores:

- `Graph.open(path)`
- `Graph.open_with_profile(path, profile="default")`
- `Graph.open_with_options(path, engine="sled", profile="default")`
- `Graph.in_memory()`
- `Graph.in_memory_with_profile(profile="default")`
- `Graph.in_memory_with_options(engine="sled", profile="default")`

Consultas:

```python
res = graph.execute_nql("""
    find p.nombre
    from (p:Persona)
    where p.activo = true
""")
```

Transacciones:

```python
tx = graph.begin_transaction()
node_id = tx.add_node("Persona", {"nombre": "Ana"})
tx.commit()
```

La transaccion Python expone `add_node`, `add_edge`, `commit` y `rollback`.
Para borrar datos desde Python usa NQL `delete`.

Exportacion Arrow:

- `to_arrow(label=None)`
- `edges_to_arrow()`
- `to_arrow_complete(label=None)`

Schema:

- `get_labels()`
- `get_edge_types()`
- `get_schema()`
- `get_label_properties(label)`
- `get_label_count(label)`
- `get_edge_type_properties(edge_type)`
- `get_edge_type_count(edge_type)`
- `rebuild_schema()`

Indices:

- `create_index(label, property, index_type="hash")`
- `drop_index(index_name)`
- `list_indexes()`

APIs bajo `python-full`:

- embeddings: `add_node_embedding`, `add_edge_embedding`,
  `add_path_reference_embedding`, `get_node_embedding`, `knn_nodes`
- OWL: `import_turtle`
- reasoner: `ELReasoner`

**Version:** 0.4.27
**Updated:** June 2026
