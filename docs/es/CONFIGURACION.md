# Guía de Configuración de NopalDB

Actualmente, NopalDB está diseñado para ofrecer una experiencia **"zero-configuration"**. Provee valores predeterminados optimizados para cargas de trabajo generales, balanceando el rendimiento de escritura y la latencia de lectura.

---

## Configuración Predeterminada

Cuando inicializas un grafo con `Graph.open("ruta")`, se aplica automáticamente la siguiente configuración:

### 💾 Motor de Almacenamiento (redb)
- **Backend**: redb, un B-tree embebido copy-on-write (el único motor desde 0.8.0).
- **Caché**: la caché de páginas de redb, según el perfil (`default` 1 GiB, `mobile` 16 MiB, `server` 256 MiB).
- **Compresión**: ninguna.
- **Durabilidad**: cada escritura pasa antes por el WAL de NopalDB; el motor confirma en ventanas cortas y un checkpoint las hace durables (ver [DURABILITY.md](../DURABILITY.md)).

### 🔄 Concurrencia (MVCC)
- **Nivel de Aislamiento**: Read Committed por defecto. Se elige otro por transacción:
  `graph.begin_transaction("read_uncommitted" | "read_committed" | "repeatable_read" | "serializable")`
  (otro texto es `ValueError`). Los locks que toma una transacción `serializable`
  se liberan en el commit y, desde 0.9.4 (#296), también en el rollback y al
  descartarse el objeto de la transacción sin ninguno de los dos.
- **Lecturas**: las consultas leen versiones MVCC y no toman locks de escritura.
  Las excepciones son las lecturas consistentes del grafo entero (`export_graph`,
  `nopaldb.networkx.to_networkx`, `to_arrow_complete(label)`), que hacen esperar
  a los escritores mientras leen.
- **Timestamping**: Timestamps monotónicos de 64 bits.
- **Garbage Collection** de versiones viejas: solo desde Rust (`graph.gc(...)` a
  demanda o `start_auto_gc(AutoGcConfig)` en segundo plano; ver
  `docs/ARCHITECTURE.md` § GC). Python no tiene API de GC; `get_stats()["gc"]`
  informa su estado.

### 🪵 Durabilidad (WAL)
- **Write-Ahead Log**: Habilitado. Todas las transacciones se escriben en el WAL antes de confirmarse.
- **Checkpoint**: automático cuando el WAL pasa de 16 MiB
  (`get_stats()["wal"]["checkpoint_threshold_bytes"]`) y en `close()`;
  `graph.checkpoint()` fuerza uno.
- **Recuperación**: Recuperación automática ante fallos al reiniciar.

---

## Optimizando el Rendimiento

Aunque no hay archivos de configuración (`nopal.conf`), puedes optimizar el rendimiento mediante tus patrones de uso:

### 1. Escrituras en Lote (Batch Writes)
Agrupa múltiples operaciones en una sola transacción para reducir la sobrecarga de sincronización a disco.

```python
# ✅ RÁPIDO: Una transacción, múltiples escrituras
tx = graph.begin_transaction()
for i in range(1000):
    tx.add_node("Item", {"id": i})
tx.commit()

# ❌ LENTO: 1000 transacciones
for i in range(1000):
    tx = graph.begin_transaction()
    tx.add_node("Item", {"id": i})
    tx.commit()
```

### 2. Índices
Los índices de adyacencia (quién conecta con quién) se mantienen automáticamente.
Los índices de propiedades son **explícitos**: nada se indexa hasta que lo declaras.

```python
graph.create_index("Item", "sku", "hash")         # igualdad
graph.create_index("Item", "precio", "btree")     # rangos
graph.create_index("Nota", "cuerpo", "fulltext", analyzer={"language": "spanish"})
graph.list_indexes()   # [(nombre, label, propiedad, tipo), ...]
```

### 3. Uso de Memoria
Para importaciones masivas (millones de nodos), usa el bulk loader: acumula
`batch_size` filas y escribe cada lote de una vez, en lugar de mantener una
transacción entera en memoria.

```python
with graph.bulk_loader(10_000) as loader:
    a = loader.add_node("Item", {"sku": "A-1"})
    b = loader.add_node("Item", {"sku": "B-2"})
    loader.add_edge(a, b, "RELACIONADO")
graph.rebuild_indexes()   # los lotes del bulk loader no llenan los índices de usuario
```

---

## Feature Flags de Compilación

NopalDB usa feature tiers para activar funcionalidad según tu caso de uso:

```bash
cargo build -p nopaldb --features core        # analytics + ML + algoritmos
cargo build -p nopaldb --features semantic    # + reasoner OWL-EL + Turtle
cargo build -p nopaldb --features full        # conjunto público completo
```

El wrapper Python no se genera con `cargo build --features full`; usa `make build-wheel` o, desde la raíz del repo, `cd nopaldb && maturin develop --release` (las features, `python-full`, salen de `nopaldb/pyproject.toml`).

Ver **[Guía de Feature Tiers](../FEATURE_TIERS.md)** para la referencia completa.

---

## Perfiles

Lo único que se configura al abrir es el perfil. Fija la caché de páginas de redb:

| perfil | caché de páginas de redb |
|---|---|
| `"default"` (el que usa `Graph.open`) | 1 GiB |
| `"mobile"` | 16 MiB |
| `"server"` | 256 MiB |

Otro nombre es `ValueError`.

```python
graph = nopaldb.Graph.open_with_profile("data/mi_grafo.db", "mobile")
graph = nopaldb.Graph.in_memory_with_profile("server")
graph.get_stats()["storage"]["profile"]   # "mobile"
```

`Graph.open_with_options(path, engine="auto", profile="default", on_progress=None)`
acepta los mismos perfiles. Los pools de hilos, el umbral de checkpoint del WAL
y la compresión no se configuran desde Python.
