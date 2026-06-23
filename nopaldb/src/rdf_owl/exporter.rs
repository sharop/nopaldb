// src/rdf_owl/exporter.rs
//
// OWL/Turtle Exporter — F1 del roadmap semántico de NopalDB.
//
// Exporta el contenido ontológico de un Graph a formato Turtle (.ttl):
//   - Clases owl:Class (NodeKind::Class)
//   - Relaciones rdfs:subClassOf (edge_type = "subClassOf")
//   - Individuos con data properties (NodeKind::Individual con propiedad "iri")
//
// El output es round-trip compatible con `import_turtle()`: importar el Turtle
// generado produce el mismo número de clases, aristas e individuos.
//
// Feature gate: compilado solo cuando `owl-import` está habilitado.

use std::collections::{HashMap, HashSet};

use crate::error::Result;
use crate::graph::Graph;
use crate::types::{Node, NodeId, NodeKind, PropertyValue};

// ---------------------------------------------------------------------------
// API pública
// ---------------------------------------------------------------------------

/// Exporta el contenido ontológico del grafo a una cadena Turtle (.ttl).
///
/// Solo se incluyen:
/// - Nodos con `NodeKind::Class` → `rdf:type owl:Class`
/// - Aristas de tipo `"subClassOf"` → `rdfs:subClassOf`
/// - Nodos con `NodeKind::Individual` y propiedad `"iri"` → instancias OWL
///   con data properties (excluyendo la propiedad `"iri"` misma)
///
/// Nodos ordinarios de NopalDB (sin propiedad `"iri"`) no se exportan,
/// lo que permite que grafos mixtos (OWL + datos) exporten solo la ontología.
///
/// El output es determinístico: clases e individuos se ordenan alfabéticamente.
pub async fn export_turtle(graph: &Graph) -> Result<String> {
    // 1. Obtener todos los nodos y aristas.
    let all_nodes = graph.get_all_nodes().await?;
    let all_edges = graph.get_all_edges().await?;

    // 2. Separar en categorías.
    let mut classes: Vec<&Node> = all_nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Class)
        .collect();
    classes.sort_by(|a, b| a.label.cmp(&b.label));

    let mut individuals: Vec<&Node> = all_nodes
        .iter()
        .filter(|n| {
            n.kind == NodeKind::Individual
                && matches!(n.properties.get("iri"), Some(PropertyValue::String(_)))
        })
        .collect();
    individuals.sort_by_key(|n| {
        n.properties
            .get("iri")
            .and_then(|v| {
                if let PropertyValue::String(s) = v {
                    Some(s.as_str())
                } else {
                    None
                }
            })
            .unwrap_or("")
    });

    // 3. Construir mapa NodeId → IRI para resolver aristas subClassOf.
    let mut id_to_iri: HashMap<NodeId, String> = HashMap::new();
    for node in &classes {
        id_to_iri.insert(node.id, format!(":{}", escape_iri_local(&node.label)));
    }
    for node in &individuals {
        if let Some(PropertyValue::String(iri)) = node.properties.get("iri") {
            id_to_iri.insert(node.id, iri.clone());
        }
    }

    // 4. Filtrar aristas subClassOf, deduplicar, y resolver a IRIs.
    let mut seen: HashSet<(NodeId, NodeId)> = HashSet::new();
    let mut subclass_triples: Vec<(String, String)> = Vec::new();
    for edge in &all_edges {
        if edge.edge_type == "subClassOf"
            && let (Some(sub_iri), Some(sup_iri)) =
                (id_to_iri.get(&edge.source), id_to_iri.get(&edge.target))
            && seen.insert((edge.source, edge.target))
        {
            subclass_triples.push((sub_iri.clone(), sup_iri.clone()));
        }
    }
    // Output determinístico.
    subclass_triples.sort();

    // 5. Construir el string Turtle.
    let cap = 300 + classes.len() * 50 + subclass_triples.len() * 60 + individuals.len() * 120;
    let mut out = String::with_capacity(cap);

    // Prefijos estándar.
    out.push_str("@prefix owl:  <http://www.w3.org/2002/07/owl#> .\n");
    out.push_str("@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .\n");
    out.push_str("@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .\n");
    out.push_str("@prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .\n");
    out.push_str("@prefix :     <http://example.org/ontology#> .\n");

    // Declaraciones de clase.
    if !classes.is_empty() {
        out.push('\n');
        for node in &classes {
            out.push_str(&format!(
                ":{} rdf:type owl:Class .\n",
                escape_iri_local(&node.label)
            ));
        }
    }

    // Relaciones subClassOf.
    if !subclass_triples.is_empty() {
        out.push('\n');
        for (sub, sup) in &subclass_triples {
            out.push_str(&format!("{} rdfs:subClassOf {} .\n", sub, sup));
        }
    }

    // Individuos OWL.
    if !individuals.is_empty() {
        out.push('\n');
        for node in &individuals {
            if let Some(PropertyValue::String(iri)) = node.properties.get("iri") {
                let class_iri = format!(":{}", escape_iri_local(&node.label));
                out.push_str(&format!("{} rdf:type {} .\n", iri, class_iri));

                // Data properties (excluir "iri").
                let mut props: Vec<(&String, &PropertyValue)> = node
                    .properties
                    .iter()
                    .filter(|(k, _)| k.as_str() != "iri")
                    .collect();
                props.sort_by_key(|(k, _)| k.as_str());
                for (key, value) in props {
                    if let Some(literal) = property_value_to_literal(value) {
                        out.push_str(&format!(
                            "{} :{} {} .\n",
                            iri,
                            escape_iri_local(key),
                            literal
                        ));
                    }
                }
            }
        }
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// Helpers internos
// ---------------------------------------------------------------------------

/// Convierte un `PropertyValue` a un literal Turtle con tipo xsd.
///
/// Retorna `None` para `Null`, `Bytes`, o floats no representables (NaN, Inf).
fn property_value_to_literal(value: &PropertyValue) -> Option<String> {
    match value {
        PropertyValue::Int(i) => Some(format!("\"{}\"^^xsd:integer", i)),
        PropertyValue::Float(f) => {
            if f.is_finite() {
                Some(format!("\"{}\"^^xsd:double", f))
            } else {
                None // NaN / Inf no representables en xsd:double
            }
        }
        PropertyValue::Bool(b) => Some(format!("\"{}\"^^xsd:boolean", b)),
        PropertyValue::String(s) => Some(format!("\"{}\"", escape_string(s))),
        PropertyValue::Null | PropertyValue::Bytes(_) => None,
        PropertyValue::Object(_) | PropertyValue::List(_) => None,
    }
}

/// Reemplaza caracteres no seguros en un local name de IRI.
/// Solo permite ASCII alfanumérico + `_` + `-`.
fn escape_iri_local(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Escapa caracteres especiales dentro de un string literal Turtle.
fn escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests unitarios
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_property_value_to_literal_int() {
        assert_eq!(
            property_value_to_literal(&PropertyValue::Int(42)),
            Some("\"42\"^^xsd:integer".to_string())
        );
    }

    #[test]
    fn test_property_value_to_literal_float() {
        assert_eq!(
            property_value_to_literal(&PropertyValue::Float(3.14)),
            Some("\"3.14\"^^xsd:double".to_string())
        );
        assert_eq!(
            property_value_to_literal(&PropertyValue::Float(f64::NAN)),
            None
        );
        assert_eq!(
            property_value_to_literal(&PropertyValue::Float(f64::INFINITY)),
            None
        );
    }

    #[test]
    fn test_property_value_to_literal_bool() {
        assert_eq!(
            property_value_to_literal(&PropertyValue::Bool(true)),
            Some("\"true\"^^xsd:boolean".to_string())
        );
    }

    #[test]
    fn test_property_value_to_literal_string() {
        assert_eq!(
            property_value_to_literal(&PropertyValue::String("Alice".to_string())),
            Some("\"Alice\"".to_string())
        );
        assert_eq!(
            property_value_to_literal(&PropertyValue::String("Say \"hi\"".to_string())),
            Some("\"Say \\\"hi\\\"\"".to_string())
        );
    }

    #[test]
    fn test_property_value_to_literal_null_bytes() {
        assert_eq!(property_value_to_literal(&PropertyValue::Null), None);
        assert_eq!(
            property_value_to_literal(&PropertyValue::Bytes(vec![1, 2])),
            None
        );
    }

    #[test]
    fn test_escape_iri_local() {
        assert_eq!(escape_iri_local("Person"), "Person");
        assert_eq!(escape_iri_local("My Class"), "My_Class");
        assert_eq!(escape_iri_local("café"), "caf_");
    }

    #[test]
    fn test_escape_string() {
        assert_eq!(escape_string("hello"), "hello");
        assert_eq!(escape_string("say \"hi\""), "say \\\"hi\\\"");
        assert_eq!(escape_string("line\nnew"), "line\\nnew");
    }
}
