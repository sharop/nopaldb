# Hands-on: CRUD de contenido del grafo con NQL, Rust y Python

> Guía práctica de inicio a fin para crear, leer, actualizar y borrar contenido de un grafo en NopalDB, usando el mismo caso de uso desde NQL, Rust y Python.

## 1. Objetivo

Al final de este hands-on vas a:

1. abrir un grafo
2. crear nodos y relaciones
3. leerlos con NQL
4. actualizar nodos y aristas
5. borrar una relación y luego un nodo

Este documento distingue claramente:

- **API Rust/Python**: crea o abre el grafo/storage
- **NQL**: hace CRUD sobre el **contenido**

## 2. Dataset de ejemplo

Vamos a usar:

- nodos:
  - `Person`
  - `Company`
- relaciones:
  - `WORKS_AT`
  - `KNOWS`

Entidades:

- Alice
- Bob
- Carol
- Acme Corp

Propiedades:

- nodos:
  - `name`
  - `role`
  - `city`
- aristas:
  - `since`
  - `strength`

## 3. Qué hace cada interfaz

### NQL

Resuelve:

- `ADD`
- `FIND`
- `UPDATE`
- `DELETE`

No resuelve:

- crear el storage/base del grafo
- `BEGIN / COMMIT / ROLLBACK`
- `CREATE GRAPH`

### Rust

Resuelve:

- apertura del grafo
- control explícito de transacciones
- API tipada
- posibilidad de ejecutar NQL dentro de una app Rust

### Python

Resuelve:

- scripting rápido
- notebooks y pipelines
- la misma lógica de grafo/NQL del core Rust con una ergonomía más ligera

## 4. Abrir el grafo

### Rust

```rust
use nopaldb::Graph;

# async fn example() -> nopaldb::Result<()> {
let graph = Graph::open("./crud_demo.ndb").await?;
# Ok(())
# }
```

### Python

```python
import nopaldb

graph = nopaldb.Graph.open("./crud_demo.ndb")
```

### NQL

NQL empieza **después** de abrir el grafo. No crea la base por sí mismo.

## 5. CREATE del contenido

### NQL

```nql
add (a:Person {name: "Alice", role: "analyst", city: "CDMX"})
    -[:KNOWS {since: 2020, strength: "high"}]->
    (b:Person {name: "Bob", role: "engineer", city: "GDL"})
```

```nql
add (c:Person {name: "Carol", role: "designer", city: "MTY"})
    -[:WORKS_AT {since: 2023}]->
    (co:Company {name: "Acme Corp", city: "CDMX"})
```

### Rust

```rust
use nopaldb::{Graph, Node, Edge, PropertyValue};

# async fn example() -> nopaldb::Result<()> {
let graph = Graph::open("./crud_demo.ndb").await?;
let mut tx = graph.begin_transaction().await?;

let alice = tx.add_node(
    Node::new("Person")
        .with_property("name", PropertyValue::String("Alice".into()))
        .with_property("role", PropertyValue::String("analyst".into()))
        .with_property("city", PropertyValue::String("CDMX".into()))
).await?;

let bob = tx.add_node(
    Node::new("Person")
        .with_property("name", PropertyValue::String("Bob".into()))
        .with_property("role", PropertyValue::String("engineer".into()))
        .with_property("city", PropertyValue::String("GDL".into()))
).await?;

tx.commit().await?;

graph.add_edge(
    Edge::new(alice, bob, "KNOWS")
        .with_property("since", PropertyValue::Int(2020))
        .with_property("strength", PropertyValue::String("high".into()))
).await?;
# Ok(())
# }
```

Particularidad de Rust:

- tú controlas exactamente cuándo empieza y termina la transacción

### Python

```python
import nopaldb

graph = nopaldb.Graph.open("./crud_demo.ndb")
tx = graph.begin_transaction()

alice = tx.add_node("Person", {
    "name": "Alice",
    "role": "analyst",
    "city": "CDMX",
})

bob = tx.add_node("Person", {
    "name": "Bob",
    "role": "engineer",
    "city": "GDL",
})

tx.commit()

tx = graph.begin_transaction()
tx.add_edge(alice, bob, "KNOWS", {
    "since": 2020,
    "strength": "high",
})
tx.commit()
```

Particularidad de Python:

- es la forma más directa para demos, notebooks y automatización ligera

## 6. READ del contenido

### NQL

```nql
find p.name, p.role, p.city
from (p:Person)
```

```nql
find a.name, r.since, r.strength, b.name
from (a:Person)-[r:KNOWS]->(b:Person)
```

### Rust ejecutando NQL

```rust
# async fn example(graph: &nopaldb::Graph) -> nopaldb::Result<()> {
let result = graph.execute_statement(r#"
    find a.name, r.since, r.strength, b.name
    from (a:Person)-[r:KNOWS]->(b:Person)
"#).await?;
# Ok(())
# }
```

### Python ejecutando NQL

```python
result = graph.execute_nql("""
    find a.name, r.since, r.strength, b.name
    from (a:Person)-[r:KNOWS]->(b:Person)
""")
```

## 7. UPDATE del contenido

### NQL: actualizar nodo

```nql
update (p:Person)
set p.city = "Monterrey"
where p.name = "Bob"
```

### NQL: actualizar arista

```nql
update (a:Person)-[r:KNOWS]->(b:Person)
set r.since = 2024, r.strength = "medium"
where a.name = "Alice" and b.name = "Bob"
```

### Rust

Rust puede actualizar:

- con API directa si quieres control fino sobre los objetos
- o ejecutando NQL si quieres una capa declarativa consistente con otras interfaces

```rust
# async fn example(graph: &nopaldb::Graph) -> nopaldb::Result<()> {
graph.execute_statement(r#"
    update (a:Person)-[r:KNOWS]->(b:Person)
    set r.since = 2024
    where a.name = "Alice" and b.name = "Bob"
"#).await?;
# Ok(())
# }
```

### Python

```python
graph.execute_nql("""
    update (p:Person)
    set p.city = "Monterrey"
    where p.name = "Bob"
""")
```

## 8. DELETE del contenido

### NQL: borrar solo una relación

```nql
delete (a:Person)-[:KNOWS]->(b:Person)
where a.name = "Alice" and b.name = "Bob"
```

Semántica actual:

- este patrón borra **solo** la arista matched
- no borra los nodos endpoint

### NQL: borrar un nodo

```nql
delete (p:Person)
where p.name = "Carol"
```

Semántica actual:

- borra el nodo
- y también sus aristas incidentes

### Rust

En Rust también puedes usar API directa:

```rust
# async fn example(graph: &nopaldb::Graph, edge_id: nopaldb::EdgeId) -> nopaldb::Result<()> {
graph.delete_edge(edge_id).await?;
# Ok(())
# }
```

### Python

```python
graph.execute_nql("""
    delete (p:Person)
    where p.name = "Carol"
""")
```

## 9. Verificación

Después de crear datos:

```nql
find a.name, r.since, b.name
from (a:Person)-[r:KNOWS]->(b:Person)
```

Después del `UPDATE` del edge:

- `r.since` debe cambiar
- `r.strength` debe reflejar el nuevo valor

Después del `DELETE` relacional:

- los nodos `Alice` y `Bob` deben seguir existiendo
- la relación `KNOWS` ya no debe existir

Después del `DELETE` del nodo:

- `Carol` no debe aparecer en `find p.name from (p:Person)`

## 10. Diferencias prácticas entre NQL, Rust y Python

| Superficie | Qué resuelve mejor | Particularidad |
|---|---|---|
| NQL | CRUD declarativo del contenido | no crea el storage |
| Rust | control fino, transacciones explícitas, embedding en apps | tipado fuerte |
| Python | scripting, notebooks, pipelines | ergonomía rápida sobre el mismo motor |

## 11. Limitaciones actuales

Esta etapa deliberadamente deja fuera:

- `CREATE GRAPH` en NQL
- `BEGIN / COMMIT / ROLLBACK` como sintaxis NQL
- `MERGE`

Si necesitas crear o abrir la base, sigue usando:

- Rust: `Graph::open(...)`, `Graph::in_memory()`
- Python: `nopaldb.Graph.open(...)`, `nopaldb.Graph.in_memory()`
