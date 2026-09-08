// src/shacl/report.rs
//! Tipos de salida del validador SHACL Core.

use serde::Serialize;

use crate::types::{NodeId, PropertyValue};

/// Severidad de una violacion, segun la especificacion SHACL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Severity {
    /// sh:Violation — incumplimiento de un constraint obligatorio (default).
    Violation,
    /// sh:Warning — advertencia; no impide conformidad.
    Warning,
    /// sh:Info — informativo.
    Info,
}

/// Violacion individual de un constraint SHACL.
///
/// Además del mensaje trae lo que hace falta para actuar sin leerlo:
/// el componente que falló (`sh:MinCountConstraintComponent`, …), el path
/// y el valor concreto que lo violó (para constraints por valor).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConstraintViolation {
    /// Nodo que no paso la validacion (focus node en terminologia SHACL).
    pub focus_node: NodeId,
    /// ID del Shape que genero la violacion.
    pub shape_id: NodeId,
    /// Nombre del shape (`sh:name` o local name de su IRI).
    pub shape_name: String,
    /// Componente SHACL que falló, p.ej. `sh:MinCountConstraintComponent`.
    pub constraint: String,
    /// Predicado (propiedad o tipo de arista) del path, si aplica (PropertyShape).
    pub path: Option<String>,
    /// El valor que violó la constraint: el literal, o el `iri` del nodo
    /// destino (su id si no tiene). `None` para constraints de cardinalidad.
    pub value: Option<PropertyValue>,
    /// Mensaje legible con el motivo del fallo.
    pub message: String,
    /// Severidad de la violacion.
    pub severity: Severity,
}

impl ConstraintViolation {
    /// Crea una violacion con severidad Violation (el caso mas comun).
    pub fn violation(
        focus_node: NodeId,
        shape_id: NodeId,
        path: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            focus_node,
            shape_id,
            shape_name: String::new(),
            constraint: String::new(),
            path,
            value: None,
            message: message.into(),
            severity: Severity::Violation,
        }
    }

    /// Fija el valor que violó la constraint.
    pub fn with_value(mut self, value: PropertyValue) -> Self {
        self.value = Some(value);
        self
    }

    /// Fija la severidad.
    pub fn with_severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }
}

/// Reporte de validacion SHACL completo.
///
/// `conforms = true` solo si no hay violaciones de severidad `Violation`.
#[derive(Debug, Clone, Serialize)]
pub struct ValidationReport {
    /// `true` si todos los focus nodes conforman con todos los shapes.
    pub conforms: bool,
    /// Lista de violaciones encontradas.
    pub violations: Vec<ConstraintViolation>,
    /// Cosas que el validador no pudo hacer y no calla: un `sh:targetNode`
    /// que no resuelve a ningún nodo, por ejemplo.
    pub notes: Vec<String>,
}

impl ValidationReport {
    /// Construye el reporte a partir de las violaciones acumuladas.
    pub fn from_violations(violations: Vec<ConstraintViolation>) -> Self {
        let conforms = violations
            .iter()
            .all(|v| v.severity != Severity::Violation);
        Self { conforms, violations, notes: vec![] }
    }

    /// Reporte de conformidad total (sin violaciones).
    pub fn conforms() -> Self {
        Self { conforms: true, violations: vec![], notes: vec![] }
    }
}

/// Lo que dejó la carga de un documento de shapes: cuánto se cargó y, línea
/// por línea con razón, cada término `sh:*` que este validador no implementa
/// y por eso NO va a comprobar. Vacío `ignored` = las shapes se aplican
/// completas.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ShapesReport {
    /// `sh:NodeShape` cargados.
    pub shapes: usize,
    /// `sh:property` cargados.
    pub property_shapes: usize,
    /// Constraints (de nodo y de propiedad) cargadas.
    pub constraints: usize,
    /// Términos no soportados o mal formados, con razón.
    pub ignored: Vec<String>,
    /// Lo que el parser Turtle asumió (prefijo vacío, `@base`), igual que en
    /// `ImportReport::warnings`.
    pub warnings: Vec<String>,
}
