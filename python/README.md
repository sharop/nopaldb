# NopalDB Python Bindings

**[English](#english) | [Español](#español)**

---

## English

### Installation

#### From PyPI
```bash
pip install nopaldb
```
Prebuilt wheels for Linux, macOS and Windows (CPython 3.10+).

#### From Source (development)
```bash
# Clone repository
git clone https://github.com/sharop/nopaldb
cd nopaldb

# Create virtual environment (recommended)
python -m venv venv
source venv/bin/activate  # On Windows: venv\Scripts\activate

# Install maturin
pip install maturin

# Build and install
maturin develop --features python,analytics
```



---

### Quick Start
```python
import nopaldb

# Create in-memory database
graph = nopaldb.Graph.in_memory()

# Begin transaction
tx = graph.begin_transaction()

# Add nodes
alice = tx.add_node("Person", {
    "name": "Alice",
    "age": 30,
    "city": "CDMX"
})

bob = tx.add_node("Person", {
    "name": "Bob",
    "age": 25,
    "city": "GDL"
})

# Add edge
tx.add_edge(alice, bob, "KNOWS")

# Commit
tx.commit()

# Query with NQL
result = graph.execute_nql("""
    find p.name, p.age
    from (p:Person)
    where p.age > 25
""")

# Iterate results
for row in result:
    print(f"{row['p.name']}: {row['p.age']} years old")
```

**Output:**
```
Alice: 30 years old
```

---

### API Reference

#### Graph Class

**Constructor Methods:**
```python
# Open persistent database
graph = nopaldb.Graph.open("./data.db")

# Create in-memory database
graph = nopaldb.Graph.in_memory()

# Optional profile tuning (default/mobile/server)
graph = nopaldb.Graph.open_with_profile("./data.db", profile="mobile")

# Explicit backend + profile
graph = nopaldb.Graph.open_with_options("./data.db", engine="sled", profile="default")
```

**Methods:**

| Method | Parameters | Returns | Description |
|--------|------------|---------|-------------|
| `begin_transaction()` | None | `Transaction` | Start a new transaction |
| `execute_nql(query)` | `query: str` | `QueryResult` | Execute NQL query |
| `to_arrow(label=None)` | `label: str \| None` | `bytes` | Export to Arrow IPC format |
| `node_count()` | None | `int` | Get total node count |

**Examples:**
```python
# Open database
graph = nopaldb.Graph.open("./mydata.db")

# Get node count
count = graph.node_count()
print(f"Total nodes: {count}")

# Execute query
result = graph.execute_nql("find * from (p:Person) limit 10")

# Export to Arrow
arrow_bytes = graph.to_arrow(label="Person")
```

---

#### Transaction Class

**Methods:**

| Method | Parameters | Returns | Description |
|--------|------------|---------|-------------|
| `add_node(label, properties)` | `label: str`<br>`properties: dict` | `str` (node ID) | Add a node |
| `add_edge(source, target, type)` | `source: str`<br>`target: str`<br>`type: str` | `str` (edge ID) | Add an edge |
| `commit()` | None | None | Commit transaction |
| `rollback()` | None | None | Rollback transaction |

**Examples:**
```python
# Begin transaction
tx = graph.begin_transaction()

# Add node (returns UUID)
person_id = tx.add_node("Person", {
    "name": "Alice",
    "age": 30,
    "active": True,
    "score": 95.5
})

# Add edge
company_id = tx.add_node("Company", {"name": "Acme Corp"})
tx.add_edge(person_id, company_id, "WORKS_AT")

# Commit changes
tx.commit()

# Rollback example
tx2 = graph.begin_transaction()
tx2.add_node("Test", {"data": "temp"})
tx2.rollback()  # Changes discarded
```

**Property Types:**

| Python Type | NopalDB Type | Example |
|-------------|--------------|---------|
| `str` | String | `"Alice"` |
| `int` | Int | `42` |
| `float` | Float | `3.14` |
| `bool` | Bool | `True` |
| `None` | Null | `None` |
| `bytes` | Bytes | `b'\x00\x01'` |

---

#### QueryResult Class

**Methods:**

| Method | Parameters | Returns | Description |
|--------|------------|---------|-------------|
| `__len__()` | None | `int` | Number of rows |
| `__iter__()` | None | Iterator | Iterate over rows |
| `__getitem__(index)` | `index: int` | `dict` | Get row by index |
| `columns` | Property | `list[str]` | Column names |

**Examples:**
```python
result = graph.execute_nql("find p.name, p.age from (p:Person)")

# Get row count
print(f"Found {len(result)} people")

# Get columns
print(f"Columns: {result.columns}")

# Iterate rows
for row in result:
    print(row['p.name'], row['p.age'])

# Access by index
first_person = result[0]
print(first_person['p.name'])

# List comprehension
names = [row['p.name'] for row in result]
```

---

### Apache Arrow Integration

Export graph data to Apache Arrow for zero-copy integration with Pandas, Polars, PyTorch, and other data science tools.

#### Basic Export
```python
import pyarrow as pa
import pandas as pd

# Export nodes with properties
arrow_bytes = graph.to_arrow(label="Person")

# Load with PyArrow
reader = pa.ipc.open_stream(arrow_bytes)
batch = reader.read_next_batch()

# Convert to Pandas (zero-copy!)
df = batch.to_pandas()

print(df.head())
```

#### ML Pipeline Example
```python
import torch
from sklearn.ensemble import RandomForestClassifier

# Export features
arrow_bytes = graph.to_arrow(label="Sample")
batch = pa.ipc.open_stream(arrow_bytes).read_next_batch()
df = batch.to_pandas()

# Prepare features
X = df[['feature_1', 'feature_2', 'feature_3']].values
y = df['target'].values

# Train scikit-learn model
clf = RandomForestClassifier()
clf.fit(X, y)

# Or convert to PyTorch
X_tensor = torch.from_numpy(X).float()
y_tensor = torch.from_numpy(y).long()
```

#### Integration with Other Tools
```python
# Polars
import polars as pl
pl_df = pl.from_arrow(batch)

# DuckDB
import duckdb
con = duckdb.connect()
con.register("nodes", batch)
result = con.execute("SELECT AVG(age) FROM nodes").fetchone()

# Export to Parquet
import pyarrow.parquet as pq
pq.write_table(batch, "graph_export.parquet")
```

---

### Examples

#### Social Network Analysis
```python
import nopaldb

graph = nopaldb.Graph.open("./social.db")

# Find mutual friends
result = graph.execute_nql("""
    find a.name, b.name, mutual.name
    from (a:Person) -> [:KNOWS] -> (mutual:Person) <- [:KNOWS] <- (b:Person)
    where a.name = "Alice" and b.name = "Bob"
""")

for row in result:
    print(f"{row['a.name']} and {row['b.name']} both know {row['mutual.name']}")
```

#### Knowledge Graph
```python
# Build knowledge graph
tx = graph.begin_transaction()

# Add concepts
ai = tx.add_node("Concept", {"name": "Artificial Intelligence"})
ml = tx.add_node("Concept", {"name": "Machine Learning"})
dl = tx.add_node("Concept", {"name": "Deep Learning"})

# Add relationships
tx.add_edge(ml, ai, "IS_SUBFIELD_OF")
tx.add_edge(dl, ml, "IS_SUBFIELD_OF")

tx.commit()

# Query hierarchy
result = graph.execute_nql("""
    find child.name, parent.name
    from (child:Concept) -> [:IS_SUBFIELD_OF] -> (parent:Concept)
""")
```

#### Recommendation System
```python
# Find items liked by similar users
result = graph.execute_nql("""
    find recommended.title
    from (user:User) -> [:LIKES] -> (item:Item) <- [:LIKES] <- (similar:User) -> [:LIKES] -> (recommended:Item)
    where user.name = "Alice"
    limit 10
""")

recommendations = [row['recommended.title'] for row in result]
print(f"Recommended: {recommendations}")
```

---

### Performance Tips

#### 1. Batch Operations
```python
# ❌ Bad: Multiple small transactions
for data in dataset:
    tx = graph.begin_transaction()
    tx.add_node("Data", data)
    tx.commit()  # Slow!

# ✅ Good: Single large transaction
tx = graph.begin_transaction()
for data in dataset:
    tx.add_node("Data", data)
tx.commit()  # Fast!
```

#### 2. Use LIMIT for Large Results
```python
# ❌ Bad: Load millions of nodes
result = graph.execute_nql("find * from (n:Node)")

# ✅ Good: Paginate results
for offset in range(0, total, 1000):
    result = graph.execute_nql(f"""
        find * from (n:Node)
        limit 1000 offset {offset}
    """)
    process_batch(result)
```

#### 3. Export to Arrow for Analysis
```python
# ❌ Bad: Query and process in Python
result = graph.execute_nql("find * from (p:Person)")
for row in result:
    # Slow row-by-row processing
    process(row)

# ✅ Good: Export to Arrow, use vectorized operations
df = graph.to_arrow(label="Person").to_pandas()
# Fast vectorized processing
df['age_group'] = pd.cut(df['age'], bins=[0, 18, 65, 100])
```

#### 4. Connection Pooling
```python
# Reuse graph connection
graph = nopaldb.Graph.open("./data.db")

# Multiple operations on same connection
for query in queries:
    result = graph.execute_nql(query)
    process(result)
```

---

### Error Handling
```python
import nopaldb

try:
    graph = nopaldb.Graph.open("./data.db")
    
    tx = graph.begin_transaction()
    node_id = tx.add_node("Person", {"name": "Alice"})
    tx.commit()
    
except RuntimeError as e:
    print(f"NopalDB error: {e}")
    # Handle database errors
    
except Exception as e:
    print(f"Unexpected error: {e}")
```

---

### Best Practices

1. ✅ **Use context managers** (coming soon)
2. ✅ **Always commit or rollback transactions**
3. ✅ **Use Arrow export for large datasets**
4. ✅ **Batch operations in transactions**
5. ✅ **Close connections when done** (auto-handled currently)
6. ✅ **Use type hints for better IDE support**
```python
from typing import Dict, List

def process_graph(graph: nopaldb.Graph) -> List[Dict]:
    result = graph.execute_nql("find * from (p:Person)")
    return [dict(row) for row in result]
```

---

### Requirements

- Python >= 3.8
- PyArrow (for Arrow export)
- Pandas (optional, for DataFrame conversion)
```bash
pip install pyarrow pandas
```

---

### See Also

- [NQL Reference](../docs/en/NQL_REFERENCE.md) - Query language syntax
- [NQL Tutorial](../docs/en/NQL_TUTORIAL.md) - Learn NQL
- [Examples](../examples/) - Code examples
- [GitHub](https://github.com/sharop/nopaldb) - Source code

---

## Español

### Instalación

#### Desde Código Fuente
```bash
# Clonar repositorio
git clone https://github.com/sharop/nopaldb
cd nopaldb

# Crear entorno virtual (recomendado)
python -m venv venv
source venv/bin/activate  # En Windows: venv\Scripts\activate

# Instalar maturin
pip install maturin

# Compilar e instalar
maturin develop --features python,analytics
```

#### Desde PyPI (Próximamente)
```bash
pip install nopaldb
```

---

### Inicio Rápido
```python
import nopaldb

# Crear base de datos en memoria
graph = nopaldb.Graph.in_memory()

# Comenzar transacción
tx = graph.begin_transaction()

# Agregar nodos
alice = tx.add_node("Person", {
    "name": "Alice",
    "age": 30,
    "city": "CDMX"
})

bob = tx.add_node("Person", {
    "name": "Bob",
    "age": 25,
    "city": "GDL"
})

# Agregar arista
tx.add_edge(alice, bob, "CONOCE")

# Confirmar
tx.commit()

# Consultar con NQL
resultado = graph.execute_nql("""
    find p.name, p.age
    from (p:Person)
    where p.age > 25
""")

# Iterar resultados
for fila in resultado:
    print(f"{fila['p.name']}: {fila['p.age']} años")
```

**Salida:**
```
Alice: 30 años
```

---

### Referencia de API

#### Clase Graph

**Métodos Constructores:**
```python
# Abrir base de datos persistente
graph = nopaldb.Graph.open("./datos.db")

# Crear base de datos en memoria
graph = nopaldb.Graph.in_memory()

# Perfil opcional (default/mobile/server)
graph = nopaldb.Graph.open_with_profile("./datos.db", profile="mobile")

# Backend + perfil explícitos
graph = nopaldb.Graph.open_with_options("./datos.db", engine="sled", profile="default")
```

**Métodos:**

| Método | Parámetros | Retorna | Descripción |
|--------|------------|---------|-------------|
| `begin_transaction()` | Ninguno | `Transaction` | Iniciar nueva transacción |
| `execute_nql(query)` | `query: str` | `QueryResult` | Ejecutar consulta NQL |
| `to_arrow(label=None)` | `label: str \| None` | `bytes` | Exportar a formato Arrow IPC |
| `node_count()` | Ninguno | `int` | Obtener conteo total de nodos |

**Ejemplos:**
```python
# Abrir base de datos
graph = nopaldb.Graph.open("./misdatos.db")

# Obtener conteo de nodos
conteo = graph.node_count()
print(f"Total de nodos: {conteo}")

# Ejecutar consulta
resultado = graph.execute_nql("find * from (p:Person) limit 10")

# Exportar a Arrow
arrow_bytes = graph.to_arrow(label="Person")
```

---

#### Clase Transaction

**Métodos:**

| Método | Parámetros | Retorna | Descripción |
|--------|------------|---------|-------------|
| `add_node(label, properties)` | `label: str`<br>`properties: dict` | `str` (ID nodo) | Agregar un nodo |
| `add_edge(source, target, type)` | `source: str`<br>`target: str`<br>`type: str` | `str` (ID arista) | Agregar una arista |
| `commit()` | Ninguno | None | Confirmar transacción |
| `rollback()` | Ninguno | None | Revertir transacción |

**Ejemplos:**
```python
# Comenzar transacción
tx = graph.begin_transaction()

# Agregar nodo (retorna UUID)
persona_id = tx.add_node("Person", {
    "name": "Alice",
    "age": 30,
    "active": True,
    "score": 95.5
})

# Agregar arista
empresa_id = tx.add_node("Company", {"name": "Acme Corp"})
tx.add_edge(persona_id, empresa_id, "TRABAJA_EN")

# Confirmar cambios
tx.commit()

# Ejemplo de rollback
tx2 = graph.begin_transaction()
tx2.add_node("Test", {"data": "temp"})
tx2.rollback()  # Cambios descartados
```

**Tipos de Propiedades:**

| Tipo Python | Tipo NopalDB | Ejemplo |
|-------------|--------------|---------|
| `str` | String | `"Alice"` |
| `int` | Int | `42` |
| `float` | Float | `3.14` |
| `bool` | Bool | `True` |
| `None` | Null | `None` |
| `bytes` | Bytes | `b'\x00\x01'` |

---

#### Clase QueryResult

**Métodos:**

| Método | Parámetros | Retorna | Descripción |
|--------|------------|---------|-------------|
| `__len__()` | Ninguno | `int` | Número de filas |
| `__iter__()` | Ninguno | Iterator | Iterar sobre filas |
| `__getitem__(index)` | `index: int` | `dict` | Obtener fila por índice |
| `columns` | Propiedad | `list[str]` | Nombres de columnas |

**Ejemplos:**
```python
resultado = graph.execute_nql("find p.name, p.age from (p:Person)")

# Obtener conteo de filas
print(f"Encontradas {len(resultado)} personas")

# Obtener columnas
print(f"Columnas: {resultado.columns}")

# Iterar filas
for fila in resultado:
    print(fila['p.name'], fila['p.age'])

# Acceder por índice
primera_persona = resultado[0]
print(primera_persona['p.name'])

# List comprehension
nombres = [fila['p.name'] for fila in resultado]
```

---

### Integración con Apache Arrow

Exporta datos de grafos a Apache Arrow para integración zero-copy con Pandas, Polars, PyTorch y otras herramientas de ciencia de datos.

#### Exportación Básica
```python
import pyarrow as pa
import pandas as pd

# Exportar nodos con propiedades
arrow_bytes = graph.to_arrow(label="Person")

# Cargar con PyArrow
reader = pa.ipc.open_stream(arrow_bytes)
batch = reader.read_next_batch()

# Convertir a Pandas (¡zero-copy!)
df = batch.to_pandas()

print(df.head())
```

#### Ejemplo de Pipeline ML
```python
import torch
from sklearn.ensemble import RandomForestClassifier

# Exportar features
arrow_bytes = graph.to_arrow(label="Sample")
batch = pa.ipc.open_stream(arrow_bytes).read_next_batch()
df = batch.to_pandas()

# Preparar features
X = df[['feature_1', 'feature_2', 'feature_3']].values
y = df['target'].values

# Entrenar modelo scikit-learn
clf = RandomForestClassifier()
clf.fit(X, y)

# O convertir a PyTorch
X_tensor = torch.from_numpy(X).float()
y_tensor = torch.from_numpy(y).long()
```

#### Integración con Otras Herramientas
```python
# Polars
import polars as pl
pl_df = pl.from_arrow(batch)

# DuckDB
import duckdb
con = duckdb.connect()
con.register("nodos", batch)
resultado = con.execute("SELECT AVG(age) FROM nodos").fetchone()

# Exportar a Parquet
import pyarrow.parquet as pq
pq.write_table(batch, "grafo_exportado.parquet")
```

---

### Ejemplos

#### Análisis de Redes Sociales
```python
import nopaldb

graph = nopaldb.Graph.open("./social.db")

# Encontrar amigos en común
resultado = graph.execute_nql("""
    find a.name, b.name, mutuo.name
    from (a:Person) -> [:CONOCE] -> (mutuo:Person) <- [:CONOCE] <- (b:Person)
    where a.name = "Alice" and b.name = "Bob"
""")

for fila in resultado:
    print(f"{fila['a.name']} y {fila['b.name']} conocen a {fila['mutuo.name']}")
```

#### Grafo de Conocimiento
```python
# Construir grafo de conocimiento
tx = graph.begin_transaction()

# Agregar conceptos
ia = tx.add_node("Concepto", {"name": "Inteligencia Artificial"})
ml = tx.add_node("Concepto", {"name": "Machine Learning"})
dl = tx.add_node("Concepto", {"name": "Deep Learning"})

# Agregar relaciones
tx.add_edge(ml, ia, "ES_SUBCAMPO_DE")
tx.add_edge(dl, ml, "ES_SUBCAMPO_DE")

tx.commit()

# Consultar jerarquía
resultado = graph.execute_nql("""
    find hijo.name, padre.name
    from (hijo:Concepto) -> [:ES_SUBCAMPO_DE] -> (padre:Concepto)
""")
```

#### Sistema de Recomendación
```python
# Encontrar items gustados por usuarios similares
resultado = graph.execute_nql("""
    find recomendado.title
    from (usuario:User) -> [:GUSTA] -> (item:Item) <- [:GUSTA] <- (similar:User) -> [:GUSTA] -> (recomendado:Item)
    where usuario.name = "Alice"
    limit 10
""")

recomendaciones = [fila['recomendado.title'] for fila in resultado]
print(f"Recomendado: {recomendaciones}")
```

---

### Tips de Performance

#### 1. Operaciones en Lote
```python
# ❌ Malo: Múltiples transacciones pequeñas
for datos in dataset:
    tx = graph.begin_transaction()
    tx.add_node("Data", datos)
    tx.commit()  # ¡Lento!

# ✅ Bueno: Una transacción grande
tx = graph.begin_transaction()
for datos in dataset:
    tx.add_node("Data", datos)
tx.commit()  # ¡Rápido!
```

#### 2. Usa LIMIT para Resultados Grandes
```python
# ❌ Malo: Cargar millones de nodos
resultado = graph.execute_nql("find * from (n:Node)")

# ✅ Bueno: Paginar resultados
for offset in range(0, total, 1000):
    resultado = graph.execute_nql(f"""
        find * from (n:Node)
        limit 1000 offset {offset}
    """)
    procesar_lote(resultado)
```

#### 3. Exportar a Arrow para Análisis
```python
# ❌ Malo: Consultar y procesar en Python
resultado = graph.execute_nql("find * from (p:Person)")
for fila in resultado:
    # Procesamiento lento fila por fila
    procesar(fila)

# ✅ Bueno: Exportar a Arrow, usar operaciones vectorizadas
df = graph.to_arrow(label="Person").to_pandas()
# Procesamiento vectorizado rápido
df['grupo_edad'] = pd.cut(df['age'], bins=[0, 18, 65, 100])
```

#### 4. Reutilizar Conexiones
```python
# Reutilizar conexión al grafo
graph = nopaldb.Graph.open("./datos.db")

# Múltiples operaciones en la misma conexión
for consulta in consultas:
    resultado = graph.execute_nql(consulta)
    procesar(resultado)
```

---

### Manejo de Errores
```python
import nopaldb

try:
    graph = nopaldb.Graph.open("./datos.db")
    
    tx = graph.begin_transaction()
    nodo_id = tx.add_node("Person", {"name": "Alice"})
    tx.commit()
    
except RuntimeError as e:
    print(f"Error de NopalDB: {e}")
    # Manejar errores de base de datos
    
except Exception as e:
    print(f"Error inesperado: {e}")
```

---

### Mejores Prácticas

1. ✅ **Usar context managers** (`with nopaldb.Graph.open(...) as graph:`)
2. ✅ **Siempre confirmar o revertir transacciones**
3. ✅ **Usar exportación Arrow para datasets grandes**
4. ✅ **Agrupar operaciones en transacciones**
5. ✅ **Cerrar conexiones al terminar** (manejado automáticamente actualmente)
6. ✅ **Usar type hints para mejor soporte IDE**
```python
from typing import Dict, List

def procesar_grafo(graph: nopaldb.Graph) -> List[Dict]:
    resultado = graph.execute_nql("find * from (p:Person)")
    return [dict(fila) for fila in resultado]
```

---

### Requisitos

- Python >= 3.8
- PyArrow (para exportación Arrow)
- Pandas (opcional, para conversión a DataFrame)
```bash
pip install pyarrow pandas
```

---

### Ver También

- [Referencia NQL](../docs/es/NQL_REFERENCIA.md) - Sintaxis del lenguaje
- [Tutorial NQL](../docs/es/NQL_TUTORIAL.md) - Aprende NQL
- [Ejemplos](../examples/) - Ejemplos de código
- [GitHub](https://github.com/sharop/nopaldb) - Código fuente

---

**NopalDB** - Built with Rust and Python by the NopalDB community.
