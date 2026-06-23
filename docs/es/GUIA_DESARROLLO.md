# Guía de Arquitectura y Desarrollo de NopalDB 🌵

> "El código es leído muchas más veces de las que es escrito. Escribe para el lector."

Bienvenido a la guía profunda de ingeniería de NopalDB. Este documento no solo te dirá *dónde* está el código, sino *por qué* está escrito así y cómo mantener la excelencia técnica del proyecto.

---

## 🏗️ Filosofía de Arquitectura

NopalDB no es solo una base de datos; es un ejercicio de **seguridad, concurrencia y performance** en Rust.

### Principios Fundamentales
1.  **Safety First**: Nunca permitimos `segfaults` ni data races. Usamos Rust safe code el 99.9% del tiempo.
2.  **Zero-Cost Abstractions**: Usamos iteradores y tipos genéricos que compilan a código máquina optimizado.
3.  **Async I/O**: Todo el I/O (lectura de disco, red) es asíncrono con **Tokio**. Nunca bloqueamos un hilo del thread pool.
4.  **ACID Real**: No "eventual consistency". Si una transacción confirma, los datos están seguros y garantizados.

---

## 🏷️ Feature Tiers (Compilación por Rol)

NopalDB organiza su funcionalidad en **tiers aditivos** mediante feature flags de Cargo:

```
full  ⊃  semantic  ⊃  core  ⊃  default
```

| Tier | Qué incluye | Para quién |
|------|-------------|------------|
| `default` | Solo Sled backend | Build mínimo, CI rápido |
| `core` | + analytics, ML, algoritmos, hipergrafos, full-text | Investigadores, data scientists |
| `semantic` | + reasoner OWL-EL, Turtle import/export | Ingenieros de knowledge graphs |
| `full` | Conjunto público completo | CI completo, desarrollo full-stack |

```bash
# Compilar el tier que necesitas
cargo build -p nopaldb --features core
cargo build -p nopaldb --features semantic
cargo test -p nopaldb --features semantic --lib

# Python es un artefacto separado
make build-wheel
cd nopaldb && maturin develop --release --features python-full
```

`cargo build -p nopaldb --features full` solo compila Rust; el wrapper Python se compila con `maturin`.

> **Referencia completa:** [`docs/FEATURE_TIERS.md`](../FEATURE_TIERS.md)

---

## 🧬 Anatomía del Código (Deep Dive)

### 1. El Grafo (`src/graph`) y el Storage (`src/storage`)
NopalDB desacopla la **topología** del grafo del **almacenamiento** físico.

*   **Abstracción**: `Graph` encapsula `Storage` y opera contra una capa de backend desacoplada (`StorageBackend` trait).
*   **Físico**: Backend por defecto **Sled** (embebido).
    *   Clave: `u64` (NodeId) -> Valor: `bincode(Node)`
*   **Perfiles runtime**: `Default` (predeterminado), `Mobile` (opcional, memoria conservadora), `Server` (cache agresivo).
*   **Indices**: Mantenemos índices secundarios (ej. búsqueda por propiedad) sincronizados atómicamente con los datos.

### 2. Transacciones y MVCC (`src/transaction`, `src/mvcc`)
Este es el corazón crítico. Si esto falla, perdemos datos.

**Modelo de Concurrencia:**
Usamos **Snapshot Isolation** (SI).
*   **Lectores**: Ven el estado de la DB en el instante `Time(T_start)`. Nunca bloquean a escritores.
*   **Escritores**: Crean nuevas versiones de los datos. Bloquean solo si hay conflicto `Write-Write`.

**Estructura Crítica (`Transaction` struct):**
```rust
pub struct Transaction {
    // Write Buffer: Cambios locales, invisibles al resto
    pending_nodes: HashMap<NodeId, Node>,
    
    // Control de Concurrencia
    isolation_level: IsolationLevel,
    read_set: Arc<RwLock<HashSet<NodeId>>>, // Para Serializable
}
```

**Guard de Seguridad (`Drop`):**
Implementamos `Drop` para `Transaction`. Si una Tx sale de scope sin llamar a `commit()`, se ejecuta automáticamente `rollback()`. **Nunca** dejamos transacciones "colgadas".

### 3. El Motor de Queries (NQL)
El parseo es una pipeline de 3 fases:

1.  **Parseo (Pest)**: String -> Pairs (Tokens).
2.  **AST**: Pairs -> `Query` Struct (Tipado fuerte).
3.  **Validación**: Chequeo semántico (¿Existe la variable `n` que usas en el `WHERE`?).

---

## 🛡️ Reglas de Oro para el Desarrollo (Best Practices)

Para mantener la calidad, todo PR debe seguir estas reglas estrictas:

### 1. Manejo de Errores
❌ **PROHIBIDO**: Usar `.unwrap()` o `.expect()` en código de producción (fuera de tests).
✅ **CORRECTO**: Propagar errores con `?` o manejar explícitamente con `match`.

*Razón*: Un `unwrap` es un posible crash del servidor. Inaceptable en una DB.

**Patrón usado**:
```rust
// src/error.rs define NopalError
fn mi_funcion() -> Result<T> {
    let archivo = File::open("db")?; // Propagación automática
    Ok(t)
}
```

### 2. Concurrencia y Async
❌ **PROHIBIDO**: Bloquear un `async fn` con operaciones pesadas (ej. `std::thread::sleep` o cálculos CPU intensivos masivos sin `yield`).
✅ **CORRECTO**: Usar `tokio::time::sleep` y `tokio::task::spawn_blocking` para CPU heavy tasks.

*Razón*: Bloquear un hilo de Tokio detiene cientos de otras conexiones asíncronas.

### 3. Testing
Tenemos 3 niveles de tests. Tu PR debe incluir los necesarios:
1.  **Unit Tests**: En el mismo archivo, módulo `tests`. Prueban funciones aisladas.
2.  **Integration Tests**: En carpeta `tests/`. Prueban la API pública (`Graph::open`, `execute_nql`) como si fueras un usuario.
3.  **Doc Tests**: Ejemplos de código en la documentación. Garantizan que los ejemplos del README no se rompan.

Comando sagrado:
```bash
cargo test
```

### 4. Linting (Clippy)
El CI fallará si tienes warnings.
```bash
cargo clippy -- -D warnings
```
Hazle caso a Clippy. Sabe más Rust que nosotros.

---

## 🎓 Ruta de Implementación: Agregando una Feature

Supongamos que quieres agregar una función matemática `SQRT()` a NQL.

1.  **Diseño (Parser)**:
    *   Modifica `nql.pest` para aceptar `SQRT`.
    *   Actualiza `ast.rs` para incluir `Func::Sqrt`.
2.  **Evaluación**:
    *   Ve a `src/query/nql/executor.rs`.
    *   En el `match function_name`, agrega el caso para `SQRT`.
    *   Maneja el error si el input es negativo (recuerda: no pánico, retorna `NopalError`).
3.  **Test**:
    *   Agrega un test case en `tests/nql_math_tests.rs`: `FIND n WHERE SQRT(n.val) > 4`.

---

## 🚨 Zonas de Peligro (Don't Touch Unless Expert)

Modificar estos archivos requiere aprobación explícita de maintainers senior:
*   `src/wal/mod.rs`: Corrupción aquí significa pérdida de datos irrecuperable.
*   `src/mvcc/watermark.rs`: Bug aquí rompe la consistencia (lecturas sucias o fantasmas).

---

Con esto, tienes las llaves del reino. NopalDB es un sistema complejo, pero modular. Respeta los tipos, confía en el compilador, y ¡bienvenido a bordo! 🚀

---

## 🔄 Motor de Ejecución de Queries: El Modelo Volcano

> Ver tambien: [`docs/ARCHITECTURE.md`](../ARCHITECTURE.md).

### ¿Qué es el Modelo Volcano?

A partir de v0.3.5, el ejecutor de queries de NopalDB usa el **Modelo Volcano** (también llamado *Iterator Model* o *Pull-based Model*). En lugar de cargar todos los nodos en un `Vec<Node>` antes de procesar, cada operador expone un método `next()` y "jala" una fila a la vez.

```
         LIMIT
          ↑ next()
         PROJECT
          ↑ next()
         FILTER
          ↑ next()
          SCAN ← Lee por lotes/cursor desde storage
```

**Beneficio principal**: Reduce el consumo de memoria de O(N) a O(1) para la fase de Scan.

### El trait `NodeStream`

El componente central es el trait `NodeStream` en `src/query/nql/executor/operators.rs`:

```rust
use futures::future::BoxFuture;

pub trait NodeStream: Send + Sync {
    fn next<'a>(&'a mut self) -> BoxFuture<'a, Result<Option<Node>>>;
}
```

> ⚠️ **Regla crítica para desarrolladores**: Si necesitas añadir un nuevo operador streaming, **SIEMPRE** usa `BoxFuture` explícito como tipo de retorno del método `next()`. **NO** uses `async fn` directamente en el trait — causará errores de "object safety" al usar `Box<dyn NodeStream>`.

### Añadir un nuevo operador

```rust
// Ejemplo: Operador LIMIT
pub struct LimitStream {
    input: Box<dyn NodeStream>,
    limit: usize,
    count: usize,
}

impl NodeStream for LimitStream {
    fn next<'a>(&'a mut self) -> BoxFuture<'a, Result<Option<Node>>> {
        Box::pin(async move {
            if self.count >= self.limit { return Ok(None); }
            let node = self.input.next().await?;
            if node.is_some() { self.count += 1; }
            Ok(node)
        })
    }
}
```

### Pipeline en `Executor::execute()`

```
execute_from_stream() → Box<dyn NodeStream>   [ScanNodesStream por lotes]
         ↓
FilterNodesStream                              [Lazy]
         ↓
ProjectNodesStream / ProjectPatternStream      [Lazy]
         ↓
materialización final en QueryResult           [compatibilidad API]
```

> **Estado actual**: el pipeline interno usa scan/filter/project lazy. La API publica mantiene `QueryResult` materializado por compatibilidad.

---

## 💾 Persistencia de Índices

> Ver tambien: [`docs/INDEXING_DOCS.MD`](../INDEXING_DOCS.MD).

A partir de v0.3.5, los índices **sobreviven reinicios**. Usamos la estrategia **Metadata + Rebuild**:

1. Al crear/eliminar un índice → se persiste metadata (`metadata.bin`).
2. Al iniciar la DB → `IndexManager::load_indices()` valida/repara metadata y reconstruye índices en un solo pass por lotes.

**Regla para desarrolladores**: Si modificas `IndexManager`, asegúrate de que **toda** operación que cambie la estructura de índices (crear, eliminar) también actualice metadata persistida y conserve la ruta de reparación al abrir.

```rust
// SIEMPRE llama save_metadata_internal después de mutar índices
self.save_metadata_internal(&index_name)?;
```

---

## 🗺️ Mapa de Componentes Actualizado

```
nopaldb/src/
├── graph/mod.rs          # Punto de entrada público. Graph::open, execute_nql
├── types.rs              # Node, Edge, NodeKind, PropertyValue, EdgeTarget
├── storage/              # KV embebido + capa pluggable basada en Sled
├── index/                # IndexManager: Hash, B-Tree, Full-Text (Tantivy), Taxonomy
├── transaction/          # Transaction + MVCC + Snapshot Isolation
├── mvcc/                 # Watermark, versioning, GC horizon
├── lock_manager/         # Detección de deadlocks
├── wal/                  # Write-Ahead Log (durabilidad)
├── query/
│   └── nql/
│       ├── parser/       # Pest grammar → AST
│       ├── planner/      # QueryPlanner: decide si usar índice o full scan
│       └── executor/
│           ├── mod.rs    # Executor: orquesta el pipeline Volcano
│           ├── operators.rs  # NodeStream trait + Scan/Filter/Limit streaming
│           ├── result.rs     # QueryResult, Row
│           ├── aggregations.rs  # count/sum/avg/min/max + pagerank/community/shortestPath
│           ├── write.rs
│           └── export.rs     # CSV/JSON/Arrow/Parquet
├── algorithms/           # PageRank, centrality, clustering, community, shortest path
├── arrow_export/         # Zero-copy Arrow IPC export
├── ml/                   # ML: Arrow tensors, PyG data extraction
├── schema/               # Runtime schema inference
├── reasoner/             # OWL-EL reasoner CR1+CR2+CR3 (feature: reasoner)
├── rdf_owl/              # Turtle importer/exporter (feature: owl-import)
└── python/               # Bindings PyO3 (feature: python)
```

---

## 🚨 Zonas de Peligro (Don't Touch Unless Expert)

Modificar estos archivos requiere aprobación explícita de maintainers senior:
*   `src/wal/mod.rs`: Corrupción aquí significa pérdida de datos irrecuperable.
*   `src/mvcc/watermark.rs`: Bug aquí rompe la consistencia (lecturas sucias o fantasmas).
*   `src/query/nql/executor/mod.rs`: Cambios aquí pueden silenciosamente cambiar semántica de queries. **Requiere tests de integración completos**.
*   `src/index/mod.rs`: Si la metadata de índices se desincroniza de los datos reales, las queries devolverán resultados incorrectos sin error.

---

## 📚 Referencias Esenciales

Para profundizar en los conceptos de NopalDB:

- [Modelo Volcano — Graefe 1994 (IEEE)](https://ieeexplore.ieee.org/document/273032)
- [CMU 15-445 Database Systems (gratuito)](https://15445.courses.cs.cmu.edu/)
- [Rust Object Safety Reference](https://doc.rust-lang.org/reference/items/traits.html#object-safety)
- [docs.rs/sled — Motor KV embebido](https://docs.rs/sled/latest/sled/)
