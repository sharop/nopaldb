# Actualizar a 0.10

> English: [docs/UPGRADING_0.10.md](../UPGRADING_0.10.md).

Esta página es para una aplicación que usa NopalDB **0.9.4** y pasa a
**0.10**. Si vienes de 0.9.3 o anterior, lee antes
[ACTUALIZAR_0.9.md](ACTUALIZAR_0.9.md): todo lo de allí sigue aplicando.
¿Ya usas 0.10.0? La sección 8 resume lo que cambia en 0.10.1.

0.10 corrige la historia MVCC (las versiones detrás de `history`, `as_of` y
`get_node_at`), el GC y la durabilidad del `update` de NQL. Es una versión
menor porque cambiaron dos APIs de Rust de forma incompatible (sección 2).
Cada punto enlaza a su issue.

Instalación:

```bash
pip install -U "nopaldb==0.10.1"   # Python 3.10+
```

```toml
nopaldb = "0.10"
```

## 1. Lo que no cambia

- **Las bases escritas por 0.9.4 se abren sin ningún paso de tu parte.** El
  formato en disco de nodos, aristas, índices y WAL es el mismo; un WAL
  pendiente que dejó un crash de 0.9.4 lo reproduce bien 0.10. Desde 0.10.1
  el primer open escribe una vez en la base: adopta la historia de lo que se
  escribió sin versión (sección 3).
- La API de Python solo creció (sección 4).

## 2. Código Rust que puede dejar de compilar o comportarse distinto

- **`GCConfig` tiene un campo público nuevo, `min_age_ms`**
  ([#310](https://github.com/sharop/nopaldb/issues/310)). Un literal que
  nombre todos los campos sin `..Default::default()` deja de compilar. Lo
  mismo pasa si construyes a mano `GcAutoSummary` o `GcSection`: los dos
  ganaron campos. Usa los constructores (`GCConfig::older_than_days(7)`,
  `GCConfig::before_timestamp(ts)`) o `..Default::default()`.
- **`get_node_at` y `as_of(ts)` devuelven errores en lugar del nodo actual**
  ([#311](https://github.com/sharop/nopaldb/issues/311)). Hasta 0.9.4, si no
  encontraban una versión válida en `ts`, devolvían el nodo tal como es hoy,
  lo que era una respuesta incorrecta sin aviso.

  | `graph.get_node_at(id, ts)` / `graph.as_of(ts).get_node(id)` | 0.10 |
  |---|---|
  | hay una versión válida en `ts` | esa versión |
  | el nodo no existía en `ts` (creado o cargado con el BulkLoader después, o ya borrado) | `NopalError::NodeNotFound` |
  | `ts` está bajo el suelo de historia del nodo (datos escritos antes de 0.10.1, adoptados en el primer open; sección 3), o bajo el horizonte del GC | `NopalError::HistoryUnavailable` (nuevo) |

  `GraphSnapshot::get_nodes` solo omite `NodeNotFound` y devuelve cualquier
  otro error; antes se tragaba todos. `NopalError` es `#[non_exhaustive]`, así
  que un `match` ya tiene una rama `_`.

## 3. Comportamiento que cambió

### El GC por antigüedad conserva la historia reciente ([#310](https://github.com/sharop/nopaldb/issues/310))

Hasta 0.9.4, `GCConfig::older_than_ms/hours/days` y `Graph::gc_default`
borraban **toda** la historia cerrada: comparaban milisegundos Unix con los
timestamps lógicos de las versiones. La historia ya borrada así no vuelve.

0.10 traduce la edad al reloj lógico con marcas de reloj de pared que la base
guarda ahora, como mucho una por segundo. La traducción es conservadora: solo
borra lo que la base puede probar que se cerró hace al menos esa edad.

**Consecuencia en bases existentes: al principio el GC por antigüedad no
recupera nada.** No recupera nada hasta que pase la edad pedida desde la
primera escritura hecha con 0.10, porque antes no hay marcas. Si dependes del
GC para liberar espacio, espera que no se borren versiones durante ese primer
periodo. `GCConfig::before_timestamp(ts)` borra por timestamp lógico de
inmediato.

### Borrar un nodo cierra su historia ([#309](https://github.com/sharop/nopaldb/issues/309))

Todos los caminos de borrado cierran la versión del nodo, y las de las
aristas borradas con él, en el timestamp del borrado: `delete_node` directo,
transacciones, `delete` de NQL, `delete(label, key, value)` de Python y la
reproducción del WAL. Volver a crear un nodo con el mismo id continúa su
cadena de versiones.

La reproducción del WAL vuelve a aplicar ahora cada escritura con el
timestamp registrado en el WAL. Hasta 0.9.4, las aristas y los borrados de
nodo recibían timestamps nuevos al reproducirse.

**Los nodos borrados antes de 0.10 no tienen timestamp de borrado
registrado.** Su historia quedó abierta. Desde 0.10.1 el primer open la
cierra en el timestamp de adopción F y le pone al nodo un suelo de historia
en F (sección siguiente): desde F el nodo es `NodeNotFound`, y antes de F
`HistoryUnavailable`, porque la fecha real del borrado no se conoce. Con
0.10.0, `as_of(ts)` posterior a su borrado los sigue encontrando.

### El `update … set` de NQL es una transacción ([#312](https://github.com/sharop/nopaldb/issues/312))

Como `add` y `delete`, cada sentencia `update` es ahora una transacción:
- se escribe en el WAL, así que un `update` confirmado sobrevive a un crash
  (hasta 0.9.4 se podía perder antes del siguiente checkpoint);
- deja una versión MVCC;
- mantiene al día los índices hash/btree/full-text creados con
  `create_index` (antes un índice hash seguía encontrando el nodo por su
  valor viejo);
- es atómica: un fallo a mitad no escribe nada.

Con `Serializable`, un `update` toma ahora el candado de escritura de cada
nodo que cambia. Desde 0.10.1, reescribir las propiedades de una arista
encadena una versión nueva (0.10.0 todavía la reemplazaba).

### Toda escritura deja historia (0.10.1, [#319](https://github.com/sharop/nopaldb/issues/319))

0.10.0 todavía escribía algunos datos sin versión, y el time travel sobre
ellos respondía `HistoryUnavailable`. Desde 0.10.1 toda escritura deja
historia:

- **`add_node` directo** y la **importación Turtle/OWL** escriben una
  versión, igual que una transacción.
- **El BulkLoader** no copia cada nodo o arista nuevos en una versión (eso
  costaba +62 % de disco). Guarda una **marca de nacimiento** de 8 bytes con
  el timestamp de la carga, uno por flush: el registro es válido desde ese
  timestamp, y antes el nodo no existía (`NodeNotFound`). La primera
  escritura o borrado posterior de ese nodo convierte la marca en una
  primera versión real. Un id que ya existe continúa su cadena como siempre.
- **Las aristas tienen cadena de versiones.** Reescribir una arista escribe
  la versión siguiente, y volver a crear una arista borrada continúa desde
  su última versión. En 0.10.0 las dos cosas pisaban la versión 1.

**Adopción de historia en el primer open.** Una base escrita antes de 0.10.1
tiene nodos y aristas sin versión (escrituras directas, BulkLoader,
importación Turtle) y cadenas que ya no coinciden con el registro actual. El
primer open con 0.10.1 toma un timestamp lógico F del reloj y:

- a cada nodo o arista sin versión le pone un **suelo de historia** en F, sin
  copia: desde F las lecturas devuelven el registro actual; antes de F
  devuelven `HistoryUnavailable`, porque no se sabe qué pasó antes;
- añade una versión en F a una cadena cuya última versión no coincide con el
  registro (y le pone el suelo F);
- cierra en F la historia abierta de nodos y aristas ya borrados (y les pone
  el suelo F).

La historia escrita por transacciones que coincide con el registro no recibe
suelo y conserva todo su pasado. Para un nodo sin cadena, `history(id)`
muestra una versión que empieza en F. `last_commit_timestamp()` alcanza F.

La pasada corre dentro de `open`, después de reproducir el WAL y antes de que
`open` regrese, también con `open_read_only`. Lee cada nodo y arista una vez:
unos 260 ms para 100k nodos y 100k aristas, unos 30 ms en los opens
siguientes. Va por tramos, así que un open interrumpido durante la pasada la
reanuda con el mismo F la próxima vez, y una vez terminada no vuelve a
correr.

**Costo en el BulkLoader.** Medido con 200k nodos × 5 propiedades: nodos
+6 % de tiempo y +6 % de disco; aristas +23 % de tiempo, por dos lecturas
más por arista para distinguir una arista nueva de una existente.

## 4. Nuevo

- `Graph::last_commit_timestamp()` (Rust y Python): el timestamp lógico de la
  última escritura aplicada, el valor que se pasa a `as_of` / `get_node_at`.
  No lo mueven `begin_transaction`, los rollbacks ni los commits vacíos.
- `Graph::gc_horizon()` (Rust y Python; también `stats().gc.horizon`,
  `get_stats()["gc"]["horizon"]`): hasta dónde recolectó el GC. A partir de
  ese timestamp la historia está completa.
- `GCConfig::before_timestamp(ts)`: un corte del GC en tiempo lógico.

## 5. Límites que siguen

- **La historia anterior a la adopción no se recupera.** Lo escrito sin
  versión antes de 0.10.1 responde `HistoryUnavailable` antes del suelo F
  (sección 3). No hay registro de ese pasado con el que reconstruirlo.
- **Las primitivas de bajo nivel `Storage::insert_node` / `insert_edge`
  siguen sin escribir versión.** Si sobrescriben un nodo o una arista que
  solo tiene marca de nacimiento o suelo, borran la marca, y esa entidad
  responde `HistoryUnavailable` desde entonces. Usa la API de `Graph`.
- **`find … at timestamp <n>` de NQL** era un error con nombre hasta 0.10.1.
  Desde 0.10.2 lee el grafo como era en ese timestamp, también desde Python,
  salvo las búsquedas vectoriales, de texto completo y de comunidades
  ([#298](https://github.com/sharop/nopaldb/issues/298),
  [referencia NQL § Viaje en el tiempo](NQL_REFERENCIA.md#viaje-en-el-tiempo-at-timestamp)).

## 6. Volver a 0.10.0 o a 0.9.4

Las dos abren una base escrita por 0.10.1, porque el formato es el mismo:
las marcas de reloj de pared, las marcas de nacimiento, los suelos y la
marca de adopción son claves nuevas que ignoran. Ignorarlas tiene costo, así
que **vuelve a la copia que guardaste antes de actualizar**, no a la base ya
actualizada:

- **No ven las marcas de nacimiento ni los suelos.** En 0.10.0, lo cargado
  con el BulkLoader y lo adoptado vuelve a responder `HistoryUnavailable`; en
  0.9.4 `get_node_at` cae al nodo actual.
- **Sus escrituras rompen la historia implícita.** Una escritura de 0.10.0 o
  0.9.4 sobre un nodo o una arista que solo tiene marca de nacimiento o suelo
  empieza una cadena sin guardar antes el registro que la marca
  representaba. De vuelta en 0.10.1, esa entidad se lee como inexistente
  entre la marca y esa escritura.
- **Sus escrituras sin versión no se adoptan.** La adopción corre una vez;
  un `add_node` directo hecho después por 0.10.0 no tiene historia en 0.10.1.
- **Solo 0.9.4:** volver a crear un id borrado re-fecha la versión cerrada, y
  el GC por antigüedad borra toda la historia cerrada (sección 3).

## 7. Qué probar

Sobre una copia de tu base:
1. abre y los conteos coinciden;
2. un GC por antigüedad que corras con regularidad borra lo que esperas,
   teniendo en cuenta el arranque de arriba;
3. el código que llama a `get_node_at` / `as_of` maneja `NodeNotFound` y
   `HistoryUnavailable`;
4. tus sentencias `update` de NQL siguen cambiando exactamente lo que
   esperas;
5. el primer open con 0.10.1 tarda lo que puedes permitirte (sección 3: la
   adopción lee cada nodo y arista una vez), y el siguiente open vuelve a
   ser rápido.

## 8. De 0.10.0 a 0.10.1

Sin cambios incompatibles de API. Nuevo en Rust: `Graph::history_floor(id)`
y `Graph::edge_history_floor(id)` devuelven el suelo de un nodo o una arista
(`None` si no tiene). Lo que cambia:
- toda escritura deja historia, el BulkLoader con una marca de nacimiento, y
  las aristas tienen cadena de versiones (sección 3);
- el primer open adopta lo que 0.10.0 escribió sin versión: escribe una vez,
  tarda en proporción al tamaño de la base y le pone un suelo de historia a
  esos datos (sección 3);
- las lecturas bajo un suelo responden `HistoryUnavailable`; un nodo cargado
  con el BulkLoader después de `ts` responde `NodeNotFound` (sección 2);
- `ELReasoner::from_graph_at(ts)` bajo un suelo responde
  `HistoryUnavailable` en vez de usar la taxonomía actual;
- no escribas en la base con 0.10.0 después de abrirla con 0.10.1
  (sección 6).

## 9. De 0.10.2 a 0.10.3

Sin cambios de API ni de formato en disco. NQL deja de descartar cosas sin
error, y algunas consultas devuelven otras filas o columnas:

| Consulta | 0.10.2 | 0.10.3 | Qué hacer |
|---|---|---|---|
| `find degree(n) as d from (n:L)` (también `pagerank`, `betweenness`, `clustering`, `community`, `leiden`) | una fila con el promedio | una fila por nodo ([#336](https://github.com/sharop/nopaldb/issues/336)) | para el promedio escribe `find avg(degree(n)) as d` |
| `find degree(n) as d, n.nombre from (n:L)` | agrupado por `n.nombre`: los nodos con el mismo nombre se fundían | una fila por nodo | añade `group by n.nombre` y `avg(...)` si querías el grupo |
| `find n.nombre as j …` (Rust) | la fila traía `j` y `n.nombre` | solo `j` ([#331](https://github.com/sharop/nopaldb/issues/331)) | lee `j`; Python y `export` ya daban solo `j` |
| `find n.nombre, count(*) as c … group by n.nombre` | columnas `n.id`, `n.nombre`, `c` | `n.nombre`, `c` | pide `n.id` si lo usabas |
| `find foo(n) …`, `id(n)`, `COUNT(*)` | columna perdida, `where` dejaba pasar todo, fila vacía | `SemanticError`; `COUNT(*)` es `count(*)` ([#328](https://github.com/sharop/nopaldb/issues/328)) | usa `n.id` en vez de `id(n)` |
| `update … set n.x = n.nombre` | OK sin actualizar nada | `SemanticError` ([#330](https://github.com/sharop/nopaldb/issues/330)) | por ahora solo literales en `set` |
| `find all(n)`, `find *` con un `where` por índice | filas vacías; en Python solo `n.id`, `n.label` | todas las propiedades | — |

## 10. De 0.10.3 a 0.10.4: las columnas del resultado

Las columnas del resultado salen del `find`, iguales sea cual sea la ruta
que ejecuta la consulta ([#335](https://github.com/sharop/nopaldb/issues/335)).
Sin cambios de API ni de formato en disco; cambian nombres y orden en Rust,
Python y `export`, y algunas consultas que respondían ahora son error:

| Consulta | 0.10.3 | 0.10.4 | Qué hacer |
|---|---|---|---|
| `find count(*), sum(n.edad) …` | columnas `count`, `sum` | `count(*)`, `sum(n.edad)` | pon un alias (`count(*) as c`) si lees la columna por nombre |
| `find degree(n), n.nombre …` | columna `degree` | `degree(n)` | igual |
| `find count(*) as c, n.ciudad … group by n.ciudad` | `n.ciudad`, `c` | `c`, `n.ciudad`: el orden del `find` | lee por nombre, o escribe el `find` en el orden que quieras |
| `find *` (Rust; Python ya daba todas las claves) | columnas `["*"]` | `n.id`, `n.label` y luego las propiedades ordenadas | — |
| `find n.a as x, n.b as x` | una sola `x`, el otro valor perdido | `SemanticError` | usa alias distintos |
| `find n.edad, count(*) … group by n.ciudad` | `n.ciudad` y `count`, `n.edad` perdida | `SemanticError` | añade `n.edad` al `group by` o agrégala (`avg(n.edad)`) |
| `find n.ciudad, pagerank(n) … group by n.ciudad` | promedio del grupo | `SemanticError` | escribe `avg(pagerank(n))` |
| `find n.nombre … order by degree(n)` | sin ordenar | `SemanticError` | proyéctalo (`degree(n) as d`) y `order by d` |
| `find … order by count(*)` con `count(*)` en el `find` | sin ordenar | ordenado | — |
| `find n.nombre, 1 …` | columna perdida | columna constante `1` | — |

## 11. De 0.10.4 a 0.10.5: aritmética, negación y varios patrones

NQL evalúa `+ - * / %`, el menos unario, `not` y `!`
([#269](https://github.com/sharop/nopaldb/issues/269)). Consultas que daban
error de parseo ahora responden; unas pocas que respondían cambian:

| Consulta | 0.10.4 | 0.10.5 | Qué hacer |
|---|---|---|---|
| `where r.porciones * 2 = 6`, `find r.porciones * 2 as d`, `order by -r.porciones`, `having count(*) * 2 > 3`, `where not r.region = "sur"`, `where !(…)` | error de parseo | se evalúan | — |
| `set r.porciones = r.porciones + 1` | `SemanticError` | escribe el valor nuevo | — |
| `path_eval` / `gather "x = a / b"` con dos enteros | decimal (`7 / 2` = `3.5`) | entero (`3`) | escribe un operando como decimal: `a / 100.0`, o `init "x = 100.0"` |
| `where n.x = 3.0` sobre una propiedad entera (también `in [3.0]`) | sin coincidencia | coincide: los números se comparan por valor | — |
| desbordamiento de enteros, `/ 0`, `% 0` en un `gather` | pánico en debug, valor que da la vuelta en release / error con nombre | error con nombre | — |
| una variable llamada `not` | permitida | `not` es palabra clave | renombra la variable |
| `where not similar_to(…)` (también `hybrid`, `in_community`) | error de parseo | `SemanticError`: una búsqueda elige candidatos, no se puede negar | — |
| `where r.porciones + 1` (aritmética como condición) | error de parseo | `SemanticError`: compárala | `where r.porciones + 1 > 0` |
| `find distinct r.porciones % 2`, o `distinct` con `order by` de una expresión | error de parseo | error: todavía no se soporta | — |
| `delete (n:L) where true` (también `update`) | no borraba nada: el WHERE de las escrituras leía como `false` lo que no fuera comparación | borra todos los `L`, como lo lee el `find` | revisa scripts que dependieran de eso |

Varios patrones en el `from` que son todos nodos solos se unen
([#345](https://github.com/sharop/nopaldb/issues/345)), como ya pasaba
cuando un patrón tenía una arista:

| Consulta | 0.10.4 | 0.10.5 | Qué hacer |
|---|---|---|---|
| `find a.nombre, b.nombre from (a:Receta), (b:Receta)` | las filas de `a`, `b` vacío | el producto cruzado (N·M filas) | relaciona los patrones en el `where` si esperas menos filas |
| `… where a.porciones < b.porciones` (condición sobre los dos) | sin filas | los pares que la cumplen | — |
| `find count(*) from (a:Receta), (b:Receta)` | contaba solo `a` | error con nombre: las agregaciones sobre varios patrones todavía no se soportan | cuenta sobre un patrón |

## 12. De 0.10.5 a 0.10.6: la unión de varios patrones

La unión de varios patrones del `from` filtra mientras combina y tiene un
tope de filas ([#349](https://github.com/sharop/nopaldb/issues/349)). Las
filas no cambian; lo que cambia:

| Situación | 0.10.5 | 0.10.6 | Qué hacer |
|---|---|---|---|
| `from (a:X), (b:X) where … limit 10` sobre etiquetas grandes | el producto entero en memoria (2000 × 2000: 10 GB, 4 s) | solo las filas que pasan (18 MB, 10 ms) | — |
| más de `Graph::max_join_rows()` filas cumplen el `where` en una unión (400 000; 100 000 con `Mobile`) | se agotaba la memoria | error con nombre | relaciona cada patrón con los anteriores en el `where`, o `set_max_join_rows(n)` |
| `from (a:X), (b:Y) order by a.x limit 10` con más pares que el tope (`order by` y `distinct` necesitan todas las filas) | respondía, con memoria de sobra | error con nombre | relaciona los patrones, o `set_max_join_rows(n)` |
| `from (a), (b), (c) where a.k = c.k` (la condición se salta un patrón) | respondía, con memoria de sobra | la unión de `a` y `b` es el producto entero y cuenta | escribe `(a), (c), (b)` |
| `explain` de varios patrones de un solo nodo | `Strategy: LABEL SCAN` | `Strategy: PATTERN PIPELINE` y una línea `Join: …` | actualiza quien lea el texto del plan |

## 13. De 0.10.9 a 0.10.10: agregaciones sobre varios patrones

Las agregaciones sobre varios patrones del `from`, y sobre un patrón de
varios saltos, responden en vez de fallar ([#362](https://github.com/sharop/nopaldb/issues/362)):

| Consulta | 0.10.9 | 0.10.10 | Qué hacer |
|---|---|---|---|
| `find count(*) from (a:Receta), (b:Receta)` | error con nombre | el número de pares | — |
| `find r.region, count(*), sum(r.porciones * i.gramos) from (r:Receta), (i:Ingrediente) where r.region = i.region group by r.region` | error con nombre | una fila por región | — |
| `find z.nombre, count(*) from (r)-[:LLEVA]->(i)-[:CRECE_EN]->(z) group by z.nombre` | error con nombre | una fila por zona | — |
| `… limit 1` sobre una de esas agregaciones | — | `limit` cuenta grupos; se agrega la unión entera | — |
| `find avg(degree(r)) from (r:Receta), (i:Ingrediente)` | error con nombre | sigue siendo error con nombre | agrega sobre un patrón |
