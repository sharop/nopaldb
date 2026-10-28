# Niveles de Aislamiento en NopalDB

## Introducción

Los niveles de aislamiento definen **qué tan visible es el trabajo de una transacción para otras transacciones concurrentes**.

---

## Lo que hace cada nivel en NopalDB

Los nombres son los de ANSI SQL, pero el comportamiento es el de esta tabla (el
código está en `nopaldb/src/transaction/mod.rs`). Requieren la feature
`full-isolation` (incluida en el tier `full` y en las wheels de PyPI); sin ella
toda transacción es `ReadCommitted` y `with_isolation` no existe.

| Nivel | Lecturas | Al hacer `commit()` |
|-------|----------|---------------------|
| `ReadUncommitted` | hoy **equivale a `ReadCommitted`**: cada transacción tiene su propio write buffer, así que no hay datos sin commit visibles (no hay dirty reads) ni es más rápido | sin validación |
| `ReadCommitted` (default) | cada lectura ve lo último commiteado: dos lecturas del mismo nodo pueden diferir | sin validación: si dos transacciones escriben el mismo nodo, gana la última |
| `RepeatableRead` | snapshot MVCC tomado al inicio de la transacción (también para los scans por etiqueta/propiedad) | **sin detección write-write**: dos transacciones que leen, modifican y escriben el mismo nodo pueden perder una actualización (lost update) |
| `Serializable` | el mismo snapshot, más un lock de lectura por nodo leído y uno de escritura por nodo escrito (2PL vía `LockManager`) y registro de las lecturas por predicado | valida conflictos write-write, read-write y fantasmas; si hay alguno devuelve `NopalError::TransactionConflict` y no escribe nada |

Límites de `Serializable`:

- Los locks son **por nodo**: las aristas no se bloquean.
- Esperar un lock tiene un timeout fijo de 5 s (`NopalError::ConcurrencyError("Lock timeout…")`); un ciclo de espera aborta con `NopalError::Deadlock`. Ver [DEADLOCK_DETECTION.md](DEADLOCK_DETECTION.md).
- Los locks se liberan en `commit()` (también si falla), `rollback()`, `rollback_async()` y al hacer drop de la transacción sin commit (desde 0.9.4, #296; antes, una transacción soltada sin commit retenía sus locks hasta el timeout).

### Ejemplo: Read Committed (default)

```rust
let tx = graph.begin_transaction().await?;   // ReadCommitted

let antes = tx.get_node(account_id).await?;
// ... otra transacción hace commit sobre account_id ...
let despues = tx.get_node(account_id).await?;
// `antes` y `despues` pueden diferir
```

### Ejemplo: Repeatable Read

```rust
use nopaldb::IsolationLevel;

let tx = graph
    .begin_transaction()
    .await?
    .with_isolation(IsolationLevel::RepeatableRead);

let antes = tx.get_node(account_id).await?;
// ... otra transacción hace commit sobre account_id ...
let despues = tx.get_node(account_id).await?;
// misma versión: ambas lecturas vienen del snapshot del inicio
```

Úsalo para lecturas consistentes (reportes, auditorías, exportes). Para
read-modify-write concurrente sobre los mismos nodos usa `Serializable`.

### Ejemplo: Serializable

No hay `update_node`: para modificar un nodo dentro de la transacción se lee,
se cambia y se vuelve a escribir con `add_node` (mismo `id`).

```rust
use nopaldb::{IsolationLevel, NopalError, PropertyValue};

let mut tx = graph
    .begin_transaction()
    .await?
    .with_isolation(IsolationLevel::Serializable);

let mut account = tx.get_node(account_id).await?;          // lock de lectura
let balance = match account.properties.get("balance") {
    Some(PropertyValue::Int(b)) => *b,
    _ => 0,
};
account
    .properties
    .insert("balance".into(), PropertyValue::Int(balance + 100));
tx.add_node(account).await?;                               // lock de escritura

match tx.commit().await {
    Ok(()) => {}
    Err(NopalError::TransactionConflict(msg)) => {
        // otra transacción cambió lo que leímos: reintentar desde el principio
        eprintln!("conflicto: {msg}");
    }
    Err(e) => return Err(e),
}
```

### Desde Python

```python
tx = graph.begin_transaction(isolation="serializable")
# también: "read_uncommitted" | "read_committed" | "repeatable_read"; None = read_committed
tx.add_node("Account", {"balance": 100})
tx.commit()
```

La transacción de Python expone `add_node`, `add_edge`, `commit` y `rollback`.

---

## Problemas de Concurrencia

### Dirty Read (Lectura Sucia)
```
Tx1: UPDATE balance = 500 (no committed)
Tx2: SELECT balance → ve 500 ❌
Tx1: ROLLBACK
Tx2: Vio datos que nunca existieron
```
No ocurre en NopalDB en ningún nivel.

### Non-Repeatable Read (Lectura No Repetible)
```
Tx2: SELECT balance → 1000
Tx1: UPDATE balance = 500 + COMMIT
Tx2: SELECT balance → 500 ❌
Tx2: El mismo dato cambió dentro de la misma tx
```
Ocurre con `ReadUncommitted`/`ReadCommitted`.

### Lost Update (Actualización Perdida)
```
Tx1: lee balance → 1000        Tx2: lee balance → 1000
Tx1: escribe 1100 + COMMIT
                               Tx2: escribe 900 + COMMIT  ← se pierde el +100
```
Ocurre con `ReadUncommitted`, `ReadCommitted` y `RepeatableRead`; `Serializable` la detecta.

### Phantom Read (Lectura Fantasma)
```
Tx2: COUNT(*) WHERE age > 18 → 100
Tx1: INSERT person(age=25) + COMMIT
Tx2: COUNT(*) WHERE age > 18 → 101 ❌
Tx2: Apareció un registro "fantasma"
```
`RepeatableRead` y `Serializable` leen del snapshot; `Serializable` además
rechaza el commit si el conjunto leído cambió.

---

## Configuración en NopalDB

### Feature Flag Requerida
```toml
# Cargo.toml
[dependencies]
nopaldb = { version = "0.10", features = ["full-isolation"] }
```

### Uso
```rust
use nopaldb::{Graph, IsolationLevel};

let graph = Graph::open("mydb.nopal").await?;

// Default (Read Committed)
let tx1 = graph.begin_transaction().await?;
tx1.commit().await?;

// Específico
let tx2 = graph
    .begin_transaction()
    .await?
    .with_isolation(IsolationLevel::Serializable);
tx2.commit().await?;
```

---

## Referencias y Bibliografía

### Libros (recomendados):
1. **"Designing Data-Intensive Applications"** - Martin Kleppmann (Capítulo 7)
    - Mejor explicación de isolation levels
    - Ejemplos prácticos

2. **"Database Internals"** - Alex Petrov
    - Implementación de MVCC
    - Algoritmos de detección de conflictos

3. **"Transaction Processing"** - Jim Gray & Andreas Reuter
    - Texto clásico (académico pero completo)

### Papers:
1. **"A Critique of ANSI SQL Isolation Levels"** (1995)
    - Berenson, Bernstein, Gray, et al.
    - Define los niveles formalmente
    - [PDF](https://www.microsoft.com/en-us/research/wp-content/uploads/2016/02/tr-95-51.pdf)

2. **"Generalized Isolation Level Definitions"** (2000)
    - Adya et al.
    - Formalización matemática

### Documentación de DBs reales:
- [PostgreSQL Isolation](https://www.postgresql.org/docs/current/transaction-iso.html)
- [MySQL InnoDB](https://dev.mysql.com/doc/refman/8.0/en/innodb-transaction-isolation-levels.html)
- [SQLite Isolation](https://www.sqlite.org/isolation.html)

### Recursos online:
- [Jepsen: Consistency Models](https://jepsen.io/consistency)
- [Database Isolation Levels Visualized](https://retool.com/blog/isolation-levels-visualized/)

---

## Debugging

### Ver nivel actual:
El nivel no tiene getter público; `commit()` lo registra en el log (nivel
`info`: `Committing transaction <id> (isolation: …)`). El id sí es público:
```rust
log::info!("Transaction {}", tx.id);
```

### Detectar conflictos:
```rust
match tx.commit().await {
    Ok(()) => println!("✅ Committed"),
    Err(NopalError::TransactionConflict(msg)) => {
        println!("⚠️ Conflict: {}", msg);
        // Reintentar la transacción completa (con backoff)
    }
    Err(e) => return Err(e),
}
```

---

## FAQ

**Q: ¿Cuál es el default?**  
A: `ReadCommitted`.

**Q: ¿Serializable es SIEMPRE correcto?**  
A: Para nodos, detecta los conflictos al hacer commit; las aristas no se bloquean. Cuesta locks y reintentos: úsalo donde haya read-modify-write concurrente.

**Q: ¿Cómo elijo el nivel?**  
A: `ReadCommitted` para escrituras sin competencia; `RepeatableRead` para lecturas consistentes; `Serializable` cuando varias transacciones modifican los mismos nodos.

**Q: ¿ReadUncommitted es más rápido o peligroso?**  
A: Ninguna de las dos: hoy se comporta igual que `ReadCommitted`.
