# Deadlock Detection en NopalDB

## ¿Qué es un Deadlock?

Un **deadlock** ocurre cuando dos o más transacciones se bloquean mutuamente esperando recursos que la otra posee.

### Ejemplo clásico:
```
Tx1: Tiene lock en Alice, necesita lock en Bob
Tx2: Tiene lock en Bob, necesita lock en Alice

Resultado: Ambas esperan indefinidamente (DEADLOCK)
```

---

## Algoritmo: Wait-for Graph

NopalDB usa un **Wait-for Graph** para detectar deadlocks:

### Estructura:
```
Nodo = Transacción
Arista (Tx1 → Tx2) = "Tx1 espera a Tx2"
```

### Detección:
- Se ejecuta **DFS (Depth-First Search)** para detectar ciclos
- Si hay ciclo → **Deadlock detectado**

### Ejemplo:
```
Tx1 → Tx2 → Tx3 → Tx1
  ↑________________|

CICLO DETECTADO = DEADLOCK
```

---

## Victim Selection

Cuando se detecta un deadlock, NopalDB **aborta una transacción** (la "víctima"):

**Estrategia**: Abortar la transacción **más reciente** (mayor Transaction ID)

**Razón**: Menos trabajo perdido
```rust
// Tx1 (ID=100) vs Tx2 (ID=200)
// Deadlock detectado
// Victim = Tx2 (ID más alto)
// Tx2 es abortada, Tx1 continúa
```

---

## Waiting Mechanism

NopalDB implementa **waiting inteligente**:

1. Transacción intenta adquirir lock
2. Si lock está ocupado → **Esperar** (con timeout)
3. Cada 500ms: Verificar deadlock
4. Si deadlock → Abortar víctima
5. Cuando lock se libera → **Wake-up** automático
```rust
// Tx1 tiene lock en Alice
let mut tx2 = graph
    .begin_transaction()
    .await?
    .with_isolation(IsolationLevel::Serializable);

// Tx2 espera automáticamente
tx2.add_node(alice).await?;  // Espera hasta que Tx1 libere (o 5 s → error)

// (en otra task) Tx1 hace commit
tx1.commit().await?;  // ← Despierta a Tx2

// Tx2 continúa inmediatamente
```

El deadlock y el timeout se devuelven **en la operación que pide el lock**
(`get_node`, `add_node`, o `commit()` para los nodos borrados), no solo en
`commit()`.

---

## Configuración

### Feature Flag

Deadlock detection requiere la feature **`full-isolation`**:
```toml
[dependencies]
nopaldb = { version = "0.9", features = ["full-isolation"] }
```

### Timeout

**5 segundos, fijo.** `LockManager::with_timeout` existe, pero cada `Graph`
crea su propio `LockManager` y no hay API en `Graph` para cambiarlo. Al
vencer, la operación devuelve `NopalError::ConcurrencyError("Lock timeout: …")`.

---

## Isolation Levels y Locks

| Nivel | Locks | Deadlock Detection |
|-------|-------|-------------------|
| ReadUncommitted | No | No aplica |
| ReadCommitted | No | No aplica |
| RepeatableRead | No* | No aplica |
| **Serializable** | **Sí** (por nodo; las aristas no se bloquean) | **Sí** |

*RepeatableRead usa snapshots, no locks tradicionales

Los locks se liberan en `commit()` (también si falla), `rollback()`,
`rollback_async()` y al hacer drop de la transacción sin commit (desde 0.9.4,
#296).

---

## Ejemplos de Uso

### Ejemplo 1: Transferencia bancaria sin deadlock
```rust
use nopaldb::{Graph, IsolationLevel, Node};

let graph = Graph::in_memory().await?;

// Setup cuentas
let mut setup = graph.begin_transaction().await?;
let alice_id = setup.add_node(Node::new("Account")).await?;
let bob_id = setup.add_node(Node::new("Account")).await?;
setup.commit().await?;

// Transferencia con orden consistente
let mut tx = graph
    .begin_transaction()
    .await?
    .with_isolation(IsolationLevel::Serializable);

// Siempre adquirir locks en el MISMO orden
let (first, second) = if alice_id < bob_id { (alice_id, bob_id) } else { (bob_id, alice_id) };

// Leer y volver a escribir con el mismo id (no hay `update_node`)
let a = tx.get_node(first).await?;
let b = tx.get_node(second).await?;
tx.add_node(a).await?;
tx.add_node(b).await?;

tx.commit().await?;
```

### Ejemplo 2: Manejo de deadlock
El deadlock puede salir de cualquier operación de la transacción, así que el
cuerpo va en una función que devuelve `Result` y el reintento mira su
resultado (con `?` dentro del `loop`, el primer `Deadlock` saldría del bucle):

```rust
use std::time::Duration;
use nopaldb::{Graph, IsolationLevel, Node, NopalError, Result};

async fn transfer(graph: &Graph, alice: Node, bob: Node) -> Result<()> {
    let mut tx = graph
        .begin_transaction()
        .await?
        .with_isolation(IsolationLevel::Serializable);

    // Lógica de negocio
    tx.add_node(alice).await?;
    tx.add_node(bob).await?;
    tx.commit().await
    // si algo falla, `tx` se suelta aquí y libera sus locks (0.9.4)
}

let mut backoff = Duration::from_millis(50);
loop {
    match transfer(&graph, alice.clone(), bob.clone()).await {
        Ok(()) => break, // ✅ Éxito
        Err(NopalError::Deadlock(_)) | Err(NopalError::TransactionConflict(_)) => {
            // ⚠️ Deadlock o conflicto: reintentar con backoff
            tokio::time::sleep(backoff).await;
            backoff *= 2;
        }
        Err(e) => return Err(e), // ❌ Otro error
    }
}
```

---

## Performance

### Overhead

- **Sin deadlock**: <5% overhead vs sin locks
- **Con deadlock**: Detección en <10ms

### Timeouts

- **Lock acquisition**: 5 segundos (fijo)
- **Deadlock check**: Cada 500ms mientras espera

### Escalabilidad

Probado con:
- ✅ 100 transacciones concurrentes
- ✅ Grafos de hasta 10,000 nodos
- ✅ Deadlocks complejos (3+ transacciones)

---

## Troubleshooting

### "Lock timeout: tx waiting for node"

**Causa**: Transacción esperó más de 5 segundos (`NopalError::ConcurrencyError`)

**Solución**:
1. Reducir duración de transacciones (el timeout no es configurable desde `Graph`)
2. Optimizar lógica de negocio
3. Antes de 0.9.4, una transacción `Serializable` soltada sin `commit()` también retenía sus locks hasta este timeout (#296): actualizar, o terminarla siempre con `commit()` / `rollback_async()`

### "Deadlock detected: tx aborted (victim)"

**Causa**: Deadlock real detectado

**Solución**:
1. Implementar retry con backoff exponencial
2. Usar orden consistente de locks
3. Reducir scope de transacciones

### Performance degradation

**Causa**: Muchos deadlocks/retries

**Solución**:
1. Usar ReadCommitted si no necesitas Serializable
2. Implementar lock ordering
3. Reducir contención (menos transacciones concurrentes)

---

## Comparación con Otras Bases de Datos

| Database | Algoritmo | Victim Selection |
|----------|-----------|-----------------|
| **NopalDB** | Wait-for Graph + DFS | Youngest tx |
| PostgreSQL | Wait-for Graph | Youngest tx |
| MySQL InnoDB | Wait-for Graph | Fewest locks |
| SQLite | Timeout | N/A (no locks) |

---

## Referencias

### Papers
- **"Concurrency Control and Recovery in Database Systems"** - Bernstein et al. (1987)
- **"A Method for Deadlock Detection in Database Systems"** - Chandy & Misra (1982)

### Implementaciones
- [PostgreSQL Deadlock Detection](https://www.postgresql.org/docs/current/explicit-locking.html#LOCKING-DEADLOCKS)
- [InnoDB Deadlock Detection](https://dev.mysql.com/doc/refman/8.0/en/innodb-deadlock-detection.html)

---

## Ver También

- [ISOLATION_LEVELS.md](./ISOLATION_LEVELS.md) - Niveles de aislamiento
- [DURABILITY.md](./DURABILITY.md) - Qué sobrevive a un crash: transacciones vs escrituras directas