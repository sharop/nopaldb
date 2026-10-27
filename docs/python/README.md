# NopalDB Python Documentation
# Documentación Python de NopalDB

**[English](#english)** | **[Español](#español)**

---

<a name="english"></a>
## 📚 English Documentation

> Note: Python docs currently live in a single folder (no `en/` subfolder). Links below point to the current files.

### Getting Started
- **[Quick Start](QUICKSTART.md)** - Get started in 5 minutes
- **[Examples](EXAMPLES.md)** - Complete usage examples

### Core Documentation
- **[API Cheat Sheet](API_CHEATSHEET.md)** - Every method on one page, checked against the stub in CI
- **[API Reference](API_REFERENCE.md)** - Core Python API (open, transactions, NQL, Arrow, results)
- **[NQL Guide](NQL_GUIDE.md)** - Query language reference
- **[Configuration](CONFIGURATION.md)** - Defaults, isolation levels, indexes, profiles
- **Transaction API** - See [API Reference](API_REFERENCE.md)

### Advanced Topics
- **[Edge Patterns](EDGE_PATTERNS.md)** - Working with relationships
- **[Arrow Export](ARROW_EXPORT.md)** - ML integration with Arrow/Pandas
- **[NetworkX](NETWORKX.md)** - Convert to and from NetworkX graphs
- **[Schema Inspection](SCHEMA_INSPECTION.md)** - Schema discovery and introspection

---

<a name="español"></a>
## 📚 Documentación en Español

> Nota: La documentación Python vive en una sola carpeta (sin subcarpeta `es/`). Los enlaces apuntan a los archivos actuales.

### Primeros Pasos
- **[Inicio Rápido](QUICKSTART.md)** - Empieza en 5 minutos
- **[Ejemplos](EXAMPLES.md)** - Ejemplos completos de uso

### Documentación Core
- **[Referencia API en español](../es/API_PYTHON.md)** - API de Python resumido
- **[Cheat sheet del API](API_CHEATSHEET.md)** - Todos los métodos en una página (en inglés), comprobado contra el stub en CI
- **[Referencia API](API_REFERENCE.md)** - API core de Python (en inglés)
- **[Guía NQL](NQL_GUIDE.md)** - Referencia del lenguaje de consultas
- **[Configuración](../es/CONFIGURACION.md)** - Valores por defecto, aislamiento, índices, perfiles
- **API de Transacciones** - Ver [Referencia API](API_REFERENCE.md)

### Temas Avanzados
- **[Patrones de Aristas](EDGE_PATTERNS.md)** - Trabajar con relaciones
- **[Exportación Arrow](ARROW_EXPORT.md)** - Integración ML con Arrow/Pandas
- **[NetworkX](NETWORKX.md)** - Convertir desde y hacia NetworkX (en inglés)
- **[Inspección de Esquema](SCHEMA_INSPECTION.md)** - Descubrimiento e introspección

---

## 🚀 Quick Links / Enlaces Rápidos

### Installation / Instalación
```bash
pip install nopaldb
```

### Example / Ejemplo
```python
import nopaldb

graph = nopaldb.Graph.open("my_graph.db")
tx = graph.begin_transaction()

alice_id = tx.add_node("Person", {"name": "Alice", "age": 30})
bob_id = tx.add_node("Person", {"name": "Bob", "age": 25})
tx.add_edge(alice_id, bob_id, "KNOWS", {"since": 2019})

tx.commit()

result = graph.execute_nql("""
    find a.name, r.since, b.name
    from (a:Person)-[r:KNOWS]->(b:Person)
""")
```

---

## 📦 What's New / Novedades

See / Ver [CHANGELOG.md](../../CHANGELOG.md) and the upgrade guide /
y la guía de actualización: [UPGRADING_0.9.md](../UPGRADING_0.9.md) ·
[ACTUALIZAR_0.9.md](../es/ACTUALIZAR_0.9.md).

---

**License:** MPL-2.0 (the `nopaldb` library; releases ≤ 0.4.31 were AGPL-3.0-only)
