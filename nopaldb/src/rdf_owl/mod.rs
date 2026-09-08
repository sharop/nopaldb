//! Turtle/OWL bridge: load an ontology and its data into the property graph,
//! and write the ontological part of the graph back out as Turtle.
//!
//! The import is a **materialization**, not an RDF store: NopalDB keeps no
//! triples, has no SPARQL and no named graphs. Keep the triple store as the
//! system of record and materialize here the subgraph you want to query,
//! reason over or embed. What follows is the exact contract.
//!
//! # Identity
//!
//! Every imported node carries its absolute IRI in the `iri` property, and
//! that IRI is its identity: a class or individual with the same IRI is
//! reused, never duplicated, and importing the same document twice creates
//! nothing. Labels, edge types and property names are the *readable* half of
//! a term — its local name (after `#` or the last `/`) — and are never used
//! to decide identity. When two classes from different namespaces share a
//! local name, the second one is labelled with its qualified name
//! (`fauna:Rosa`), so a lookup by label still tells them apart; the import
//! reports it. In NQL, `instanceOf(n, C)` names the class by label,
//! `prefix:Local` or full IRI, and reads every `instanceOf` edge of the node.
//!
//! # What the import keeps
//!
//! | Turtle | Graph |
//! |--------|-------|
//! | `:X a owl:Class` | node `X`, `NodeKind::Class`, `iri` |
//! | `:X rdfs:subClassOf :Y` | edge `X → Y` of type `subClassOf`, plus the taxonomy index |
//! | `:x a :X` (every type) | node with label = local name of the **first** type, `iri`; one edge `x → X` of type `instanceOf` per type |
//! | `:x :p :y` (resource object) | edge `x → y` of type `p` (local name), with the predicate IRI in the edge's `iri` property |
//! | `:x :p "literal"` | property `p` on the node, typed by datatype (table below); the same predicate twice → a `List` |
//! | `rdfs:label` / `rdfs:comment` on anything | properties `rdfs_label` / `rdfs_comment` (`label` is the `Node` field, and `n.label` in NQL must keep meaning that) |
//! | `:x a :Z` with `Z` never declared | the class `Z` is created, and the import reports it |
//! | `:x :p :y` with `:y` never described | a **placeholder** node for `:y` (`iri`, `rdf_placeholder = true`), upgraded in place when a later document types it — edges pointing at it stay valid |
//! | `:x a owl:NamedIndividual` | consumed as a no-op |
//!
//! Reserved edge types: `rdf:type` becomes `instanceOf` and `rdfs:subClassOf`
//! stays `subClassOf`. A user predicate whose local name is one of those is
//! written under its qualified name (`ex:subClassOf`) and reported, so the
//! taxonomy is never fed a data edge. The taxonomy rebuilt on every
//! `Graph::open` also only accepts `subClassOf` edges between two classes.
//!
//! Prefixes declared by the imported documents are kept in the graph catalog
//! (`Graph::rdf_prefixes`), merged across imports with the latest declaration
//! winning.
//!
//! # What the import skips
//!
//! Reserved vocabulary the importer does not model, and it skips it whole:
//! `owl:Ontology` headers and every statement about them, property
//! declarations (`owl:ObjectProperty`, `owl:DatatypeProperty`, …),
//! `rdfs:domain` / `rdfs:range` / `rdfs:subPropertyOf`, `owl:equivalentClass`
//! and the other OWL axioms. Each adds one to
//! `ImportReport::triples_skipped`. Everything in a user namespace is kept,
//! so a non-zero count is a list of axioms, never of data.
//!
//! What is kept but *narrowed*: language tags (`"rosa"@es` lands as the
//! string `rosa`), datatypes other than integer/decimal/boolean (`xsd:date`
//! lands as its text), and edge properties — RDF has no such thing, so an
//! edge imported from Turtle carries only the predicate IRI.
//!
//! # Parsing
//!
//! Parsing is done by `oxttl`, a full Turtle grammar: `a`, `@prefix`/`PREFIX`,
//! `@base`, language tags, blank nodes (`_:b` and `[ … ]`), collections,
//! multi-line literals. **Malformed input is an error** with line and column
//! ([`crate::NopalError::RdfParseError`]) and nothing is written.
//!
//! Two things the parser needs that a document may lack are assumed and
//! reported in [`importer::ImportReport::warnings`]: an empty prefix (`:Foo`)
//! without `@prefix :` resolves against [`importer::DEFAULT_NAMESPACE`], and
//! a relative IRI without `@base` against [`importer::DEFAULT_BASE`].
//!
//! Literals map to property values by datatype, not by the shape of the text:
//!
//! | datatype | `PropertyValue` |
//! |---|---|
//! | `xsd:integer` family (`int`, `long`, `short`, `byte`, unsigned, `nonNegativeInteger`, …) | `Int` |
//! | `xsd:decimal`, `xsd:double`, `xsd:float` | `Float` |
//! | `xsd:boolean` | `Bool` |
//! | `xsd:string`, plain literal, language-tagged literal, anything else | `String` (lexical value) |
//!
//! A typed literal whose text does not parse as its type is kept as `String`
//! and reported. Blank nodes get a synthetic identity scoped to the document
//! (`_:<hash>-<label>`), so re-importing the same file is idempotent and two
//! files that both say `_:b0` never collide.
//!
//! # Databases imported before 0.5.10
//!
//! The first importer identified everything by label and stored the raw token
//! (`:Alice`) in `iri`. Such nodes still export correctly. A new import next
//! to them does not merge: a class without `iri` under the same label is
//! adopted (it gets its IRI), but an individual with a raw token is a
//! different identity from the same individual with its absolute IRI, and the
//! import reports how many it saw. The upgrade path is to re-import into a
//! fresh database.
//!
//! # What the export writes
//!
//! `export_turtle` is the mirror of the import. For every node with an `iri`
//! (classes and individuals; ordinary NopalDB nodes stay out, so a mixed graph
//! exports clean RDF) it writes:
//!
//! | In the graph | In the document |
//! |---|---|
//! | Class node | `a owl:Class`, its literals (`rdfs_label` → `rdfs:label`) |
//! | `subClassOf` edge between two classes | `rdfs:subClassOf` |
//! | `instanceOf` edge | `a <C>`, one per edge (no edges: the label is the class; a placeholder gets no type) |
//! | any other edge to an exported node | one triple, predicate = the edge's `iri`, or the export namespace + edge type when it has none |
//! | `Int` / `Float` / `Bool` / `String` | `xsd:integer` / `xsd:double` / `xsd:boolean` / plain literal |
//! | `List` | one triple per element (the inverse of how the import builds a list) |
//!
//! Prefixes are the graph's catalog (`rdf_prefixes()`, what the imported
//! documents declared) plus `rdf`, `rdfs`, `owl`, `xsd`; the empty prefix is
//! the catalog's, or `http://example.org/ontology#` for a graph that never
//! imported Turtle. Serialization goes through `oxttl`, the same family as
//! the parser, so escaping and IRI validity are the library's, not ours; the
//! output is deterministic (subjects by IRI, predicates by name).
//!
//! What RDF cannot carry is not dropped in silence: `Null`, `Bytes`, `Object`,
//! nested lists, NaN/∞, edge properties other than the predicate IRI, and
//! edges towards nodes without `iri` are each one line in
//! [`ExportReport::skipped`](exporter::ExportReport::skipped), with the
//! reason. Empty `skipped` means import → export → import gives back the same
//! graph: same nodes by IRI, same edges, same literals with the same types
//! (there is a test that checks exactly that). Blank nodes keep their
//! synthetic id inside one database, but a re-import re-hashes them, as RDF
//! allows.
//!
#[cfg(feature = "owl-import")]
pub mod importer;

#[cfg(feature = "owl-import")]
pub mod exporter;
