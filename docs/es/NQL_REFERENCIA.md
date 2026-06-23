# Lenguaje de Consultas NQL - Referencia Rápida

**NQL (NopalDB Query Language)** es un lenguaje de consultas para grafos intuitivo y poderoso, diseñado tanto para desarrolladores como para **científicos de datos, analistas y sociólogos**. Su sintaxis de patrones para grafos de propiedades optimiza la búsqueda de patrones y conexiones complejas en tus datos.

---

## Tabla de Contenidos

- [Conceptos Básicos](#conceptos-básicos)
- [Sintaxis General](#sintaxis-general)
- [Búsqueda de Patrones (FROM)](#búsqueda-de-patrones-from)
- [Selección de Datos (FIND)](#selección-de-datos-find)
- [Filtrado (WHERE)](#filtrado-where)
- [Operaciones de Escritura (ADD/UPDATE/DELETE)](#operaciones-de-escritura-addupdatedelete)
- [Agregaciones y Funciones](#agregaciones-y-funciones)
- [Exportación de Datos (EXPORT)](#exportación-de-datos-export)
- [Operadores](#operadores)
- [Ejemplos por Disciplina](#ejemplos-por-disciplina)

---

## Conceptos Básicos

- **Nodos**: Las entidades de tus datos (Personas, Ciudades, Transacciones). Se escriben como `(n:Persona)`.
- **Relaciones**: Cómo se conectan los nodos. Se representan con flechas `->`.
- **Propiedades**: Detalles de nodos o relaciones, como `{nombre: "Ana", edad: 30}`.

---

## Sintaxis General

La estructura base de una consulta NQL es sencilla:

```nql
FIND <qué_quieres_ver>
FROM <patrón_de_conexiones>
WHERE <condiciones_filtro>
LIMIT <número_resultados>
```

**Ejemplo simple:**
```nql
find p.nombre, p.edad
from (p:Persona)
where p.edad > 25
limit 10
```

---

## Búsqueda de Patrones (FROM)

La cláusula `FROM` define el "dibujo" o patrón que buscas en el grafo.

### Patrones de Nodos
- `(p:Persona)`: Busca nodos con la etiqueta "Persona" y llámalos `p`.
- `(n)`: Busca cualquier nodo.
- `(p:Persona {ciudad: "CDMX"})`: **¡Nuevo!** Busca nodos Persona que tengan la propiedad `ciudad` igual a "CDMX".

### Patrones de Relaciones
Las relaciones conectan nodos. La dirección de la flecha importa.

- `->`: Relación saliente (cualquier tipo).
- `<-`: Relación entrante (cualquier tipo).
- `<->`: Relación bidireccional (cualquier dirección).
- `-[:AMIGO]->`: Relación saliente de tipo específico "AMIGO".
- `<-[:COMPRA]-`: Relación entrante de tipo "COMPRA".

### Ejemplos de Conexiones
```nql
-- A conoce a B
from (a:Persona) -> [:CONOCE] -> (b:Persona)

-- Cadena de influencia: A influye en B, B influye en C
from (a:Persona) -> [:INFLUYE] -> (b:Persona) -> [:INFLUYE] -> (c:Persona)

-- Colaboración mutua
from (autor1:Investigador) <-> [:COLABORA] <-> (autor2:Investigador)
```

---

## Selección de Datos (FIND)

Especifica qué información recuperar de los patrones encontrados.

### Sintaxis
- `find p.nombre`: Devuelve la propiedad `nombre` del nodo `p`.
- `find *`: Devuelve **todas** las propiedades de los elementos encontrados.
- `find count(*)`: Cuenta cuántos resultados coinciden.

### Ejemplos
```nql
find p.nombre, p.email
from (p:Cliente)
```

---

## Filtrado (WHERE)

Refina tu búsqueda con condiciones lógicas.

### Operadores
- **Comparación**: `=`, `!=`, `<`, `>`, `<=`, `>=`
- **Lógico**: `and`, `or`, `not`

### Ejemplos
```nql
-- Clientes de alto valor
where c.compras_totales > 50000

-- Segmentación geográfica y demográfica
where (p.ciudad = "CDMX" or p.ciudad = "GDL") and p.edad < 30

-- Exclusión
where not p.estado = "Inactivo"
```

---

## Path Metadata y PROFILE (F2)

`F2` agrega metadata sobre el path completo del match lineal:

- `path.depth`
- `path.nodes`
- `path.edges`

Ejemplo:

```nql
find b.nombre, path.depth, path.nodes
from (a:Persona {nombre: "Ana"})-[:CONOCE]->{1,2}(b:Persona)
where path.depth >= 1
order by path.depth desc
```

Reglas:
- `path.depth` funciona en `FIND`, `WHERE` y `ORDER BY`
- `path.nodes` y `path.edges` funcionan solo en `FIND`
- `path.*` requiere un solo patrón lineal con al menos una relación

`PROFILE` también está disponible vía `execute_statement()`:

```nql
profile
find b.nombre, path.depth
from (a:Persona {nombre: "Ana"})-[:CONOCE]->{1,2}(b:Persona)
```

---

## Operaciones de Escritura (ADD/UPDATE/DELETE)

NQL ya soporta CRUD sobre el **contenido** del grafo:

- `ADD`: crea nodos y relaciones
- `UPDATE`: actualiza propiedades de nodos y aristas
- `DELETE`: borra nodos o relaciones matched
- `CREATE INDEX` / `DROP INDEX`: administración de índices

Importante:
- NQL **no** crea el storage o base en sí. Eso se hace vía API con `Graph.open(...)` o `Graph.in_memory()`.
- NQL **no** soporta todavía `CREATE GRAPH`, `BEGIN`, `COMMIT`, `ROLLBACK` ni `MERGE`.

### ADD
```nql
add (a:Person {name: "Alice"})-[:KNOWS {since: 2020, strength: "high"}]->(b:Person {name: "Bob"})
```

### UPDATE
```nql
update (a:Person)-[r:KNOWS]->(b:Person)
set r.since = 2024, b.city = "CDMX"
where a.name = "Alice" and b.name = "Bob"
```

### DELETE
```nql
-- Borra solo la relación matched
delete (a:Person)-[:KNOWS]->(b:Person)
where a.name = "Alice" and b.name = "Bob"

-- Borra nodos (y sus aristas incidentes)
delete (p:Person)
where p.name = "Bob"
```

Para un estado detallado y ejemplos completos:
- `docs/NQL_WRITE_CRUD_STATUS.md`
- `docs/NQL_WRITE_CRUD_HANDS_ON.md`

---

## Agregaciones y Funciones

NQL soporta funciones para resumir datos, ideales para análisis estadístico.

- `count(*)`: Cuenta el total de coincidencias.
- `sum(p.monto)`: Suma los valores de una propiedad numérica.
- `avg(p.edad)`: Calcula el promedio.
- `min(p.score)`, `max(p.score)`: Encuentra valores mínimos y máximos.
- `degree(n)`, `pagerank(n)`, `betweenness(n)`, `clustering(n)`: Agregaciones de analítica de grafos.
- `community(n)`: Detección exacta global de comunidades (basada en Louvain).
- `community_fast(n)`: Detección aproximada local de comunidades para exploración de baja latencia.
- `shortestPath("uuid-origen", "uuid-destino")`: Distancia más corta entre dos nodos (regresa `-1.0` si no hay camino).

`community(n)` calcula una partición global, por lo que `LIMIT` se aplica después de la agregación y no reduce su costo en primera ejecución.
Las ejecuciones repetidas de `community(n)` reutilizan caché versionado por topología hasta que cambian nodos/aristas.

**Ejemplo de Análisis:**
```nql
find count(*), avg(p.edad)
from (p:Usuario)
where p.activo = true
```

```nql
-- Exploración (rápido/aproximado)
find community_fast(n) as cluster_fast
from (n)
limit 1

-- Resultado final/reporte (exacto)
find community(n) as cluster
from (n)
limit 1
```

---

## Exportación de Datos (EXPORT)

Exporta resultados a CSV o JSON.
La práctica recomendada es poner `export` al final de la consulta.
La ruta es opcional.
Si no se especifica ruta, el resultado se devuelve inline.

### Sintaxis
```nql
find ...
from ...
export csv with path="ruta/al/archivo.csv", header=true, separator=","
```

```nql
find ...
from ...
export json with path="ruta/al/archivo.json", pretty=true
```

```nql
find ...
from ...
export json with jsonl=true
```

```nql
-- No soportado:
export csv with header=true
find ...
```

### Formatos Soportados
- **CSV**: Ruta opcional. Opciones: `header=true|false`, `delimiter=","`.
- **JSON**: Ruta opcional. Opciones: `pretty=true|false`, `jsonl=true|false`.
- **ARROW/PARQUET**: No se exportan vía NQL. Usa la API de Graph (`to_arrow()`, `export_parquet()`).

**Ejemplo (export CSV):**
```nql
find p.nombre, p.edad
from (p:Persona)
order by p.edad
export csv with path="./usuarios.csv", header=true
```

---

## Comentarios

```nql
// Comentario de una línea
/* Comentario en bloque */
```

---

## Tipos de Datos

| Tipo | Ejemplo | Nota |
|------|---------|------|
| String | `"Texto"` o `'Texto'` | Usa comillas dobles o simples. |
| Int | `42` | Números enteros. |
| Float | `3.14` | Decimales. |
| Bool | `true`, `false` | Lógica booleana. |
| Null | `null` | Ausencia de valor. |

---

## Ejemplos por Disciplina

### 🔬 Para Sociólogos: Análisis de Redes
Encontrar líderes de opinión en una comunidad.
```nql
-- ¿Quién es "seguido" por personas que a su vez son muy seguidas?
find lider.nombre
from (lider:Persona) <- [:SIGUE] <- (seguidor:Persona) <- [:SIGUE] <- (fan:Persona)
limit 20
```

### 📊 Para Mercadólogos: Segmentación
Identificar clientes jóvenes que han comprado productos de tecnología.
```nql
find c.nombre, c.email
from (c:Cliente) -> [:COMPRO] -> (p:Producto)
where c.edad < 25 and p.categoria = "Tecnología"
```

### 🛡️ Para Analistas de Seguridad: Detección de Fraude
Detectar cadenas de transacciones sospechosas entre cuentas.
```nql
find a.id, b.id, c.id
from (a:Cuenta) -> [:TRANSFIERE] -> (b:Cuenta) -> [:TRANSFIERE] -> (c:Cuenta)
where a.flag_riesgo = true and c.flag_riesgo = true
```

### 💻 Para Desarrolladores: API Backend
Recuperar perfil de usuario y sus permisos.
```nql
find u.username, r.rol_nombre
from (u:Usuario {id: "12345"}) -> [:TIENE_ROL] -> (r:Rol)
```

---

## ¿Dudas?

- **Integración Python:** Consulta `NQL_TUTORIAL.md`.
- **Github:** [Repositorio NopalDB](https://github.com/sharop/nopaldb)

**NopalDB** - *Grafos para todos.* 🌵
