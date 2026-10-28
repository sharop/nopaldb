# Actualizar a 0.10

> English: [docs/UPGRADING_0.10.md](../UPGRADING_0.10.md).

Esta página es para una aplicación que usa NopalDB **0.9.4** y pasa a
**0.10.0**. Si vienes de 0.9.3 o anterior, lee antes
[ACTUALIZAR_0.9.md](ACTUALIZAR_0.9.md): todo lo de allí sigue aplicando.

0.10 corrige la historia MVCC (las versiones detrás de `history`, `as_of` y
`get_node_at`), el GC y la durabilidad del `update` de NQL. Es una versión
menor porque cambiaron dos APIs de Rust de forma incompatible (sección 2).
Cada punto enlaza a su issue.

Instalación:

```bash
pip install -U "nopaldb==0.10.0"   # Python 3.10+
```

```toml
nopaldb = "0.10"
```

## 1. Lo que no cambia

- **Las bases escritas por 0.9.4 se abren tal como están.** El formato en
  disco de nodos, aristas, índices y WAL es el mismo; un WAL pendiente que
  dejó un crash de 0.9.4 lo reproduce bien 0.10.
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
  | el nodo no existía en `ts` (creado después, o ya borrado) | `NopalError::NodeNotFound` |
  | el nodo no tiene historia (escrito con `add_node` directo), o `ts` está bajo el horizonte del GC | `NopalError::HistoryUnavailable` (nuevo) |

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

**Los nodos borrados antes de 0.10 conservan su historia abierta.** No hay un
timestamp de borrado registrado con el que repararlos, así que `as_of(ts)`
posterior a su borrado los sigue encontrando. Solo se cierran los borrados
hechos con 0.10.

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
nodo que cambia. Reescribir las propiedades de una arista todavía reemplaza
su versión en lugar de encadenar otra.

## 4. Nuevo

- `Graph::last_commit_timestamp()` (Rust y Python): el timestamp lógico de la
  última escritura aplicada, el valor que se pasa a `as_of` / `get_node_at`.
  No lo mueven `begin_transaction`, los rollbacks ni los commits vacíos.
- `Graph::gc_horizon()` (Rust y Python; también `stats().gc.horizon`,
  `get_stats()["gc"]["horizon"]`): hasta dónde recolectó el GC. A partir de
  ese timestamp la historia está completa.
- `GCConfig::before_timestamp(ts)`: un corte del GC en tiempo lógico.

## 5. Límites que siguen

- **Sin historia para `add_node` directo, el BulkLoader y la importación
  Turtle/OWL.** Estas escrituras todavía no dejan versión, así que el time
  travel sobre ellas responde `HistoryUnavailable`. Versionar todas las
  escrituras es el siguiente paso.
- **`find … at timestamp <n>` de NQL sigue siendo un error con nombre**
  ([#298](https://github.com/sharop/nopaldb/issues/298)). Las lecturas en un
  instante son la API de Rust de arriba; Python todavía no tiene ninguna.

## 6. Volver a 0.9.4

0.9.4 abre una base escrita por 0.10, porque el formato es el mismo. Las
marcas de reloj de pared son una entrada del catálogo que 0.9.4 ignora.
Guarda una copia antes de volver atrás, como siempre.

## 7. Qué probar

Sobre una copia de tu base:
1. abre y los conteos coinciden;
2. un GC por antigüedad que corras con regularidad borra lo que esperas,
   teniendo en cuenta el arranque de arriba;
3. el código que llama a `get_node_at` / `as_of` maneja `NodeNotFound` y
   `HistoryUnavailable`;
4. tus sentencias `update` de NQL siguen cambiando exactamente lo que
   esperas.
