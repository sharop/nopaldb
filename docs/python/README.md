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
- **[API Reference](API_REFERENCE.md)** - Complete Python API
- **[NQL Guide](NQL_GUIDE.md)** - Query language reference
- **Transaction API** - See [API Reference](API_REFERENCE.md)

### Advanced Topics
- **[Edge Patterns](EDGE_PATTERNS.md)** - Working with relationships
- **[Arrow Export](ARROW_EXPORT.md)** - ML integration with Arrow/Pandas
- **[Schema Inspection](SCHEMA_INSPECTION.md)** - Schema discovery and introspection

---

<a name="español"></a>
## 📚 Documentación en Español

> Nota: La documentación Python vive en una sola carpeta (sin subcarpeta `es/`). Los enlaces apuntan a los archivos actuales.

### Primeros Pasos
- **[Inicio Rápido](QUICKSTART.md)** - Empieza en 5 minutos
- **[Ejemplos](EXAMPLES.md)** - Ejemplos completos de uso

### Documentación Core
- **[Referencia API](API_REFERENCE.md)** - API completo de Python
- **[Guía NQL](NQL_GUIDE.md)** - Referencia del lenguaje de consultas
- **API de Transacciones** - Ver [Referencia API](API_REFERENCE.md)

### Temas Avanzados
- **[Patrones de Aristas](EDGE_PATTERNS.md)** - Trabajar con relaciones
- **[Exportación Arrow](ARROW_EXPORT.md)** - Integración ML con Arrow/Pandas
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

## 📦 What's New in v0.2.0 / Novedades en v0.2.0

**English:**
- ✨ Edge properties support
- ✨ Auto-generated variables in patterns
- ✨ Enhanced wildcard projection
- ✨ Improved Arrow export

**Español:**
- ✨ Soporte de propiedades en aristas
- ✨ Variables auto-generadas en patrones
- ✨ Proyección wildcard mejorada
- ✨ Exportación Arrow mejorada

---

**Version:** 0.2.0
**Last Updated:** January 2026
**License:** MPL-2.0 (the `nopaldb` library; releases ≤ 0.4.31 were AGPL-3.0-only)
