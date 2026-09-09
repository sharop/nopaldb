// src/shacl/turtle.rs
//! Carga de shapes SHACL desde un documento Turtle estándar.
//!
//! Reusa el parser del puente RDF (`rdf_owl::importer::parse_turtle`): la
//! misma gramática, los mismos errores con línea y columna, los mismos
//! avisos sobre lo que el documento no declaró. De ahí salen triples; este
//! módulo los lee como shapes.
//!
//! Lo que no entiende NO lo ignora en silencio: cada término `sh:*` que
//! este validador no implementa (aún) es una línea con razón en
//! [`ShapesReport::ignored`], para que quien escribió la shape sepa qué
//! partes no se están comprobando.

use std::collections::{BTreeMap, HashMap, HashSet};

use oxrdf::{NamedOrBlankNode, Term, Triple};
use uuid::Uuid;

use crate::error::{NopalError, Result};
use crate::rdf_owl::importer::{literal_to_property_value, local_name, parse_turtle};
use crate::types::PropertyValue;
use super::report::{Severity, ShapesReport};
use super::shape::{ConstraintType, DatatypeKind, PathSpec, PropertyShape, Shape, ShaclNodeKind, Target};

const SH: &str = "http://www.w3.org/ns/shacl#";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDF_FIRST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#first";
const RDF_REST: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#rest";
const RDF_NIL: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#nil";

/// Términos `sh:*` que este validador todavía no implementa, con el issue
/// que los cubre o la razón. Todo lo demás desconocido se reporta como tal.
fn unsupported_reason(local: &str) -> Option<&'static str> {
    Some(match local {
        "closed" | "ignoredProperties" => "sh:closed no está implementado (toda propiedad extra se acepta)",
        "qualifiedValueShape" | "qualifiedMinCount" | "qualifiedMaxCount" | "qualifiedValueShapesDisjoint" => {
            "sh:qualifiedValueShape no está implementado"
        }
        "equals" | "disjoint" | "lessThan" | "lessThanOrEquals" => {
            "comparaciones entre propiedades (sh:equals, sh:disjoint, sh:lessThan…) no están implementadas"
        }
        "languageIn" | "uniqueLang" => "los lang tags no se conservan en el import, así que no hay qué comprobar",
        "flags" => "sh:flags no se aplica al patrón (la regex se compila tal cual)",
        "targetSubjectsOf" | "targetObjectsOf" => "sh:targetSubjectsOf / sh:targetObjectsOf no están implementados",
        "sparql" | "select" | "ask" => "SHACL-SPARQL está fuera de alcance",
        _ => return None,
    })
}

/// Identidad textual de un término sujeto/objeto dentro del documento.
fn key(term: &NamedOrBlankNode) -> String {
    match term {
        NamedOrBlankNode::NamedNode(n) => n.as_str().to_string(),
        NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
    }
}

fn object_key(term: &Term) -> Option<String> {
    match term {
        Term::NamedNode(n) => Some(n.as_str().to_string()),
        Term::BlankNode(b) => Some(format!("_:{}", b.as_str())),
        _ => None,
    }
}

/// Nombre legible de un término para los mensajes: `sh:minCount`, `:Receta`.
fn show(iri: &str, prefixes: &BTreeMap<String, String>) -> String {
    if let Some(local) = iri.strip_prefix(SH) {
        return format!("sh:{local}");
    }
    let best = prefixes
        .iter()
        .filter(|(_, ns)| !ns.is_empty() && iri.starts_with(ns.as_str()))
        .max_by_key(|(_, ns)| ns.len());
    match best {
        Some((p, ns)) => format!("{p}:{}", &iri[ns.len()..]),
        None => {
            if iri.starts_with("_:") {
                iri.to_string()
            } else {
                format!("<{iri}>")
            }
        }
    }
}

/// Todo lo dicho sobre cada sujeto del documento.
struct Doc {
    by_subject: HashMap<String, Vec<(String, Term)>>,
    prefixes: BTreeMap<String, String>,
}

impl Doc {
    fn new(triples: &[Triple], prefixes: BTreeMap<String, String>) -> Self {
        let mut by_subject: HashMap<String, Vec<(String, Term)>> = HashMap::new();
        for t in triples {
            by_subject
                .entry(key(&t.subject))
                .or_default()
                .push((t.predicate.as_str().to_string(), t.object.clone()));
        }
        Self { by_subject, prefixes }
    }

    fn statements(&self, subject: &str) -> &[(String, Term)] {
        self.by_subject.get(subject).map(Vec::as_slice).unwrap_or(&[])
    }

    fn objects<'a>(&'a self, subject: &str, predicate: &str) -> impl Iterator<Item = &'a Term> + 'a {
        let predicate = predicate.to_string();
        self.statements(subject).iter().filter(move |(p, _)| *p == predicate).map(|(_, o)| o)
    }

    fn has_type(&self, subject: &str, class: &str) -> bool {
        self.objects(subject, RDF_TYPE).any(|o| matches!(o, Term::NamedNode(n) if n.as_str() == class))
    }

    /// Los miembros de una lista RDF (`( a b c )`), en orden.
    fn list(&self, head: &str) -> Vec<Term> {
        let mut out = Vec::new();
        let mut cur = head.to_string();
        let mut seen = HashSet::new();
        while cur != RDF_NIL && seen.insert(cur.clone()) {
            let Some(first) = self.objects(&cur, RDF_FIRST).next() else { break };
            out.push(first.clone());
            match self.objects(&cur, RDF_REST).next().and_then(object_key) {
                Some(next) => cur = next,
                None => break,
            }
        }
        out
    }

    fn show(&self, iri: &str) -> String {
        show(iri, &self.prefixes)
    }
}

/// Lee las shapes de `source`. Devuelve las shapes y el reporte de carga.
///
/// Un documento malformado es `Err(RdfParseError)` con línea y columna, como
/// en `import_turtle`. Una shape es todo sujeto tipado `sh:NodeShape` o con
/// `sh:targetClass` / `sh:targetNode`; sus `sh:property` (blank nodes o
/// IRIs) son property shapes.
pub fn parse_shapes(source: &str) -> Result<(Vec<Shape>, ShapesReport)> {
    let parsed = parse_turtle(source)?;
    let mut report = ShapesReport { warnings: parsed.warnings.clone(), ..Default::default() };
    let doc = Doc::new(&parsed.triples, parsed.prefixes.clone());

    // Shape subjects, in document order, once each.
    let mut subjects: Vec<String> = Vec::new();
    let mut seen = HashSet::new();
    for t in &parsed.triples {
        let s = key(&t.subject);
        if !seen.contains(&s)
            && (doc.has_type(&s, &format!("{SH}NodeShape"))
                || doc.objects(&s, &format!("{SH}targetClass")).next().is_some()
                || doc.objects(&s, &format!("{SH}targetNode")).next().is_some())
        {
            seen.insert(s.clone());
            subjects.push(s);
        }
    }
    // A property shape referenced from a node shape is never a node shape itself.
    let referenced: HashSet<String> = subjects
        .iter()
        .flat_map(|s| doc.objects(s, &format!("{SH}property")).filter_map(object_key).collect::<Vec<_>>())
        .collect();
    subjects.retain(|s| !referenced.contains(s));

    let mut shapes = Vec::new();
    for subject in subjects {
        let shape = shape_body(&doc, &subject, &mut report)?;
        report.shapes += 1;
        shapes.push(shape);
    }

    Ok((shapes, report))
}

/// Una shape (con nombre o anónima, de nivel superior o miembro de un
/// combinador) a partir de todo lo dicho sobre `subject`. Un `sh:pattern`
/// inválido es `Err` con la shape y el patrón: no se carga nada.
fn shape_body(doc: &Doc, subject: &str, report: &mut ShapesReport) -> Result<Shape> {
    let mut shape = Shape::new(local_name(subject));
    shape.iri = Some(subject.to_string());
    shape.id = Uuid::new_v4();
    let shown = doc.show(subject);

    for (pred, obj) in doc.statements(subject) {
        let Some(local) = pred.strip_prefix(SH) else {
            if pred != RDF_TYPE {
                report.ignored.push(format!("{shown}: predicado {} fuera del vocabulario SHACL, ignorado", doc.show(pred)));
            }
            continue;
        };
        match local {
            "name" => {
                if let Term::Literal(l) = obj {
                    shape.name = l.value().to_string();
                }
            }
            "description" => {}
            "targetClass" => match obj {
                // The full IRI: the validator resolves it through the
                // taxonomy (subclasses included) and falls back to the
                // local name as a label when the graph has none.
                Term::NamedNode(n) => shape.targets.push(Target::Class(n.as_str().to_string())),
                other => report.ignored.push(format!("{shown}: sh:targetClass espera un IRI, no `{other}`")),
            },
            "targetNode" => match obj {
                Term::NamedNode(n) => shape.targets.push(Target::NodeIri(n.as_str().to_string())),
                other => report.ignored.push(format!("{shown}: sh:targetNode espera un IRI, no `{other}`")),
            },
            "severity" => match severity_of(obj) {
                Some(sev) => shape.severity = sev,
                None => report.ignored.push(format!("{shown}: sh:severity espera sh:Violation, sh:Warning o sh:Info, no `{obj}`")),
            },
            "message" => match obj {
                Term::Literal(l) => shape.message = Some(l.value().to_string()),
                other => report.ignored.push(format!("{shown}: sh:message espera una cadena, no `{other}`")),
            },
            "deactivated" => {
                shape.deactivated = matches!(obj, Term::Literal(l) if l.value() == "true" || l.value() == "1");
            }
            "property" => {
                let Some(ps_key) = object_key(obj) else {
                    report.ignored.push(format!("{shown}: sh:property espera un nodo, no un literal"));
                    continue;
                };
                if let Some(ps) = property_shape(doc, &ps_key, &shown, report)? {
                    report.constraints += ps.constraints.len();
                    report.property_shapes += 1;
                    shape.property_shapes.push(ps);
                }
            }
            _ => {
                if let Some(c) = constraint(doc, local, obj, &shown, report)? {
                    report.constraints += 1;
                    shape.constraints.push(c);
                }
            }
        }
    }
    Ok(shape)
}

fn severity_of(obj: &Term) -> Option<Severity> {
    match obj {
        Term::NamedNode(n) => match n.as_str().strip_prefix(SH)? {
            "Violation" => Some(Severity::Violation),
            "Warning" => Some(Severity::Warning),
            "Info" => Some(Severity::Info),
            _ => None,
        },
        _ => None,
    }
}

/// Las shapes miembro de `sh:and/or/xone ( … )`: cada elemento de la lista
/// RDF es una shape (anónima o nombrada) del mismo documento.
fn member_shapes(doc: &Doc, obj: &Term, where_: &str, local: &str, report: &mut ShapesReport) -> Result<Option<Vec<Shape>>> {
    let Some(head) = object_key(obj) else {
        report.ignored.push(format!("{where_}: sh:{local} espera una lista ( shape shape … ), no `{obj}`; se ignora"));
        return Ok(None);
    };
    let members = doc.list(&head);
    if members.is_empty() {
        report.ignored.push(format!("{where_}: sh:{local} con lista vacía; se ignora"));
        return Ok(None);
    }
    let mut shapes = Vec::new();
    for m in members {
        match object_key(&m) {
            Some(key) => shapes.push(shape_body(doc, &key, report)?),
            None => report.ignored.push(format!("{where_}: sh:{local} espera shapes, no el literal `{m}`; ese miembro se ignora")),
        }
    }
    Ok(Some(shapes))
}

/// Un `sh:property [ … ]`: su path y sus constraints.
fn property_shape(doc: &Doc, ps_key: &str, owner: &str, report: &mut ShapesReport) -> Result<Option<PropertyShape>> {
    let where_ = format!("{owner} sh:property {}", doc.show(ps_key));
    let mut path: Option<PathSpec> = None;
    let mut constraints = Vec::new();
    let mut severity = None;
    let mut message = None;
    for (pred, obj) in doc.statements(ps_key) {
        let Some(local) = pred.strip_prefix(SH) else {
            if pred != RDF_TYPE {
                report.ignored.push(format!("{where_}: predicado {} fuera del vocabulario SHACL, ignorado", doc.show(pred)));
            }
            continue;
        };
        match local {
            "path" => match obj {
                Term::NamedNode(n) => path = Some(PathSpec::Predicate(local_name(n.as_str()))),
                Term::BlankNode(_) => {
                    report.ignored.push(format!(
                        "{where_}: sh:path compuesto (secuencia, inverso, alternativa o cierre) no soportado: issue #101; la property shape se descarta"
                    ));
                    return Ok(None);
                }
                other => {
                    report.ignored.push(format!("{where_}: sh:path espera un IRI, no `{other}`; la property shape se descarta"));
                    return Ok(None);
                }
            },
            "name" | "description" => {}
            "severity" => match severity_of(obj) {
                Some(sev) => severity = Some(sev),
                None => report.ignored.push(format!("{where_}: sh:severity espera sh:Violation, sh:Warning o sh:Info, no `{obj}`")),
            },
            "message" => match obj {
                Term::Literal(l) => message = Some(l.value().to_string()),
                other => report.ignored.push(format!("{where_}: sh:message espera una cadena, no `{other}`")),
            },
            _ => {
                if let Some(c) = constraint(doc, local, obj, &where_, report)? {
                    constraints.push(c);
                }
            }
        }
    }
    match path {
        Some(path) => {
            let mut ps = PropertyShape::new(path, constraints);
            ps.severity = severity;
            ps.message = message;
            Ok(Some(ps))
        }
        None => {
            report.ignored.push(format!("{where_}: sin sh:path; la property shape se descarta"));
            Ok(None)
        }
    }
}

fn usize_of(obj: &Term) -> Option<usize> {
    match obj {
        Term::Literal(l) => l.value().parse::<usize>().ok(),
        _ => None,
    }
}

fn f64_of(obj: &Term) -> Option<f64> {
    match obj {
        Term::Literal(l) => l.value().parse::<f64>().ok(),
        _ => None,
    }
}

fn literal_value(obj: &Term, warnings: &mut Vec<String>) -> Option<PropertyValue> {
    match obj {
        Term::Literal(l) => Some(literal_to_property_value(l, warnings)),
        _ => None,
    }
}

/// Una constraint `sh:<local> <obj>`; `None` si no se soporta o está mal
/// formada (con la razón en el reporte).
fn constraint(doc: &Doc, local: &str, obj: &Term, where_: &str, report: &mut ShapesReport) -> Result<Option<ConstraintType>> {
    let bad = |what: &str, report: &mut ShapesReport| {
        report.ignored.push(format!("{where_}: sh:{local} espera {what}, no `{obj}`; se ignora"));
        None
    };
    Ok(match local {
        "minCount" => usize_of(obj).map(ConstraintType::MinCount).or_else(|| bad("un entero", report)),
        "maxCount" => usize_of(obj).map(ConstraintType::MaxCount).or_else(|| bad("un entero", report)),
        "minLength" => usize_of(obj).map(ConstraintType::MinLength).or_else(|| bad("un entero", report)),
        "maxLength" => usize_of(obj).map(ConstraintType::MaxLength).or_else(|| bad("un entero", report)),
        "minInclusive" => f64_of(obj).map(ConstraintType::MinInclusive).or_else(|| bad("un número", report)),
        "maxInclusive" => f64_of(obj).map(ConstraintType::MaxInclusive).or_else(|| bad("un número", report)),
        "minExclusive" => f64_of(obj).map(ConstraintType::MinExclusive).or_else(|| bad("un número", report)),
        "maxExclusive" => f64_of(obj).map(ConstraintType::MaxExclusive).or_else(|| bad("un número", report)),
        "datatype" => match obj {
            Term::NamedNode(n) => {
                let kind = DatatypeKind::from_xsd(n.as_str());
                if kind == DatatypeKind::Str && !n.as_str().ends_with("#string") {
                    report.warnings.push(format!(
                        "{where_}: sh:datatype {} se comprueba como texto (el importer guarda ese datatype como texto)",
                        doc.show(n.as_str())
                    ));
                }
                Some(ConstraintType::Datatype(kind))
            }
            _ => bad("un IRI xsd:*", report),
        },
        "pattern" => match obj {
            Term::Literal(l) => Some(ConstraintType::pattern(l.value()).map_err(|e| {
                NopalError::custom(format!("{where_}: {e}"))
            })?),
            _ => bad("una cadena", report),
        },
        "and" => member_shapes(doc, obj, where_, local, report)?.map(ConstraintType::And),
        "or" => member_shapes(doc, obj, where_, local, report)?.map(ConstraintType::Or),
        "xone" => member_shapes(doc, obj, where_, local, report)?.map(ConstraintType::Xone),
        "not" => match object_key(obj) {
            Some(key) => Some(ConstraintType::Not(Box::new(shape_body(doc, &key, report)?))),
            None => bad("una shape", report),
        },
        "node" => match obj {
            Term::NamedNode(n) => Some(ConstraintType::Node(n.as_str().to_string())),
            _ => bad("el IRI de una shape del documento", report),
        },
        "in" => {
            let Some(head) = object_key(obj) else { return Ok(bad("una lista ( … )", report)) };
            let mut values = Vec::new();
            for member in doc.list(&head) {
                match literal_value(&member, &mut report.warnings) {
                    Some(v) => values.push(v),
                    None => report.ignored.push(format!(
                        "{where_}: sh:in con el IRI `{member}`: solo se comparan literales, ese miembro se ignora"
                    )),
                }
            }
            Some(ConstraintType::In(values))
        }
        "hasValue" => match obj {
            Term::Literal(l) => Some(ConstraintType::HasValue(literal_to_property_value(l, &mut report.warnings))),
            Term::NamedNode(n) => Some(ConstraintType::HasValue(PropertyValue::String(n.as_str().to_string()))),
            _ => bad("un literal o un IRI", report),
        },
        "class" => match obj {
            Term::NamedNode(n) => Some(ConstraintType::Class(n.as_str().to_string())),
            _ => bad("un IRI de clase", report),
        },
        "nodeKind" => match obj {
            Term::NamedNode(n) => match n.as_str().strip_prefix(SH).and_then(ShaclNodeKind::from_local_name) {
                Some(kind) => Some(ConstraintType::NodeKindShacl(kind)),
                None => bad("sh:IRI, sh:BlankNode, sh:Literal o sus combinaciones", report),
            },
            _ => bad("sh:IRI, sh:BlankNode, sh:Literal o sus combinaciones", report),
        },
        other => {
            let reason = unsupported_reason(other).unwrap_or("término desconocido para este validador");
            report.ignored.push(format!("{where_}: sh:{other} no se comprueba — {reason}"));
            None
        }
    })
}

impl std::fmt::Display for ShapesReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} shape(s), {} property shape(s), {} constraint(s), {} ignorado(s), {} aviso(s)",
            self.shapes,
            self.property_shapes,
            self.constraints,
            self.ignored.len(),
            self.warnings.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECETARIO: &str = r#"
@prefix sh:  <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix :    <http://cocina.example/> .

:RecetaShape a sh:NodeShape ;
  sh:name "Receta bien descrita" ;
  sh:targetClass :Receta ;
  sh:property [ sh:path :nombre ;     sh:minCount 1 ; sh:datatype xsd:string ] ,
              [ sh:path :tiempoMin ;  sh:datatype xsd:integer ; sh:minInclusive 1 ] ,
              [ sh:path :dificultad ; sh:in ( "fácil" "media" "difícil" ) ] ,
              [ sh:path :usa ;        sh:minCount 2 ; sh:class :Ingrediente ] ;
  sh:closed true .

:IngredienteShape sh:targetClass :Ingrediente ;
  sh:nodeKind sh:IRI ;
  sh:property [ sh:path ( :origen :region ) ; sh:minCount 1 ] .
"#;

    #[test]
    fn parses_shapes_property_shapes_and_constraints() {
        let (shapes, report) = parse_shapes(RECETARIO).unwrap();
        assert_eq!(shapes.len(), 2, "{shapes:?}");
        let receta = shapes.iter().find(|s| s.iri.as_deref() == Some("http://cocina.example/RecetaShape")).unwrap();
        assert_eq!(receta.name, "Receta bien descrita");
        assert_eq!(receta.targets, vec![Target::Class("http://cocina.example/Receta".into())]);
        assert_eq!(receta.property_shapes.len(), 4);
        let usa = receta.property_shapes.iter().find(|p| p.path == PathSpec::Predicate("usa".into())).unwrap();
        assert_eq!(
            usa.constraints,
            vec![ConstraintType::MinCount(2), ConstraintType::Class("http://cocina.example/Ingrediente".into())]
        );
        let dif = receta.property_shapes.iter().find(|p| p.path == PathSpec::Predicate("dificultad".into())).unwrap();
        assert_eq!(
            dif.constraints,
            vec![ConstraintType::In(vec![
                PropertyValue::String("fácil".into()),
                PropertyValue::String("media".into()),
                PropertyValue::String("difícil".into()),
            ])]
        );
        let ing = shapes.iter().find(|s| s.name == "IngredienteShape").unwrap();
        assert_eq!(ing.constraints, vec![ConstraintType::NodeKindShacl(ShaclNodeKind::Iri)]);
        assert!(ing.property_shapes.is_empty(), "the sequence path is dropped, with a reason");

        assert_eq!(report.shapes, 2);
        assert_eq!(report.property_shapes, 4);
        assert_eq!(report.constraints, 1 + 2 + 2 + 1 + 2);
        let ignored = report.ignored.join("\n");
        assert!(ignored.contains("sh:closed"), "{ignored}");
        assert!(ignored.contains("issue #101"), "{ignored}");
        assert_eq!(report.ignored.len(), 2, "{ignored}");
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
    }

    #[test]
    fn logical_combinators_severity_message_and_pattern_errors() {
        let ttl = r#"
@prefix sh:  <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix :    <http://cocina.example/> .
:IngredienteShape a sh:NodeShape ; sh:targetClass :Ingrediente ;
  sh:property [ sh:path :nombre ; sh:minCount 1 ] .
:RecetaShape a sh:NodeShape ; sh:targetClass :Receta ; sh:severity sh:Warning ; sh:message "receta rara" ;
  sh:property [ sh:path :tiempoMin ; sh:or ( [ sh:datatype xsd:integer ] [ sh:datatype xsd:decimal ] ) ] ,
              [ sh:path :estado ; sh:not [ sh:in ( "crudo" ) ] ; sh:severity sh:Info ; sh:message "no crudo" ] ,
              [ sh:path :usa ; sh:node :IngredienteShape ] ;
  sh:xone ( [ sh:class :Receta ] [ sh:class :Postre ] ) ;
  sh:deactivated false .
"#;
        let (shapes, report) = parse_shapes(ttl).unwrap();
        assert!(report.ignored.is_empty(), "{:?}", report.ignored);
        let receta = shapes.iter().find(|s| s.name == "RecetaShape").unwrap();
        assert_eq!((receta.severity, receta.message.as_deref()), (Severity::Warning, Some("receta rara")));
        assert!(!receta.deactivated);
        assert!(matches!(&receta.constraints[..], [ConstraintType::Xone(v)] if v.len() == 2));
        let tiempo = receta.property_shapes.iter().find(|p| p.path == PathSpec::Predicate("tiempoMin".into())).unwrap();
        assert!(matches!(&tiempo.constraints[..], [ConstraintType::Or(v)] if v.len() == 2 && v[0].constraints == vec![ConstraintType::Datatype(DatatypeKind::Int)]));
        let estado = receta.property_shapes.iter().find(|p| p.path == PathSpec::Predicate("estado".into())).unwrap();
        assert!(matches!(&estado.constraints[..], [ConstraintType::Not(_)]));
        assert_eq!((estado.severity, estado.message.as_deref()), (Some(Severity::Info), Some("no crudo")));
        let usa = receta.property_shapes.iter().find(|p| p.path == PathSpec::Predicate("usa".into())).unwrap();
        assert_eq!(usa.constraints, vec![ConstraintType::Node("http://cocina.example/IngredienteShape".into())]);
        // Member shapes count as constraints of their own; the report counts the top level.
        assert_eq!(report.shapes, 2);

        let err = parse_shapes(r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix :   <http://cocina.example/> .
:S sh:targetClass :Receta ; sh:property [ sh:path :nombre ; sh:pattern "(" ] .
"#).unwrap_err().to_string();
        assert!(err.contains(":S sh:property") && err.contains("patron regex invalido '('"), "{err}");
    }

    #[test]
    fn malformed_turtle_is_an_error_with_position() {
        let err = parse_shapes("@prefix sh: <http://www.w3.org/ns/shacl#> .\n:S a sh:NodeShape ; sh:minCount .").unwrap_err();
        assert!(err.to_string().contains("line 2"), "{err}");
    }

    #[test]
    fn unknown_and_malformed_terms_are_reported_not_silent() {
        let ttl = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix :   <http://cocina.example/> .
:S a sh:NodeShape ; sh:targetClass :Receta ;
   sh:closed true ;
   sh:property [ sh:path :nombre ; sh:minCount "muchos" ; sh:frobnicate 3 ] ;
   sh:property [ sh:minCount 1 ] .
"#;
        let (shapes, report) = parse_shapes(ttl).unwrap();
        assert_eq!(shapes.len(), 1);
        assert_eq!(shapes[0].property_shapes.len(), 1);
        assert!(shapes[0].property_shapes[0].constraints.is_empty());
        let ignored = report.ignored.join("\n");
        assert!(ignored.contains("sh:closed"), "{ignored}");
        assert!(ignored.contains("sh:minCount espera un entero"), "{ignored}");
        assert!(ignored.contains("sh:frobnicate"), "{ignored}");
        assert!(ignored.contains("sin sh:path"), "{ignored}");
        assert_eq!(report.ignored.len(), 4, "{ignored}");
    }

    #[test]
    fn datatype_outside_the_importer_table_warns() {
        let ttl = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix :   <http://cocina.example/> .
:S sh:targetClass :Receta ; sh:property [ sh:path :fecha ; sh:datatype xsd:date ] .
"#;
        let (shapes, report) = parse_shapes(ttl).unwrap();
        assert_eq!(shapes[0].property_shapes[0].constraints, vec![ConstraintType::Datatype(DatatypeKind::Str)]);
        assert!(report.warnings.iter().any(|w| w.contains("xsd:date")), "{:?}", report.warnings);
    }
}
