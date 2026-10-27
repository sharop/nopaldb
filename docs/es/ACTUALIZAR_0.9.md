# Actualizar a 0.9.4

> English: [docs/UPGRADING_0.9.md](../UPGRADING_0.9.md).

Esta página es para una aplicación que usa NopalDB 0.6.x, 0.7.x u 0.8.x y
pasa a **0.9.4**. La mayoría de las actualizaciones no requieren cambiar
código: no hay ningún paso manual de migración y la API de Python solo ganó
métodos.
Lo que merece atención son los arreglos por los que una consulta devuelve
**otras filas** que antes, y dos bugs viejos que pueden haber **escrito datos
incorrectos ya**. Cada punto enlaza a su entrada del
[CHANGELOG](../../CHANGELOG.md).

Instalación:

```bash
pip install -U "nopaldb==0.9.4"          # Python 3.10+
pip install -U "nopaldb[networkx]==0.9.4" # con la conversión a NetworkX
```

```toml
nopaldb = "0.9"   # Rust; 0.9.4 o posterior
```

## 1. Lo que no cambia

- **Las bases redb se abren tal como están**, sin ningún paso manual. Si
  vienes de 0.6.11 o anterior, la **primera apertura** hace una pasada O(n)
  por cada índice que reconstruye: el índice de propiedades pasa a su formato
  v3 (0.6.11) y se construye el índice de etiquetas (0.6.12). Las aperturas
  siguientes son rápidas. Detalle en la
  [matriz de compatibilidad](MIGRACION_0.6.md#matriz-de-compatibilidad).
- **El volcado del índice HNSW se lee como antes.** Se carga en la **primera
  búsqueda vectorial** de cada modelo, no al abrir. Un volcado escrito por
  0.6.10 o anterior (formato 2) se reconstruye una vez en esa primera
  búsqueda. 0.9.3 trae una copia interna de `hnsw_rs` 0.3.4 con un arreglo de
  deadlock; el código del volcado no cambia.
- **La API de Python solo creció.** Entre 0.6 y 0.9.4 no se quitó ni se
  renombró ningún método ni parámetro de Python.

## 2. Revisa primero tus datos: bugs que escribieron datos incorrectos

Un tercero, listas de enteros pequeños guardadas de forma que vuelven como bytes, se arregla en 0.9.4: ver [Arreglado en 0.9.4](#arreglado-en-094-consultas-que-daban-filas-incorrectas-sin-error).

Los dos están arreglados, pero actualizar no repara lo que ya escribieron.
Solo afectan a escrituras hechas **con NQL**. Las escrituras con la API no se
vieron afectadas: en Python `upsert`, `upsert_many`, `delete` y
`tx.add_node` / `tx.add_edge`; en Rust también `Graph::add_node`,
`Graph::delete_node` y los métodos de la transacción.

### Los números negativos perdían el signo (arreglado en 0.8.3)

Antes de 0.8.3, un número negativo en NQL perdía el `-` en `where`, `set` y
`delete` ([0.8.3](../../CHANGELOG.md#083---2026-10-23)):

| NQL antes de 0.8.3 | Qué hacía |
|---|---|
| `update … set m.temp = -7` | escribía **7** |
| `delete … where m.x = -5` | borraba los nodos con **x = 5** |
| `where m.temp < -1` | encontraba también el 0 |

Las listas (`in [-5]`) y los mapas de propiedades (`{temp: -7}`) no estaban
afectados.

**Cómo revisarlo:** busca en tu código las cadenas NQL con un número negativo
literal (por ejemplo, con la expresión regular `[=<>,(]\s*-\d`). En cada `set`
con valor negativo, lo guardado es el positivo: vuelve a escribirlo desde tu
fuente de verdad. En cada `delete … where … = -N`, se borraron en su lugar los
nodos con `N`.

### DELETE y UPDATE con patrón escribían de más (arreglado en 0.8.4)

Antes de 0.8.4, `delete` y `update` con patrón usaban solo las etiquetas y el
tipo de la arista ([0.8.4](../../CHANGELOG.md#084---2026-10-23)). Ignoraban la
dirección, las propiedades inline y todos los saltos después del primero:

- `delete (c:Chunk)<-[:MENTIONS]-(e:Entity)` borraba las aristas `c -> e`,
  aunque no hubiera ninguna en ese sentido;
- `delete …-[:MENTIONS {w: 1}]->…` borraba también las aristas con `w = 2`;
- un patrón de dos saltos se recortaba al primero.

**Cómo revisarlo:** busca en tu código `delete (` y `update (` seguidos de un
patrón con `<-`, `<->`, `{…}` en una arista o más de un salto. Si usaste
alguno, pueden faltar o haber cambiado aristas o propiedades fuera del
patrón. En Rust, `Graph::edge_history`, `Graph::history` y
`Graph::as_of(timestamp)` muestran las versiones anteriores si la recolección
de basura no las ha eliminado. Python no tiene API de time-travel, así que
reconstruye desde tu fuente de verdad.

Desde 0.8.4, un patrón significa lo mismo al leer y al escribir, y un patrón
de escritura con más de un salto o con cuantificador da error.

## 3. Consultas que devuelven otras filas con el mismo texto

Son arreglos: el resultado nuevo es el correcto. Si tu aplicación, un test o
un resultado guardado dependen del anterior, verán la diferencia.

| Desde | Consulta | Antes | Ahora |
|---|---|---|---|
| [0.7.0](../../CHANGELOG.md#070---2026-10-21) | `where hybrid(c, …, k = 10) and c.tenant = "t7"` (también `similar_to`, `c.id in [...]` y props inline) | puntuaba la etiqueta entera y luego descartaba los demás tenants: menos de `k` filas, a menudo ninguna | los 10 mejores **dentro** del tenant; `rank(c)` / `score(c)` calculados en ese universo |
| 0.7.0 | `where hybrid(…) or c.x = "B"` | devolvía solo los hits de la búsqueda, sin aviso | error con nombre: una búsqueda tiene que ser una condición del `and` raíz |
| [0.8.4](../../CHANGELOG.md#084---2026-10-23) | `(e:Entity)<-[:MENTIONS]-(c:Chunk)` (un salto) | ignoraba `<-`: filas incorrectas o ninguna | recorre las aristas entrantes, como los patrones de varios saltos |
| 0.8.4 | `-[:T {w: 1}]->` en un patrón de dos o más saltos | encontraba también `w = 2` | solo `w = 1` |
| 0.8.4 | `find distinct r.region … limit 3` | el límite antes del `distinct`: podía dar 1 fila | 3 filas distintas |
| [0.8.3](../../CHANGELOG.md#083---2026-10-23) | `where a and (b or c)` | error de parseo | agrupa |
| 0.8.3 | `r.p * 2`, `-expr`, `!x`, `not (…)` | se ignoraban en silencio (`r.p * 2 = 6` corría como `r.p = 6`; `not (…)` dejaba pasar todas las filas) | **error con nombre** |

### Arreglado en 0.9.4: consultas que daban filas incorrectas sin error

Una revisión de la documentación contra 0.9.3 encontró más casos. Si usas
0.9.3, estas son las consultas que conviene volver a revisar al actualizar:

| Consulta | 0.9.3 | 0.9.4 |
|---|---|---|
| `find u.name from ()-[:F]->(u)`: un extremo anónimo ([#293](https://github.com/sharop/nopaldb/issues/293)) | `u` era el **origen** | `u` es el destino |
| `(a)-[:F]->(b)-[:F]->(a)`: una variable repetida (#293) | caminos que no cierran | la segunda `a` es la primera |
| `where p.active`: una propiedad, variable o literal sueltos ([#294](https://github.com/sharop/nopaldb/issues/294)) | `find` dejaba pasar todos los nodos; `delete`/`update` no tocaban ninguno | error con nombre: escribe `p.active = true` |
| `find degree(n) as dg … where dg > 1`: un alias de la proyección (#294) | 0 filas; con el alias proyectado, `where degree(n) > 1` se ignoraba | el alias es su expresión |
| `avg(r.w) as m … having m > 0.5` (#294) | ninguna fila | filtra los grupos |
| `export parquet "f"` / `export arrow` ([#295](https://github.com/sharop/nopaldb/issues/295)) | respondía "Exported" sin escribir nada | escribe el archivo |
| `export csv with delimiter=";"` (#295) | opción ignorada, comas | error con nombre que lista las opciones (`separator`) |
| `path_anomaly_score` con una referencia de otra dimensión ([#297](https://github.com/sharop/nopaldb/issues/297)) | `1.0` para todos los caminos | error con nombre con las dos dimensiones |
| `find … group by p.town having count(*) > 1` sin aristas ([#299](https://github.com/sharop/nopaldb/issues/299)) | todos los grupos | solo los grupos que cumplen |
| `find … at timestamp 3` ([#298](https://github.com/sharop/nopaldb/issues/298)) | el estado actual | error con nombre (el time travel es la API de Rust) |

Fuera de NQL: con `Serializable`, una transacción descartada o revertida con
`rollback()` conservaba sus candados de nodo, y el siguiente escritor fallaba
con `Lock timeout` a los 5 s ([#296](https://github.com/sharop/nopaldb/issues/296)).
0.9.4 los suelta en toda salida.

**Listas que volvían como bytes** ([#300](https://github.com/sharop/nopaldb/issues/300)). Una lista de enteros 0–255 (`[1, 2]`), una tupla de ellos o una lista vacía (`[]`) se guardaban de forma que volvían como `bytes`; una lista de pares `[texto, valor]` volvía como dict, y un dict vacío como `bytes`. 0.9.4 escribe esas formas sin ambigüedad. **Lo guardado así antes de 0.9.4 sigue leyéndose como `bytes`** (en disco no se distingue de unos bytes de verdad): reescríbelo, por ejemplo leyendo el nodo y volviendo a escribir la propiedad como lista. Una base escrita por 0.9.4 que contenga uno de esos valores no la abre 0.9.3 o anterior. Las sentencias `sketch` / `commit` por
`execute_nql` respondían un mensaje OK y ahora son error.

Además, desde 0.6.13 la búsqueda híbrida con filtros selectivos devuelve más
de los documentos permitidos (`search_hybrid` y `hybrid()` en NQL): aparecen
resultados que antes faltaban. Si cortas por un umbral de `score(c)` con
`hybrid`, espera más filas por encima.

Qué significa una consulta está escrito en
[SEMANTICA_NQL.md](SEMANTICA_NQL.md): filas por camino, el universo de la
búsqueda y el orden de evaluación.

## 4. Exportación Arrow: otras columnas y valores (0.9.1)

`to_arrow_complete` en Python, y en Rust `graph_to_arrow`,
`nodes_to_arrow_with_properties` y `edges_to_arrow_with_properties`, daban
datos incorrectos o incompletos
([0.9.1](../../CHANGELOG.md#091---2026-10-24)). Ahora:

- con una etiqueta, se exportan solo las aristas con los dos extremos dentro
  de ella: el subgrafo inducido. Antes salían todas las aristas del grafo;
- las columnas de propiedades van ordenadas por nombre, así que el mismo grafo
  da el mismo esquema;
- listas, objetos y bytes van en JSON, no como `<complex>` / `<bytes:N>`, y un
  `Null` explícito es un nulo de Arrow, no el texto `"null"`;
- una propiedad con nombre de columna estructural (`id`, `label`; `source`,
  `target`, `edge_type` en aristas) sale como `property.<nombre>`;
- una columna con tipos mezclados es texto en lugar de perder valores;
- una exportación vacía devuelve un lote vacío con su esquema en lugar de un
  error.

**Revisa:** el código que lee columnas por posición, compara con `<complex>` o
`"null"`, o lee de la exportación una propiedad llamada `id` / `label`.

## 5. Cambios en la API de Rust

- **Retirado en 0.9.0** ([0.9.0](../../CHANGELOG.md#090---2026-10-24)):

  | Retirado | Usa en su lugar |
  |---|---|
  | `Graph::update_node_with_tx` (no hacía nada) | `tx.add_node(node)` con el mismo id |
  | `Graph::delete_node_with_tx`, `Graph::add_node_with_label_and_props` (ignoraban la transacción) | `tx.delete_node(id)` / `tx.add_node(...)`, o `Graph::delete_node` / `Graph::add_node` |
  | `mvcc::VersionManager` (placeholder vacío) | `Graph::get_node_at`, `Graph::history` |
  | `HnswIndex::repaired()` (siempre 0) | `HnswIndex::orphans()` |

- **`ExpandOptions` tiene un campo nuevo, `within`** (0.9.0). El código que la
  construye con `..Default::default()` no cambia; un literal que nombre todos
  los campos sin él deja de compilar.
- **`hnsw_rs` deja de ser dependencia** (0.9.3). Ninguna API de NopalDB
  exponía sus tipos. Si tu crate usaba `hnsw_rs` sin declararlo, porque
  llegaba a través de NopalDB, agrégalo a tu propio `Cargo.toml`.
- **No actives `hnsw-simd`.** No compila fuera de x86 (un bug de `anndists`),
  y la distancia coseno de NopalDB no tiene camino SIMD, así que no acelera
  nada.
- **`import_parquet`** (0.8.5) importa ahora todas las filas en una
  transacción y rechaza un archivo con columnas distintas de `id`, `label` y
  `property_count`, con nulos, con ids inválidos o repetidos, o con ids que ya
  existen. Antes se quedaba con las primeras 1 024 filas y reemplazaba
  los nodos existentes por nodos vacíos.

## 6. Bases sled

Una base creada con 0.5.x (o con `engine="sled"` en 0.6–0.7) ya no abre desde
0.8.0: el error lo dice y da la receta. Mígrala una vez con 0.7 y abre la
copia con 0.9.4. Ver [MIGRACION_0.6.md](MIGRACION_0.6.md).

## 7. Después de actualizar: qué probar

Corre esto sobre una copia de tu base real. La lista también es lo que te
pedimos que reportes:

1. **Abrir y contar.** La base abre, y el número de nodos y aristas coincide
   con la versión anterior.
2. **Búsqueda vectorial tras reiniciar.** Corre `knn_nodes` o
   `search_hybrid`, cierra la base, reábrela y corre la misma consulta: los
   mismos ids. Esto comprueba el volcado HNSW, que se carga en la primera
   búsqueda vectorial después de abrir.
3. **Búsqueda híbrida con tus filtros reales** (tenant, documento, ACL): el
   número de filas, y si ahora vuelven `k` filas donde antes volvían menos.
   `search_hybrid_explain` / `explain` en NQL muestran el camino tomado.
4. **Tus escrituras NQL** (`update … set`, `delete` con patrón) sobre una
   copia: cambian exactamente lo que esperas. Revisa antes las secciones 2 y
   3.
5. **Exportaciones** (Arrow, NetworkX) que alimentan otra herramienta: el paso
   siguiente sigue leyendo las columnas que necesita.
6. **Tiempos** de las operaciones que te importan (ingesta, construcción del
   índice, consultas), versión anterior contra la nueva.

**Qué reportar:** la versión desde la que actualizaste; el sistema operativo y
el procesador (Linux, macOS o **Windows**, x86_64 o ARM); qué falló, con el
texto del error; y los tiempos del punto 6. Los reportes desde Windows son
especialmente útiles: los tests solo corren en Linux, y las wheels de Windows
se compilan pero no se prueban.
