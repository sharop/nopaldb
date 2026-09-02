//! Turtle/OWL bridge: load an ontology into the property graph, and write the
//! ontological part of the graph back out as Turtle.
//!
//! This is an **ontology loader**, not an RDF store. It exists so that a class
//! hierarchy and its individuals can be queried with `instanceOf` /
//! `subClassOf` in NQL and fed to the OWL-EL reasoner. Anyone evaluating
//! NopalDB to interoperate with a triple store needs the exact contract below
//! before deciding — "lossy" undersells what is lost.
//!
//! # What the import keeps
//!
//! | Turtle | Graph |
//! |--------|-------|
//! | `:X rdf:type owl:Class` | node `X` with `NodeKind::Class` |
//! | `:X rdfs:subClassOf :Y` | edge `X → Y` of type `subClassOf`, plus the taxonomy index |
//! | `:x rdf:type :X` (X a known class) | node with label `X`, property `iri = ":x"` |
//! | `:x :p "literal"` (x an individual) | property `p` on that node; `"42"` → `Int`, `"3.5"` → `Float`, `"true"` → `Bool`, otherwise `String` |
//!
//! Re-importing the same file is idempotent: classes are matched by label,
//! individuals by their `iri` property. A class declared by an earlier import
//! counts as known, so an ontology in one file and its instances in another
//! work.
//!
//! # What the import loses
//!
//! - **IRI identity.** Terms are reduced to their local name (the part after
//!   `#` or the last `/`); prefixes and `@base` are discarded. Two IRIs with the
//!   same local name in different namespaces collapse into one node. Only the
//!   `iri` property of individuals keeps the original token, and it keeps it
//!   as written (`:x` or `<http://…/x>`), not expanded.
//! - **Object properties.** A triple whose object is a resource
//!   (`:x :knows :y`) does **not** create an edge: the object is stored as a
//!   string property. The only edges the bridge creates are `subClassOf`.
//! - **Multiple types.** An individual keeps its first `rdf:type` only.
//! - **Multi-valued predicates.** `:x :tag "a", "b"` keeps the last value.
//! - **Class metadata.** `rdfs:label`, `rdfs:comment` and any other triple
//!   whose subject is a class (not an individual) are dropped.
//! - **Instances of unknown classes.** `:x rdf:type :Y` where `Y` was never
//!   declared as `owl:Class` is skipped, together with the data properties
//!   of `:x`.
//! - **Everything else.** `owl:Ontology` headers, property declarations,
//!   restrictions, equivalence axioms — anything the table above does not
//!   list.
//!
//! Each dropped triple adds one to `ImportReport::triples_skipped`.
//! The data properties of individuals do not: they are imported, and the
//! count is exactly what left nothing in the graph.
//!
//! # What the parser does not understand
//!
//! The parser is a hand-written tokenizer for a subset of Turtle. It does not
//! support the `a` keyword, language tags (`"x"@en`), blank nodes (`_:b` or
//! `[ … ]`), collections, `@base`, or multi-line literals. Datatype suffixes
//! (`^^xsd:integer`) are stripped, not honoured: `"42"^^xsd:string` becomes an
//! `Int`. **Malformed input does not fail**: unrecognized tokens are silently
//! dropped or mis-grouped, so a parse problem shows up as a wrong count, not
//! as an error.
//!
//! # What the export omits
//!
//! `export_turtle` writes classes, `subClassOf` edges and individuals with
//! their scalar data properties. It omits every other edge (nothing but
//! `subClassOf` is emitted), nodes without an `iri` property, and `Null`,
//! `Bytes`, `List`, `Object` and non-finite float properties. Local names are
//! sanitized to ASCII `[A-Za-z0-9_-]` (anything else becomes `_`), and the
//! default namespace is a fixed `http://example.org/ontology#` regardless of
//! what the imported file declared. A round trip therefore preserves the
//! counts of classes, subclass edges and individuals, but not IRIs,
//! namespaces, or relationships between individuals.
//!
//! # Where this is going
//!
//! A faithful bridge — real Turtle grammar with errors, IRI identity, edges for
//! resource-valued triples, a symmetric exporter — is tracked in the public
//! roadmap. NopalDB will not become a triple store (no SPARQL, no named
//! graphs); the intended shape is coexistence: keep the triple store, and
//! materialize the subgraph you query here.

pub mod rdf;

#[cfg(feature = "owl-import")]
pub mod importer;

#[cfg(feature = "owl-import")]
pub mod exporter;
