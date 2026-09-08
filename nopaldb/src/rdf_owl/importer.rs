// src/rdf_owl/importer.rs
//
// OWL/Turtle Importer.
//
// Parsing is delegated to `oxttl` (a real Turtle grammar with a positioned
// error on malformed input). This module decides what the parsed triples MEAN
// for the property graph — the contract is in the module docs (`rdf_owl/mod.rs`):
//   - `:X a owl:Class`          → Class node (identity: the IRI)
//   - `:X rdfs:subClassOf :Y`   → `subClassOf` edge + taxonomy
//   - `:x a :X`                 → individual node + `instanceOf` edge, one per type
//   - `:x :p :y`                → edge `p` (placeholder for `:y` if unknown)
//   - `:x :p "literal"`         → property `p`, typed by datatype
//
// Every triple that none of the passes consumed is counted in
// `triples_skipped`; what is assumed on the document's behalf goes to
// `warnings`. Malformed input is an error and writes nothing.
//
// Feature gate: compiled only when `owl-import` is enabled.

use std::collections::{BTreeMap, HashMap, HashSet};

use oxrdf::vocab::{rdf, rdfs, xsd};
use oxrdf::{NamedNodeRef, NamedOrBlankNode, Term, Triple};
use oxttl::TurtleParser;

use crate::error::{NopalError, Result};
use crate::graph::Graph;
use crate::index::taxonomy::TaxonomyIndex;
use crate::types::{Edge, Node, NodeId, NodeKind, PropertyValue};

// ---------------------------------------------------------------------------
// Vocabulary and defaults
// ---------------------------------------------------------------------------

/// `owl:Class`. `oxrdf` ships `rdf`, `rdfs` and `xsd` vocabularies but not
/// `owl`, so the terms this importer needs are declared here.
pub(crate) const OWL_CLASS: NamedNodeRef<'static> =
    NamedNodeRef::new_unchecked("http://www.w3.org/2002/07/owl#Class");
/// `owl:NamedIndividual`: says "this is an individual" and nothing else. It is
/// consumed as a no-op — the individual's real class is its other `rdf:type`.
const OWL_NAMED_INDIVIDUAL: NamedNodeRef<'static> =
    NamedNodeRef::new_unchecked("http://www.w3.org/2002/07/owl#NamedIndividual");

pub(crate) const NS_RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
pub(crate) const NS_RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
pub(crate) const NS_OWL: &str = "http://www.w3.org/2002/07/owl#";
pub(crate) const NS_XSD: &str = "http://www.w3.org/2001/XMLSchema#";

/// Edge type of `rdf:type`. Reserved: a user predicate whose local name is
/// `instanceOf` or `subClassOf` is written with its qualified name instead,
/// so the taxonomy never picks up a user edge.
pub const EDGE_INSTANCE_OF: &str = "instanceOf";
/// Edge type of `rdfs:subClassOf` (unchanged since the first importer).
pub const EDGE_SUBCLASS_OF: &str = "subClassOf";

/// Property carrying a node's IRI (classes and individuals alike).
pub const PROP_IRI: &str = "iri";
/// Set (to `true`) on a node created only because something pointed at it.
/// Removed the moment the node gets a type.
pub const PROP_PLACEHOLDER: &str = "rdf_placeholder";
/// `rdfs:label` / `rdfs:comment` land here. Not `label`: that is the name of
/// the `Node` field, and `n.label` in NQL must keep meaning the field.
pub const PROP_RDFS_LABEL: &str = "rdfs_label";
pub const PROP_RDFS_COMMENT: &str = "rdfs_comment";

/// Namespace assumed for the empty prefix (`:Foo`) when the document does not
/// declare one. It is the namespace `export_turtle` has always written, so a
/// document exported by NopalDB and one written by hand without a prefix
/// block land on the same IRIs.
///
/// Why not fail instead: every fixture, tutorial and test written against the
/// previous hand-rolled parser uses `:Foo` without an `@prefix :` line, and
/// so do most people's first Turtle files. Failing would turn a working import
/// into a syntax error with no data benefit; the assumption is reported in
/// [`ImportReport::warnings`] instead.
pub const DEFAULT_NAMESPACE: &str = "http://example.org/ontology#";

/// Base IRI used to resolve relative references (`<foo>`) when the document
/// has no `@base`. Turtle makes a relative IRI without a base a hard error;
/// resolving against a fixed base keeps the import going and is reported.
pub const DEFAULT_BASE: &str = "http://example.org/ontology/";

fn is_reserved_namespace(iri: &str) -> bool {
    iri.starts_with(NS_RDF) || iri.starts_with(NS_RDFS) || iri.starts_with(NS_OWL) || iri.starts_with(NS_XSD)
}

// ---------------------------------------------------------------------------
// Public result type
// ---------------------------------------------------------------------------

/// Summary of a Turtle import operation. Every counter counts what this call
/// **created**; a second import of the same document reports zeros.
#[derive(Debug, Clone, Default)]
pub struct ImportReport {
    /// `owl:Class` nodes created (declared, or created lazily because a
    /// `rdfs:subClassOf` or an `rdf:type` named them first).
    pub classes_added: usize,
    /// `subClassOf` edges created.
    pub subclass_edges_added: usize,
    /// Individual nodes created (typed subjects; placeholders are counted in
    /// `placeholders_created` instead, and move here only in spirit when a
    /// later import types them).
    pub instances_added: usize,
    /// Edges created other than `subClassOf`: one `instanceOf` per
    /// `rdf:type`, plus one per resource-valued triple (`:x :p :y`).
    pub edges_created: usize,
    /// Nodes created only because a triple pointed at them (`:x :p :y` with
    /// `:y` never declared): they carry `iri` and `rdf_placeholder = true`
    /// and nothing else until a later import types them.
    pub placeholders_created: usize,
    /// Triples that left nothing in the graph. With this importer that is
    /// exactly the reserved vocabulary it does not model: `owl:Ontology`
    /// headers, property declarations (`owl:ObjectProperty`, …),
    /// `rdfs:domain`/`rdfs:range`, `owl:equivalentClass` and friends, and any
    /// statement whose subject is such a vocabulary resource. Everything in a
    /// user namespace is kept, so a non-zero count is a list of axioms, never
    /// of data.
    ///
    /// Computed once, after all passes, from the same tests the passes used.
    pub triples_skipped: usize,
    /// Things the import did on the document's behalf that the author should
    /// know about: an assumed default namespace, a class it had to create
    /// because a type named it without declaring it, a label it had to
    /// qualify because two classes share a local name, nodes left over from
    /// an import made before IRIs were kept. Empty means the document was
    /// taken exactly as written.
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Parsed document
// ---------------------------------------------------------------------------

/// What `parse_turtle` hands to the passes.
pub(crate) struct ParsedDocument {
    pub triples: Vec<Triple>,
    /// Prefixes the document ended up with (declared ones plus the seeded
    /// empty prefix), persisted in the graph catalog by the import.
    pub prefixes: BTreeMap<String, String>,
    pub warnings: Vec<String>,
    /// FNV-1a of the source text; the identity of this document's blank nodes.
    pub document_hash: u64,
}

/// Parse a Turtle source string with `oxttl`.
///
/// Errors on the first syntax error, with the 1-based line and column in the
/// message. The parser does not stop by itself on an error (it recovers and
/// keeps yielding), so the short-circuit is explicit here: a document with a
/// syntax error is not imported at all, rather than imported up to the error.
pub(crate) fn parse_turtle(source: &str) -> Result<ParsedDocument> {
    let mut warnings = Vec::new();

    if !declares_empty_prefix(source) {
        warnings.push(format!(
            "el documento no declara `@prefix :` — se asumió `{DEFAULT_NAMESPACE}`"
        ));
    }
    if !declares_base(source) && has_relative_iri_ref(source) {
        warnings.push(format!(
            "el documento usa IRIs relativos sin `@base` — se resolvieron contra `{DEFAULT_BASE}`"
        ));
    }

    let mut parser = TurtleParser::new()
        .with_base_iri(DEFAULT_BASE)
        .map_err(|e| NopalError::RdfParseError(format!("base IRI inválida: {e}")))?
        .with_prefix("", DEFAULT_NAMESPACE)
        .map_err(|e| NopalError::RdfParseError(format!("prefijo por defecto inválido: {e}")))?
        .for_slice(source);

    // `by_ref()` keeps the parser alive after the collect: the prefix map is
    // only filled as `@prefix` lines are consumed, so it must be read after.
    let triples: Vec<Triple> = parser
        .by_ref()
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| {
            let at = e.location().start;
            NopalError::RdfParseError(format!(
                "Turtle syntax error at line {} column {}: {}",
                at.line + 1,
                at.column + 1,
                e.message()
            ))
        })?;

    let prefixes = parser
        .prefixes()
        .map(|(name, iri)| (name.to_string(), iri.to_string()))
        .collect();

    Ok(ParsedDocument {
        triples,
        prefixes,
        warnings,
        document_hash: fnv1a_64(source.as_bytes()),
    })
}

/// Does the document declare the empty prefix itself (`@prefix :` or the
/// SPARQL-style `PREFIX :`)? Textual check on purpose: once parsing is done
/// the seeded default and a declared one are indistinguishable.
fn declares_empty_prefix(source: &str) -> bool {
    source.lines().any(|line| {
        let l = line.trim_start();
        l.starts_with("@prefix :") || l.to_ascii_lowercase().starts_with("prefix :")
    })
}

fn declares_base(source: &str) -> bool {
    source.lines().any(|line| {
        let l = line.trim_start();
        l.starts_with("@base") || l.to_ascii_lowercase().starts_with("base ")
    })
}

/// A `<...>` reference with no scheme (`<foo>`, `<#bar>`, `</x/y>`) is
/// relative. Used only to decide whether to warn; the parser resolves it.
fn has_relative_iri_ref(source: &str) -> bool {
    let mut rest = source;
    while let Some(start) = rest.find('<') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('>') else { break };
        let inner = &after[..end];
        let has_scheme = inner.split_once(':').is_some_and(|(scheme, _)| {
            !scheme.is_empty()
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.')
        });
        if !has_scheme && !inner.contains(char::is_whitespace) {
            return true;
        }
        rest = &after[end + 1..];
    }
    false
}

/// FNV-1a, 64-bit. Written out instead of using `DefaultHasher` because the
/// value is persisted (in blank-node identities) and `DefaultHasher` makes no
/// stability promise across Rust versions.
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

// ---------------------------------------------------------------------------
// Term helpers
// ---------------------------------------------------------------------------

/// Identity string of a subject: the absolute IRI, or for a blank node a
/// synthetic IRI scoped to this document (`_:<document hash>-<label>`).
///
/// Scoping by document hash is what makes re-importing the same file
/// idempotent (same hash → same identities) while two different files that
/// both use `_:b0` never collide. Blank nodes have no identity in RDF; this
/// gives them the most useful one a graph store can.
fn subject_iri(subject: &NamedOrBlankNode, document_hash: u64) -> String {
    match subject {
        NamedOrBlankNode::NamedNode(n) => n.as_str().to_string(),
        NamedOrBlankNode::BlankNode(b) => blank_iri(b.as_str(), document_hash),
    }
}

fn blank_iri(label: &str, document_hash: u64) -> String {
    format!("_:{document_hash:016x}-{label}")
}

/// Object of a triple, reduced to what the passes distinguish.
enum Object<'a> {
    /// A named node or a blank node, as an identity string.
    Resource(String),
    Literal(&'a oxrdf::Literal),
}

fn object_of(term: &Term, document_hash: u64) -> Object<'_> {
    match term {
        Term::NamedNode(n) => Object::Resource(n.as_str().to_string()),
        Term::BlankNode(b) => Object::Resource(blank_iri(b.as_str(), document_hash)),
        Term::Literal(l) => Object::Literal(l),
    }
}

/// Local name of an IRI: the part after `#`, or after the last `/`. For a
/// synthetic blank-node IRI the label after the hash. This is what becomes a
/// node label, an edge type or a property name — the readable half of a
/// term. Identity is the full IRI, never this.
pub(crate) fn local_name(iri: &str) -> String {
    if let Some(rest) = iri.strip_prefix("_:") {
        return rest.split_once('-').map(|(_, l)| l).unwrap_or(rest).to_string();
    }
    if let Some(pos) = iri.rfind('#') {
        return iri[pos + 1..].to_string();
    }
    if let Some(pos) = iri.rfind('/') {
        return iri[pos + 1..].to_string();
    }
    iri.to_string()
}

/// `prefix:local` for an IRI, using the longest declared prefix that covers
/// it; the full IRI when none does. Used for labels that would otherwise
/// collide and for edge types that would otherwise shadow the reserved ones.
fn qualified_name(iri: &str, prefixes: &BTreeMap<String, String>) -> String {
    let best = prefixes
        .iter()
        .filter(|(_, ns)| !ns.is_empty() && iri.starts_with(ns.as_str()))
        .max_by_key(|(_, ns)| ns.len());
    match best {
        Some((p, ns)) => format!("{p}:{}", &iri[ns.len()..]),
        None => iri.to_string(),
    }
}

/// Is this stored `iri` value from an import made before IRIs were kept?
/// Those imports stored the raw token (`:Alice`, `<http://…>`), never an
/// absolute IRI or a synthetic blank-node IRI.
fn is_legacy_iri(stored: &str) -> bool {
    !(stored.starts_with("_:") || stored.contains("://"))
}

/// Map an RDF literal to a `PropertyValue`. The table is the contract (it is
/// repeated in the module docs):
///
/// | datatype | value |
/// |---|---|
/// | `xsd:integer` family (`int`, `long`, `short`, `byte`, unsigned, `nonNegativeInteger`, …) | `Int` |
/// | `xsd:decimal`, `xsd:double`, `xsd:float` | `Float` |
/// | `xsd:boolean` | `Bool` |
/// | `xsd:string`, plain literal, language-tagged literal, anything else | `String` with the lexical value |
///
/// A plain `"42"` is `xsd:string` by the RDF spec and stays a string here. The
/// first parser guessed `Int` from the shape of the text, so `"42"^^xsd:string`
/// became a number; that guess is gone. A typed literal whose text does not
/// parse as its type falls back to `String` and is reported in `warnings`
/// rather than dropped or coerced.
pub(crate) fn literal_to_property_value(lit: &oxrdf::Literal, warnings: &mut Vec<String>) -> PropertyValue {
    let value = lit.value();
    let dt = lit.datatype();

    let is_integer_type = [
        xsd::INTEGER,
        xsd::INT,
        xsd::LONG,
        xsd::SHORT,
        xsd::BYTE,
        xsd::UNSIGNED_INT,
        xsd::UNSIGNED_LONG,
        xsd::UNSIGNED_SHORT,
        xsd::UNSIGNED_BYTE,
        xsd::NON_NEGATIVE_INTEGER,
        xsd::NON_POSITIVE_INTEGER,
        xsd::NEGATIVE_INTEGER,
        xsd::POSITIVE_INTEGER,
    ]
    .contains(&dt);

    if is_integer_type {
        return match value.parse::<i64>() {
            Ok(i) => PropertyValue::Int(i),
            Err(_) => {
                warnings.push(format!(
                    "literal `{value}` tipado {} no es un entero de 64 bits — se guardó como texto",
                    dt.as_str()
                ));
                PropertyValue::String(value.to_string())
            }
        };
    }
    if dt == xsd::DECIMAL || dt == xsd::DOUBLE || dt == xsd::FLOAT {
        return match value.parse::<f64>() {
            Ok(f) => PropertyValue::Float(f),
            Err(_) => {
                warnings.push(format!(
                    "literal `{value}` tipado {} no es un número — se guardó como texto",
                    dt.as_str()
                ));
                PropertyValue::String(value.to_string())
            }
        };
    }
    if dt == xsd::BOOLEAN {
        return match value {
            "true" | "1" => PropertyValue::Bool(true),
            "false" | "0" => PropertyValue::Bool(false),
            _ => {
                warnings.push(format!(
                    "literal `{value}` tipado xsd:boolean no es true/false — se guardó como texto"
                ));
                PropertyValue::String(value.to_string())
            }
        };
    }
    PropertyValue::String(value.to_string())
}

// ---------------------------------------------------------------------------
// Import context
// ---------------------------------------------------------------------------

/// Everything a pass needs: the graph, the taxonomy, the report being built,
/// and the per-import caches (IRI → node id) that keep lookups to one per
/// term instead of one per triple.
struct Import<'a> {
    graph: &'a Graph,
    taxonomy: &'a mut TaxonomyIndex,
    report: ImportReport,
    prefixes: &'a BTreeMap<String, String>,
    /// IRI → node id, for every node touched by this import (classes and
    /// individuals; `None` means "looked up, not there").
    nodes: HashMap<String, Option<NodeId>>,
    /// Class labels already taken in the graph, by IRI, to detect collisions.
    class_label_owner: HashMap<String, String>,
    /// Individuals that had an `rdf:type` in this document, with their first
    /// type's IRI (that type decides the label).
    typed: HashMap<String, String>,
    /// Subjects typed only with reserved vocabulary (`owl:Ontology`,
    /// `owl:ObjectProperty`, …): vocabulary resources, not data.
    vocabulary_subjects: HashSet<String>,
    /// Nodes whose stored `iri` is a pre-0.5.10 raw token, seen while looking
    /// for a node this document names. Reported once, at the end.
    legacy_seen: usize,
}

impl Import<'_> {
    /// Node by IRI, through the property index (one lookup, cached).
    async fn lookup(&mut self, iri: &str) -> Result<Option<NodeId>> {
        if let Some(hit) = self.nodes.get(iri) {
            return Ok(*hit);
        }
        let ids = self
            .graph
            .get_all_nodes_by_property(PROP_IRI, &PropertyValue::String(iri.to_string()))
            .await?;
        let found = ids.first().copied();
        self.nodes.insert(iri.to_string(), found);
        Ok(found)
    }

    /// Detect nodes left by an import made before IRIs were kept: same label,
    /// `iri` stored as the raw token whose local name matches. They are not
    /// adopted — merging would need the prefix map that import never had —
    /// only reported, once, with the upgrade path.
    async fn note_legacy(&mut self, label: &str, iri: &str) -> Result<()> {
        let local = local_name(iri);
        let hits = self
            .graph
            .get_nodes_by_label(label)
            .await?
            .into_iter()
            .filter(|n| {
                matches!(n.properties.get(PROP_IRI), Some(PropertyValue::String(v))
                    if is_legacy_iri(v) && local_name(v.trim_start_matches(':')) == local)
            })
            .count();
        self.legacy_seen += hits;
        Ok(())
    }

    /// The class node for `iri`, creating it if needed. `declared` says
    /// whether this document has an `a owl:Class` for it; a class that is
    /// only *named* (by a type or a subclass axiom) is created too, with a
    /// warning, so nothing is dropped in silence.
    async fn ensure_class(&mut self, iri: &str, declared: bool) -> Result<NodeId> {
        if let Some(id) = self.lookup(iri).await? {
            return Ok(id);
        }

        let local = local_name(iri);
        let label = self.class_label_for(iri, &local).await?;

        // A pre-0.5.10 import stored classes without an `iri`. Adopting the
        // existing node (same label, kind Class, no `iri`) keeps the
        // individuals that already point at it; the alternative — a second
        // class with the same label — would split the hierarchy in two.
        let legacy = self
            .graph
            .get_nodes_by_label(&label)
            .await?
            .into_iter()
            .find(|n| n.kind == NodeKind::Class && !n.properties.contains_key(PROP_IRI));
        let id = if let Some(mut node) = legacy {
            node.properties.insert(PROP_IRI.into(), PropertyValue::String(iri.into()));
            self.graph.add_node(node.clone()).await?;
            self.report.warnings.push(format!(
                "la clase `{label}` existía sin IRI (import anterior a 0.5.10): se le asignó `{iri}`"
            ));
            node.id
        } else {
            let mut node = Node::new(label.clone());
            node.kind = NodeKind::Class;
            node.properties.insert(PROP_IRI.into(), PropertyValue::String(iri.into()));
            let id = self.graph.add_node(node).await?;
            self.report.classes_added += 1;
            if !declared {
                self.report.warnings.push(format!(
                    "la clase `{}` se usa sin declararla (`a owl:Class`): se creó",
                    qualified_name(iri, self.prefixes)
                ));
            }
            id
        };

        self.taxonomy.register_class(id, &label);
        self.taxonomy.register_class_iri(id, iri);
        self.nodes.insert(iri.to_string(), Some(id));
        self.class_label_owner.insert(label, iri.to_string());
        Ok(id)
    }

    /// Label for a new class: its local name, unless another class (different
    /// IRI) already owns that label in the graph. Then the qualified name
    /// (`fauna:Rosa`), with a warning: the taxonomy and `instanceOf` are
    /// label-keyed, and two classes under one label would silently become one.
    async fn class_label_for(&mut self, iri: &str, local: &str) -> Result<String> {
        let owner = match self.class_label_owner.get(local) {
            Some(o) => Some(o.clone()),
            None => self
                .graph
                .get_nodes_by_label(local)
                .await?
                .into_iter()
                .find(|n| n.kind == NodeKind::Class)
                .and_then(|n| match n.properties.get(PROP_IRI) {
                    Some(PropertyValue::String(v)) => Some(v.clone()),
                    // A legacy class without IRI is adopted by the caller, not a collision.
                    _ => None,
                }),
        };
        Ok(match owner {
            Some(o) if o != iri => {
                let q = qualified_name(iri, self.prefixes);
                self.report.warnings.push(format!(
                    "dos clases con local name `{local}` (`{o}` y `{iri}`): la segunda se etiquetó `{q}`"
                ));
                q
            }
            _ => local.to_string(),
        })
    }

    /// The node for an individual `iri`. `class` is the label of its first
    /// type when the document types it; `None` creates (or keeps) a
    /// placeholder. A placeholder that gets a type here is upgraded in place:
    /// same id, so edges already pointing at it stay valid.
    async fn ensure_individual(&mut self, iri: &str, class: Option<&str>) -> Result<NodeId> {
        if let Some(id) = self.lookup(iri).await? {
            if let Some(class) = class {
                let mut node = self.graph.get_node(id).await?;
                if node.properties.remove(PROP_PLACEHOLDER).is_some() {
                    node.label = class.to_string();
                    self.graph.add_node(node).await?;
                    self.report.instances_added += 1;
                }
            }
            return Ok(id);
        }

        let mut node = match class {
            Some(class) => Node::new(class),
            None => Node::new(local_name(iri)),
        };
        node.properties.insert(PROP_IRI.into(), PropertyValue::String(iri.into()));
        if let Some(class) = class {
            self.note_legacy(class, iri).await?;
            self.report.instances_added += 1;
        } else {
            node.properties.insert(PROP_PLACEHOLDER.into(), PropertyValue::Bool(true));
            self.report.placeholders_created += 1;
        }
        let id = self.graph.add_node(node).await?;
        self.nodes.insert(iri.to_string(), Some(id));
        Ok(id)
    }

    /// Any node for a subject or object IRI: a class if the graph has one
    /// under that IRI, else the individual (or placeholder).
    async fn ensure_node(&mut self, iri: &str) -> Result<NodeId> {
        let class = self.typed.get(iri).map(|c| local_name(c));
        self.ensure_individual(iri, class.as_deref()).await
    }

    /// Create `source -[edge_type {iri}]-> target` unless it already exists.
    /// Dedup is what makes a second import of the same document write nothing.
    async fn ensure_edge(&mut self, source: NodeId, target: NodeId, edge_type: &str, pred_iri: &str) -> Result<bool> {
        let exists = self
            .graph
            .get_outgoing_edges(source)
            .await?
            .iter()
            .any(|e| e.target == target && e.edge_type == edge_type);
        if exists {
            return Ok(false);
        }
        let edge = Edge::new(source, target, edge_type)
            .with_property(PROP_IRI, PropertyValue::String(pred_iri.into()));
        self.graph.add_edge(edge).await?;
        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Import a Turtle source string into `graph`, registering classes, the
/// `subClassOf` hierarchy, individuals with their `instanceOf` edges, the
/// edges between individuals, and their data properties; updates the
/// `taxonomy` index and the graph's prefix catalog.
///
/// Identity is the IRI: a class or individual with the same IRI is reused,
/// never duplicated, and a second import of the same document creates
/// nothing. What the bridge keeps, what it assumes and what it skips is
/// spelled out in the [module docs](crate::rdf_owl).
///
/// Malformed Turtle is an error ([`NopalError::RdfParseError`], with line and
/// column), and nothing is written: a document is imported whole or not at all.
pub async fn import_turtle(
    graph: &Graph,
    taxonomy: &mut TaxonomyIndex,
    source: &str,
) -> Result<ImportReport> {
    // Step 1 — parse. Fails here, before any write, on malformed input.
    let doc = parse_turtle(source)?;
    let hash = doc.document_hash;
    let triples = &doc.triples;

    let mut im = Import {
        graph,
        taxonomy,
        report: ImportReport { warnings: doc.warnings.clone(), ..Default::default() },
        prefixes: &doc.prefixes,
        nodes: HashMap::new(),
        class_label_owner: HashMap::new(),
        typed: HashMap::new(),
        vocabulary_subjects: HashSet::new(),
        legacy_seen: 0,
    };

    // Pass 0 — classify subjects by their rdf:type, in document order.
    //   declared classes:      `a owl:Class`
    //   typed individuals:     `a :X` (first X wins the label)
    //   vocabulary resources:  typed ONLY with reserved terms other than
    //                          owl:Class / owl:NamedIndividual (ontology
    //                          headers, property declarations): skipped whole.
    let mut declared_classes: HashSet<String> = HashSet::new();
    let mut reserved_typed: HashSet<String> = HashSet::new();
    for t in triples {
        if t.predicate != rdf::TYPE {
            continue;
        }
        let Object::Resource(obj) = object_of(&t.object, hash) else { continue };
        let subj = subject_iri(&t.subject, hash);
        if obj == OWL_CLASS.as_str() {
            declared_classes.insert(subj);
        } else if obj == OWL_NAMED_INDIVIDUAL.as_str() {
            // Says "individual", not which class: nothing to record.
        } else if is_reserved_namespace(&obj) {
            reserved_typed.insert(subj);
        } else {
            im.typed.entry(subj).or_insert(obj);
        }
    }
    for subj in reserved_typed {
        if !declared_classes.contains(&subj) && !im.typed.contains_key(&subj) {
            im.vocabulary_subjects.insert(subj);
        }
    }

    // Pass 1 — declared classes, in document order (a class is created
    // before anything can point at it).
    for t in triples {
        if t.predicate != rdf::TYPE {
            continue;
        }
        let Object::Resource(obj) = object_of(&t.object, hash) else { continue };
        if obj != OWL_CLASS.as_str() {
            continue;
        }
        let subj = subject_iri(&t.subject, hash);
        im.ensure_class(&subj, true).await?;
    }

    // Pass 2 — subClassOf edges + taxonomy.
    for t in triples {
        if t.predicate != rdfs::SUB_CLASS_OF {
            continue;
        }
        let Object::Resource(obj) = object_of(&t.object, hash) else { continue };
        let subj = subject_iri(&t.subject, hash);
        let sub_id = im.ensure_class(&subj, declared_classes.contains(&subj)).await?;
        let super_id = im.ensure_class(&obj, declared_classes.contains(&obj)).await?;
        if im.ensure_edge(sub_id, super_id, EDGE_SUBCLASS_OF, rdfs::SUB_CLASS_OF.as_str()).await? {
            im.report.subclass_edges_added += 1;
        }
        // Convention: add_subclass(parent, child) means child ⊑ parent. Idempotent.
        im.taxonomy.add_subclass(super_id, sub_id)?;
    }

    // Pass 3 — individuals: node (label = first type) + one instanceOf edge per type.
    let typed_order: Vec<(String, String)> = {
        let mut seen = HashSet::new();
        let mut v = Vec::new();
        for t in triples {
            if t.predicate != rdf::TYPE {
                continue;
            }
            let subj = subject_iri(&t.subject, hash);
            if let Some(first) = im.typed.get(&subj)
                && seen.insert(subj.clone())
            {
                v.push((subj, first.clone()));
            }
        }
        v
    };
    for (subj, first_type) in &typed_order {
        if declared_classes.contains(subj) {
            continue; // a class is not also an individual
        }
        let class_id = im.ensure_class(first_type, declared_classes.contains(first_type)).await?;
        let class_label = im.graph.get_node(class_id).await?.label;
        im.ensure_individual(subj, Some(&class_label)).await?;
    }
    for t in triples {
        if t.predicate != rdf::TYPE {
            continue;
        }
        let Object::Resource(obj) = object_of(&t.object, hash) else { continue };
        let subj = subject_iri(&t.subject, hash);
        if obj == OWL_CLASS.as_str()
            || obj == OWL_NAMED_INDIVIDUAL.as_str()
            || is_reserved_namespace(&obj)
            || declared_classes.contains(&subj)
        {
            continue;
        }
        let class_id = im.ensure_class(&obj, declared_classes.contains(&obj)).await?;
        let Some(ind_id) = im.lookup(&subj).await? else { continue };
        if im.ensure_edge(ind_id, class_id, EDGE_INSTANCE_OF, rdf::TYPE.as_str()).await? {
            im.report.edges_created += 1;
        }
        // The taxonomy snapshot mirrors the edge so `instanceOf(n, C)` in NQL
        // sees this type without reading storage.
        im.taxonomy.register_instance(ind_id, class_id);
    }

    // Pass 4 — statements: edges for resource objects, properties for literals.
    // Properties are gathered per subject and applied once, so a node is
    // rewritten at most once and only when something actually changed.
    let mut props: HashMap<String, BTreeMap<String, Vec<PropertyValue>>> = HashMap::new();
    for t in triples {
        if t.predicate == rdf::TYPE || t.predicate == rdfs::SUB_CLASS_OF {
            continue;
        }
        let pred = t.predicate.as_str();
        let subj = subject_iri(&t.subject, hash);
        if im.vocabulary_subjects.contains(&subj) {
            continue;
        }
        let key = match pred {
            p if p == rdfs::LABEL.as_str() => PROP_RDFS_LABEL.to_string(),
            p if p == rdfs::COMMENT.as_str() => PROP_RDFS_COMMENT.to_string(),
            p if is_reserved_namespace(p) => continue, // rdfs:domain, owl:equivalentClass, …: skipped
            p => local_name(p),
        };
        if key.is_empty() {
            continue;
        }
        match object_of(&t.object, hash) {
            Object::Literal(l) => {
                let v = literal_to_property_value(l, &mut im.report.warnings);
                props.entry(subj).or_default().entry(key).or_default().push(v);
            }
            Object::Resource(obj) => {
                let source = im.ensure_node(&subj).await?;
                let target = im.ensure_node(&obj).await?;
                let edge_type = if key == EDGE_INSTANCE_OF || key == EDGE_SUBCLASS_OF {
                    let q = qualified_name(pred, im.prefixes);
                    im.report.warnings.push(format!(
                        "el predicado `{q}` colisiona con la arista reservada `{key}`: se escribió como `{q}`"
                    ));
                    q
                } else {
                    key
                };
                if im.ensure_edge(source, target, &edge_type, pred).await? {
                    im.report.edges_created += 1;
                }
            }
        }
    }
    for (subj, values) in props {
        let id = im.ensure_node(&subj).await?;
        let mut node = im.graph.get_node(id).await?;
        let mut changed = false;
        for (k, mut vs) in values {
            let v = if vs.len() == 1 { vs.pop().expect("len checked") } else { PropertyValue::List(vs) };
            if node.properties.get(&k) != Some(&v) {
                node.properties.insert(k, v);
                changed = true;
            }
        }
        if changed {
            im.graph.add_node(node).await?;
        }
    }

    // Prefix catalog: the document's prefixes join the graph's (later import wins),
    // and the taxonomy snapshot gets the merged catalog so NQL can say `flora:Rosa`.
    im.graph.merge_rdf_prefixes(&doc.prefixes).await?;
    let catalog = im.graph.rdf_prefixes().await?;
    im.taxonomy.set_prefixes(catalog);

    if im.legacy_seen > 0 {
        im.report.warnings.push(format!(
            "{} nodo(s) con `iri` de un import anterior a 0.5.10 (`:x`) coinciden por nombre con nodos de este documento: se crearon nodos nuevos, no se fusionaron. Para una sola identidad, re-importa en una base nueva.",
            im.legacy_seen
        ));
    }

    // Final tally: a triple is "skipped" when no pass consumed it, decided
    // here, once, with the same tests the passes used.
    for t in triples {
        let subj = subject_iri(&t.subject, hash);
        let consumed = if im.vocabulary_subjects.contains(&subj) {
            false
        } else if t.predicate == rdf::TYPE {
            match object_of(&t.object, hash) {
                Object::Resource(obj) => !is_reserved_namespace(&obj) || obj == OWL_CLASS.as_str() || obj == OWL_NAMED_INDIVIDUAL.as_str(),
                Object::Literal(_) => false,
            }
        } else if t.predicate == rdfs::SUB_CLASS_OF {
            matches!(object_of(&t.object, hash), Object::Resource(_))
        } else {
            let p = t.predicate.as_str();
            p == rdfs::LABEL.as_str() || p == rdfs::COMMENT.as_str() || !is_reserved_namespace(p)
        };
        if !consumed {
            im.report.triples_skipped += 1;
        }
    }

    Ok(im.report)
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;
    use crate::index::taxonomy::TaxonomyIndex;
    use tempfile::TempDir;

    /// Prefix block shared by the fixtures. The empty prefix is declared so
    /// that the "assumed default namespace" warning never fires here and the
    /// tests can assert `warnings.is_empty()`.
    const PREFIXES: &str = r#"
@prefix owl:  <http://www.w3.org/2002/07/owl#> .
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .
@prefix :     <http://example.org/ontology#> .
"#;

    fn ttl(body: &str) -> String {
        format!("{PREFIXES}{body}")
    }

    async fn open_temp_graph() -> (Graph, TempDir) {
        let dir = TempDir::new().unwrap();
        let graph = Graph::open(dir.path().to_str().unwrap()).await.unwrap();
        (graph, dir)
    }

    fn individual(nodes: &[Node]) -> &Node {
        nodes.iter().find(|n| n.kind != NodeKind::Class).expect("an individual")
    }

    fn iri_of(node: &Node) -> &str {
        match node.properties.get(PROP_IRI) {
            Some(PropertyValue::String(s)) => s,
            other => panic!("iri expected, got {other:?}"),
        }
    }

    async fn by_iri(graph: &Graph, iri: &str) -> Node {
        let ids = graph
            .get_all_nodes_by_property(PROP_IRI, &PropertyValue::String(iri.into()))
            .await
            .unwrap();
        assert_eq!(ids.len(), 1, "exactly one node for {iri}");
        graph.get_node(ids[0]).await.unwrap()
    }

    fn zero(r: &ImportReport) -> bool {
        r.classes_added == 0
            && r.subclass_edges_added == 0
            && r.instances_added == 0
            && r.edges_created == 0
            && r.placeholders_created == 0
    }

    // -----------------------------------------------------------------------
    // Test 1 — single class declaration
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_import_simple_class() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let report = import_turtle(&graph, &mut taxonomy, &ttl(":Animal rdf:type owl:Class ."))
            .await
            .unwrap();

        assert_eq!(report.classes_added, 1, "should have added 1 class");
        assert_eq!(report.subclass_edges_added, 0);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);

        let nodes = graph.get_nodes_by_label("Animal").await.unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].kind, NodeKind::Class);
        assert_eq!(iri_of(&nodes[0]), "http://example.org/ontology#Animal");

        assert!(taxonomy.find_by_label("Animal").is_some());
    }

    // -----------------------------------------------------------------------
    // Test 2 — subclass chain A ⊑ B ⊑ C
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_import_subclass_chain() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let source = ttl(r#"
:Animal rdf:type owl:Class .
:Mammal rdf:type owl:Class .
:Dog    rdf:type owl:Class .
:Mammal rdfs:subClassOf :Animal .
:Dog    rdfs:subClassOf :Mammal .
"#);

        let report = import_turtle(&graph, &mut taxonomy, &source).await.unwrap();

        assert_eq!(report.classes_added, 3);
        assert_eq!(report.subclass_edges_added, 2);

        let animal_id = taxonomy.find_by_label("Animal").unwrap();
        let dog_id = taxonomy.find_by_label("Dog").unwrap();
        assert!(taxonomy.is_subclass_of(dog_id, animal_id));
    }

    // -----------------------------------------------------------------------
    // Test 3 — data properties of an individual are imported, not skipped
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_import_data_properties_are_not_skipped() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let source = ttl(r#"
:Animal rdf:type owl:Class .
:fido rdf:type :Animal .
:fido rdfs:label "Fido" .
:fido :age "5"^^xsd:integer .
"#);

        let report = import_turtle(&graph, &mut taxonomy, &source).await.unwrap();

        assert_eq!(report.classes_added, 1);
        assert_eq!(report.instances_added, 1);
        assert_eq!(report.edges_created, 1, "one instanceOf edge");
        assert_eq!(report.triples_skipped, 0, "both data properties end up on the node");

        let nodes = graph.get_nodes_by_label("Animal").await.unwrap();
        let fido = individual(&nodes);
        // rdfs:label lands in `rdfs_label`: `label` is the Node field.
        assert_eq!(fido.properties.get(PROP_RDFS_LABEL), Some(&PropertyValue::String("Fido".into())));
        assert_eq!(fido.properties.get("age"), Some(&PropertyValue::Int(5)));
    }

    // -----------------------------------------------------------------------
    // Test 3b — triples_skipped counts exactly what left nothing in the graph
    // -----------------------------------------------------------------------
    //
    // With IRI identity, everything in a user namespace is kept — a class
    // that is only named gets created (with a warning), and a property on a
    // class is a property. What is skipped is the reserved vocabulary the
    // importer does not model, and it is skipped whole.
    #[tokio::test]
    async fn test_triples_skipped_counts_only_discarded_triples() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        // Everything here is consumed: one class, one individual, one data property.
        let source = ttl(r#"
:Planta rdf:type owl:Class .
:Rosa rdf:type :Planta .
:Rosa :nombreComun "rosa" .
"#);
        let report = import_turtle(&graph, &mut taxonomy, &source).await.unwrap();
        assert_eq!(report.classes_added, 1);
        assert_eq!(report.instances_added, 1);
        assert_eq!(report.triples_skipped, 0);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);

        // Three triples that really are dropped, mixed with consumed ones:
        //   - an owl:Ontology header and its rdfs:label (vocabulary resource: 2)
        //   - owl:equivalentClass between classes (axiom not modelled: 1)
        // and two things that are NOT dropped any more:
        //   - rdfs:label on a class → `rdfs_label` on the class node
        //   - rdf:type pointing at an undeclared class → the class is created, with a warning
        let source = ttl(r#"
<http://example.org/ontology> a owl:Ontology ; rdfs:label "Plantas" .
:Planta rdfs:label "Planta" .
:Planta owl:equivalentClass :Vegetal .
:Cactus rdf:type :Suculenta .
:Cactus :nombreComun "cactus" .
:Tulipan rdf:type :Planta .
"#);
        let report = import_turtle(&graph, &mut taxonomy, &source).await.unwrap();
        assert_eq!(report.classes_added, 1, "Suculenta, created because a type named it");
        assert_eq!(report.instances_added, 2, "Cactus and Tulipan");
        assert_eq!(report.triples_skipped, 3, "{report:?}");
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert!(report.warnings[0].contains("Suculenta"));

        let planta = by_iri(&graph, "http://example.org/ontology#Planta").await;
        assert_eq!(planta.kind, NodeKind::Class);
        assert_eq!(planta.properties.get(PROP_RDFS_LABEL), Some(&PropertyValue::String("Planta".into())));
    }

    // -----------------------------------------------------------------------
    // Test 3c — re-importing the same file creates nothing and skips nothing
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_reimport_is_zero_writes() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let source = ttl(r#"
:Planta rdf:type owl:Class .
:Arbol rdf:type owl:Class .
:Arbol rdfs:subClassOf :Planta .
:Rosa rdf:type :Planta ; :nombreComun "rosa" ; :creceEn :Jardin .
:Jardin a :Lugar .
"#);
        let first = import_turtle(&graph, &mut taxonomy, &source).await.unwrap();
        let edges_after_first = graph.get_all_edges().await.unwrap().len();
        let nodes_after_first = graph.get_all_nodes().await.unwrap().len();

        let second = import_turtle(&graph, &mut taxonomy, &source).await.unwrap();

        assert_eq!(first.triples_skipped, 0);
        assert!(zero(&second), "second import must create nothing: {second:?}");
        assert_eq!(second.triples_skipped, 0, "already-present data is not 'lost'");
        assert_eq!(graph.get_all_edges().await.unwrap().len(), edges_after_first, "no duplicate edges");
        assert_eq!(graph.get_all_nodes().await.unwrap().len(), nodes_after_first, "no duplicate nodes");
    }

    // -----------------------------------------------------------------------
    // Test 4 — idempotent at the node level
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_import_idempotent() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let source = ttl(":Animal rdf:type owl:Class .");
        import_turtle(&graph, &mut taxonomy, &source).await.unwrap();
        import_turtle(&graph, &mut taxonomy, &source).await.unwrap();

        let nodes = graph.get_nodes_by_label("Animal").await.unwrap();
        assert_eq!(nodes.len(), 1, "second import should not duplicate the node");
        assert_eq!(taxonomy.size(), 1);
    }

    // -----------------------------------------------------------------------
    // Test 5 — diamond hierarchy
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_import_diamond_hierarchy() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let source = ttl(r#"
:A rdf:type owl:Class .
:B rdf:type owl:Class .
:C rdf:type owl:Class .
:D rdf:type owl:Class .
:B rdfs:subClassOf :A .
:C rdfs:subClassOf :A .
:D rdfs:subClassOf :B .
:D rdfs:subClassOf :C .
"#);

        let report = import_turtle(&graph, &mut taxonomy, &source).await.unwrap();

        assert_eq!(report.classes_added, 4);
        assert_eq!(report.subclass_edges_added, 4);

        let a_id = taxonomy.find_by_label("A").unwrap();
        let d_id = taxonomy.find_by_label("D").unwrap();
        assert!(taxonomy.is_subclass_of(d_id, a_id));
        assert_eq!(taxonomy.ancestors(d_id).len(), 3);
    }

    // -----------------------------------------------------------------------
    // Test 6 — local_name / qualified_name
    // -----------------------------------------------------------------------
    #[test]
    fn test_local_name_extraction() {
        assert_eq!(local_name("http://example.org/Animal"), "Animal");
        assert_eq!(local_name("http://www.w3.org/2002/07/owl#Class"), "Class");
        assert_eq!(local_name("http://example.org/ontology#Animal"), "Animal");
        assert_eq!(local_name("_:00000000deadbeef-b0"), "b0");

        let mut prefixes = BTreeMap::new();
        prefixes.insert("".to_string(), "http://example.org/ontology#".to_string());
        prefixes.insert("fauna".to_string(), "http://fauna.example/".to_string());
        assert_eq!(qualified_name("http://fauna.example/Rosa", &prefixes), "fauna:Rosa");
        assert_eq!(qualified_name("http://example.org/ontology#Rosa", &prefixes), ":Rosa");
        assert_eq!(qualified_name("http://other.example/Rosa", &prefixes), "http://other.example/Rosa");
    }

    // -----------------------------------------------------------------------
    // Test 7 — the xsd → PropertyValue table
    // -----------------------------------------------------------------------
    #[test]
    fn test_literal_to_property_value() {
        use oxrdf::Literal;
        let mut w = Vec::new();
        let typed = |v: &str, dt: NamedNodeRef<'_>| Literal::new_typed_literal(v, dt.into_owned());

        assert_eq!(literal_to_property_value(&typed("42", xsd::INTEGER), &mut w), PropertyValue::Int(42));
        assert_eq!(literal_to_property_value(&typed("7", xsd::NON_NEGATIVE_INTEGER), &mut w), PropertyValue::Int(7));
        assert_eq!(literal_to_property_value(&typed("3.14", xsd::DOUBLE), &mut w), PropertyValue::Float(3.14));
        assert_eq!(literal_to_property_value(&typed("2.5", xsd::DECIMAL), &mut w), PropertyValue::Float(2.5));
        assert_eq!(literal_to_property_value(&typed("true", xsd::BOOLEAN), &mut w), PropertyValue::Bool(true));
        assert_eq!(literal_to_property_value(&typed("false", xsd::BOOLEAN), &mut w), PropertyValue::Bool(false));
        assert_eq!(literal_to_property_value(&typed("42", xsd::STRING), &mut w), PropertyValue::String("42".into()));
        assert_eq!(
            literal_to_property_value(&Literal::new_simple_literal("42"), &mut w),
            PropertyValue::String("42".into())
        );
        assert_eq!(
            literal_to_property_value(&typed("2026-09-02", xsd::DATE), &mut w),
            PropertyValue::String("2026-09-02".into())
        );
        assert!(w.is_empty(), "{w:?}");

        assert_eq!(literal_to_property_value(&typed("many", xsd::INTEGER), &mut w), PropertyValue::String("many".into()));
        assert_eq!(w.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Test 8 — malformed Turtle is an error with a position, and writes nothing
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_malformed_turtle_is_err_with_position() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        // PREFIXES starts with a newline and has 5 declarations → lines 1-6;
        // the broken statement is on line 8.
        let source = ttl(":Animal rdf:type owl:Class .\n:Dog rdf:type :Animal :oops .\n");
        let err = import_turtle(&graph, &mut taxonomy, &source).await.unwrap_err();
        let msg = err.to_string();
        assert!(matches!(err, NopalError::RdfParseError(_)), "{msg}");
        assert!(msg.contains("line 8"), "position expected in {msg}");
        assert!(msg.contains("column"), "{msg}");

        assert!(graph.get_nodes_by_label("Animal").await.unwrap().is_empty());
        assert_eq!(taxonomy.size(), 0);
    }

    // -----------------------------------------------------------------------
    // Test 9 — `a`, language tags, blank nodes, @base, default prefix
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_a_keyword_and_language_tags() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let source = ttl(r#"
:Planta a owl:Class .
:rosa a :Planta ;
      rdfs:label "rosa"@es, "rose"@en ;
      :altura "40"^^xsd:integer .
"#);
        let report = import_turtle(&graph, &mut taxonomy, &source).await.unwrap();
        assert_eq!(report.classes_added, 1);
        assert_eq!(report.instances_added, 1);
        assert_eq!(report.triples_skipped, 0, "a lang tag must not desync the statement");

        let rosa = by_iri(&graph, "http://example.org/ontology#rosa").await;
        assert_eq!(rosa.label, "Planta");
        assert_eq!(
            rosa.properties.get(PROP_RDFS_LABEL),
            Some(&PropertyValue::List(vec![
                PropertyValue::String("rosa".into()),
                PropertyValue::String("rose".into()),
            ]))
        );
        assert_eq!(rosa.properties.get("altura"), Some(&PropertyValue::Int(40)));
    }

    #[tokio::test]
    async fn test_blank_node_subject_is_stable_across_reimport() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let source = ttl(r#"
:Planta a owl:Class .
_:anon a :Planta ; :nombreComun "sin nombre" .
"#);
        let first = import_turtle(&graph, &mut taxonomy, &source).await.unwrap();
        let second = import_turtle(&graph, &mut taxonomy, &source).await.unwrap();
        assert_eq!(first.instances_added, 1);
        assert!(zero(&second), "same document → same blank-node identity: {second:?}");

        let nodes = graph.get_nodes_by_label("Planta").await.unwrap();
        let anon = individual(&nodes);
        let iri = iri_of(anon);
        assert!(iri.starts_with("_:") && iri.ends_with("-anon"), "{iri}");

        let other = ttl("_:anon a :Planta ; :nombreComun \"otra\" .\n");
        let third = import_turtle(&graph, &mut taxonomy, &other).await.unwrap();
        assert_eq!(third.instances_added, 1, "a different document is a different blank node");
    }

    #[tokio::test]
    async fn test_base_iri_resolves_relative_references() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let source = format!(
            "@base <http://plants.example/> .\n{PREFIXES}<Planta> a owl:Class .\n<flor/tulipan> a <Planta> .\n"
        );
        let report = import_turtle(&graph, &mut taxonomy, &source).await.unwrap();
        assert_eq!(report.classes_added, 1);
        assert_eq!(report.instances_added, 1);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);

        let tulipan = by_iri(&graph, "http://plants.example/flor/tulipan").await;
        assert_eq!(tulipan.label, "Planta");
    }

    #[tokio::test]
    async fn test_missing_default_prefix_is_assumed_and_reported() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let source = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
:Animal rdf:type owl:Class .
:fido rdf:type :Animal .
"#;
        let report = import_turtle(&graph, &mut taxonomy, source).await.unwrap();
        assert_eq!(report.classes_added, 1);
        assert_eq!(report.instances_added, 1);
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert!(report.warnings[0].contains(DEFAULT_NAMESPACE));

        let fido = by_iri(&graph, &format!("{DEFAULT_NAMESPACE}fido")).await;
        assert_eq!(fido.label, "Animal");
    }

    // -----------------------------------------------------------------------
    // Test 10 — IRI identity: same local name, two namespaces → two nodes
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_same_local_name_in_two_namespaces_is_two_nodes() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let source = ttl(r#"
@prefix flora: <http://flora.example/> .
@prefix fauna: <http://fauna.example/> .
flora:Rosa a owl:Class .
fauna:Rosa a owl:Class .
flora:rosal a flora:Rosa .
fauna:rosalia a fauna:Rosa .
"#);
        let report = import_turtle(&graph, &mut taxonomy, &source).await.unwrap();
        assert_eq!(report.classes_added, 2);
        assert_eq!(report.instances_added, 2);

        let flora = by_iri(&graph, "http://flora.example/Rosa").await;
        let fauna = by_iri(&graph, "http://fauna.example/Rosa").await;
        assert_ne!(flora.id, fauna.id);
        assert_eq!(flora.label, "Rosa", "first one keeps the plain local name");
        assert_eq!(fauna.label, "fauna:Rosa", "second one is qualified so the taxonomy can tell them apart");
        assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
        assert!(report.warnings[0].contains("fauna:Rosa"));

        // Each individual is labelled by its own class, and the taxonomy knows both.
        assert_eq!(by_iri(&graph, "http://flora.example/rosal").await.label, "Rosa");
        assert_eq!(by_iri(&graph, "http://fauna.example/rosalia").await.label, "fauna:Rosa");
        assert!(taxonomy.find_by_label("Rosa").is_some());
        assert!(taxonomy.find_by_label("fauna:Rosa").is_some());
    }

    // -----------------------------------------------------------------------
    // Test 11 — resource-valued triple → edge; unknown object → placeholder
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_resource_object_creates_edge_and_placeholder() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let source = ttl(r#"
:Planta a owl:Class .
:rosa a :Planta ; :creceEn :jardin .
"#);
        let report = import_turtle(&graph, &mut taxonomy, &source).await.unwrap();
        assert_eq!(report.instances_added, 1);
        assert_eq!(report.placeholders_created, 1, ":jardin was never declared");
        assert_eq!(report.edges_created, 2, "instanceOf + creceEn");
        assert_eq!(report.triples_skipped, 0);

        let rosa = by_iri(&graph, "http://example.org/ontology#rosa").await;
        let jardin = by_iri(&graph, "http://example.org/ontology#jardin").await;
        assert_eq!(jardin.label, "jardin");
        assert_eq!(jardin.properties.get(PROP_PLACEHOLDER), Some(&PropertyValue::Bool(true)));

        let edges = graph.get_outgoing_edges(rosa.id).await.unwrap();
        let crece = edges.iter().find(|e| e.edge_type == "creceEn").expect("creceEn edge");
        assert_eq!(crece.target, jardin.id);
        assert_eq!(
            crece.properties.get(PROP_IRI),
            Some(&PropertyValue::String("http://example.org/ontology#creceEn".into()))
        );
        // Not a string property any more.
        assert!(rosa.properties.get("creceEn").is_none());

        // A later file types the placeholder: same node (same id), upgraded in place.
        let later = ttl(r#"
:Lugar a owl:Class .
:jardin a :Lugar ; :nombre "jardín trasero" .
"#);
        let report = import_turtle(&graph, &mut taxonomy, &later).await.unwrap();
        assert_eq!(report.placeholders_created, 0);
        assert_eq!(report.instances_added, 1, "the placeholder became an instance");
        let jardin2 = by_iri(&graph, "http://example.org/ontology#jardin").await;
        assert_eq!(jardin2.id, jardin.id, "edges pointing at it stay valid");
        assert_eq!(jardin2.label, "Lugar");
        assert!(jardin2.properties.get(PROP_PLACEHOLDER).is_none());
        assert_eq!(jardin2.properties.get("nombre"), Some(&PropertyValue::String("jardín trasero".into())));
    }

    // -----------------------------------------------------------------------
    // Test 12 — several rdf:type → one instanceOf edge each, first one labels
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_multiple_types_create_instanceof_edges() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let source = ttl(r#"
:Planta a owl:Class .
:Medicinal a owl:Class .
:manzanilla a :Planta, :Medicinal ; a owl:NamedIndividual .
"#);
        let report = import_turtle(&graph, &mut taxonomy, &source).await.unwrap();
        assert_eq!(report.instances_added, 1);
        assert_eq!(report.edges_created, 2, "one instanceOf per class; owl:NamedIndividual is a no-op");
        assert_eq!(report.triples_skipped, 0);

        let manzanilla = by_iri(&graph, "http://example.org/ontology#manzanilla").await;
        assert_eq!(manzanilla.label, "Planta", "the first type names the node");
        let mut targets: Vec<String> = Vec::new();
        for e in graph.get_outgoing_edges(manzanilla.id).await.unwrap() {
            assert_eq!(e.edge_type, EDGE_INSTANCE_OF);
            targets.push(graph.get_node(e.target).await.unwrap().label);
        }
        targets.sort();
        assert_eq!(targets, vec!["Medicinal", "Planta"]);
    }

    // -----------------------------------------------------------------------
    // Test 13 — classes in one file, instances in another: nothing skipped
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_two_file_import_nothing_skipped() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let ontology = ttl(r#"
:Planta a owl:Class .
:Arbol a owl:Class ; rdfs:subClassOf :Planta .
"#);
        let data = ttl(r#"
:roble a :Arbol ; :altura "20"^^xsd:integer ; :vecinoDe :haya .
:haya a :Arbol .
"#);
        let first = import_turtle(&graph, &mut taxonomy, &ontology).await.unwrap();
        let second = import_turtle(&graph, &mut taxonomy, &data).await.unwrap();
        assert_eq!(first.classes_added, 2);
        assert_eq!(second.classes_added, 0, "Arbol comes from the graph, not this file");
        assert_eq!(second.instances_added, 2);
        assert_eq!(second.edges_created, 3, "2 instanceOf + vecinoDe");
        assert_eq!(second.placeholders_created, 0, "haya is typed in the same file");
        assert_eq!(second.triples_skipped, 0);
        assert!(second.warnings.is_empty(), "{:?}", second.warnings);
    }

    // -----------------------------------------------------------------------
    // Test 14 — a user predicate named like a reserved edge is qualified
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_user_predicate_named_instanceof_is_qualified() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let source = ttl(r#"
:Planta a owl:Class .
:a a :Planta . :b a :Planta .
:a :instanceOf :b .
:a :subClassOf :b .
"#);
        let report = import_turtle(&graph, &mut taxonomy, &source).await.unwrap();
        assert_eq!(report.edges_created, 4);
        assert_eq!(report.warnings.len(), 2, "{:?}", report.warnings);

        let a = by_iri(&graph, "http://example.org/ontology#a").await;
        let mut types: Vec<String> = graph
            .get_outgoing_edges(a.id)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.edge_type)
            .collect();
        types.sort();
        assert_eq!(types, vec![":instanceOf", ":subClassOf", "instanceOf"]);
        // And the taxonomy is untouched by the user edge.
        assert_eq!(taxonomy.size(), 1);
    }

    // -----------------------------------------------------------------------
    // Test 15 — nodes from a pre-0.5.10 import are reported, not merged
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_legacy_nodes_are_reported() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        // What the old importer left: class without iri, individual with the raw token.
        let mut planta = Node::new("Planta");
        planta.kind = NodeKind::Class;
        let planta_id = graph.add_node(planta).await.unwrap();
        let mut rosa = Node::new("Planta");
        rosa.properties.insert(PROP_IRI.into(), PropertyValue::String(":rosa".into()));
        graph.add_node(rosa).await.unwrap();

        let source = ttl(r#"
:Planta a owl:Class .
:rosa a :Planta .
"#);
        let report = import_turtle(&graph, &mut taxonomy, &source).await.unwrap();
        // The class is adopted (same node, now with an iri); the individual is not.
        assert_eq!(report.classes_added, 0);
        assert_eq!(by_iri(&graph, "http://example.org/ontology#Planta").await.id, planta_id);
        assert_eq!(report.instances_added, 1);
        assert_eq!(graph.get_nodes_by_label("Planta").await.unwrap().len(), 3, "class + legacy + new");
        assert!(report.warnings.iter().any(|w| w.contains("anterior a 0.5.10")), "{:?}", report.warnings);
        assert!(report.warnings.iter().any(|w| w.contains("existía sin IRI")), "{:?}", report.warnings);
    }

    // -----------------------------------------------------------------------
    // Test 15b — a raw `subClassOf` edge touching an individual does not enter
    // the taxonomy when the graph is (re)built from storage
    // -----------------------------------------------------------------------
    //
    // `TaxonomyIndex::add_subclass` accepts any pair of ids, so before the
    // guard a `subClassOf` edge from an individual to a class made that
    // individual a "subclass" on every open. Any client can write such an
    // edge; the importer is only the most likely one.
    #[tokio::test]
    async fn test_rebuild_taxonomy_ignores_edges_touching_individuals() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let source = ttl(r#"
:Planta a owl:Class .
:Arbol a owl:Class ; rdfs:subClassOf :Planta .
:roble a :Arbol .
"#);
        import_turtle(&graph, &mut taxonomy, &source).await.unwrap();
        let planta = by_iri(&graph, "http://example.org/ontology#Planta").await;
        let arbol = by_iri(&graph, "http://example.org/ontology#Arbol").await;
        let roble = by_iri(&graph, "http://example.org/ontology#roble").await;

        graph.add_edge(Edge::new(roble.id, planta.id, EDGE_SUBCLASS_OF)).await.unwrap();

        graph.rebuild_taxonomy_from_graph().await.unwrap();
        let mut rebuilt = graph.get_taxonomy_sync().expect("taxonomy after rebuild");
        assert!(rebuilt.is_subclass_of(arbol.id, planta.id), "the declared axiom survives");
        assert!(!rebuilt.is_subclass_of(roble.id, planta.id), "the individual is not a class");
        assert!(rebuilt.is_subclass_of_label("Arbol", planta.id));
    }

    // -----------------------------------------------------------------------
    // Test 16 — prefixes persist in the catalog and merge across imports
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_prefixes_persist_and_merge() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_str().unwrap().to_string();
        {
            let graph = Graph::open(&path).await.unwrap();
            let mut taxonomy = TaxonomyIndex::new();
            assert!(graph.rdf_prefixes().await.unwrap().is_empty());
            let a = ttl("@prefix flora: <http://flora.example/> .\nflora:Rosa a owl:Class .\n");
            import_turtle(&graph, &mut taxonomy, &a).await.unwrap();
            let b = ttl("@prefix fauna: <http://fauna.example/> .\n@prefix flora: <http://flora.example/v2/> .\nfauna:Lobo a owl:Class .\n");
            import_turtle(&graph, &mut taxonomy, &b).await.unwrap();
            graph.close().await.unwrap();
        }
        let graph = Graph::open(&path).await.unwrap();
        let prefixes = graph.rdf_prefixes().await.unwrap();
        assert_eq!(prefixes.get("").map(String::as_str), Some("http://example.org/ontology#"));
        assert_eq!(prefixes.get("fauna").map(String::as_str), Some("http://fauna.example/"));
        assert_eq!(prefixes.get("flora").map(String::as_str), Some("http://flora.example/v2/"), "later import wins");
        assert_eq!(prefixes.get("xsd").map(String::as_str), Some("http://www.w3.org/2001/XMLSchema#"));
    }
}
