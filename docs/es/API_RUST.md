# Referencia corta de API Rust

Esta pagina resume las entradas publicas principales exportadas por
`nopaldb/src/lib.rs`. Para una lista exhaustiva usa `cargo doc -p nopaldb --open`.

## Imports comunes

```rust
use nopaldb::{
    Edge, Graph, IndexType, Node, PropertyValue, Result,
};
```

## Abrir un grafo

```rust
let graph = Graph::open("data/mi_grafo.db").await?;
```

Variantes disponibles:

- `Graph::open(path)`
- `Graph::open_with_profile(path, StorageProfile)`
- `Graph::open_with_options(path, StorageOptions)`
- `Graph::in_memory()`
- `Graph::in_memory_with_profile(StorageProfile)`
- `Graph::in_memory_with_options(StorageOptions)`

## CRUD basico

```rust
let alice = Node::new("Person")
    .with_property("name", PropertyValue::String("Alice".into()))
    .with_property("age", PropertyValue::Int(30));

let alice_id = graph.add_node(alice).await?;

let bob_id = graph
    .add_node(Node::new("Person").with_property(
        "name",
        PropertyValue::String("Bob".into()),
    ))
    .await?;

let edge = Edge::new(alice_id, bob_id, "KNOWS")
    .with_property("since", PropertyValue::Int(2024));

let edge_id = graph.add_edge(edge).await?;

let node = graph.get_node(alice_id).await?;
let relationship = graph.get_edge(edge_id).await?;

graph.delete_edge(edge_id).await?;
graph.delete_node(bob_id).await?;
```

## Transacciones

```rust
let tx = graph.begin_transaction().await?;

let user_id = tx
    .add_node(Node::new("User").with_property(
        "name",
        PropertyValue::String("Example User".into()),
    ))
    .await?;

tx.commit().await?;
```

`Transaction` expone operaciones de lectura/escritura sobre nodos y aristas y
termina con `commit()`, `rollback()` o `rollback_async()`.

## NQL

```rust
let result = graph
    .execute_nql(
        r#"
        find p.name, p.age
        from (p:Person)
        where p.age >= 18
        order by p.name
        "#,
    )
    .await?;

println!("{}", result.summary());
```

`execute_nql` retorna `NqlResult`, que cubre consultas de lectura, escrituras,
`EXPLAIN`, `PROFILE` y exportaciones soportadas por NQL.

## Indices

```rust
let index_name = graph
    .create_index("Person", "email", IndexType::Hash)
    .await?;

let indexes = graph.list_indexes().await;

graph.drop_index(&index_name).await?;
```

Tipos publicos principales:

- `IndexType::Hash`
- `IndexType::BTree`
- `IndexType::FullText` cuando el build incluye full-text

## Schema inspection

```rust
let labels = graph.get_labels().await?;
let edge_types = graph.get_edge_types().await?;
let schema = graph.get_schema().await?;

graph.rebuild_schema().await?;
```

Tambien existen conteos y propiedades por label/tipo de arista, como
`get_label_count`, `get_label_properties`, `get_edge_type_count` y
`get_edge_type_properties`.

## Arrow

Con las features publicas que incluyen Arrow:

```rust
let nodes = graph.to_arrow(None).await?;
let edges = graph.edges_to_arrow().await?;
let complete = graph.to_arrow_complete(None).await?;
```

## Embeddings, reasoner y SHACL

Estas APIs estan detras de features:

- `embeddings`: almacenamiento y busqueda de vectores.
- `reasoner`: `ELReasoner`.
- `owl-import`: import/export Turtle.
- `shacl`: `ShaclValidator`.

Build Rust completo:

```bash
cargo build -p nopaldb --release --features full
```

Wrapper Python:

```bash
make build-wheel
cd nopaldb && maturin develop --release --features python-full
```

`cargo build --features full` compila la libreria Rust; el wrapper Python se
construye con `maturin` para enlazar correctamente PyO3 contra Python.
