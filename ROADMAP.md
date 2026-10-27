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

Done in 0.6.0: redb is the default engine and sled stays available behind
`storage-sled` as a read/migration engine for at least 0.6 and 0.7, so nobody
is forced to migrate on upgrade — an existing sled database opens as before
(`StorageEngine::Auto` detects it) and `Graph.migrate` / `copy_database`
convert it with verification. The switch was gated on evidence: four weeks of
nightly crash harness on redb without incidents, a verified round trip of a
million-pair database, and the benchmark set at or above sled (reads,
transactional commits, bulk ingest, GC), with the two remaining trade-offs
measured and documented in [DURABILITY.md](docs/DURABILITY.md). Done in
0.8.0: sled was removed (#141); redb is the only engine, and 0.5.x databases
are migrated once with 0.7. Migration guide: [MIGRATION_0.6.md](docs/MIGRATION_0.6.md).

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
- **Declarative SHACL** (shipped, 0.5.14–0.5.17): shapes load from a standard
  `.ttl` with a report of what is not checked
  ([#98](https://github.com/sharop/nopaldb/issues/98)); `sh:targetClass`
  honours the class hierarchy ([#99](https://github.com/sharop/nopaldb/issues/99));
  `sh:and`/`sh:or`/`sh:not`/`sh:xone`, `sh:node`, `sh:severity`/`sh:message`
  ([#100](https://github.com/sharop/nopaldb/issues/100)); sequence paths
  `sh:path ( :a :b )` ([#101](https://github.com/sharop/nopaldb/issues/101)).
  Not covered, and reported as such when a shape uses them: inverse,
  alternative and closure paths, `sh:closed`, qualified shapes, property
  comparisons, SHACL-SPARQL. See `docs/SHACL.md`.

### 3. Retrieval honesty and control

Hybrid search explains why each hit ranked where it did, and full-text indexes
take a configurable analyzer — stemming, stopwords, accent folding
([#74](https://github.com/sharop/nopaldb/issues/74), shipped in 0.5.13). The
NQL `hybrid()` function takes `rrf_k`, `ef_search`, `overfetch` and
`text_index` ([#115](https://github.com/sharop/nopaldb/issues/115), shipped in
0.5.19), and since 0.6.9 both `similar_to` and `hybrid()` accept a literal
vector and seed a one-hop pattern, so search and expansion fit in one query.
The follow-ups once listed here are done
([#174](https://github.com/sharop/nopaldb/issues/174),
[#176](https://github.com/sharop/nopaldb/issues/176)): `hybrid()` no longer
scans the label and `similar_to` returns K rows when K exist (0.6.10); NQL has
`score(var)` (0.6.10); `neighborhood` can rank by Personalized PageRank
(`rank="ppr"`, 0.6.10); and the NQL `where` is the search universe of
`hybrid()` / `similar_to`, so property filters apply before the top-k (0.7.0).
What shipped after that — GraphRAG with Leiden communities and community
reports, `CandidateSet` (#214), the NQL semantic contracts, the NetworkX
adapter — is in the [CHANGELOG](CHANGELOG.md).

### 4. Vector index durability

Done: a new embedding is inserted into the cached index instead of
invalidating it ([#113](https://github.com/sharop/nopaldb/issues/113), shipped
in 0.5.19), the index is persisted and loaded on reopen
([#114](https://github.com/sharop/nopaldb/issues/114), shipped in 0.5.20), and
`benches/hnsw_ops.rs` measures build, insert, search and reopen
([#112](https://github.com/sharop/nopaldb/issues/112)). Embeddings load in
batches with `add_node_embeddings` ([#175](https://github.com/sharop/nopaldb/issues/175),
shipped in 0.6.10). Still open: measuring retrieval quality and latency with
real text at the dimensions real models use (384, 1024), not only the
64-dimension synthetic set ([#212](https://github.com/sharop/nopaldb/issues/212)).

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
  process.** The storage engine (redb) is single-process by design, and there are no
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

Hecho en 0.6.0: redb es el motor por defecto y sled sigue disponible tras
`storage-sled` como motor de lectura/migración al menos en 0.6 y 0.7, para que
nadie se vea forzado a migrar al actualizar: una base sled existente abre como
antes (`StorageEngine::Auto` la detecta) y `Graph.migrate` / `copy_database`
la convierten con verificación. El cambio se decidió con evidencia: cuatro
semanas de harness de crash nightly sobre redb sin incidentes, un round trip
verificado de una base de un millón de pares y el set de benchmarks igual o
mejor que sled (lecturas, commits transaccionales, ingesta, GC), con los dos
trade-offs restantes medidos y documentados en [DURABILITY.md](docs/DURABILITY.md).
Hecho en 0.8.0: sled se retiró (#141); redb es el único motor y las bases de
0.5.x se migran una vez con 0.7. Guía: [MIGRACION_0.6.md](docs/es/MIGRACION_0.6.md).

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
- **SHACL declarativo** (liberado, 0.5.14–0.5.17): las shapes se cargan desde
  un `.ttl` estándar con reporte de lo que no se comprueba
  ([#98](https://github.com/sharop/nopaldb/issues/98)); `sh:targetClass`
  respeta la jerarquía de clases ([#99](https://github.com/sharop/nopaldb/issues/99));
  `sh:and`/`sh:or`/`sh:not`/`sh:xone`, `sh:node`, `sh:severity`/`sh:message`
  ([#100](https://github.com/sharop/nopaldb/issues/100)); secuencias
  `sh:path ( :a :b )` ([#101](https://github.com/sharop/nopaldb/issues/101)).
  No cubierto, y reportado como tal cuando una shape lo usa: paths inversos,
  alternativos y cierres, `sh:closed`, qualified shapes, comparaciones entre
  propiedades, SHACL-SPARQL. Ver `docs/SHACL.md`.

### 3. Honestidad y control del retrieval

La búsqueda híbrida explica por qué cada hit quedó donde quedó, y los índices
full-text aceptan un analyzer configurable —stemming, stopwords, plegado de
acentos ([#74](https://github.com/sharop/nopaldb/issues/74), entregado en
0.5.13)—. La función `hybrid()` de NQL acepta `rrf_k`, `ef_search`,
`overfetch` y `text_index` ([#115](https://github.com/sharop/nopaldb/issues/115),
entregado en 0.5.19), y desde 0.6.9 `similar_to` y `hybrid()` aceptan un vector
literal y siembran un patrón de un salto (de dos desde 0.6.19, #231), así que
buscar y expandir cabe en una consulta. Los pendientes que antes se listaban aquí están hechos
([#174](https://github.com/sharop/nopaldb/issues/174),
[#176](https://github.com/sharop/nopaldb/issues/176)): `hybrid()` ya no
recorre la etiqueta y `similar_to` devuelve K filas cuando existen (0.6.10);
NQL tiene `score(var)` (0.6.10); `neighborhood` puede rankear con Personalized
PageRank (`rank="ppr"`, 0.6.10), y el `where` de NQL es el universo de
`hybrid()` / `similar_to`, así que los filtros de propiedad se aplican antes
del top-k (0.7.0). Lo entregado después —GraphRAG con comunidades Leiden y
reportes de comunidad, `CandidateSet` (#214), los contratos semánticos de NQL,
el adaptador NetworkX— está en el [CHANGELOG](CHANGELOG.md).

### 4. Durabilidad del índice vectorial

Hecho: un embedding nuevo se inserta en el índice cacheado en vez de
invalidarlo ([#113](https://github.com/sharop/nopaldb/issues/113), entregado en
0.5.19), el índice se persiste y se carga al reabrir
([#114](https://github.com/sharop/nopaldb/issues/114), entregado en 0.5.20), y
`benches/hnsw_ops.rs` mide construcción, inserción, búsqueda y reapertura
([#112](https://github.com/sharop/nopaldb/issues/112)). Los embeddings se
cargan por lotes con `add_node_embeddings`
([#175](https://github.com/sharop/nopaldb/issues/175), entregado en 0.6.10).
Sigue abierto: medir calidad y latencia del retrieval con texto real y las
dimensiones de los modelos reales (384, 1024), no solo con el set sintético de
64 dimensiones ([#212](https://github.com/sharop/nopaldb/issues/212)).

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
  de un proceso.** El motor de almacenamiento (redb) es de un solo proceso por diseño y no hay
  primitivas de tenant. El patrón soportado es un directorio por tenant y un
  único proceso curador por directorio; para leer una base viva desde otro
  lado, backup en frío y abrir la copia (`docs/BACKUP_AND_READ_ONLY.md`).
