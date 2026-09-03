// src/rdf_owl/importer.rs
//
// OWL/Turtle Importer.
//
// Parsing is delegated to `oxttl` (a real Turtle grammar: `a`, language tags,
// blank nodes, `@base`, collections, comments inside literals, and a
// positioned error on malformed input). This module decides what the parsed
// triples MEAN for the property graph:
//   - Class declarations:      `:Foo a owl:Class .`
//   - Subclass axioms:         `:Foo rdfs:subClassOf :Bar .`
//   - Individual declarations: `:alice a :Person .`            (pass 3)
//   - Data properties:         `:alice :age "30"^^xsd:integer .` (pass 3)
//
// Every triple that none of the passes consumed is counted in
// `triples_skipped` and ignored. The full list of what survives the bridge
// and what does not lives in the module docs (`rdf_owl/mod.rs`).
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
/// `owl`, so the one term this importer needs is declared here.
const OWL_CLASS: NamedNodeRef<'static> =
    NamedNodeRef::new_unchecked("http://www.w3.org/2002/07/owl#Class");

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

// ---------------------------------------------------------------------------
// Public result type
// ---------------------------------------------------------------------------

/// Summary of a Turtle import operation.
#[derive(Debug, Clone, Default)]
pub struct ImportReport {
    /// Number of `owl:Class` declarations processed (new nodes added to graph).
    pub classes_added: usize,
    /// Number of `rdfs:subClassOf` edges added.
    pub subclass_edges_added: usize,
    /// Number of individual (`rdf:type <non-Class>`) instances added to graph.
    pub instances_added: usize,
    /// Number of triples that no pass consumed, i.e. that left nothing in the
    /// graph: metadata on classes (`rdfs:label`, `rdfs:comment`), `rdf:type`
    /// pointing at a class this import does not know, `owl:Ontology` headers,
    /// and any other axiom the importer does not model.
    ///
    /// Data properties of individuals are **not** skipped — they become node
    /// properties in pass 3 — so this count is exactly what was lost. It is
    /// computed once, after all passes, from the same predicate each pass
    /// used to decide; counting inside a pass over-reported (pass 2 used to
    /// count every non-`type` triple, including the properties pass 3 then
    /// imported).
    pub triples_skipped: usize,
    /// Things the import did on the document's behalf that the author should
    /// know about: an assumed default namespace, a literal typed
    /// `xsd:integer` that did not parse as one, and so on. Empty means the
    /// document was taken exactly as written.
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Parsed document
// ---------------------------------------------------------------------------

/// What `parse_turtle` hands to the passes: the triples and the parser-level
/// warnings. (The document's prefix map is read from the parser too; it
/// becomes part of this struct when the graph starts persisting prefixes.)
pub(crate) struct ParsedDocument {
    pub triples: Vec<Triple>,
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

    Ok(ParsedDocument {
        triples,
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
/// synthetic blank-node IRI the label after the hash.
///
/// This is the identity of classes on this side of the bridge for now: two
/// IRIs with the same local name in different namespaces collapse into one
/// node. Keeping IRI identity is the next step of the bridge (see the module
/// docs); this function is where it will change.
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
/// previous parser guessed `Int` from the shape of the text, so `"42"^^xsd:string`
/// became a number; that guess is gone. A typed literal whose text does not
/// parse as its type falls back to `String` and is reported in `warnings`
/// rather than dropped or coerced.
fn literal_to_property_value(lit: &oxrdf::Literal, warnings: &mut Vec<String>) -> PropertyValue {
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
// Public API
// ---------------------------------------------------------------------------

/// Import a Turtle source string into `graph`, registering `owl:Class` nodes,
/// `rdfs:subClassOf` edges, individual instances, and updating the `taxonomy` index.
///
/// Only the following triple patterns are processed:
/// - `?s rdf:type owl:Class`         → node with `NodeKind::Class`
/// - `?s rdfs:subClassOf ?o`         → `subClassOf` edge + taxonomy edge
/// - `?s rdf:type :SomeClass`        → Individual node with label = SomeClass (Pass 3)
/// - `?s :prop ?o` (where s is an individual)  → property on Individual node
///
/// Every other triple is counted in [`ImportReport::triples_skipped`] and
/// dropped. What the bridge keeps and what it loses is spelled out in the
/// [module docs](crate::rdf_owl).
///
/// Malformed Turtle is an error ([`NopalError::RdfParseError`], with line and
/// column), and nothing is written: a document is imported whole or not at all.
///
/// The function is idempotent: if a class or individual with the same IRI already
/// exists, it is reused rather than duplicated.
pub async fn import_turtle(
    graph: &Graph,
    taxonomy: &mut TaxonomyIndex,
    source: &str,
) -> Result<ImportReport> {
    let mut report = ImportReport::default();

    // Step 1 — parse. Fails here, before any write, on malformed input.
    let doc = parse_turtle(source)?;
    report.warnings.extend(doc.warnings.iter().cloned());
    let hash = doc.document_hash;
    let triples = &doc.triples;

    // Step 2 — resolve and collect classes first (pass 1).
    // We need all classes before wiring subClassOf edges.
    let mut label_to_id: HashMap<String, NodeId> = HashMap::new();

    // Track which subjects are known owl:Class IRIs (for Pass 3 exclusion).
    let mut class_iris: HashSet<String> = HashSet::new();

    // Pass 1: find rdf:type owl:Class triples.
    for triple in triples {
        if triple.predicate != rdf::TYPE {
            continue;
        }
        let Object::Resource(obj) = object_of(&triple.object, hash) else { continue };
        if obj != OWL_CLASS.as_str() {
            continue;
        }
        let subj = subject_iri(&triple.subject, hash);
        let class_label = local_name(&subj);
        if class_label.is_empty() {
            continue;
        }

        class_iris.insert(subj);

        // Idempotency: reuse existing node if label already in graph.
        let node_id = match find_class(graph, &class_label).await? {
            Some(id) => id,
            None => {
                let mut node = Node::new(class_label.clone());
                node.kind = NodeKind::Class;
                graph.add_node(node).await?
            }
        };

        label_to_id.insert(class_label.clone(), node_id);

        // Register in taxonomy (idempotent).
        taxonomy.register_class(node_id, &class_label);
        report.classes_added += 1;
    }

    // Pass 2: wire rdfs:subClassOf edges.
    for triple in triples {
        if triple.predicate != rdfs::SUB_CLASS_OF {
            continue;
        }
        let Object::Resource(obj) = object_of(&triple.object, hash) else { continue };
        let sub_label = local_name(&subject_iri(&triple.subject, hash));
        let super_label = local_name(&obj);

        if sub_label.is_empty() || super_label.is_empty() {
            // Counted as skipped in the final tally below.
            continue;
        }

        // Ensure both endpoints are known (lazily create if missing).
        let sub_id = ensure_class(graph, taxonomy, &mut label_to_id, &sub_label).await?;
        let super_id = ensure_class(graph, taxonomy, &mut label_to_id, &super_label).await?;

        // Add graph edge (duplicate edges are rare in TTL and taxonomy is idempotent).
        let edge = Edge {
            id: uuid::Uuid::new_v4(),
            source: sub_id,
            target: super_id,
            edge_type: "subClassOf".to_string(),
            properties: Default::default(),
        };
        graph.add_edge(edge).await?;

        // Wire taxonomy (idempotent: add_subclass ignores duplicates).
        // Convention: add_subclass(parent, child) means child ⊑ parent.
        taxonomy.add_subclass(super_id, sub_id)?;
        report.subclass_edges_added += 1;
    }

    // Pass 3: import individuals (rdf:type <non-owl:Class>) and their data properties.
    //
    // Strategy: two mini-passes over triples.
    //   3a. Identify individual subjects: those with rdf:type whose object resolves
    //       to a known class label (but the object IRI is NOT an owl:Class itself).
    //   3b. Collect data properties for those subjects.
    //   3c. Create Individual nodes (idempotent via IRI property check).

    // 3a: collect individual subject → class label mapping.
    let mut individuals: HashMap<String, String> = HashMap::new(); // subj_iri → class_label

    for triple in triples {
        if triple.predicate != rdf::TYPE {
            continue;
        }
        let Object::Resource(obj_iri) = object_of(&triple.object, hash) else { continue };
        if obj_iri == OWL_CLASS.as_str() {
            continue;
        }
        let obj = local_name(&obj_iri);
        if obj.is_empty() {
            continue;
        }
        let subj = subject_iri(&triple.subject, hash);
        // Skip subjects that were declared as owl:Class themselves.
        if class_iris.contains(&subj) {
            continue;
        }
        // The object is a class label (e.g. "Person"). Only add if the class
        // is known: declared as owl:Class in this file, or already in the
        // graph from an earlier import (ontology in one file, instances in
        // another). Otherwise the triple is counted as skipped below.
        if !label_to_id.contains_key(&obj)
            && let Some(id) = find_class(graph, &obj).await?
        {
            taxonomy.register_class(id, &obj);
            label_to_id.insert(obj.clone(), id);
        }
        if label_to_id.contains_key(&obj) {
            individuals.entry(subj).or_insert_with(|| obj.clone());
        }
    }

    if !individuals.is_empty() {
        // 3b: collect data properties per individual. A predicate that appears
        // more than once for the same subject keeps every value, as a `List`.
        let mut props_map: HashMap<String, BTreeMap<String, Vec<PropertyValue>>> = HashMap::new();
        for triple in triples {
            if triple.predicate == rdf::TYPE {
                continue;
            }
            let subj = subject_iri(&triple.subject, hash);
            if !individuals.contains_key(&subj) {
                continue;
            }
            let pred = local_name(triple.predicate.as_str());
            if pred.is_empty() {
                continue;
            }
            let val = match object_of(&triple.object, hash) {
                Object::Literal(l) => literal_to_property_value(l, &mut report.warnings),
                // A resource-valued object is stored as its IRI, as text.
                // Turning it into an edge is the next step of the bridge.
                Object::Resource(iri) => PropertyValue::String(iri),
            };
            props_map.entry(subj).or_default().entry(pred).or_default().push(val);
        }

        // 3c: create Individual nodes (idempotent).
        for (subj_iri, class_label) in &individuals {
            let props = props_map.remove(subj_iri).unwrap_or_default();

            // Idempotency: check if a node with this IRI already exists.
            let existing = graph.get_nodes_by_label(class_label).await?;
            let already_exists = existing.iter().any(|n| {
                matches!(n.properties.get("iri"), Some(PropertyValue::String(v)) if v == subj_iri)
            });

            if already_exists {
                continue;
            }

            let mut node = Node::new(class_label.as_str());
            node.properties.insert(
                "iri".to_string(),
                PropertyValue::String(subj_iri.clone()),
            );
            for (k, mut values) in props {
                let v = if values.len() == 1 {
                    values.pop().expect("len checked")
                } else {
                    PropertyValue::List(values)
                };
                node.properties.insert(k, v);
            }
            graph.add_node(node).await?;
            report.instances_added += 1;
        }
    }

    // Final tally: a triple is "skipped" when no pass consumed it. This is
    // decided here, once, with the same tests the passes used — so the count
    // is what actually left nothing in the graph, not a per-pass guess.
    // Idempotent re-imports still count their triples as consumed: the data
    // is in the graph, whether this call put it there or an earlier one did.
    for triple in triples {
        let subj = subject_iri(&triple.subject, hash);
        let consumed = if triple.predicate == rdf::TYPE {
            match object_of(&triple.object, hash) {
                // Pass 1 (class declaration) or pass 3a (individual of a known class).
                Object::Resource(obj) if obj == OWL_CLASS.as_str() => !local_name(&subj).is_empty(),
                Object::Resource(obj) => individuals.get(&subj) == Some(&local_name(&obj)),
                Object::Literal(_) => false,
            }
        } else if triple.predicate == rdfs::SUB_CLASS_OF {
            // Pass 2 — both endpoints had to resolve to a label.
            match object_of(&triple.object, hash) {
                Object::Resource(obj) => !local_name(&subj).is_empty() && !local_name(&obj).is_empty(),
                Object::Literal(_) => false,
            }
        } else {
            // Pass 3b — data property of an individual.
            !local_name(triple.predicate.as_str()).is_empty() && individuals.contains_key(&subj)
        };
        if !consumed {
            report.triples_skipped += 1;
        }
    }

    Ok(report)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Find the `NodeKind::Class` node labelled `label` already stored in the graph,
/// if any. Labels are the identity of classes on this side of the bridge (see
/// the module docs on why prefixes are dropped).
async fn find_class(graph: &Graph, label: &str) -> Result<Option<NodeId>> {
    let existing = graph.get_nodes_by_label(label).await?;
    Ok(existing
        .into_iter()
        .find(|n| n.kind == NodeKind::Class)
        .map(|n| n.id))
}

/// Ensure a class node with `label` exists in graph + taxonomy, creating it if
/// necessary. Returns its `NodeId`.
async fn ensure_class(
    graph: &Graph,
    taxonomy: &mut TaxonomyIndex,
    cache: &mut HashMap<String, NodeId>,
    label: &str,
) -> Result<NodeId> {
    if let Some(&id) = cache.get(label) {
        return Ok(id);
    }

    // Look in graph.
    if let Some(id) = find_class(graph, label).await? {
        taxonomy.register_class(id, label);
        cache.insert(label.to_string(), id);
        return Ok(id);
    }

    // Create new.
    let mut node = Node::new(label);
    node.kind = NodeKind::Class;
    let id = graph.add_node(node).await?;
    taxonomy.register_class(id, label);
    cache.insert(label.to_string(), id);
    Ok(id)
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

        // Class node exists in graph
        let nodes = graph.get_nodes_by_label("Animal").await.unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].kind, NodeKind::Class);

        // Registered in taxonomy
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

        // Transitive: Dog ⊑ Animal
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
        assert_eq!(report.triples_skipped, 0, "both data properties end up on the node");

        let nodes = graph.get_nodes_by_label("Animal").await.unwrap();
        let fido = individual(&nodes);
        assert_eq!(fido.properties.get("label"), Some(&PropertyValue::String("Fido".into())));
        assert_eq!(fido.properties.get("age"), Some(&PropertyValue::Int(5)));
    }

    // -----------------------------------------------------------------------
    // Test 3b — triples_skipped counts exactly what left nothing in the graph
    // -----------------------------------------------------------------------
    //
    // Regression for the over-count: pass 2 used to add every non-`type`
    // triple to the tally, so the two data properties below were reported as
    // lost while they were sitting on the node.
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

        // Now three triples that really are dropped, mixed with consumed ones:
        //   - rdfs:label on a CLASS (pass 3 only takes properties of individuals)
        //   - rdf:type pointing at a class nobody declared
        //   - a data property whose subject is that undeclared-class individual
        let source = ttl(r#"
:Planta rdfs:label "Planta" .
:Cactus rdf:type :Suculenta .
:Cactus :nombreComun "cactus" .
:Tulipan rdf:type :Planta .
:Tulipan :nombreComun "tulipán" .
"#);
        let report = import_turtle(&graph, &mut taxonomy, &source).await.unwrap();
        assert_eq!(report.classes_added, 0);
        assert_eq!(
            report.instances_added, 1,
            "Tulipan is a Planta (declared by the previous import); Cactus has no known class"
        );
        assert_eq!(report.triples_skipped, 3);
    }

    // -----------------------------------------------------------------------
    // Test 3c — re-importing the same file skips nothing (the data is there)
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_reimport_reports_no_skipped_triples() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let source = ttl(r#"
:Planta rdf:type owl:Class .
:Arbol rdf:type owl:Class .
:Arbol rdfs:subClassOf :Planta .
:Rosa rdf:type :Planta .
:Rosa :nombreComun "rosa" .
"#);
        let first = import_turtle(&graph, &mut taxonomy, &source).await.unwrap();
        let second = import_turtle(&graph, &mut taxonomy, &source).await.unwrap();

        assert_eq!(first.triples_skipped, 0);
        assert_eq!(second.instances_added, 0, "idempotent: node already there");
        assert_eq!(second.triples_skipped, 0, "already-present data is not 'lost'");
    }

    // -----------------------------------------------------------------------
    // Test 4 — idempotent: import same TTL twice → no duplicate nodes
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

        // Taxonomy size stays at 1
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

        // D ⊑ A transitively
        assert!(taxonomy.is_subclass_of(d_id, a_id));

        // Ancestors of D: B, C, A (3 ancestors)
        let anc = taxonomy.ancestors(d_id);
        assert_eq!(anc.len(), 3);
    }

    // -----------------------------------------------------------------------
    // Test 6 — local_name over absolute IRIs and synthetic blank-node IRIs
    // -----------------------------------------------------------------------
    #[test]
    fn test_local_name_extraction() {
        assert_eq!(local_name("http://example.org/Animal"), "Animal");
        assert_eq!(local_name("http://www.w3.org/2002/07/owl#Class"), "Class");
        assert_eq!(local_name("http://example.org/ontology#Animal"), "Animal");
        assert_eq!(local_name("_:00000000deadbeef-b0"), "b0");
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
        // xsd:string stays a string even when it looks like a number.
        assert_eq!(literal_to_property_value(&typed("42", xsd::STRING), &mut w), PropertyValue::String("42".into()));
        // A plain literal is xsd:string by definition: no guessing from the shape.
        assert_eq!(
            literal_to_property_value(&Literal::new_simple_literal("42"), &mut w),
            PropertyValue::String("42".into())
        );
        // Unknown datatypes keep the lexical value.
        assert_eq!(
            literal_to_property_value(&typed("2026-09-02", xsd::DATE), &mut w),
            PropertyValue::String("2026-09-02".into())
        );
        assert!(w.is_empty(), "{w:?}");

        // A typed literal that does not parse as its type: text + warning, never dropped.
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

        // Whole-or-nothing: the valid first statement was not written either.
        assert!(graph.get_nodes_by_label("Animal").await.unwrap().is_empty());
        assert_eq!(taxonomy.size(), 0);
    }

    // -----------------------------------------------------------------------
    // Test 9 — `a`, language tags, blank nodes, @base, several prefixes
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

        let nodes = graph.get_nodes_by_label("Planta").await.unwrap();
        let rosa = individual(&nodes);
        // Two values for the same predicate → a List; the tags themselves are dropped.
        assert_eq!(
            rosa.properties.get("label"),
            Some(&PropertyValue::List(vec![
                PropertyValue::String("rosa".into()),
                PropertyValue::String("rose".into()),
            ]))
        );
        assert_eq!(rosa.properties.get("altura"), Some(&PropertyValue::Int(40)));
        assert_eq!(
            rosa.properties.get("iri"),
            Some(&PropertyValue::String("http://example.org/ontology#rosa".into()))
        );
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
        assert_eq!(second.instances_added, 0, "same document → same blank-node identity");

        let nodes = graph.get_nodes_by_label("Planta").await.unwrap();
        let anon = individual(&nodes);
        let Some(PropertyValue::String(iri)) = anon.properties.get("iri") else { panic!() };
        assert!(iri.starts_with("_:") && iri.ends_with("-anon"), "{iri}");

        // A different document with the same label is a different node.
        let other = ttl(r#"
_:anon a :Planta ; :nombreComun "otra" .
"#);
        let third = import_turtle(&graph, &mut taxonomy, &other).await.unwrap();
        assert_eq!(third.instances_added, 1);
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

        let nodes = graph.get_nodes_by_label("Planta").await.unwrap();
        let tulipan = individual(&nodes);
        assert_eq!(
            tulipan.properties.get("iri"),
            Some(&PropertyValue::String("http://plants.example/flor/tulipan".into()))
        );
    }

    #[tokio::test]
    async fn test_missing_default_prefix_is_assumed_and_reported() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        // No `@prefix :` line at all — the shape of every pre-existing fixture.
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

        let nodes = graph.get_nodes_by_label("Animal").await.unwrap();
        assert_eq!(
            individual(&nodes).properties.get("iri"),
            Some(&PropertyValue::String(format!("{DEFAULT_NAMESPACE}fido")))
        );
    }

    // -----------------------------------------------------------------------
    // Test 10 — what this step does NOT do yet, stated as a test
    // -----------------------------------------------------------------------
    //
    // Two IRIs with the same local name still collapse into one node: identity
    // is by label here. The IRI step of the bridge turns this into two nodes;
    // when it lands, this test flips.
    #[tokio::test]
    async fn test_same_local_name_in_two_namespaces_still_collapses() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let source = ttl(r#"
@prefix flora: <http://flora.example/> .
@prefix fauna: <http://fauna.example/> .
flora:Rosa a owl:Class .
fauna:Rosa a owl:Class .
"#);
        let report = import_turtle(&graph, &mut taxonomy, &source).await.unwrap();
        assert_eq!(report.classes_added, 2, "both declarations are processed…");
        assert_eq!(
            graph.get_nodes_by_label("Rosa").await.unwrap().len(),
            1,
            "…but they are one node: identity is by label at this step"
        );
    }
}
