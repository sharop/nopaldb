// src/shacl/report.rs
//! Tipos de salida del validador SHACL Core.

use crate::types::NodeId;

/// Severidad de una violacion, segun la especificacion SHACL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    /// sh:Violation — incumplimiento de un constraint obligatorio (default).
    Violation,
    /// sh:Warning — advertencia; no impide conformidad.
    Warning,
    /// sh:Info — informativo.
    Info,
}

/// Violacion individual de un constraint SHACL.
#[derive(Debug, Clone)]
pub struct ConstraintViolation {
    /// Nodo que no paso la validacion (focus node en terminologia SHACL).
    pub focus_node: NodeId,
    /// ID del Shape que genero la violacion.
    pub shape_id: NodeId,
    /// Propiedad o edge_type del path, si aplica (PropertyShape).
    pub path: Option<String>,
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
            path,
            message: message.into(),
            severity: Severity::Violation,
        }
    }
}

/// Reporte de validacion SHACL completo.
///
/// `conforms = true` solo si no hay violaciones de severidad `Violation`.
#[derive(Debug)]
pub struct ValidationReport {
    /// `true` si todos los focus nodes conforman con todos los shapes.
    pub conforms: bool,
    /// Lista de violaciones encontradas.
    pub violations: Vec<ConstraintViolation>,
}

impl ValidationReport {
    /// Construye el reporte a partir de las violaciones acumuladas.
    pub fn from_violations(violations: Vec<ConstraintViolation>) -> Self {
        let conforms = violations
            .iter()
            .all(|v| v.severity != Severity::Violation);
        Self { conforms, violations }
    }

    /// Reporte de conformidad total (sin violaciones).
    pub fn conforms() -> Self {
        Self { conforms: true, violations: vec![] }
    }
}
