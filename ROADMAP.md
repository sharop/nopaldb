# NopalDB Roadmap

*(Español más abajo.)*

This is direction, not a schedule: themes we are actively working toward, in
the order we expect to land them, and — just as important — what NopalDB will
**not** become. Nothing here is a date commitment. Progress is tracked in
[issues](https://github.com/sharop/nopaldb/issues) by label (`rdf`, `retrieval`,
`durability`, `reliability`); shipped work is in the [CHANGELOG](CHANGELOG.md).

NopalDB is an embedded property graph with native embeddings, MVCC
transactions and a query language of its own (NQL). Everything below serves
that shape.

## Themes

### 1. Storage hardening: switching the default engine

Two storage engines ship today: sled (default) and redb (experimental, behind
`storage-redb`), with verified migration between them. The plan is to make redb
the default in the next minor release, keeping sled available as a
read/migration engine for at least two minors afterwards so nobody is forced
to migrate on upgrade.

The switch is gated on evidence, not on a date: a multi-week nightly soak on
redb without incidents, a verified round trip of a large database, and no
performance regression on the benchmark set. Until the gate passes, redb stays
opt-in and the prebuilt Python wheels keep shipping sled only.

### 2. Semantic cycle: a faithful Turtle bridge, then declarative SHACL

The current Turtle import/export is an ontology loader, not an RDF bridge —
its exact limits are documented in `docs/ADOPTION.md` and in the `rdf_owl`
module docs. The work is to make it faithful:

- **Importer** ([#69](https://github.com/sharop/nopaldb/issues/69), shipped
  in 0.5.10): a real Turtle grammar that reports errors instead of silently
  mis-parsing, IRI identity (no collapsing of same-named terms), an edge for
  every resource-valued triple, and one `instanceOf` edge per `rdf:type`.
- **`instanceOf` in NQL reads those edges**
  ([#93](https://github.com/sharop/nopaldb/issues/93), shipped in 0.5.11): the
  predicate accepts a label, `prefix:Local` or a full IRI and answers for every
  type of a node, not just the first one, so one vocabulary means the same
  thing as a predicate and as a traversal. Direct instances count too, which
  they did not before.
- **Exporter** ([#70](https://github.com/sharop/nopaldb/issues/70), shipped in
  0.5.12): symmetric with the importer, emitting the document's own namespaces
  and all edges, with a verified import → export → import round trip and an
  `ExportReport` that lists what Turtle cannot carry; export exposed in Python.
- **Declarative SHACL**: the constraint engine exists; what is missing is
  loading shapes from a standard `.ttl`, `sh:targetClass` that honours the
  class hierarchy, logical constraints (`sh:and`/`sh:or`/`sh:not`) and
  multi-hop paths.

### 3. Retrieval honesty and control

Hybrid search already explains why each hit ranked where it did. Still open:
a configurable full-text analyzer per index — stemming, stopwords, accent
normalization ([#74](https://github.com/sharop/nopaldb/issues/74)) — and wiring
the `hybrid()` NQL function to the same parameters the Rust and Python APIs
take (`ef_search`, `rrf_k`, filters), which are fixed constants today.

### 4. Vector index durability

The HNSW index is rebuilt from stored vectors when needed, and a new embedding
invalidates the model's index, so the next search pays a full rebuild. The
work is incremental insertion, persistence of the index across restarts, and
a first benchmark set so the two can be measured instead of assumed. This
theme is scheduled after the storage switch, because the persistence design
depends on the engine that stays.

## Out of scope

Decided, not deferred. Each item comes with what we recommend instead.

- **SPARQL, named graphs, RDF-star, being a triple store.** Competing with
  dedicated RDF stores is a multi-year effort orthogonal to what NopalDB is.
  Need SPARQL? Keep a triple store as the system of record and materialize
  into NopalDB the subgraph you want to query, reason over or embed — that is
  what the Turtle bridge above is for.
- **A network server in this repository.** NopalDB is embedded; the process
  that opens the database owns it. To share one database across clients, put
  the [MCP server](https://github.com/Anxious-Mind-Group/nopaldb-mcp) or your
  own service in front of it.
- **Several processes on one data directory, or multi-tenancy inside one
  process.** Both engines are single-process by design, and there are no
  tenant primitives. The supported pattern is one data directory per tenant
  and a single curator process per directory; to read a live database from
  elsewhere, take a cold backup and open the copy
  (`docs/BACKUP_AND_READ_ONLY.md`).

---

# Roadmap de NopalDB

Esto es dirección, no calendario: los temas en los que trabajamos, en el orden
en que esperamos entregarlos, y —igual de importante— lo que NopalDB **no** va
a ser. Nada de aquí es un compromiso de fecha. El avance se sigue en
[issues](https://github.com/sharop/nopaldb/issues) por label (`rdf`,
`retrieval`, `durability`, `reliability`); lo entregado está en el
[CHANGELOG](CHANGELOG.md).

NopalDB es un property graph embebido con embeddings nativos, transacciones
MVCC y su propio lenguaje de consulta (NQL). Todo lo de abajo sirve a esa forma.

## Temas

### 1. Endurecimiento del storage: cambio del motor por defecto

Hoy se distribuyen dos motores: sled (por defecto) y redb (experimental, tras
`storage-redb`), con migración verificada entre ambos. El plan es hacer de redb
el default en el siguiente minor, manteniendo sled como motor de
lectura/migración al menos dos minors más para que nadie se vea forzado a
migrar al actualizar.

El cambio se decide con evidencia, no con fecha: un soak nightly de varias
semanas sobre redb sin incidentes, un round trip verificado de una base grande
y cero regresión en el set de benchmarks. Hasta que el gate pase, redb sigue
siendo opt-in y las wheels de Python traen solo sled.

### 2. Ciclo semántico: puente Turtle fiel y luego SHACL declarativo

El import/export Turtle actual es un cargador de ontologías, no un puente RDF:
sus límites exactos están en `docs/ADOPTION.md` y en el doc del módulo
`rdf_owl`. El trabajo es volverlo fiel:

- **Importer** ([#69](https://github.com/sharop/nopaldb/issues/69), liberado en
  0.5.10): gramática Turtle real que reporta errores en vez de parsear mal en
  silencio, identidad IRI (sin colapsar términos homónimos), una arista por cada
  triple con objeto-recurso y una arista `instanceOf` por cada `rdf:type`.
- **`instanceOf` en NQL lee esas aristas**
  ([#93](https://github.com/sharop/nopaldb/issues/93), liberado en 0.5.11): el
  predicado acepta label, `prefijo:Local` o IRI completo y responde por todos
  los tipos de un nodo, no solo el primero, para que un mismo vocabulario
  signifique lo mismo como predicado y como traversal. Las instancias directas
  también cuentan, cosa que antes no pasaba.
- **Exporter** ([#70](https://github.com/sharop/nopaldb/issues/70), liberado en
  0.5.12): simétrico al importer, con los namespaces del documento y todas las
  aristas, round trip import → export → import verificado y un `ExportReport`
  que lista lo que Turtle no puede llevar; export expuesto en Python.
- **SHACL declarativo**: el motor de constraints existe; falta cargar shapes
  desde un `.ttl` estándar, `sh:targetClass` consciente de la jerarquía de
  clases, constraints lógicas (`sh:and`/`sh:or`/`sh:not`) y paths multi-salto.

### 3. Honestidad y control del retrieval

La búsqueda híbrida ya explica por qué cada hit quedó donde quedó. Sigue
abierto: un analyzer full-text configurable por índice —stemming, stopwords,
normalización de acentos ([#74](https://github.com/sharop/nopaldb/issues/74))—
y cablear la función `hybrid()` de NQL a los mismos parámetros que aceptan las
APIs de Rust y Python (`ef_search`, `rrf_k`, filtros), hoy constantes fijas.

### 4. Durabilidad del índice vectorial

El índice HNSW se reconstruye desde los vectores almacenados cuando hace
falta, y un embedding nuevo invalida el índice de su modelo, así que la
siguiente búsqueda paga la reconstrucción completa. El trabajo es inserción
incremental, persistencia del índice entre reinicios y un primer set de
benchmarks para medir en vez de suponer. Este tema va después del cambio de
motor, porque el diseño de persistencia depende del motor que se queda.

## Fuera del alcance

Decidido, no pospuesto. Cada punto viene con lo que recomendamos en su lugar.

- **SPARQL, named graphs, RDF-star, ser un triple store.** Competir con los
  stores RDF dedicados es un esfuerzo de años, ortogonal a lo que NopalDB es.
  ¿Necesitas SPARQL? Mantén un triple store como fuente de verdad y materializa
  en NopalDB el subgrafo que quieres consultar, razonar o embeber: para eso es
  el puente Turtle de arriba.
- **Un servidor de red en este repositorio.** NopalDB es embebido: el proceso
  que abre la base es su dueño. Para compartir una base entre clientes, pon
  delante el [servidor MCP](https://github.com/Anxious-Mind-Group/nopaldb-mcp)
  o tu propio servicio.
- **Varios procesos sobre un mismo directorio de datos, o multi-tenant dentro
  de un proceso.** Ambos motores son de un solo proceso por diseño y no hay
  primitivas de tenant. El patrón soportado es un directorio por tenant y un
  único proceso curador por directorio; para leer una base viva desde otro
  lado, backup en frío y abrir la copia (`docs/BACKUP_AND_READ_ONLY.md`).
