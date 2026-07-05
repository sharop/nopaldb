# NopalDB Python API Reference
# Referencia del API de NopalDB Python

**[English](#english)** | **[Español](#español)**

---

<a name="english"></a>
## 🔷 English

### Graph Class

Main entry point for working with NopalDB.

#### Constructor Methods

##### `Graph.open(path: str) -> Graph`

Open or create a persistent graph database.

```python
graph = nopaldb.Graph.open("data/my_graph.db")
```

**Parameters:**
- `path` (str): Path to database directory

**Returns:** Graph instance

**See also:** [Configuration Guide](CONFIGURATION.md) for details on default settings.

---

##### `Graph.in_memory() -> Graph`

Create an in-memory graph (non-persistent).

```python
graph = nopaldb.Graph.in_memory()
```

**Returns:** Graph instance

**Note:** Data is lost when the process ends.

---

#### Transaction Methods

##### `begin_transaction() -> Transaction`

Start a new transaction.

```python
tx = graph.begin_transaction()
# ... add nodes/edges ...
tx.commit()
```

**Returns:** Transaction instance

**Important:** Always commit or rollback transactions.

---

#### Query Methods

##### `execute_nql(query: str) -> NqlResult`

Execute any NQL statement and return a unified `NqlResult`.

```python
# Read
result = graph.execute_nql("find p.name from (p:Person)")
rows = result.query

# Write
write_result = graph.execute_nql("add (p:Person {name: 'Alice'})")
counts = write_result.write

# Profile
profile = graph.execute_nql("profile find p.name from (p:Person)")
stats = profile.profile
```

**Parameters:**
- `query` (str): NQL query string

**Returns:** `NqlResult`

**Raises:** RuntimeError if query fails

---

#### Export Methods

##### `to_arrow(label: str = None) -> bytes`

Export nodes to Apache Arrow format.

```python
nodes_bytes = graph.to_arrow(label="Person")
```

**Returns:** Arrow IPC stream (bytes)

---

##### `edges_to_arrow() -> bytes`

Export edges to Apache Arrow format.

```python
edges_bytes = graph.edges_to_arrow()
```

**Returns:** Arrow IPC stream (bytes)

---

##### `to_arrow_complete(label: str = None) -> tuple[bytes, bytes]`

Export complete graph (nodes + edges) to Arrow.

```python
nodes_bytes, edges_bytes = graph.to_arrow_complete()
```

**Returns:** Tuple of (nodes_bytes, edges_bytes)

---

### Transaction Class

Manages atomic operations on the graph.

#### Methods

##### `add_node(label: str, properties: dict = None) -> str`

Add a node to the graph.

```python
tx.add_node("Person", {"name": "Alice"})
```

**Properties Support:**
- `str`, `int`, `float`, `bool`, `None`, `bytes`

##### `add_edge(source: str, target: str, edge_type: str, properties: dict = None) -> str`

Add an edge between two nodes.

```python
tx.add_edge(id1, id2, "KNOWS", {"since": 2024})
```

##### `delete_node(id: str)`
Mark a node for deletion.

##### `delete_edge(id: str)`
Mark an edge for deletion.

##### `commit()` / `rollback()`
Finalize or cancel the transaction.

---

### NqlResult Class

- `kind`: `"query" | "write" | "index" | "explain" | "profile" | "export" | "message"`
- `query`: `QueryResult | None`
- `write`: `dict | None`
- `explain`: `str | None`
- `profile`: `ProfileResult | None`
- `message`: `str | None`
- `summary`: resumen legible del resultado

### QueryResult Class

Result of a read query (`FIND`).

#### Methods
- `__len__()`: Row count
- `__iter__()`: Iterator over rows
- `__getitem__(i)`: Get row at index
- `columns`: List of column names

### ProfileResult Class

Structured result for `PROFILE <query>`.

- `plan`
- `statement_type`
- `execution_ms`
- `rows_returned`
- `columns`
- `path_query`
- `path_metrics`

---

<a name="español"></a>
## 🔷 Español

### Clase Graph

Punto de entrada principal para trabajar con NopalDB.

#### Métodos de Consulta

##### `execute_nql(query: str) -> NqlResult`

Ejecuta una consulta NQL. Soporta tanto lectura (`FIND`) como escritura (`ADD`, `UPDATE`, `DELETE`).

```python
# Lectura
res = graph.execute_nql("find p.nombre from (p:Persona)")
for row in res.query:
    print(row["p.nombre"])

# Escritura
res = graph.execute_nql("add (p:Persona {nombre: 'Alice'})")
print(res.write)
```

**Parámetros:**
- `query` (str): Cadena de consulta NQL

**Ver también:** [Guía NQL](NQL_GUIDE.md)

---

#### Métodos de Exportación

##### `to_arrow(label: str = None) -> bytes`

Exporta nodos a formato Apache Arrow.

##### `edges_to_arrow() -> bytes`

Exporta aristas a formato Apache Arrow.

---

### Clase Transaction

#### Métodos

##### `add_node(label: str, properties: dict = None) -> str`
Agrega un nodo en la transacción actual.

##### `add_edge(source: str, target: str, edge_type: str, properties: dict = None) -> str`
Agrega una arista. Soporta propiedades en la arista.

---

### Reference / Referencia

**Version:** 0.2.0  
**Updated:** January 2026
