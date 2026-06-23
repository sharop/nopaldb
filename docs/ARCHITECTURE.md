# Arquitectura de NopalDB

> Vision general del diseño interno y las decisiones arquitectonicas de NopalDB v0.4.x.
> Para convenciones de desarrollo, reglas de codigo y comandos de build ver [CONTRIBUTING.md](../CONTRIBUTING.md).

---

## Stack de capas

```
┌──────────────────────────────────────────────────────────┐
│  Python Bindings (PyO3)          src/python/             │
├──────────────────────────────────────────────────────────┤
│  Query Engine (NQL)              src/query/nql/          │
├──────────────────────────────────────────────────────────┤
│  Transaction (MVCC / WAL)        src/transaction/        │
│                                  src/mvcc/               │
│                                  src/wal/                │
├──────────────────────────────────────────────────────────┤
│  Graph + Indexing                src/graph/              │
│                                  src/index/              │
│                                  src/embeddings/         │
├──────────────────────────────────────────────────────────┤
│  Storage (Sled)                  src/storage/            │
└──────────────────────────────────────────────────────────┘
```

Las capas se comunican unicamente hacia abajo. El query engine llama a la API de transacciones; las transacciones llaman al grafo; el grafo llama al storage. No hay dependencias circulares.

---

## Modelo de datos

### Node

```rust
pub struct Node {
    pub id:         NodeId,           // UUID v4
    pub label:      String,           // tipo del nodo ("Person", "Viral", "sh:NodeShape")
    pub kind:       NodeKind,         // Individual | Class | ObjectProperty | ...
    pub properties: HashMap<String, PropertyValue>,
}
```

### Edge

```rust
pub struct Edge {
    pub id:         EdgeId,
    pub source:     NodeId,
    pub target:     NodeId,
    pub edge_type:  String,           // "KNOWS", "subClassOf", "tratado_con", ...
    pub properties: HashMap<String, PropertyValue>,
}

// Nota:
// El tiempo de sistema/MVCC no vive en `Node` ni `Edge`.
// Vive en `VersionedNode` / `VersionedEdge` via `timestamp`, `valid_from` y `valid_to`.
// Si el dominio necesita un campo de negocio como `created_at`, debe ir en `properties`.
```

### PropertyValue

```rust
pub enum PropertyValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
}
```

### Serializacion

Nodos y aristas se serializan a disco con **MessagePack** (`rmp-serde`). Los metadatos de indices usan **Bincode** (`indexes/metadata.bin`). **No cambiar el formato ni los nombres de campos** — romperia bases de datos existentes.

---

## Capa de almacenamiento

Abstraccion KV sobre el backend embebido publico:

| Backend | Feature | Uso recomendado |
|---------|---------|-----------------|
| Sled | `storage-sled` (default) | desarrollo y despliegues embebidos |

El storage expone `insert`, `get`, `delete` y operaciones de scan sobre tres namespaces: nodos, aristas y versiones MVCC.

---

## MVCC y transacciones

### Version chain

Cada escritura crea un `VersionedNode` nuevo en lugar de sobreescribir el anterior:

```
VersionedNode {
    id:           NodeId,
    version:      u64,          // contador monotono
    timestamp:    u64,          // unix seconds al momento del commit
    node_data:    Node,
    valid_from:   u64,
    valid_to:     Option<u64>,  // None = version actual
    prev_version: Option<u64>,
}
```

`is_valid_at(t)` → `valid_from <= t && valid_to.map(|to| t < to).unwrap_or(true)`.

### Time-travel

```rust
// Snapshot Datomic-style
let snapshot = graph.as_of(timestamp);
let node = snapshot.get_node(node_id).await?;

// Consulta directa
let node = graph.get_node_at(node_id, timestamp).await?;

// Historial completo
let history = graph.history(node_id).await?;
```

Las aristas tienen cadena MVCC completa via `VersionedEdge` (tree `"versioned_edges"`). `get_edges_of_type_at(edge_type, timestamp)` consulta el historial; `graph.edge_history(id)` retorna todas las versiones.

### Niveles de aislamiento

| Nivel | Comportamiento |
|-------|---------------|
| `ReadCommitted` | default; ve commits ya confirmados |
| `RepeatableRead` | snapshot MVCC al inicio de la tx |
| `Serializable` | validacion de conflictos en writes |

**Estado actual:** `RepeatableRead` y `Serializable` leen snapshot MVCC via `get_node_at_strict()` con timestamp monotono. Serializable parcialmente implementado — no confiar en el para joins multi-hop criticos todavia.

### WAL

El Write-Ahead Log (`src/wal/`) registra cada escritura antes de que llegue al storage. `Graph::open()` hace replay automatico del WAL al arrancar. Es zona peligrosa — un cambio que corrompa la recuperacion puede destruir la base de datos.

### GC

Las versiones antiguas se limpian con `graph.gc(config)` (manual) o `graph.start_auto_gc(config)` (background). El GC respeta el horizonte seguro: el minimo timestamp de todas las transacciones activas.

---

## Query Engine (NQL)

NQL es el lenguaje de consulta de grafos de propiedades de NopalDB. El parser
convierte la consulta en AST, el planner elige una estrategia física y el
executor produce resultados tabulares o exportaciones Arrow.

### Pipeline

```
SQL-like string
     ↓
Pest grammar (src/query/nql/parser/nql.pest)
     ↓
AST (src/query/nql/parser/ast.rs)
     ↓
Query Planner (src/query/nql/planner.rs)
     ↓
Executor Volcano — streaming pull-based
     ↓
ResultSet / RecordBatch (Arrow)
```

### Modelo Volcano (streaming)

El executor usa el patron pull-based para no cargar grafos grandes en RAM. Cada operador implementa:

```rust
pub trait NodeStream: Send + Sync {
    fn next<'a>(&'a mut self) -> BoxFuture<'a, Result<Option<Node>>>;
}
```

**`async fn` en traits no es object-safe en Rust stable.** Se usa `BoxFuture` + `Box::pin(async move { ... })` obligatoriamente para todos los stream operators. No usar `async fn` en implementaciones de `NodeStream`.

### Orden de clausulas (estricto)

```
FIND … FROM … [WHERE] [GROUP BY] [HAVING] [ORDER BY] [LIMIT [OFFSET]] [AT TIMESTAMP] [EXPORT]
```

El parser rechaza cualquier otro orden.

### Capacidades principales

NQL ya soporta:

- patrones 1-hop
- cadenas lineales multi-hop y cuantificadores
- joins
- `WHERE`, agregaciones y funciones de grafo
- metadatos de path (`path.depth`, `path.nodes`, `path.edges`)
- `PROFILE <query>` estructurado
- exportacion a Arrow/CSV/JSON segun la consulta

---

## Indexing

| Tipo | Complejidad | Caso de uso |
|------|------------|-------------|
| Hash | O(1) equality | busqueda exacta por valor |
| B-Tree | O(log N) range | rangos, ordenamiento |
| Full-Text (Tantivy) | inverted index | busqueda de texto libre |
| TaxonomyIndex | O(1) `is_subclass_of` | clausura transitiva de jerarquia de clases |
| HnswIndex | ANN aproximado | similitud semantica sobre embeddings |

Los indices se persisten como metadatos en `indexes/metadata.bin` (bincode). Al arrancar, `IndexManager::load_indices()` reconstruye los indices desde los datos del storage — no desde un snapshot guardado del indice.

---

## Tier semantico

Tres modulos feature-gated que extienden el grafo con capacidades ontologicas:

### ELReasoner (`feature = reasoner`)

Implementa OWL-EL inference rules:
- **CR1**: transitividad de subclases (`A ⊑ B, B ⊑ C → A ⊑ C`)
- **CR2**: conjuncion (`A ⊑ B ⊓ C → A ⊑ B, A ⊑ C`)
- **CR3**: existencial (`A ⊑ ∃r.B → ...)

No modifica el grafo. Opera sobre una copia interna de la taxonomia. Soporta time-travel via `ELReasoner::from_graph_at(graph, timestamp)`.

### OWL/Turtle importer/exporter (`feature = owl-import`)

- Importer: 3 pasadas (clases, subclases, individuos) sobre Turtle/RDF
- Exporter: round-trip con tipos xsd

### ShaclValidator (`feature = shacl`)

Valida shapes contra el grafo. No modifica el grafo. Constraints implementados:
`minCount`, `maxCount`, `datatype`, `minInclusive/maxInclusive/minExclusive/maxExclusive`,
`minLength`, `maxLength`, `pattern` (regex), `in`, `hasValue`, `nodeKind`, `class`,
`PathSpec::Edge` (single-hop).

Ver `docs/FEATURE_TIERS.md` para los comandos de compilacion por feature tier.

---

## Feature tiers

```
default (storage-sled)
  └─ core (+ analytics + ml + algorithms + hypergraph + embeddings)
       └─ semantic (+ reasoner + owl-import + shacl)
            └─ full (conjunto publico completo)
```

Tiers son aditivos. Compilar con `--features semantic` incluye todo lo de `core`. Los bindings Python se construyen aparte con `maturin` usando `--features python-full`.

---

## Decisiones de diseno no obvias

| Decision | Razon |
|----------|-------|
| `BoxFuture` en `NodeStream` en lugar de `async fn` | `async fn` en traits no es object-safe en Rust stable; sin esto no se puede hacer `Box<dyn NodeStream>` |
| `NodeKind` con `#[derive(Default)]` | el valor default (`Individual`) se usa en los constructores `Node::new()` y `Node::with_id()` para que el campo no sea obligatorio al crear nodos de datos comunes |
| `regex` solo en feature `shacl` | evitar que el crate `regex` (pesado) contamine builds que no usan SHACL |
| `ELReasoner` y `ShaclValidator` standalone | no modifican el grafo; pueden correr en paralelo con lecturas; resultado deterministico y reproducible |
| MessagePack para nodes/edges | mas compacto que JSON, mas flexible que bincode para esquemas evolutivos |
| Bincode solo para metadatos de indices | los indices se reconstruyen desde los datos; solo el schema del indice necesita persistencia rapida |
| Dual storage para aristas (`"edges"` + `"versioned_edges"`) | `"edges"` mantiene lookup O(1) del estado actual sin romper backward compat; `"versioned_edges"` almacena el historial MVCC. Transacciones usan `add_edge_at(ts)` para coherencia con `commit_timestamp` |

---

## Invariantes criticos

1. **No `.unwrap()` en codigo de produccion** — un panic durante escritura en WAL corrompe la base de datos.
2. **No bloquear el runtime de Tokio** — usar `tokio::task::spawn_blocking` para trabajo CPU-intensivo.
3. **Usar `NopalError` en APIs publicas** — no `anyhow` en `pub fn` que retornen `Result`.
4. **No cambiar el formato de serializacion** — romperia bases de datos existentes.
5. **`cargo clippy -- -D warnings` debe pasar sin warnings** antes de cualquier commit.
