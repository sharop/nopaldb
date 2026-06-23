# Guía de Configuración de NopalDB

Actualmente, NopalDB está diseñado para ofrecer una experiencia **"zero-configuration"**. Provee valores predeterminados optimizados para cargas de trabajo generales, balanceando el rendimiento de escritura y la latencia de lectura.

---

## Configuración Predeterminada

Cuando inicializas un grafo con `Graph.open("ruta")`, se aplica automáticamente la siguiente configuración:

### 💾 Motor de Almacenamiento (Sled)
- **Backend**: Árbol LSM (Hybrid Log-Structured Merge Tree).
- **Caché**: Gestión automática de caché de páginas. Sin límite manual configurado por ahora.
- **Compresión**: habilitada por defecto (Zstd/Snappy según la compilación).
- **Escritura**: Flushing asíncrono a disco para alto rendimiento.

### 🔄 Concurrencia (MVCC)
- **Nivel de Aislamiento**: Snapshot Isolation. Los lectores nunca bloquean a los escritores, y viceversa.
- **Timestamping**: Timestamps monotónicos de 64 bits.
- **Garbage Collection**: Manual por ahora vía APIs internas (auto-vacuuming planeado para futuras versiones).

### 🪵 Durabilidad (WAL)
- **Write-Ahead Log**: Habilitado. Todas las transacciones se escriben en el WAL antes de confirmarse.
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
Los índices de propiedades se crean automáticamente para todas las propiedades añadidas.

### 3. Uso de Memoria
Para importaciones masivas (millones de nodos), considera usar la **API Bulk Loader** (si está disponible en tu versión) o asegúrate de tener suficiente RAM, ya que los buffers de transacción residen en memoria antes del commit.

---

## Feature Flags de Compilación

NopalDB usa feature tiers para activar funcionalidad según tu caso de uso:

```bash
cargo build -p nopaldb --features core        # analytics + ML + algoritmos
cargo build -p nopaldb --features semantic    # + reasoner OWL-EL + Turtle
cargo build -p nopaldb --features full        # conjunto público completo
```

El wrapper Python no se genera con `cargo build --features full`; usa `make build-wheel` o `maturin develop --release --features python-full`.

Ver **[Guía de Feature Tiers](../FEATURE_TIERS.md)** para la referencia completa.

---

## Configuración Futura

Próximas versiones introducirán un objeto `Config` para ajustar:
- Límites de tamaño de caché
- Frecuencia de checkpoints del WAL
- Tamaño del pool de hilos
- Niveles de compresión
