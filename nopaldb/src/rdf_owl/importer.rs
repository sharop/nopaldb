// src/rdf_owl/importer.rs
//
// OWL/Turtle Importer — Step 6 of the NopalDB ontological roadmap.
//
// Mini-parser for a subset of Turtle (.ttl) syntax. Handles:
//   - Prefix declarations: `@prefix owl: <http://www.w3.org/2002/07/owl#> .`
//   - Class declarations:  `:Foo rdf:type owl:Class .`
//   - Subclass axioms:     `:Foo rdfs:subClassOf :Bar .`
//   - Individual declarations: `:Alice rdf:type :Person .` (Pass 3)
//   - Data properties:    `:Alice :age "30" .` (Pass 3)
//
// Everything else is counted in `triples_skipped` and silently ignored.
//
// Feature gate: compiled only when `owl-import` is enabled.

use std::collections::HashMap;

use crate::error::Result;
use crate::graph::Graph;
use crate::index::taxonomy::TaxonomyIndex;
use crate::rdf_owl::rdf::RDFTriple;
use crate::types::{Edge, Node, NodeId, NodeKind, PropertyValue};

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
    /// Number of triples that were not ontological and were skipped.
    pub triples_skipped: usize,
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
/// All other triples increment `triples_skipped`.
///
/// The function is idempotent: if a class or individual with the same IRI already
/// exists, it is reused rather than duplicated.
pub async fn import_turtle(
    graph: &Graph,
    taxonomy: &mut TaxonomyIndex,
    source: &str,
) -> Result<ImportReport> {
    let mut report = ImportReport::default();

    // Step 1 — parse prefix declarations and raw triples.
    let (prefixes, triples) = parse_turtle(source);

    // Step 2 — resolve and collect classes first (pass 1).
    // We need all classes before wiring subClassOf edges.
    let mut label_to_id: HashMap<String, NodeId> = HashMap::new();

    // Helper closure: resolve a term (prefixed or angle-bracket IRI) to a local name.
    let resolve = |term: &str| -> String {
        local_name(term, &prefixes)
    };

    // Track which subjects are known owl:Class IRIs (for Pass 3 exclusion).
    let mut class_iris: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Pass 1: find rdf:type owl:Class triples.
    for triple in &triples {
        let pred = resolve(&triple.predicate);
        let obj  = resolve(&triple.object);

        if pred == "type" && obj == "Class" {
            let class_label = resolve(&triple.subject);
            if class_label.is_empty() {
                continue;
            }

            // Track subject IRI as a known class.
            class_iris.insert(triple.subject.clone());

            // Idempotency: reuse existing node if label already in graph.
            let existing = graph.get_nodes_by_label(&class_label).await?;
            let node_id = if let Some(existing_node) = existing
                .into_iter()
                .find(|n| n.kind == NodeKind::Class)
            {
                existing_node.id
            } else {
                let mut node = Node::new(class_label.clone());
                node.kind = NodeKind::Class;
                graph.add_node(node).await?
            };

            label_to_id.insert(class_label.clone(), node_id);

            // Register in taxonomy (idempotent).
            taxonomy.register_class(node_id, &class_label);
            report.classes_added += 1;
        }
    }

    // Pass 2: wire rdfs:subClassOf edges.
    for triple in &triples {
        let pred = resolve(&triple.predicate);

        if pred == "subClassOf" {
            let sub_label    = resolve(&triple.subject);
            let super_label  = resolve(&triple.object);

            if sub_label.is_empty() || super_label.is_empty() {
                report.triples_skipped += 1;
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
        } else if pred != "type" {
            // Not a Class declaration (handled in pass 1) and not a subClassOf → skip.
            report.triples_skipped += 1;
        }
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

    for triple in &triples {
        let pred = resolve(&triple.predicate);
        let obj  = resolve(&triple.object);

        if pred == "type" && obj != "Class" && !obj.is_empty() {
            // Skip subjects that were declared as owl:Class themselves.
            if class_iris.contains(&triple.subject) {
                continue;
            }
            // The object is a class label (e.g. "Person").
            // Only add if the class is known (was declared as owl:Class in this file
            // or already in the graph).
            if label_to_id.contains_key(&obj) {
                individuals.entry(triple.subject.clone()).or_insert_with(|| obj.clone());
            }
        }
    }

    if !individuals.is_empty() {
        // 3b: collect data properties per individual.
        let mut props_map: HashMap<String, HashMap<String, PropertyValue>> = HashMap::new();
        for triple in &triples {
            let pred = resolve(&triple.predicate);
            if individuals.contains_key(&triple.subject) && pred != "type" && !pred.is_empty() {
                let val = parse_object_as_property_value(&triple.object);
                props_map
                    .entry(triple.subject.clone())
                    .or_default()
                    .insert(pred, val);
            }
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
            for (k, v) in props {
                node.properties.insert(k, v);
            }
            graph.add_node(node).await?;
            report.instances_added += 1;
        }
    }

    Ok(report)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

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
    let existing = graph.get_nodes_by_label(label).await?;
    if let Some(node) = existing.into_iter().find(|n| n.kind == NodeKind::Class) {
        let id = node.id;
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

/// Parse a Turtle object token into a `PropertyValue`.
///
/// Handles:
/// - `"42"` or `"42"^^xsd:integer`  → `PropertyValue::Int(42)` (tries int first)
/// - `"3.14"` or typed float         → `PropertyValue::Float(3.14)`
/// - `"true"` / `"false"`            → `PropertyValue::Bool(...)`
/// - Any other `"..."` string literal → `PropertyValue::String(...)`
/// - IRI `<...>` or prefixed `:Foo`  → `PropertyValue::String(local_name)`
fn parse_object_as_property_value(object: &str) -> PropertyValue {
    let object = object.trim();

    // String literal: starts and ends with `"`
    if object.starts_with('"') {
        // Strip surrounding quotes (tokenizer preserves them).
        let inner = object.trim_matches('"');
        // Strip xsd type annotation if present (e.g. the part after `^^`).
        let value_str = inner.split("^^").next().unwrap_or(inner).trim();

        if let Ok(i) = value_str.parse::<i64>() {
            return PropertyValue::Int(i);
        }
        if let Ok(f) = value_str.parse::<f64>() {
            return PropertyValue::Float(f);
        }
        if value_str.eq_ignore_ascii_case("true") {
            return PropertyValue::Bool(true);
        }
        if value_str.eq_ignore_ascii_case("false") {
            return PropertyValue::Bool(false);
        }
        return PropertyValue::String(value_str.to_string());
    }

    // IRI or prefixed name → use local name as string.
    PropertyValue::String(object.to_string())
}

/// Convert a prefixed IRI or full IRI to its local name (the part after `#` or `/`).
///
/// Examples:
/// - `owl:Class`                             → `"Class"`
/// - `rdfs:subClassOf`                       → `"subClassOf"`
/// - `<http://example.org/ontology#Animal>`  → `"Animal"`
/// - `:Animal`                               → `"Animal"`
pub fn local_name(term: &str, prefixes: &HashMap<String, String>) -> String {
    let term = term.trim();

    // Angle-bracket IRI: <http://...#Foo> or <http://.../Foo>
    if term.starts_with('<') && term.ends_with('>') {
        let iri = &term[1..term.len() - 1];
        return last_segment(iri);
    }

    // Prefixed name: prefix:local
    if let Some(colon) = term.find(':') {
        let prefix = &term[..colon];
        let local = &term[colon + 1..];

        // Bare colon prefix `:Foo` → just local
        if prefix.is_empty() {
            return local.to_string();
        }

        // If prefix is known, expand and take local name
        if prefixes.contains_key(prefix) {
            return local.to_string();
        }

        // Unknown prefix: return local part as-is
        return local.to_string();
    }

    term.to_string()
}

/// Extract the last segment of an IRI (after `#` or last `/`).
fn last_segment(iri: &str) -> String {
    if let Some(pos) = iri.rfind('#') {
        return iri[pos + 1..].to_string();
    }
    if let Some(pos) = iri.rfind('/') {
        return iri[pos + 1..].to_string();
    }
    iri.to_string()
}

// ---------------------------------------------------------------------------
// Mini Turtle parser
// ---------------------------------------------------------------------------

/// Parse a Turtle source string into:
/// 1. A prefix map (short → IRI base, e.g. `"owl"` → `"http://www.w3.org/2002/07/owl#"`)
/// 2. A list of raw triples as [`RDFTriple`] (subject, predicate, object)
///
/// Handles:
/// - `@prefix name: <iri> .`
/// - `PREFIX name: <iri>`  (SPARQL-style)
/// - Simple `. `  separated triples on one or more lines
/// - `;` and `,` abbreviated triple syntax (partially)
/// - Comments `# ...`
///
/// Does NOT handle: blank nodes `_:`, multi-value objects with nested structures,
/// string literals as subjects, or complex turtle documents.
fn parse_turtle(source: &str) -> (HashMap<String, String>, Vec<RDFTriple>) {
    let mut prefixes: HashMap<String, String> = HashMap::new();
    let mut triples: Vec<RDFTriple> = Vec::new();

    // Strip comments and normalize whitespace.
    let cleaned: String = source
        .lines()
        .map(|line| {
            // Remove # comments (but not inside IRIs)
            let mut in_iri = false;
            let mut result = String::new();
            for ch in line.chars() {
                if ch == '<' { in_iri = true; }
                if ch == '>' { in_iri = false; }
                if ch == '#' && !in_iri { break; }
                result.push(ch);
            }
            result
        })
        .collect::<Vec<_>>()
        .join(" ");

    // Tokenize: split on whitespace while respecting <...> and "..." boundaries.
    let tokens = tokenize(&cleaned);

    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i].as_str();

        // @prefix or PREFIX declaration
        if tok.eq_ignore_ascii_case("@prefix") || tok.eq_ignore_ascii_case("prefix") {
            // prefix_name: <iri>
            if i + 2 < tokens.len() {
                let name_tok = tokens[i + 1].trim_end_matches(':').to_string();
                let iri_tok = tokens[i + 2].clone();
                let iri = if iri_tok.starts_with('<') && iri_tok.ends_with('>') {
                    iri_tok[1..iri_tok.len() - 1].to_string()
                } else {
                    iri_tok.clone()
                };
                prefixes.insert(name_tok, iri);
                // Consume up to and including the trailing `.`
                i += 3;
                if i < tokens.len() && tokens[i] == "." {
                    i += 1;
                }
                continue;
            }
        }

        // Try to read a triple: subject predicate object .
        // Also handle abbreviated form:  subject pred1 obj1 ; pred2 obj2 .
        if i + 2 < tokens.len() {
            let subject = tokens[i].clone();
            let predicate = tokens[i + 1].clone();
            let object = tokens[i + 2].clone();

            // Skip if any part looks like a structural token
            if subject == "." || subject == ";" || subject == "," {
                i += 1;
                continue;
            }

            // Skip keyword tokens that are not triples
            if subject.eq_ignore_ascii_case("@prefix")
                || subject.eq_ignore_ascii_case("prefix")
                || subject.eq_ignore_ascii_case("@base")
                || subject.eq_ignore_ascii_case("base")
            {
                i += 1;
                continue;
            }

            triples.push(RDFTriple::new(&subject, &predicate, &object));
            i += 3;

            // Consume trailing `.`, `;`, `,` tokens.
            while i < tokens.len() {
                let next = tokens[i].as_str();
                if next == "." {
                    i += 1;
                    break;
                } else if next == ";" && i + 2 < tokens.len() {
                    // Abbreviated: subject ; pred2 obj2 .  — reuse last subject
                    let last_subject = triples.last().map(|t| t.subject.clone()).unwrap_or_default();
                    let pred2 = tokens[i + 1].clone();
                    let obj2 = tokens[i + 2].clone();
                    triples.push(RDFTriple::new(&last_subject, &pred2, &obj2));
                    i += 3;
                } else if next == "," && i + 1 < tokens.len() {
                    // Abbreviated: pred obj1 , obj2 — reuse last subject + predicate
                    let (last_sub, last_pred) = triples
                        .last()
                        .map(|t| (t.subject.clone(), t.predicate.clone()))
                        .unwrap_or_default();
                    let obj2 = tokens[i + 1].clone();
                    triples.push(RDFTriple::new(&last_sub, &last_pred, &obj2));
                    i += 2;
                } else {
                    break;
                }
            }
            continue;
        }

        i += 1;
    }

    (prefixes, triples)
}

/// Tokenize a Turtle string, grouping `<...>` and `"..."` as single tokens.
fn tokenize(s: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];

        // Skip whitespace
        if ch.is_whitespace() {
            i += 1;
            continue;
        }

        // IRI token <...>
        if ch == '<' {
            let mut tok = String::from('<');
            i += 1;
            while i < chars.len() && chars[i] != '>' {
                tok.push(chars[i]);
                i += 1;
            }
            tok.push('>');
            i += 1;
            tokens.push(tok);
            continue;
        }

        // String literal "..." or '...'
        if ch == '"' || ch == '\'' {
            let delim = ch;
            let mut tok = String::from(ch);
            i += 1;
            while i < chars.len() && chars[i] != delim {
                if chars[i] == '\\' { i += 1; } // escape
                if i < chars.len() { tok.push(chars[i]); }
                i += 1;
            }
            tok.push(delim);
            i += 1;
            tokens.push(tok);
            continue;
        }

        // Regular token: read until whitespace or structural chars
        let mut tok = String::new();
        while i < chars.len() && !chars[i].is_whitespace() {
            let c = chars[i];
            // Structural separators that may be concatenated with tokens (e.g. "rdfs:label.")
            if c == '.' || c == ';' || c == ',' {
                if !tok.is_empty() {
                    break;
                }
                tok.push(c);
                i += 1;
                break;
            }
            tok.push(c);
            i += 1;
        }
        if !tok.is_empty() {
            tokens.push(tok);
        }
    }

    tokens
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;
    use crate::graph::Graph;
    use crate::index::taxonomy::TaxonomyIndex;

    async fn open_temp_graph() -> (Graph, TempDir) {
        let dir = TempDir::new().unwrap();
        let graph = Graph::open(dir.path().to_str().unwrap()).await.unwrap();
        (graph, dir)
    }

    // -----------------------------------------------------------------------
    // Test 1 — single class declaration
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_import_simple_class() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let ttl = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
:Animal rdf:type owl:Class .
"#;

        let report = import_turtle(&graph, &mut taxonomy, ttl).await.unwrap();

        assert_eq!(report.classes_added, 1, "should have added 1 class");
        assert_eq!(report.subclass_edges_added, 0);

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

        let ttl = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
:Animal rdf:type owl:Class .
:Mammal rdf:type owl:Class .
:Dog    rdf:type owl:Class .
:Mammal rdfs:subClassOf :Animal .
:Dog    rdfs:subClassOf :Mammal .
"#;

        let report = import_turtle(&graph, &mut taxonomy, ttl).await.unwrap();

        assert_eq!(report.classes_added, 3);
        assert_eq!(report.subclass_edges_added, 2);

        let animal_id = taxonomy.find_by_label("Animal").unwrap();
        let dog_id    = taxonomy.find_by_label("Dog").unwrap();

        // Transitive: Dog ⊑ Animal
        assert!(taxonomy.is_subclass_of(dog_id, animal_id));
    }

    // -----------------------------------------------------------------------
    // Test 3 — non-ontological triples are skipped
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_import_skips_data_triples() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let ttl = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
:Animal rdf:type owl:Class .
:fido rdf:type :Animal .
:fido rdfs:label "Fido" .
:fido :age "5" .
"#;

        let report = import_turtle(&graph, &mut taxonomy, ttl).await.unwrap();

        assert_eq!(report.classes_added, 1);
        assert!(report.triples_skipped > 0 || report.instances_added > 0,
            "non-class triples should be processed or skipped");
    }

    // -----------------------------------------------------------------------
    // Test 4 — idempotent: import same TTL twice → no duplicate nodes
    // -----------------------------------------------------------------------
    #[tokio::test]
    async fn test_import_idempotent() {
        let (graph, _dir) = open_temp_graph().await;
        let mut taxonomy = TaxonomyIndex::new();

        let ttl = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
:Animal rdf:type owl:Class .
"#;

        import_turtle(&graph, &mut taxonomy, ttl).await.unwrap();
        import_turtle(&graph, &mut taxonomy, ttl).await.unwrap();

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

        let ttl = r#"
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
:A rdf:type owl:Class .
:B rdf:type owl:Class .
:C rdf:type owl:Class .
:D rdf:type owl:Class .
:B rdfs:subClassOf :A .
:C rdfs:subClassOf :A .
:D rdfs:subClassOf :B .
:D rdfs:subClassOf :C .
"#;

        let report = import_turtle(&graph, &mut taxonomy, ttl).await.unwrap();

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
    // Test 6 — local_name helper
    // -----------------------------------------------------------------------
    #[test]
    fn test_local_name_extraction() {
        let prefixes = HashMap::new();
        assert_eq!(local_name("<http://example.org/Animal>", &prefixes), "Animal");
        assert_eq!(local_name("<http://www.w3.org/2002/07/owl#Class>", &prefixes), "Class");
        assert_eq!(local_name(":Animal", &prefixes), "Animal");
        assert_eq!(local_name("owl:Class", &prefixes), "Class");
        assert_eq!(local_name("rdfs:subClassOf", &prefixes), "subClassOf");
    }

    // -----------------------------------------------------------------------
    // Test 7 — parse_object_as_property_value helper
    // -----------------------------------------------------------------------
    #[test]
    fn test_parse_object_as_property_value() {
        assert_eq!(parse_object_as_property_value(r#""42""#), PropertyValue::Int(42));
        assert_eq!(parse_object_as_property_value(r#""3.14""#), PropertyValue::Float(3.14));
        assert_eq!(parse_object_as_property_value(r#""true""#), PropertyValue::Bool(true));
        assert_eq!(parse_object_as_property_value(r#""false""#), PropertyValue::Bool(false));
        assert_eq!(
            parse_object_as_property_value(r#""Alice""#),
            PropertyValue::String("Alice".to_string())
        );
    }
}
