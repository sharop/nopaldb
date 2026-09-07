// src/rdf_owl/exporter.rs
//
// Turtle exporter, the symmetric half of the bridge: everything the importer
// writes into the graph goes back out as the Turtle it came from, and what
// cannot be written is reported, never dropped in silence.
//
// What comes out, per exported node (a node with an `iri`, class or individual):
//   - `a owl:Class` for classes; `rdfs:subClassOf` for Class→Class `subClassOf` edges
//   - `a <C>` for every `instanceOf` edge of an individual (label fallback for
//     nodes without them: built by hand or imported before 0.5.10)
//   - one triple per other edge, predicate = the edge's `iri` property when it
//     has one (the importer always sets it), else the export namespace + edge type
//   - one literal per property, typed by the same table the importer reads
//     (`Int` → xsd:integer, `Float` → xsd:double, `Bool` → xsd:boolean,
//     `String` → plain literal); a `List` becomes one triple per element,
//     which is how the importer produced it
//
// Serialization is delegated to `oxttl::TurtleSerializer`, the same family as
// the parser, so the output is Turtle by construction (escaping, prefixed-name
// validity, IRI validity) and not by hand-rolled string building. Prefixes are
// the graph's own catalog (`Graph::rdf_prefixes`) plus the four standard ones.
//
// Feature gate: compiled only with `owl-import`.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use oxrdf::vocab::{rdf, rdfs, xsd};
use oxrdf::{BlankNode, Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use oxttl::TurtleSerializer;

use crate::error::{NopalError, Result};
use crate::graph::Graph;
use crate::rdf_owl::importer::{
    DEFAULT_NAMESPACE, EDGE_INSTANCE_OF, EDGE_SUBCLASS_OF, NS_OWL, NS_RDF, NS_RDFS, NS_XSD, OWL_CLASS, PROP_IRI,
    PROP_PLACEHOLDER, PROP_RDFS_COMMENT, PROP_RDFS_LABEL,
};
use crate::types::{Edge, Node, NodeId, NodeKind, PropertyValue};

// ---------------------------------------------------------------------------
// Public result types
// ---------------------------------------------------------------------------

/// What an export wrote and what it could not write.
///
/// Every counter counts triples actually serialized. `skipped` lists, one
/// line each, the values and edges that have no Turtle representation and
/// were left out, so a caller can tell a faithful export (`skipped` empty)
/// from a narrowed one without diffing files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExportReport {
    /// Class nodes written (`a owl:Class`).
    pub classes: usize,
    /// `rdfs:subClassOf` triples written (Class → Class edges only).
    pub subclass_edges: usize,
    /// Individuals that wrote at least one triple (a placeholder nothing
    /// describes appears only as an object and is not counted).
    pub individuals: usize,
    /// `rdf:type` triples written for individuals (one per `instanceOf` edge).
    pub type_triples: usize,
    /// Edges written as object properties (everything but `instanceOf` and
    /// Class → Class `subClassOf`).
    pub edges: usize,
    /// Literal triples written (a `List` counts one per element).
    pub literals: usize,
    /// Total triples handed to the serializer.
    pub triples_written: usize,
    /// One line per value or edge left out, with the reason. Empty means the
    /// graph's RDF content round-trips whole.
    pub skipped: Vec<String>,
}

/// The Turtle text of an export together with its [`ExportReport`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurtleExport {
    /// The document, ready to write to a `.ttl` file or feed to a parser.
    pub turtle: String,
    /// What was written and what was skipped.
    pub report: ExportReport,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Export the RDF content of `graph` as Turtle.
///
/// Exported nodes are those with an `iri` property (every node the importer
/// creates, class or individual); ordinary NopalDB nodes are not, so a mixed
/// graph exports only its ontology and instance data. Edges leaving an
/// exported node towards a node without `iri` are reported in
/// [`ExportReport::skipped`], as are property values Turtle cannot carry
/// (`Null`, `Bytes`, `Object`, nested lists, NaN/∞) and edge properties other
/// than the predicate IRI (RDF edges have none).
///
/// The empty prefix is the one the imported documents declared (the graph's
/// prefix catalog); a graph that never imported Turtle gets
/// `http://example.org/ontology#`, the namespace the importer assumes for a
/// document without `@prefix :`. Output is deterministic: subjects are
/// ordered by IRI, predicates by name, so two exports of the same graph are
/// byte-identical and diffable.
pub async fn export_turtle(graph: &Graph) -> Result<TurtleExport> {
    let all_nodes = graph.get_all_nodes().await?;
    let all_edges = graph.get_all_edges().await?;
    let prefixes = graph.rdf_prefixes().await?;
    Export::new(all_nodes, all_edges, prefixes).run()
}

// ---------------------------------------------------------------------------
// Export context
// ---------------------------------------------------------------------------

struct Export {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    prefixes: BTreeMap<String, String>,
    /// Namespace behind the empty prefix, used for every term the graph has no
    /// IRI for (property keys, edge types without `iri`, legacy labels).
    export_ns: String,
    /// Exported node → its RDF term. A node absent here is not exported.
    terms: HashMap<NodeId, NamedOrBlankNode>,
    is_class: HashMap<NodeId, bool>,
    /// Label → class term, for individuals without `instanceOf` edges.
    class_by_label: HashMap<String, NamedOrBlankNode>,
    report: ExportReport,
    triples: Vec<Triple>,
}

impl Export {
    fn new(nodes: Vec<Node>, edges: Vec<Edge>, mut prefixes: BTreeMap<String, String>) -> Self {
        // The standard vocabularies always compact; the catalog never overrides them.
        prefixes.insert("rdf".into(), NS_RDF.into());
        prefixes.insert("rdfs".into(), NS_RDFS.into());
        prefixes.insert("owl".into(), NS_OWL.into());
        prefixes.insert("xsd".into(), NS_XSD.into());
        let export_ns = prefixes.entry(String::new()).or_insert_with(|| DEFAULT_NAMESPACE.into()).clone();
        Self {
            nodes,
            edges,
            prefixes,
            export_ns,
            terms: HashMap::new(),
            is_class: HashMap::new(),
            class_by_label: HashMap::new(),
            report: ExportReport::default(),
            triples: Vec::new(),
        }
    }

    fn run(mut self) -> Result<TurtleExport> {
        self.assign_terms();

        // Deterministic order: subjects by term text, then per-subject groups.
        let mut order: Vec<(String, usize)> = self
            .nodes
            .iter()
            .enumerate()
            .filter_map(|(i, n)| self.terms.get(&n.id).map(|t| (t.to_string(), i)))
            .collect();
        order.sort();

        let edges = std::mem::take(&mut self.edges);
        let mut out_edges: HashMap<NodeId, Vec<&Edge>> = HashMap::new();
        for e in &edges {
            out_edges.entry(e.source).or_default().push(e);
        }

        let nodes = std::mem::take(&mut self.nodes);
        for (_, i) in order {
            let node = &nodes[i];
            let edges = out_edges.get(&node.id).map(|v| v.as_slice()).unwrap_or(&[]);
            self.write_node(node, edges);
        }

        let turtle = self.serialize()?;
        Ok(TurtleExport { turtle, report: self.report })
    }

    /// Decide which nodes are exported and under which term.
    fn assign_terms(&mut self) {
        for node in &self.nodes {
            let term = match node.properties.get(PROP_IRI) {
                Some(PropertyValue::String(stored)) => stored_term(stored, &self.export_ns),
                // A class from a database older than 0.5.10 has no IRI: its label
                // under the export namespace is what the old exporter wrote too.
                _ if node.kind == NodeKind::Class => named(&format!("{}{}", self.export_ns, iri_safe(&node.label))),
                _ => None,
            };
            let Some(term) = term else {
                if node.kind == NodeKind::Class || node.properties.contains_key(PROP_IRI) {
                    self.report.skipped.push(format!(
                        "nodo `{}` ({}): su `iri` no es un IRI ni un blank node válido, no se exportó",
                        node.label, node.id
                    ));
                }
                continue;
            };
            let class = node.kind == NodeKind::Class;
            if class {
                self.class_by_label.entry(node.label.clone()).or_insert_with(|| term.clone());
            }
            self.is_class.insert(node.id, class);
            self.terms.insert(node.id, term);
        }
    }

    fn write_node(&mut self, node: &Node, edges: &[&Edge]) {
        let subject = self.terms[&node.id].clone();
        let class = self.is_class[&node.id];
        let placeholder = matches!(node.properties.get(PROP_PLACEHOLDER), Some(PropertyValue::Bool(true)));

        let written_before = self.report.triples_written;

        // 1. Types.
        if class {
            self.emit(subject.clone(), rdf::TYPE.into_owned(), OWL_CLASS.into_owned().into());
            self.report.classes += 1;
        } else {
            let mut types: Vec<NamedOrBlankNode> = edges
                .iter()
                .filter(|e| e.edge_type == EDGE_INSTANCE_OF)
                .filter_map(|e| self.terms.get(&e.target).cloned())
                .collect();
            if types.is_empty() && !placeholder {
                // No `instanceOf` edges: the label is the class (hand-made node,
                // NQL `add`, or import older than 0.5.10). A placeholder has no
                // type by definition: inventing one would create a class on
                // re-import that the source document never declared.
                let t = self
                    .class_by_label
                    .get(&node.label)
                    .cloned()
                    .or_else(|| named(&format!("{}{}", self.export_ns, iri_safe(&node.label))));
                types.extend(t);
            }
            types.sort_by_key(ToString::to_string);
            types.dedup();
            // The importer makes the FIRST type the node's label. Writing the
            // label's class first keeps that choice across a round trip;
            // otherwise a re-import would relabel `:x a :B, :A` as `A`.
            if let Some(label_class) = self.class_by_label.get(&node.label)
                && let Some(pos) = types.iter().position(|t| t == label_class)
                && pos != 0
            {
                let first = types.remove(pos);
                types.insert(0, first);
            }
            for t in types {
                self.emit(subject.clone(), rdf::TYPE.into_owned(), t.into());
                self.report.type_triples += 1;
            }
        }

        // 2. Edges: subClassOf between classes, then everything else as object properties.
        let mut object_triples: Vec<(NamedNode, NamedOrBlankNode)> = Vec::new();
        for e in edges {
            if e.edge_type == EDGE_INSTANCE_OF {
                continue;
            }
            let Some(target) = self.terms.get(&e.target).cloned() else {
                self.report.skipped.push(format!(
                    "arista `{}` de {} → nodo sin `iri` ({}): fuera del export",
                    e.edge_type, subject, e.target
                ));
                continue;
            };
            let target_is_class = self.is_class.get(&e.target).copied().unwrap_or(false);
            if e.edge_type == EDGE_SUBCLASS_OF && class && target_is_class {
                object_triples.push((rdfs::SUB_CLASS_OF.into_owned(), target));
                self.report.subclass_edges += 1;
                continue;
            }
            let predicate = match e.properties.get(PROP_IRI) {
                Some(PropertyValue::String(iri)) => named_node(iri),
                _ => named_node(&format!("{}{}", self.export_ns, iri_safe(&e.edge_type))),
            };
            let Some(predicate) = predicate else {
                self.report.skipped.push(format!(
                    "arista `{}` de {}: su `iri` no es un IRI válido, no se exportó",
                    e.edge_type, subject
                ));
                continue;
            };
            // RDF edges carry nothing but their predicate.
            let mut extra: Vec<&String> = e.properties.keys().filter(|k| k.as_str() != PROP_IRI).collect();
            if !extra.is_empty() {
                extra.sort();
                let list = extra.iter().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", ");
                self.report.skipped.push(format!(
                    "arista `{}` de {} → {}: RDF no tiene propiedades de arista, se perdieron {list}",
                    e.edge_type, subject, target
                ));
            }
            object_triples.push((predicate, target));
            self.report.edges += 1;
        }
        object_triples.sort_by(|a, b| (a.0.as_str(), a.1.to_string()).cmp(&(b.0.as_str(), b.1.to_string())));
        object_triples.dedup();
        for (p, o) in object_triples {
            self.emit(subject.clone(), p, o.into());
        }

        // 3. Literals.
        let mut props: Vec<(&String, &PropertyValue)> = node
            .properties
            .iter()
            .filter(|(k, _)| k.as_str() != PROP_IRI && k.as_str() != PROP_PLACEHOLDER)
            .collect();
        props.sort_by_key(|(k, _)| k.as_str());
        for (key, value) in props {
            let Some(predicate) = self.property_predicate(key) else {
                self.report.skipped.push(format!(
                    "{subject} `{key}`: la clave no forma un IRI válido, no se exportó"
                ));
                continue;
            };
            let values: Vec<&PropertyValue> = match value {
                PropertyValue::List(items) => items.iter().collect(),
                v => vec![v],
            };
            if values.is_empty() {
                self.report.skipped.push(format!("{subject} `{key}`: lista vacía, no hay triple que escribir"));
            }
            for v in values {
                match literal_for(v) {
                    Ok(lit) => {
                        self.emit(subject.clone(), predicate.clone(), lit.into());
                        self.report.literals += 1;
                    }
                    Err(reason) => self.report.skipped.push(format!("{subject} `{key}`: {reason}")),
                }
            }
        }

        // A placeholder that nothing describes writes no triple of its own: it
        // exists in the document only as an object, exactly as it came in.
        if !class && self.report.triples_written > written_before {
            self.report.individuals += 1;
        }
    }

    fn property_predicate(&self, key: &str) -> Option<NamedNode> {
        match key {
            k if k == PROP_RDFS_LABEL => Some(rdfs::LABEL.into_owned()),
            k if k == PROP_RDFS_COMMENT => Some(rdfs::COMMENT.into_owned()),
            k => named_node(&format!("{}{}", self.export_ns, iri_safe(k))),
        }
    }

    fn emit(&mut self, s: NamedOrBlankNode, p: NamedNode, o: Term) {
        self.triples.push(Triple::new(s, p, o));
        self.report.triples_written += 1;
    }

    fn serialize(&self) -> Result<String> {
        let mut serializer = TurtleSerializer::new();
        for (p, ns) in &self.prefixes {
            serializer = match serializer.clone().with_prefix(p, ns) {
                Ok(s) => s,
                // A catalog entry that is not an IRI cannot be a prefix; the
                // terms under it are still written, in full.
                Err(_) => serializer,
            };
        }
        let mut writer = serializer.for_writer(Vec::new());
        for t in &self.triples {
            writer
                .serialize_triple(t)
                .map_err(|e| NopalError::custom(format!("Turtle export: {e}")))?;
        }
        let bytes = writer.finish().map_err(|e| NopalError::custom(format!("Turtle export: {e}")))?;
        String::from_utf8(bytes).map_err(|e| NopalError::custom(format!("Turtle export: {e}")))
    }
}

// ---------------------------------------------------------------------------
// Term helpers
// ---------------------------------------------------------------------------

/// RDF term for a stored `iri`: an absolute IRI, a synthetic blank node
/// (`_:<hash>-<label>`, as the importer writes them), or a raw prefixed token
/// (`:Alice`) left by an import older than 0.5.10, resolved under the export
/// namespace, which is where that importer would have put it.
fn stored_term(stored: &str, export_ns: &str) -> Option<NamedOrBlankNode> {
    if let Some(id) = stored.strip_prefix("_:") {
        return BlankNode::new(id).ok().map(NamedOrBlankNode::BlankNode);
    }
    if let Some(local) = stored.strip_prefix(':') {
        return named(&format!("{export_ns}{}", iri_safe(local)));
    }
    named(stored)
}

fn named(iri: &str) -> Option<NamedOrBlankNode> {
    named_node(iri).map(NamedOrBlankNode::NamedNode)
}

fn named_node(iri: &str) -> Option<NamedNode> {
    NamedNode::new(iri).ok()
}

/// Make a label or property key usable as the local part of an IRI.
///
/// Non-ASCII stays as it is: IRIs allow it (`café_de_olla` is a valid local
/// name, and it is what a Turtle author wrote). Only the ASCII characters that
/// an IRI reference cannot contain are percent-encoded (the earlier exporter
/// replaced them, and every non-ASCII letter, with `_`, which is lossy).
fn iri_safe(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let keep = !ch.is_ascii() || ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~');
        if keep {
            out.push(ch);
        } else {
            let mut buf = [0u8; 4];
            for b in ch.encode_utf8(&mut buf).bytes() {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

/// Typed literal for a property value, by the same table the importer reads;
/// `Err` carries the reason a value has no Turtle form.
fn literal_for(value: &PropertyValue) -> std::result::Result<Literal, &'static str> {
    match value {
        PropertyValue::Int(i) => Ok(Literal::new_typed_literal(i.to_string(), xsd::INTEGER)),
        PropertyValue::Float(f) if f.is_finite() => Ok(Literal::new_typed_literal(f.to_string(), xsd::DOUBLE)),
        PropertyValue::Float(_) => Err("NaN/∞ no tienen forma en xsd:double"),
        PropertyValue::Bool(b) => Ok(Literal::new_typed_literal(b.to_string(), xsd::BOOLEAN)),
        PropertyValue::String(s) => Ok(Literal::new_simple_literal(s.as_str())),
        PropertyValue::Null => Err("Null no tiene forma en RDF (se omite el triple)"),
        PropertyValue::Bytes(_) => Err("Bytes no tiene representación en este puente"),
        PropertyValue::Object(_) => Err("Object no tiene representación en este puente"),
        PropertyValue::List(_) => Err("lista anidada: solo listas planas se escriben (un triple por elemento)"),
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_literal_for_scalars() {
        assert_eq!(literal_for(&PropertyValue::Int(42)).unwrap(), Literal::new_typed_literal("42", xsd::INTEGER));
        assert_eq!(literal_for(&PropertyValue::Float(3.5)).unwrap(), Literal::new_typed_literal("3.5", xsd::DOUBLE));
        assert_eq!(literal_for(&PropertyValue::Bool(true)).unwrap(), Literal::new_typed_literal("true", xsd::BOOLEAN));
        assert_eq!(literal_for(&PropertyValue::String("a \"b\"".into())).unwrap(), Literal::new_simple_literal("a \"b\""));
    }

    #[test]
    fn test_literal_for_unrepresentable_values_say_why() {
        assert!(literal_for(&PropertyValue::Float(f64::NAN)).is_err());
        assert!(literal_for(&PropertyValue::Float(f64::INFINITY)).is_err());
        assert!(literal_for(&PropertyValue::Null).is_err());
        assert!(literal_for(&PropertyValue::Bytes(vec![1])).is_err());
        assert!(literal_for(&PropertyValue::Object(vec![])).is_err());
        assert!(literal_for(&PropertyValue::List(vec![])).is_err());
    }

    #[test]
    fn test_iri_safe_keeps_unicode_and_encodes_ascii_specials() {
        assert_eq!(iri_safe("Person"), "Person");
        assert_eq!(iri_safe("café_de_olla"), "café_de_olla");
        assert_eq!(iri_safe("My Class"), "My%20Class");
        assert_eq!(iri_safe("a/b#c"), "a%2Fb%23c");
    }

    #[test]
    fn test_stored_term_forms() {
        let ns = "http://example.org/ontology#";
        assert_eq!(
            stored_term("http://plantas.example/rosa", ns).unwrap().to_string(),
            "<http://plantas.example/rosa>"
        );
        assert_eq!(stored_term(":Alice", ns).unwrap().to_string(), "<http://example.org/ontology#Alice>");
        assert_eq!(stored_term("_:0123abcd-b0", ns).unwrap().to_string(), "_:0123abcd-b0");
        assert!(stored_term("not an iri", ns).is_none());
    }

    #[test]
    fn test_export_ns_comes_from_catalog_or_default() {
        let e = Export::new(vec![], vec![], BTreeMap::new());
        assert_eq!(e.export_ns, DEFAULT_NAMESPACE);
        let e = Export::new(
            vec![],
            vec![],
            BTreeMap::from([(String::new(), "http://plantas.example/".to_string())]),
        );
        assert_eq!(e.export_ns, "http://plantas.example/");
        // Standard prefixes are always present and never overridden by the catalog.
        let e = Export::new(vec![], vec![], BTreeMap::from([("xsd".to_string(), "http://bad/".to_string())]));
        assert_eq!(e.prefixes["xsd"], NS_XSD);
    }
}
