# Niveles de Aislamiento en NopalDB

## Introducción

Los niveles de aislamiento definen **qué tan visible es el trabajo de una transacción para otras transacciones concurrentes**.

---

## Los 4 Niveles (de menos a más estricto)

### 1. Read Uncommitted

**Permite**: Dirty reads, non-repeatable reads, phantom reads  
**Performance**: ⚡⚡⚡⚡ (más rápido)  
**Consistencia**: ⭐ (menos consistente)

#### Ejemplo:
```rust
let tx = graph.begin_transaction()
    .await?
    .with_isolation(IsolationLevel::ReadUncommitted);

// Puede ver cambios NO commiteados de otras transacciones
```

#### Cuándo usar:
- Analytics de baja prioridad
- Dashboards en tiempo real (aproximados)
- Queries de solo lectura donde precisión no es crítica

---

### 2. Read Committed (DEFAULT)

**Previene**: Dirty reads  
**Permite**: Non-repeatable reads, phantom reads  
**Performance**: ⚡⚡⚡  
**Consistencia**: ⭐⭐⭐

#### Ejemplo:
```rust
let tx = graph.begin_transaction().await?;
// Default = ReadCommitted

// Solo ve datos commiteados
let balance1 = tx.get_node(account_id).await?;

// Otra tx hace commit...

let balance2 = tx.get_node(account_id).await?;
// balance1 != balance2 (puede cambiar)
```

#### Cuándo usar:
- La mayoría de aplicaciones web
- APIs REST
- CRUD operations estándar

---

### 3. Repeatable Read

**Previene**: Dirty reads, non-repeatable reads  
**Permite**: Phantom reads  
**Performance**: ⚡⚡  
**Consistencia**: ⭐⭐⭐⭐

#### Ejemplo:
```rust
let tx = graph.begin_transaction()
    .await?
    .with_isolation(IsolationLevel::RepeatableRead);

// Ve un snapshot del grafo al inicio de la tx
let balance1 = tx.get_node(account_id).await?;

// Otras tx hacen cambios...

let balance2 = tx.get_node(account_id).await?;
// ✅ balance1 == balance2 (repeatable)
```

#### Cuándo usar:
- Reportes financieros
- Auditorías
- Backups consistentes

---

### 4. Serializable

**Previene**: Dirty reads, non-repeatable reads, phantom reads  
**Performance**: ⚡  
**Consistencia**: ⭐⭐⭐⭐⭐ (máxima)

#### Ejemplo:
```rust
let tx = graph.begin_transaction()
    .await?
    .with_isolation(IsolationLevel::Serializable);

// Detecta conflictos read-write
let balance = tx.get_node(account_id).await?;
tx.update_node(account_id, balance + 100).await?;

tx.commit().await?;  // ← Puede fallar si hay conflicto
```

#### Cuándo usar:
- Transacciones bancarias
- Control de inventario
- Reservas de tickets/asientos
- Cualquier caso donde consistencia > performance

---

## Problemas de Concurrencia

### Dirty Read (Lectura Sucia)
```
Tx1: UPDATE balance = 500 (no committed)
Tx2: SELECT balance → ve 500 ❌
Tx1: ROLLBACK
Tx2: Vio datos que nunca existieron
```

### Non-Repeatable Read (Lectura No Repetible)
```
Tx2: SELECT balance → 1000
Tx1: UPDATE balance = 500 + COMMIT
Tx2: SELECT balance → 500 ❌
Tx2: El mismo dato cambió dentro de la misma tx
```

### Phantom Read (Lectura Fantasma)
```
Tx2: COUNT(*) WHERE age > 18 → 100
Tx1: INSERT person(age=25) + COMMIT
Tx2: COUNT(*) WHERE age > 18 → 101 ❌
Tx2: Apareció un registro "fantasma"
```

---

## Trade-offs

| Aspecto | Read Uncommitted | Read Committed | Repeatable Read | Serializable |
|---------|-----------------|----------------|-----------------|--------------|
| **Throughput** | Muy alto | Alto | Medio | Bajo |
| **Latencia** | Muy baja | Baja | Media | Alta |
| **Consistencia** | Baja | Media | Alta | Muy alta |
| **Conflictos** | Ninguno | Pocos | Algunos | Muchos |
| **Retries** | No | No | Raros | Frecuentes |

---

## Configuración en NopalDB

### Feature Flag Requerida
```toml
# Cargo.toml
[dependencies]
nopaldb = { version = "0.1", features = ["full-isolation"] }
```

### Uso
```rust
use nopaldb::{Graph, IsolationLevel};

let graph = Graph::open("mydb.nopal").await?;

// Default (Read Committed)
let tx1 = graph.begin_transaction().await?;

// Específico
let tx2 = graph.begin_transaction()
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
```rust
log::info!("Transaction {} using {:?}", tx.id, tx.isolation_level);
```

### Detectar conflictos:
```rust
match tx.commit().await {
    Ok(_) => println!("✅ Committed"),
    Err(NopalError::TransactionConflict(msg)) => {
        println!("⚠️ Conflict: {}", msg);
        // Retry con backoff exponencial
    }
    Err(e) => return Err(e),
}
```

---

## FAQ

**Q: ¿Cuál es el default?**  
A: `ReadCommitted` (como PostgreSQL, MySQL)

**Q: ¿Serializable es SIEMPRE correcto?**  
A: Sí, pero puede ser muy lento. Usa solo cuando lo necesites.

**Q: ¿Cómo elijo el nivel?**  
A: Empieza con ReadCommitted. Si ves bugs de concurrencia, sube a RepeatableRead o Serializable.

**Q: ¿ReadUncommitted es peligroso?**  
A: Para datos críticos, sí. Para analytics, está bien.