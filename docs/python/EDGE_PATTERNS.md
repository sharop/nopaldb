# Patrones de Aristas (Edge Patterns) 🔗

Guía completa para trabajar con relaciones en NopalDB.

---

## 🎯 Sintaxis de Edge Patterns

### Básico
```nql
(a)->(b)                    # Cualquier relación
(a)-->(b)                   # Bidireccional
(a)<-(b)                    # Relación entrante
```

### Con Variable
```nql
(a)-[r]->(b)                # Variable 'r' para la arista
```

### Con Tipo
```nql
(a)-[:KNOWS]->(b)           # Solo relaciones KNOWS
(a)-[:WORKS_AT|LIVES_IN]->(b)  # Múltiples tipos (futuro)
```

### Con Variable y Tipo
```nql
(a)-[r:KNOWS]->(b)          # Variable + tipo
```

### Sin Variables (Auto-generadas) ✨ NUEVO
```nql
()-[r:KNOWS]->()            # Variables _source, _target
()->(b)                     # Solo _target
(a)->()                     # Solo _source
```

---

## 📊 Propiedades de Aristas

### Crear Edges con Propiedades

```python
tx = graph.begin_transaction()

tx.add_edge(alice_id, bob_id, "KNOWS", {
    "since": 2019,
    "strength": "strong",
    "context": "work",
    "trust_score": 0.95
})

tx.commit()
```

### Consultar Propiedades

```nql
-- Proyección simple
find r.since, r.strength
from (a:Person)-[r:KNOWS]->(b:Person)

-- Con nombres de nodos
find a.name, r.since, b.name
from (a:Person)-[r:KNOWS]->(b:Person)

-- Wildcard (incluye propiedades de edges)
find *
from (a:Person)-[r:KNOWS]->(b:Person)
```

### Propiedad Especial: `r.type`

```nql
find r.type
from (a)-[r]->(b)
```

Retorna el tipo de la relación (ej: "KNOWS", "WORKS_AT")

---

## 🔍 Filtros en Edge Properties

### Filtros Simples
```nql
find a.name, b.name
from (a:Person)-[r:KNOWS]->(b:Person)
where r.since > 2020
```

### Filtros Múltiples
```nql
find a.name, r.strength, b.name
from (a:Person)-[r:KNOWS]->(b:Person)
where r.since >= 2018 
  and r.strength in ['strong', 'very strong']
  and r.trust_score > 0.8
```

---

## 💡 Ejemplos Prácticos

### Red Social
```python
# Crear red
tx = graph.begin_transaction()

users = {}
for name in ["Alice", "Bob", "Charlie"]:
    users[name] = tx.add_node("User", {"name": name})

# Follows con contexto
tx.add_edge(users["Alice"], users["Bob"], "FOLLOWS", {
    "since": 2020,
    "notificationsActive": True
})

tx.commit()

# Query
result = graph.execute_nql("""
    find a.name, r.since, b.name
    from (a:User)-[r:FOLLOWS]->(b:User)
    where r.notificationsActive = true
""")
```

### Colaboración Laboral
```python
# Modelo
tx.add_edge(person_id, company_id, "WORKS_AT", {
    "since": 2021,
    "position": "Engineer",
    "salary": 120000,
    "remote": True
})

# Análisis
result = graph.execute_nql("""
    find p.name, r.position, r.salary, c.name
    from (p:Person)-[r:WORKS_AT]->(c:Company)
    where r.remote = true and r.salary > 100000
    order by r.salary desc
""")
```

### Knowledge Graph
```python
# Relaciones semánticas
tx.add_edge(entity1, entity2, "RELATED_TO", {
    "confidence": 0.92,
    "source": "ML_model",
    "evidence": ["doc1", "doc2"]
})

# Consulta confiable
result = graph.execute_nql("""
    find e1.name, r.confidence, e2.name
    from (e1:Entity)-[r:RELATED_TO]->(e2:Entity)
    where r.confidence > 0.9
    order by r.confidence desc
    limit 20
""")
```

---

## 🎓 Patrones Avanzados

### Agregaciones por Edge Properties
```nql
find r.strength, count(*) as total
from (a:Person)-[r:KNOWS]->(b:Person)
group by r.strength
```

### Análisis Temporal
```nql
-- Conexiones por año
find year(r.since) as año, count(*) as conexiones
from ()-[r:KNOWS]->()
group by año
order by año desc
```

### Score Promedio
```nql
find a.name, avg(r.trust_score) as confianza_promedio
from (a:Person)-[r:KNOWS]->()
group by a.name
having avg(r.trust_score) > 0.8
```

---

## ⚡ Performance

### Indexación Automática
- Los tipos de edge se indexan automáticamente
- Las propiedades de edge NO se indexan (aún)

### Optimizar Queries
```nql
-- Rápido: tipo específico
from (a)-[:KNOWS]->(b)

-- Lento: filtrar después
from (a)-[r]->(b)
where r.type = 'KNOWS'
```

---

## 🔄 Casos de Uso Reales

### Synthetic Offshore Network
```nql
-- Oficiales en múltiples jurisdicciones
find o.name, 
       count(distinct e.jurisdiction) as jurisdicciones,
       avg(r.ownership_percentage) as porcentaje_promedio
from (o:Officer)-[r:OFFICER_OF]->(e:Entity)
where r.ownership_percentage > 0
group by o.name
having count(distinct e.jurisdiction) > 3
order by jurisdicciones desc
```

### Detección de Fraude
```nql
-- Transacciones sospechosas
find t.amount, t.timestamp, a.name, b.name
from (a:Account)-[t:TRANSFER]->(b:Account)
where t.amount > 10000 
  and t.timestamp > '2024-01-01'
  and t.flagged = true
```

---

## 📚 Ver También

- **[NQL Guide](NQL_GUIDE.md)** - Lenguaje completo
- **[API Reference](API_REFERENCE.md)** - Python API
- **[Examples](EXAMPLES.md)** - Más ejemplos

---

**¡Modela relaciones complejas con edge properties!** 🔗
