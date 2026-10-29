# Lenguaje de Consultas NQL - Referencia Rápida

**NQL (NopalDB Query Language)** es un lenguaje de consultas para grafos intuitivo y poderoso, diseñado tanto para desarrolladores como para **científicos de datos, analistas y sociólogos**. Su sintaxis inspirada en Cypher optimiza la búsqueda de patrones y conexiones complejas en tus datos.

> Qué **significa** una consulta (filas por camino, qué filas ordena una búsqueda, orden de evaluación, qué es exacto) está en [Semántica de NQL](SEMANTICA_NQL.md). Esta página es la sintaxis.

---

## Tabla de Contenidos

- [Conceptos Básicos](#conceptos-básicos)
- [Sintaxis General](#sintaxis-general)
- [Búsqueda de Patrones (FROM)](#búsqueda-de-patrones-from)
- [Selección de Datos (FIND)](#selección-de-datos-find)
- [Filtrado (WHERE)](#filtrado-where)
- [Predicados de Ontología](#predicados-de-ontología-instanceof--subclassof)
- [Operaciones de Escritura (ADD/UPDATE/DELETE)](#operaciones-de-escritura-addupdatedelete)
- [Índices (CREATE INDEX / DROP INDEX)](#índices-create-index--drop-index)
- [Búsqueda vectorial e híbrida en WHERE](#búsqueda-vectorial-e-híbrida-en-where)
- [Agregaciones y Funciones](#agregaciones-y-funciones)
- [Exportación de Datos (EXPORT)](#exportación-de-datos-export)
- [Viaje en el tiempo (AT TIMESTAMP)](#viaje-en-el-tiempo-at-timestamp)
- [Operadores](#operadores)
- [Ejemplos por Disciplina](#ejemplos-por-disciplina)

---

## Conceptos Básicos

- **Nodos**: Las entidades de tus datos (Personas, Ciudades, Transacciones). Se representan entre paréntesis `(n:Persona)`.
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
- `<-[:COLABORA]->`: Relación de tipo "COLABORA" en cualquier dirección.
  Con corchetes la flecha se escribe `<-[...]->`; `<->` solo no lleva corchetes.

### Ejemplos de Conexiones
```nql
// A conoce a B
from (a:Persona) -> [:CONOCE] -> (b:Persona)

// Cadena de influencia: A influye en B, B influye en C
from (a:Persona) -> [:INFLUYE] -> (b:Persona) -> [:INFLUYE] -> (c:Persona)

// Colaboración mutua
from (autor1:Investigador)<-[:COLABORA]->(autor2:Investigador)
```

### Variables en un patrón (0.9.4)

- **Una variable repetida es el mismo nodo.** `(a)-[:SIGUE]->(b)-[:SIGUE]->(a)`
  devuelve los ciclos de 2 (`a` sigue a `b` y `b` sigue de vuelta a `a`), y
  `(a)-[:SIGUE]->(a)` devuelve los self-loops. Una variable de arista repetida
  es la misma arista. Hasta 0.9.3 la segunda `a` se volvía a ligar a cualquier
  nodo, así que el primer patrón también devolvía caminos que no cierran.
- **Un extremo anónimo nunca toma un nombre del patrón.** En
  `find u.nombre from ()-[:SIGUE]->(u)`, `u` es el destino y `()` sigue
  anónimo. Hasta 0.9.3 el nodo anónimo podía ligarse a `u`, y la consulta
  devolvía los orígenes.

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
- **Comparación**: `=`, `!=`, `<`, `>`, `<=`, `>=`, `in [..]`, `not in [..]`
- **Lógico**: `and`, `or`. `and` liga más que `or`: `a and b or c` es
  `(a and b) or c`. Los paréntesis agrupan: `a and (b or c)`.
- **Literales**: cadenas, booleanos, enteros y decimales, también negativos
  (`-5`, `-2.5`), y listas (`[1, -2]`).

Todavía no se soportan en expresiones: aritmética (`+ - * / %`), negación
(`!`, `not`) y el menos unario sobre algo que no sea un número. Dan un error
de parseo que los nombra, nunca una condición distinta en silencio. Hasta
0.8.2 se descartaban: `x * 2 = 6` se ejecutaba como `x = 6`, y `-5` perdía
el signo. Para excluir, usa `!=` o `not in`.

### Las condiciones son comparaciones (0.9.4)

Una propiedad, variable o literal suelto no es una condición: `where p.activo`
es un error con nombre que pide la comparación, `where p.activo = true`. Lo
mismo en `having`. Hasta 0.9.3 el `find` dejaba pasar todos los nodos y
`delete`/`update` no tocaban ninguno.

### Alias de la proyección en WHERE y HAVING (0.9.4)

Un alias del `find` vale por su expresión en `where` y `having`:

```nql
// Nodos con más de una arista
find n.nombre, degree(n) as dg from (n:Persona) where dg > 1

// Grupos cuyo peso promedio pasa de 0.5
find a.nombre, avg(r.w) as m from (a:Persona)-[r:CALIFICA]->(b:Receta)
group by a.nombre having m > 0.5
```

Un alias con el mismo nombre que una variable del patrón no se sustituye: el
nombre sigue siendo la variable. Hasta 0.9.3 el alias se leía como una
propiedad inexistente, así que las dos consultas de arriba no devolvían filas.

### Ejemplos
```nql
// Clientes de alto valor
where c.compras_totales > 50000

// Segmentación geográfica y demográfica
where (p.ciudad = "CDMX" or p.ciudad = "GDL") and p.edad < 30

// Exclusión
where p.estado != "Inactivo"

// Bajo cero
where m.temp < -1
```

La exclusión no devuelve los nodos sin `estado`: una comparación sobre una
propiedad que falta es falsa, también con `!=`. Ver
[Semántica de NQL § Comparaciones y valores](SEMANTICA_NQL.md#comparaciones-y-valores).

---

## Predicados de Ontología (instanceOf / subClassOf)

Cuando el grafo tiene una jerarquía de clases (importada de Turtle/OWL, o
construida con nodos `NodeKind::Class` y aristas `subClassOf`), dos predicados
del `where` consultan la taxonomía en vez de comparar labels:

- `instanceOf(n, "C")` — `n` es un individuo cuyo tipo declarado es `C` **o una
  subclase de `C`**, a cualquier profundidad. Cuentan todos sus tipos: un nodo
  importado con dos `rdf:type` cumple con cualquiera de los dos, no solo con el
  que se muestra como label.
- `subClassOf(c, "C")` — `c` es un nodo clase que es subclase estricta de `C`
  (una clase no es subclase de sí misma).

La clase se puede nombrar de tres formas; gana la primera que resuelve:

| Forma | Ejemplo | Se resuelve por |
|---|---|---|
| Label | `"Arbol"` | el label del nodo clase (la única forma en grafos construidos a mano) |
| Nombre con prefijo | `"flora:Arbol"` | el catálogo de prefijos de los documentos importados (`graph.rdf_prefixes()`); si el prefijo no existe se prueba como label, que es como el importer etiqueta una clase cuyo local name colisionó (`fauna:Rosa`) |
| IRI completo | `"http://plantas.example/flora#Arbol"` | la propiedad `iri` del nodo clase |

```nql
// Todas las plantas: instancias directas y de subclases por igual
find n.label from (n) where instanceOf(n, "Planta")

// La misma clase, sin ambigüedad entre namespaces
find n.iri from (n) where instanceOf(n, "flora:Rosa")

// Solo las clases bajo Planta
find c.label from (c) where subClassOf(c, "Planta")
```

Las consultas de camino aplican la misma prueba sobre los nodos del camino:
`path_start_instanceOf("C")`, `path_end_instanceOf("C")`,
`path_any_instanceOf("C")`, `path_all_instanceOf("C")` y sus equivalentes con
`subClassOf`. Reciben solo el nombre de la clase, en cualquiera de las tres formas.

Sin taxonomía (sin nodos clase en el grafo) ambos predicados son `false` para
todos los nodos.

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

`PROFILE` ejecuta la consulta y devuelve sus tiempos. Desde Rust va por
`graph.execute_statement("profile …")` (el resultado es `NqlResult::Profile`;
`execute_nql` lo rechaza); desde Python, `graph.execute_nql("profile …")` y
los tiempos quedan en `result.profile`:

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
- NQL **no** soporta todavía `CREATE GRAPH`, `BEGIN`, `ROLLBACK` ni `MERGE`. Cada `add`/`update`/`delete` corre en su propia transacción; las transacciones explícitas son de la API (`graph.begin_transaction()`).
- La gramática tiene `sketch <nombre> = <sentencia>` y `commit <nombre>` (cambios preparados), pero `execute_statement` (Rust) y `execute_nql` (Python) los rechazan con un error con nombre desde 0.9.4; los sketches son la API Rust `nopaldb::query::SketchManager`. Hasta 0.9.3 contestaban un mensaje OK sin hacer nada.
- Los patrones de escritura tienen un salto y respetan la dirección y las propiedades inline de nodos y aristas (0.8.4): cambian exactamente lo que devuelve `find` con el mismo patrón. Un patrón de escritura más largo o con cuantificador es un error con nombre.

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

El valor de `set` es un literal: texto, número, booleano, `null` o lista. Leer una propiedad (`set b.city = a.city`) o llamar a una función es un error (desde 0.10.3; hasta 0.10.2 respondía OK y no actualizaba nada).

### DELETE
```nql
// Borra solo la relación matched
delete (a:Person)-[:KNOWS]->(b:Person)
where a.name = "Alice" and b.name = "Bob"

// Borra nodos (y sus aristas incidentes)
delete (p:Person)
where p.name = "Bob"
```

Ejemplos completos en NQL, Rust y Python:
[NQL_WRITE_CRUD_HANDS_ON.md](../NQL_WRITE_CRUD_HANDS_ON.md).

---

## Índices (CREATE INDEX / DROP INDEX)

```nql
create index on Persona(email)                     // hash (default): búsquedas por igualdad
create index on Persona(edad) type btree           // rangos
create index on Articulo(cuerpo) type fulltext     // búsqueda de texto BM25, la usa hybrid()
drop index Persona_email                           // el nombre es Label_propiedad
```

### Analyzer full-text

Un índice full-text tokeniza sus documentos una vez, al indexar, y toda consulta
contra él pasa por el **mismo** analyzer. Por defecto es el tokenizer `default`
de tantivy: separar por no-alfanuméricos y pasar a minúsculas, nada más, así que
`clasificacion` no encuentra `clasificación` y `catálogos` no encuentra
`catálogo`. La cláusula `with (...)` configura el analyzer por índice:

```nql
// Todo activo para español: stemming, stopwords y plegado de acentos
create index on Nota(cuerpo) type fulltext with (language = "spanish")

// Cada interruptor se puede apagar (o encender sin idioma, solo el plegado)
create index on Nota(cuerpo) type fulltext with (language = "spanish", stopwords = false)
create index on Nota(cuerpo) type fulltext with (ascii_folding = true)
```

| Opción | Valores | Efecto |
|---|---|---|
| `language` | `"arabic"`, `"danish"`, `"dutch"`, `"english"`, `"finnish"`, `"french"`, `"german"`, `"greek"`, `"hungarian"`, `"italian"`, `"norwegian"`, `"portuguese"`, `"romanian"`, `"russian"`, `"spanish"`, `"swedish"`, `"tamil"`, `"turkish"` | Idioma del stemmer y de la lista de stopwords. Solo, activa los tres interruptores de abajo. |
| `stemming` | `true` / `false` | Reduce las palabras a su raíz (`catálogos`, `catálogo` → `catalog`). Requiere `language`. |
| `stopwords` | `true` / `false` | Descarta las stopwords del idioma (`de`, `la`, `el`). Requiere `language`; hay listas para danish, dutch, english, finnish, french, german, hungarian, italian, norwegian, portuguese, russian, spanish, swedish. |
| `ascii_folding` | `true` / `false` | Pliega acentos a ASCII (`clasificación` → `clasificacion`), así una consulta sin acentos encuentra. Funciona sin idioma. |

`with (...)` solo es válido con `type fulltext`; una opción desconocida o un
valor del tipo equivocado es error de parseo, nunca un ajuste ignorado en silencio.

**Cambiar el analyzer exige reindexar.** Los tokens en disco los produjo la
cadena anterior, así que no hay cambio en sitio: `drop index Nota_cuerpo` y
crearlo de nuevo (se repuebla desde los nodos). Crear un índice que ya existe te
lo dice exactamente así. El analyzer se guarda junto al índice y sobrevive a
reabrir la base; los índices creados antes de 0.5.13 conservan el default.

Rust: `graph.create_index_with(label, property, IndexType::FullText, IndexOptions { analyzer: Some(FullTextAnalyzer::for_language("spanish")) })` y `graph.describe_index(name)`. Python: `graph.create_index("Nota", "cuerpo", "fulltext", analyzer={"language": "spanish"})` y `graph.describe_index("Nota_cuerpo")`.

---

## Búsqueda vectorial e híbrida en WHERE

Con las features `embeddings-index` / `hybrid`, dos predicados convierten un
patrón en una búsqueda top-K en vez de un scan. El vector de consulta sale de
un **nodo de referencia** buscado por su propiedad `name` (forma posicional)
o, desde 0.6.9, de un **vector literal escrito en la consulta** (forma con
nombre): un agente de GraphRAG embebe la pregunta y busca con ella sin crear
ningún nodo. K es `k = N` si se da. Sin `k`, una consulta de un solo nodo usa
su `LIMIT` (10 si no hay) y un patrón usa 10, tenga el `LIMIT` que tenga. Las
filas salen mejor-primero salvo que haya `ORDER BY`.

```nql
// Vecinos más cercanos del nodo llamado "q" en el espacio de embeddings "minilm"
find n.titulo from (n:Articulo) where similar_to(n, "q", "minilm") limit 10

// Lo mismo con el vector de la pregunta dentro de la consulta
find n.titulo from (n:Articulo)
where similar_to(n, vector = [0.12, -0.03, 1e-05, …], model = "minilm", k = 10)

// Texto completo + vector, fusionados por Reciprocal Rank Fusion
find n.titulo from (n:Articulo) where hybrid(n, "memoria de grafo", "q", "minilm") limit 10
find n.titulo from (n:Articulo)
where hybrid(n, text = "memoria de grafo", vector = [...], model = "minilm", k = 10)

// Buscar y expandir en una sola consulta: la búsqueda siembra un patrón de un salto
find c.texto, e.nombre from (c:Chunk)-[:MENTIONS]->(e:Entity)
where similar_to(c, vector = [...], model = "minilm", k = 10)

// … o de dos (chunk → entidad → entidad relacionada), con el score del chunk en cada fila
find c.texto, f.nombre, score(c) from (c:Chunk)-[:MENTIONS]->(e:Entity)-[:RELATED]->(f:Entity)
where hybrid(c, text = "riego", vector = [...], model = "minilm", k = 10)

// Afinado: cada parámetro de HybridQuery es una opción con nombre
find n.titulo from (n:Articulo)
where hybrid(n, "memoria de grafo", "q", "minilm", rrf_k = 30, ef_search = 128, overfetch = 8, text_index = "Articulo_cuerpo")
limit 10
```

| Función | Formas | Notas |
|---|---|---|
| `similar_to(n, "ref_name"[, "modelo"][, k = N][, opciones…])` | nodo de referencia por `name` | k-NN por HNSW; exacto por debajo de 1024 vectores; el top-K se calcula dentro del label del patrón |
| `similar_to(n, vector = [...], model = "…"[, k = N][, opciones…])` | vector literal | igual, con el vector de la pregunta |
| `hybrid(n, "texto", "ref_name", "modelo"[, opciones…])` | nodo de referencia | RRF de texto y vector; el top-K se calcula dentro del label del patrón |
| `hybrid(n, text = "…", vector = [...], model = "…"[, k = N][, opciones…])` | con nombre; `text` y/o `vector` | se admite híbrida solo-texto o solo-vector |

Reglas: `vector` es una lista no vacía de números (se aceptan enteros y
`1e-05`) cuya longitud debe coincidir con el índice del modelo; `vector`
exige `model`; una llamada da o el nodo de referencia o el literal, no ambos.
Opciones con nombre de `hybrid`: `rrf_k` (número > 0, default 60), `ef_search`
(entero ≥ 1, default 30), `overfetch` (entero ≥ 1, default 4), `text_index`
(cadena; default: el primer índice full-text del label). Una opción
desconocida, un valor del tipo equivocado o un número incorrecto de
posicionales es un error de validación que nombra el problema; ninguna otra
función acepta opciones con nombre.

Opciones con nombre de `similar_to`: `overfetch` (entero ≥ 1, default 4) y
`ef_search` (entero ≥ 1, default 30). Con un patrón con label se piden
`overfetch·k` vecinos al índice y se conservan los primeros `k` del label; si
quedan menos, la petición crece ×4 (hasta 4096), y si el label es tan escaso
entre los vecinos más cercanos se busca entre los propios nodos del label. Así
la consulta devuelve `k` filas siempre que el label tenga `k` nodos con
embedding (0.6.10; antes, un `4·k` fijo podía devolver menos filas, incluso
ninguna).

**`score(var)` en FIND (0.6.10).** El score de la búsqueda vectorial del WHERE
para el nodo de cada fila: similitud coseno (`1 - distancia`) con
`similar_to`, el score RRF con `hybrid` (con las dos, el de `similar_to`, que
es el orden de las filas). En ambos, mayor es mejor. Sirve para recortar el
contexto por umbral o repartir un presupuesto de tokens:

```nql
find c.text, score(c) as s from (c:Chunk)
where similar_to(c, vector = [...], model = "minilm", k = 10)
```

`score(var)` exige un `similar_to`/`hybrid` sobre esa misma variable, funciona
en patrones de uno y dos saltos (cada fila expandida lleva el score de su nodo
buscado) y solo es columna del FIND: las filas ya salen mejor-primero, así que
se rechaza en WHERE, ORDER BY, GROUP BY y HAVING, y junto a agregaciones.
EXPLAIN nombra su fuente (`Score: score(c) = cosine similarity of similar_to(c) …`).

**Lo que devuelve `score(var)` depende de la búsqueda, y con ello el umbral
que tiene sentido (#228).** Desde 0.6.17 las piezas crudas son columnas
propias, con las mismas reglas que `score`:

| columna | con `similar_to` | con `hybrid` | con ambas |
|---|---|---|---|
| `score(var)` | similitud coseno, en `[0, 1]` | score RRF `Σ 1/(rrf_k + rango)`: **como mucho `2/rrf_k`** (0.033 con el default 60) | el de `similar_to` (es el orden de las filas) |
| `rank(var)` | posición 0-based en la búsqueda | posición 0-based en el ranking fusionado | posición en `similar_to` |
| `text_score(var)` | `null` | BM25 crudo de la rama de texto; `null` si esa rama no puntuó el nodo | el de `hybrid` |
| `vector_score(var)` | similitud coseno (= `score`) | similitud coseno de la rama vectorial; `null` si esa rama no puntuó el nodo | coseno de `similar_to` |

Corta por umbral absoluto solo sobre un coseno: `score(c)` con `similar_to`,
o `vector_score(c)` con `hybrid`. Un score RRF es una mezcla de rangos, no
una similitud: un umbral como `s > 0.45` no deja pasar nada, nunca. Con
`hybrid`, corta por `rank(c)` o por `limit`, y usa `text_score` /
`vector_score` para ver qué rama trajo la fila:

```nql
find c.text, rank(c), text_score(c) as bm25, vector_score(c) as cos
from (c:Chunk)
where hybrid(c, text = "riego por goteo", vector = [...], model = "minilm", k = 10)
```

Dónde puede ir la búsqueda: un patrón de un solo nodo, o **un** patrón lineal
de uno o dos saltos `(a)-[:T1]->(b)[-[:T2]->(c)]` cuyo primer nodo es la
variable buscada (los candidatos siembran el pipeline: `k` acota las semillas
y `LIMIT` las filas expandidas, así que conviene dar `k`; `score(a)` y los
scores crudos viajan con cada fila expandida; los predicados del `where`
sobre las otras variables se aplican tras expandir). Dos saltos es el multi-hop de GraphRAG, "chunk → entidad →
entidad relacionada", en una consulta (0.6.19). Varios patrones, cadenas más
largas, cuantificadores o buscar una variable destino son errores de
validación. Antes de 0.6.9 esas
consultas, y cualquier `similar_to` mal formado, se ignoraban en silencio y
devolvían todas las filas. `explain` imprime los parámetros efectivos (líneas
`SimilarTo:` / `Hybrid:`, el vector como `<literal dim=N>`), la estrategia
(`VECTOR SEARCH (similar_to)`, `PATTERN PIPELINE (seed: HYBRID, hops: 2)`, …),
una línea `Search universe:` con los predicados empujados a la búsqueda y, en
un patrón, una línea `Seeds:` con `k`, los saltos y el `LIMIT`.

**El `where` es el universo de la búsqueda (0.7.0, #214 fase 2).** Los
predicados sobre la variable buscada que se pueden calcular antes de puntuar
se empujan a la búsqueda, así que `k` es el top-k *dentro* de ellos:

```nql
find c.texto, score(c) from (c:Chunk)
where hybrid(c, text = "riego", vector = [...], model = "minilm", k = 10)
  and c.tenant = "t7" and c.idioma in ["es", "pt"]
```

devuelve los 10 mejores chunks del tenant `t7` en español o portugués. Hasta
0.6.20 puntuaba la etiqueta entera y descartaba los otros tenants después, así
que podía devolver menos de 10 filas, o ninguna. Se empujan: `v.p = literal` y
`v.p in [literales]` con escalares no nulos (cadena, entero, flotante,
booleano; la igualdad es tipada, `1` no es `1.0`, como en todo NQL),
`v.id = "…"` / `v.id in [...]`, y las propiedades inline del patrón
`(c:Chunk {tipo: "x"})`. Todo lo demás — rangos, `!=`, `not in`, `null`,
valores lista u objeto, funciones, otras variables — se sigue aplicando a las
filas de la búsqueda, y el `where` entero se re-aplica siempre, así que
empujar nunca admite una fila que no lo cumpla. `rank(c)` y `score(c)` se
calculan en ese universo, igual que `search_hybrid(props=...)` desde Python:
una fila que ya salía puede subir de rango y de `score` RRF, y un umbral sobre
él deja pasar más filas que antes (el coseno de `similar_to` no cambia). La
búsqueda debe ser una condición del `and` raíz del `where`: bajo `or`,
una segunda búsqueda del mismo tipo o una búsqueda dentro de otra función son
errores de validación (hasta 0.6.20 `hybrid(...) or c.x = 1` devolvía en
silencio solo los hits de la búsqueda). `and` liga más que `or`.
Detalle y API Rust/Python: [HYBRID_SEARCH.md](../HYBRID_SEARCH.md), [EMBEDDINGS.md](../EMBEDDINGS.md),
[GRAPHRAG.md](../GRAPHRAG.md).

**Buscar dentro de una comunidad: `in_community` (#214 fase 2b).** Una
comunidad materializada (`materialize_communities`) también es un universo:

```nql
find c.text, score(c) from (c:Chunk)
where hybrid(c, text = "riego", vector = [...], model = "minilm", k = 10)
  and in_community(c, "leiden/L0/…", via = "MENTIONS")
```

ordena dentro de los chunks que tienen una arista `MENTIONS` hacia un miembro
de esa comunidad. Sin `via`, `in_community(e, key)` son los propios
miembros, para buscar entre las entidades. Es una restricción de semilla
([semántica de NQL](SEMANTICA_NQL.md)):
- se intersecta con los demás predicados empujados (`c.id in [...]`,
  propiedades);
- es el mismo universo que `community_members(key, via=…)` en Python;
- es igual, como conjunto, a lo que devuelve el patrón
  `(c)-[:MENTIONS]->(e)-[:IN_COMMUNITY]->(k:Community) where k.key = "…"`.

Solo vale como condición del `and` raíz, junto a `hybrid` o `similar_to`
sobre la misma variable. En otro sitio, con una opción desconocida o con una
clave inexistente, es un error con nombre. Para filtrar filas por comunidad
sin búsqueda, escribe ese patrón.

---

## Agregaciones y Funciones

NQL soporta funciones para resumir datos, ideales para análisis estadístico.

Los nombres de las funciones no distinguen mayúsculas (`COUNT(*)` es `count(*)`). Un nombre que no es una función de NQL es un error en toda cláusula (desde 0.10.3; hasta 0.10.2 se descartaba sin error). No hay `id(n)`: el id es la propiedad `n.id`.

- `count(*)`: Cuenta el total de coincidencias.
- `sum(p.monto)`: Suma los valores de una propiedad numérica.
- `avg(p.edad)`: Calcula el promedio.
- `min(p.score)`, `max(p.score)`: Encuentra valores mínimos y máximos.
- `degree(n)`, `pagerank(n)`, `betweenness(n)`, `clustering(n)`: Agregaciones de analítica de grafos.
- `community(n)`: Detección exacta global de comunidades (basada en Louvain).
- `community_fast(n)`: Detección aproximada local de comunidades para exploración de baja latencia.
- `leiden(n)`: Detección exacta global de comunidades con **Leiden / CPM** (Traag et al. 2019). Garantiza comunidades bien conectadas internamente. Usa un caché distinto al de `community()`, así que ambas pueden convivir en la misma consulta. Opciones (0.6.11): `leiden(n, labels = ["Entity"], edge_types = ["RELATED"], weight = "w", gamma = 0.2)` acota la partición a esas etiquetas y tipos de arista (los nodos de fuera reciben `null`), pondera cada par con la suma de la propiedad numérica `w` de sus aristas y fija la resolución CPM (0.1 por defecto). Una configuración por consulta; detalle en [ALGORITHMS.md](../ALGORITHMS.md).
- `shortestPath("uuid-origen", "uuid-destino")`: Distancia más corta entre dos nodos (regresa `-1.0` si no hay camino).

`community(n)` y `leiden(n)` calculan particiones globales, por lo que `LIMIT` se aplica después de la agregación y no reduce su costo en primera ejecución.
Cada una reutiliza su propio caché versionado por topología; un cambio de nodos o aristas invalida ambos.

**Ejemplo de Análisis:**
```nql
find count(*), avg(p.edad)
from (p:Usuario)
where p.activo = true
```

```nql
// Exploración (rápido/aproximado)
find community_fast(n) as cluster_fast
from (n)
limit 1

// Resultado final/reporte (exacto, Louvain)
find community(n) as cluster
from (n)
limit 1

// Leiden (Traag et al. 2019): comunidades bien conectadas, determinista
find n.nombre, leiden(n) as cluster_leiden
from (n)
order by cluster_leiden
limit 20

// Comparar Louvain y Leiden en una sola consulta (cachés independientes)
find n.nombre, community(n) as louvain, leiden(n) as comunidad_leiden
from (n)
limit 10
```

---

## Exportación de Datos (EXPORT)

Exporta el resultado de un `find` a CSV, JSON, Arrow IPC o Parquet. `export`
va al final de la consulta, después de `limit`.

### Sintaxis
```nql
find p.nombre, p.edad
from (p:Persona)
export csv with path="ruta/al/archivo.csv", header=true, separator=","
```

```nql
find p.nombre, p.edad
from (p:Persona)
export json with path="ruta/al/archivo.json", pretty=true
```

```nql
// Sin ruta: el resultado vuelve inline, un objeto JSON por línea
find p.nombre, p.edad
from (p:Persona)
export json with jsonl=true
```

```nql
// Los formatos binarios siempre escriben un archivo
find p.nombre, p.edad from (p:Persona) export arrow with path="personas.arrow"
find p.nombre, p.edad from (p:Persona) export parquet "personas.parquet"
```

`export` antes de `find` no está soportado.

### Formatos y opciones

| Formato | Opciones | Sin ruta |
|---|---|---|
| `csv` | `path`, `separator` (default `","`), `header` (default `true`) | se devuelve inline |
| `json` | `path`, `pretty` (default `false`), `jsonl` (default `false`) | se devuelve inline |
| `arrow` | `path` (obligatoria): escribe un archivo Arrow IPC | error |
| `parquet` | ninguna: la ruta va en la sintaxis, `export parquet "archivo.parquet"` | — |

Las opciones se validan por formato (0.9.4). Una opción desconocida es un
error con nombre que lista las válidas: `export csv with delimiter=";"` falla
y dice que la opción es `separator`. `arrow` y `parquet` necesitan la feature
`analytics` (la wheel de Python la incluye); sin ella son un error con nombre.
Hasta 0.9.3 las opciones desconocidas se ignoraban, `export arrow` devolvía un
resumen de texto y `export parquet` respondía éxito sin escribir el archivo.

Export inline (csv y json sin `path`): en Rust,
`graph.execute_statement(q)` devuelve `NqlResult::Export { data, .. }` con el
texto; en Python, `graph.execute_nql(q).data` (nuevo en 0.9.4). Con ruta,
`data` lleva el mismo texto (csv/json) o la ruta escrita (arrow/parquet).

**Ejemplo (export CSV):**
```nql
find p.nombre, p.edad
from (p:Persona)
order by p.edad
export csv with path="./usuarios.csv", header=true
```

---

## Viaje en el tiempo (AT TIMESTAMP)

Desde 0.10.2 una consulta puede leer el grafo como era en un momento pasado.
`at timestamp <ts>` va después de `limit` y antes de `export`:

```nql
find p.nombre from (p:Persona) limit 10 at timestamp 42
find a.nombre, b.nombre from (a:Persona)-[k:CONOCE]->(b:Persona) where k.desde > 2020 at timestamp 42
```

`<ts>` es un valor del reloj lógico de la base, que crece con cada commit; no
es tiempo Unix. `graph.last_commit_timestamp()` (Rust y Python) da el
timestamp de la última escritura: guárdalo junto a lo que quieras volver a
leer después.

La consulta corre sobre el grafo válido en `ts`: cada nodo y arista en su
versión de ese momento, nada de lo creado después, y lo borrado después
todavía presente. Los patrones, el `where`, las agregaciones, `order by`, los
algoritmos de grafos (`degree`, `pagerank`, `community`…), `instanceOf` /
`subClassOf` y `export` funcionan sobre ese grafo. En el último commit, o
después, la respuesta es la actual. Es también la primera lectura en un
instante desde Python (`graph.execute_nql(...)`).

Lo que no puede responder:
- **`ts` bajo el horizonte del GC** (`graph.gc_horizon()`): el recolector ya
  borró esa historia. `HistoryUnavailable`.
- **`ts` bajo un suelo de historia**: lo escrito sin versión antes de 0.10.1
  tiene historia solo desde el suelo F que puso la primera apertura con
  0.10.1 ([ACTUALIZAR_0.10.md](ACTUALIZAR_0.10.md)). `HistoryUnavailable`,
  que nombra la entidad y F.
- **Datos sin historia**: `similar_to`, `hybrid`, `in_community`, los scores
  de búsqueda (`score`, `rank`, `text_score`, `vector_score`),
  `has_embedding` y las funciones de embeddings de caminos y patrones leen
  vectores, texto completo y comunidades guardadas, que solo existen en su
  estado actual. Con `at timestamp` son un error con nombre.

**Costo.** Cada consulta lee la historia entera y arma en memoria el grafo en
`ts` antes de correr: tiempo O(todas las versiones) y memoria para todo lo
válido en `ts`. `explain` lo muestra:

```
Time travel: graph materialized at timestamp 42 (1200 nodes, 3400 edges), reading the whole history; the plan below runs on that copy
```

Los índices creados con `create index` no se copian: sobre el grafo pasado la
consulta recorre. Para muchas lecturas de nodos sueltos la API de Rust es más
barata: `graph.get_node_at(id, ts)` y el snapshot `graph.as_of(ts)`. Hasta
0.10.1 `at timestamp` era un error con nombre, y hasta 0.9.3 leía el estado
actual sin aviso.

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
| Lista | `[1, -2]`, `["a", "b"]` | Listas literales, para `in [..]` y `vector = [...]`. |

---

## Ejemplos por Disciplina

### 🔬 Para Sociólogos: Análisis de Redes
Encontrar líderes de opinión en una comunidad.
```nql
// ¿Quién es "seguido" por personas que a su vez son muy seguidas?
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
