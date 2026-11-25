# Cypher de solo lectura

> Versión en inglés: [../CYPHER.md](../CYPHER.md)

El lenguaje de consulta de NopalDB es NQL ([referencia](NQL_REFERENCIA.md)).
Con la feature opcional `cypher` (apagada por defecto; no forma parte de
`full`) también lee un subconjunto pequeño de openCypher: `MATCH … WHERE …
RETURN … ORDER BY … SKIP … LIMIT`. La consulta Cypher se lee con
[`open-cypher`](https://crates.io/crates/open-cypher) 0.5.0 y se traduce a
texto NQL, que luego corre como cualquier consulta NQL, con el mismo
validador y el mismo ejecutor.

```toml
nopaldb = { version = "0.11", features = ["cypher"] }
```

```rust
// El NQL en que se convierte:
let nql = nopaldb::query::cypher::to_nql(
    "MATCH (c:Cuidador)-[:CUIDA]->(a) WHERE a.edad > 3 RETURN a.nombre ORDER BY a.nombre",
)?;
assert_eq!(nql, "find a.nombre from (c:Cuidador)-[:CUIDA]->(a) where a.edad > 3 order by a.nombre");

// O ejecutarla directamente:
let resultado = graph.execute_cypher("MATCH (a:Animal) RETURN a.zona, count(*) AS n").await?;
```

Lo que queda fuera del subconjunto es un error que nombra la parte
rechazada. Nada se aproxima ni se descarta.

## Qué se acepta

Cada fila se comprueba en `nopaldb/tests/cypher_read_only_test.rs`: el NQL es
lo que devuelve `to_nql`, y el test lo ejecuta sobre un grafo pequeño.

| Cypher | NQL |
| --- | --- |
| `MATCH (a:Animal) WHERE a.edad > 3 RETURN a.nombre` | `find a.nombre from (a:Animal) where a.edad > 3` |
| `MATCH (c:Cuidador)-[:CUIDA]->(a) RETURN a.nombre ORDER BY a.nombre` | `find a.nombre from (c:Cuidador)-[:CUIDA]->(a) order by a.nombre` |
| `MATCH (a:Animal) RETURN a.zona, count(*) AS n ORDER BY n DESC` | `find a.zona, count(*) as n from (a:Animal) order by n desc` |
| `MATCH (a:Animal {nombre: 'michi'})-[:JUEGA_CON*1..2]->(b) RETURN b.nombre` | `find b.nombre from (a:Animal {nombre: "michi"})-[:JUEGA_CON]->{1,2}(b)` |
| `MATCH (a:Animal {nombre: 'tom'})-[:JUEGA_CON]-(b) RETURN b.nombre` | `find b.nombre from (a:Animal {nombre: "tom"})-[:JUEGA_CON]-(b)` |
| `MATCH (a:Animal)<-[:CUIDA]-(c) RETURN DISTINCT c.nombre` | `find distinct c.nombre from (a:Animal)<-[:CUIDA]-(c)` |
| `MATCH (c:Cuidador) MATCH (c)-[:CUIDA]->(a) WHERE a.especie = 'ave' RETURN c.nombre, a.nombre` | `find c.nombre, a.nombre from (c:Cuidador), (c)-[:CUIDA]->(a) where a.especie = "ave"` |
| `MATCH (a:Animal) RETURN a.nombre ORDER BY a.edad DESC, a.nombre SKIP 1 LIMIT 2` | `find a.nombre from (a:Animal) order by a.edad desc, a.nombre limit 2 offset 1` |
| `MATCH (a:Animal) WHERE a.especie IN ['ave', 'reptil'] OR a.edad <> 3 RETURN a.nombre` | `find a.nombre from (a:Animal) where (a.especie in ["ave", "reptil"]) or (a.edad != 3)` |
| `MATCH (a:Animal) RETURN min(a.edad) AS menor, max(a.edad) AS mayor, sum(a.edad) AS total` | `find min(a.edad) as menor, max(a.edad) as mayor, sum(a.edad) as total from (a:Animal)` |
| `MATCH (a:Animal WHERE a.zona = 'norte')-[r:JUEGA_CON]->(b) RETURN type(r) AS t, b.nombre` | `find r.type as t, b.nombre from (a:Animal)-[r:JUEGA_CON]->(b) where a.zona = "norte"` |
| `MATCH (a:Animal) WHERE a.edad IS NULL RETURN a.nombre` | `find a.nombre from (a:Animal) where !(a.edad != null)` |
| `MATCH (a:Cuidador) RETURN a, id(a) AS i` | `find all(a), a.id as i from (a:Cuidador)` |

En detalle:

- **Patrones:** un nodo con cero o una etiqueta y un mapa de propiedades
  literales; relaciones `->`, `<-` y `-` sin dirección, con un tipo o una
  unión `:A|B`; largo variable con las dos cotas (`*2`, `*1..3`, `*..3`);
  varios patrones y varios `MATCH`, que comparten variables. Un `WHERE`
  dentro de un patrón se suma al `WHERE` de la consulta.
- **Expresiones:** literales (strings, enteros en cualquier base, flotantes,
  booleanos, `null`, listas), `n.prop`, `= <> < <= > >=`, `AND OR NOT`, `IN`
  con una lista literal, `+ - * / %`, `IS [NOT] NULL`, y las funciones
  `count` (también `count(*)`), `sum`, `avg`, `min`, `max`, `id`/`elementId`
  y `type`.
- **Proyección:** `RETURN [DISTINCT]` con alias `AS` o `*`; `ORDER BY` con
  `ASC`/`DESC`; `SKIP`/`OFFSET` con `LIMIT`. Los agregados agrupan por las
  demás columnas, como en Cypher.

## Qué se rechaza

| Cypher | Por qué |
| --- | --- |
| `CREATE`, `MERGE`, `SET`, `DELETE`, `REMOVE`, `FOREACH`, `LOAD CSV` | solo lectura; las escrituras son `add`, `upsert`, `update`, `delete` de NQL |
| `OPTIONAL MATCH`, `WITH`, `UNWIND`, `UNION`, `CALL` | sin equivalente en NQL por ahora |
| `$param` | sin parámetros; el valor va en la consulta |
| `STARTS WITH`, `ENDS WITH`, `CONTAINS`, `=~`, `XOR`, `^`, `\|\|` | sin operador en NQL |
| `count(DISTINCT x)`, `collect`, `size`, `labels`, `coalesce`, `toUpper`, … | sin función en NQL |
| `(n:A:B)`, `(n:!A)`, `(n:%)` | en NopalDB un nodo tiene una etiqueta |
| `p = (a)-->(b)`, `shortestPath(…)` | sin valores de camino; para el más corto NQL tiene `find path(a, b) from (a), (b) where …` |
| `-[*]->`, `-[*0..2]->`, `-[*2..]->`, `-[r*1..2]->` | NQL pide al menos un salto y una cota superior, y no liga la lista de aristas |
| `SKIP` sin `LIMIT`, `SKIP`/`LIMIT` no literales | el `offset` de NQL va con `limit` |
| `RETURN n AS m`, `RETURN n, count(*)` | un nodo entero no lleva alias en NQL; agrupa por sus propiedades |
| `` `un nombre` `` con espacios o símbolos | en NQL los nombres son letras, dígitos y `_` |
| `EXPLAIN`, `PROFILE` | traducir con `to_nql` y correr `explain <nql>` |

## Dónde el resultado difiere de Cypher

- **`RETURN n`** da las columnas de `all(n)`: `n.id`, `n.label` y una por
  propiedad, no un objeto nodo.
- **Nombres de columna** de funciones siguen a NQL: `type(r)` es la columna
  `r.type` e `id(n)` es `n.id`. Con `AS` se nombran.
- **Propiedades ausentes.** NQL usa lógica de dos valores: comparar con una
  propiedad ausente es falso, no `null`. Así, `NOT a.edad = 3` conserva los
  nodos sin `edad`, que Cypher descarta. Para el resultado de Cypher:
  `a.edad IS NOT NULL AND NOT a.edad = 3`.
- **`sum` de enteros** es flotante (`8.0`, no `8`).
- **Nodos de clase.** Desde 0.11.5 `(n:Etiqueta)` no admite nodos de clase
  (`NodeKind::Class`, p. ej. un `owl:Class` importado); en NQL se piden con
  `is_class(n)`.
- **Ids** son los de NopalDB (strings UUID), no enteros.

Los null se ordenan como en Cypher: al final con `ASC`, al principio con
`DESC`.

## Cobertura

De las 994 consultas de lectura del TCK de openCypher 2024.3 que empiezan
con `MATCH`, 237 se traducen y 231 de ellas corren en NQL. Las otras seis
fallan con un error de NQL, por ejemplo un agregado en `ORDER BY` que no es
columna. Los rechazos más comunes son `WITH` (284 consultas) y las variables
de camino (193). Es un corpus de sintaxis: dice qué formas de consulta
pasan, no que las filas coincidan con las de Cypher (eso lo comprueban los
tests de arriba).
