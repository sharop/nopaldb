// src/python/reasoner.rs
//
// PyO3 Python bindings for ELReasoner — Step 10 of the NopalDB ontological roadmap.
//
// Feature gate: compiled only when both `python` and `reasoner` features are enabled
// (i.e. the `python-reasoner` combined feature or when both are specified explicitly).

use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::reasoner::{Axiom, CompletionRule, ELReasoner};
use crate::types::NodeId;
use super::to_py_result;

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn inference_to_py(inf: &crate::reasoner::Inference) -> PyInference {
    let (sub, super_class) = match &inf.axiom {
        Axiom::SubClassOf { sub, super_class } => {
            (sub.to_string(), super_class.to_string())
        }
        Axiom::ConjunctionInclusion { result, left, .. } => {
            // ConjunctionInclusion axioms are asserted (not derived) so they
            // should not appear here; use left/result as a fallback display.
            (left.to_string(), result.to_string())
        }
        Axiom::ExistentialRestriction { sub, role, filler } => {
            // Preserve the 2-field PyInference shape with a stable textual
            // encoding for existential forms.
            (sub.to_string(), format!("exists({}).{}", role, filler))
        }
        Axiom::ExistentialDomain { role, filler, result } => {
            (format!("exists({}).{}", role, filler), result.to_string())
        }
    };
    let rule = match inf.rule {
        CompletionRule::CR1 => "CR1",
        CompletionRule::CR2 => "CR2",
        CompletionRule::CR3 => "CR3",
    };
    PyInference { sub, super_class, rule: rule.to_string() }
}

// ---------------------------------------------------------------------------
// PyInference
// ---------------------------------------------------------------------------

/// A single inference derived by the EL reasoner.
///
/// Attributes
/// ----------
/// sub : str
///     UUID string of the subclass node.
/// super_class : str
///     UUID string of the superclass node.
/// rule : str
///     Completion rule that fired: ``"CR1"``, ``"CR2"``, or ``"CR3"``.
#[pyclass(name = "Inference", skip_from_py_object)]
pub struct PyInference {
    #[pyo3(get)]
    pub sub: String,
    #[pyo3(get)]
    pub super_class: String,
    #[pyo3(get)]
    pub rule: String,
}

#[pymethods]
impl PyInference {
    fn __repr__(&self) -> String {
        format!(
            "Inference(sub={}, super_class={}, rule={})",
            self.sub, self.super_class, self.rule
        )
    }

    /// Convert to a plain Python dict.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("sub", &self.sub)?;
        d.set_item("super_class", &self.super_class)?;
        d.set_item("rule", &self.rule)?;
        Ok(d)
    }
}

// ---------------------------------------------------------------------------
// PyELReasoner
// ---------------------------------------------------------------------------

/// An OWL-EL reasoner for class hierarchy classification.
///
/// Supports:
///
/// - **CR1** — transitivity: A ⊑ B ∧ B ⊑ C → A ⊑ C
/// - **CR2** — conjunction: A ⊑ B ∧ A ⊑ C ∧ B ⊓ C ⊑ D → A ⊑ D
/// - **CR3** — existential: A ⊑ ∃R.B ∧ B ⊑ C ∧ ∃R.C ⊑ D → A ⊑ D
///
/// Quick start
/// -----------
/// ::
///
///     from nopaldb import ELReasoner
///     import uuid
///
///     r = ELReasoner()
///     a = str(uuid.uuid4())
///     b = str(uuid.uuid4())
///     c = str(uuid.uuid4())
///
///     r.register_class(a, "Animal")
///     r.register_class(b, "Mammal")
///     r.register_class(c, "Dog")
///
///     r.assert_subclass(b, a)  # Mammal ⊑ Animal
///     r.assert_subclass(c, b)  # Dog ⊑ Mammal
///
///     r.classify_all()
///     assert r.is_subclass_of(c, a)   # Dog ⊑ Animal (transitively)
#[pyclass(name = "ELReasoner")]
pub struct PyELReasoner {
    inner: ELReasoner,
}

#[pymethods]
impl PyELReasoner {
    /// Create a new, empty ELReasoner.
    #[new]
    pub fn new() -> Self {
        Self { inner: ELReasoner::new() }
    }

    /// Register a class node in the reasoner's internal taxonomy.
    ///
    /// Parameters
    /// ----------
    /// node_id : str
    ///     UUID string of the class node (e.g. ``"550e8400-e29b-41d4-a716-446655440000"``).
    /// label : str
    ///     Human-readable class label (e.g. ``"Animal"``).
    pub fn register_class(&mut self, node_id: &str, label: &str) -> PyResult<()> {
        let id: NodeId = node_id.parse().map_err(|_| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid UUID: {}", node_id))
        })?;
        self.inner.register_class(id, label);
        Ok(())
    }

    /// Assert a direct SubClassOf axiom: ``sub ⊑ super_class``.
    ///
    /// Applies CR1 incrementally and returns any new inferences derived.
    ///
    /// Parameters
    /// ----------
    /// sub : str
    ///     UUID of the subclass.
    /// super_class : str
    ///     UUID of the superclass.
    ///
    /// Returns
    /// -------
    /// list[Inference]
    pub fn assert_subclass(&mut self, sub: &str, super_class: &str) -> PyResult<Vec<PyInference>> {
        let sub_id: NodeId = sub.parse().map_err(|_| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid UUID: {}", sub))
        })?;
        let super_id: NodeId = super_class.parse().map_err(|_| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid UUID: {}", super_class))
        })?;
        let inferences = to_py_result(self.inner.assert_subclass(sub_id, super_id))?;
        Ok(inferences.iter().map(inference_to_py).collect())
    }

    /// Assert a conjunction inclusion axiom: ``left ⊓ right ⊑ result``.
    ///
    /// Applies CR2 immediately and returns any new inferences derived.
    ///
    /// Parameters
    /// ----------
    /// left : str
    ///     UUID of the left conjunct class.
    /// right : str
    ///     UUID of the right conjunct class.
    /// result : str
    ///     UUID of the result superclass.
    ///
    /// Returns
    /// -------
    /// list[Inference]
    pub fn assert_conjunction(
        &mut self,
        left: &str,
        right: &str,
        result: &str,
    ) -> PyResult<Vec<PyInference>> {
        let l: NodeId = left.parse().map_err(|_| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid UUID: {}", left))
        })?;
        let r: NodeId = right.parse().map_err(|_| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid UUID: {}", right))
        })?;
        let res: NodeId = result.parse().map_err(|_| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid UUID: {}", result))
        })?;
        let inferences = to_py_result(self.inner.assert_conjunction(l, r, res))?;
        Ok(inferences.iter().map(inference_to_py).collect())
    }

    /// Assert an existential restriction axiom: ``sub ⊑ ∃role.filler``.
    ///
    /// Applies CR3 incrementally against known existential domains.
    ///
    /// Parameters
    /// ----------
    /// sub : str
    ///     UUID of the subclass A.
    /// role : str
    ///     Property/role name R.
    /// filler : str
    ///     UUID of the filler class B.
    ///
    /// Returns
    /// -------
    /// list[Inference]
    pub fn assert_existential(
        &mut self,
        sub: &str,
        role: &str,
        filler: &str,
    ) -> PyResult<Vec<PyInference>> {
        let sub_id: NodeId = sub.parse().map_err(|_| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid UUID: {}", sub))
        })?;
        let filler_id: NodeId = filler.parse().map_err(|_| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid UUID: {}", filler))
        })?;
        let inferences = to_py_result(self.inner.assert_existential(sub_id, role, filler_id))?;
        Ok(inferences.iter().map(inference_to_py).collect())
    }

    /// Assert an existential domain axiom: ``∃role.filler ⊑ result``.
    ///
    /// Applies CR3 incrementally against known existential restrictions.
    ///
    /// Parameters
    /// ----------
    /// role : str
    ///     Property/role name R.
    /// filler : str
    ///     UUID of the filler class C.
    /// result : str
    ///     UUID of the result superclass D.
    ///
    /// Returns
    /// -------
    /// list[Inference]
    pub fn assert_existential_domain(
        &mut self,
        role: &str,
        filler: &str,
        result: &str,
    ) -> PyResult<Vec<PyInference>> {
        let filler_id: NodeId = filler.parse().map_err(|_| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid UUID: {}", filler))
        })?;
        let result_id: NodeId = result.parse().map_err(|_| {
            pyo3::exceptions::PyValueError::new_err(format!("Invalid UUID: {}", result))
        })?;
        let inferences = to_py_result(
            self.inner.assert_existential_domain(role, filler_id, result_id)
        )?;
        Ok(inferences.iter().map(inference_to_py).collect())
    }

    /// Run CR1, CR2, and CR3 to saturation over all currently known axioms.
    ///
    /// Returns
    /// -------
    /// list[Inference]
    ///     All NEW inferences derived in this pass (empty if already saturated).
    pub fn classify_all(&mut self) -> Vec<PyInference> {
        self.inner.classify_all().iter().map(inference_to_py).collect()
    }

    /// Check if ``sub`` is a subclass of ``super_class`` (direct or inferred).
    ///
    /// Parameters
    /// ----------
    /// sub : str
    ///     UUID of the candidate subclass.
    /// super_class : str
    ///     UUID of the candidate superclass.
    ///
    /// Returns
    /// -------
    /// bool
    pub fn is_subclass_of(&mut self, sub: &str, super_class: &str) -> bool {
        let Ok(sub_id) = sub.parse::<NodeId>() else { return false; };
        let Ok(super_id) = super_class.parse::<NodeId>() else { return false; };
        self.inner.is_subclass_of(sub_id, super_id)
    }

    /// All superclasses of ``node`` — direct and inferred — as UUID strings.
    ///
    /// Parameters
    /// ----------
    /// node : str
    ///     UUID of the node.
    ///
    /// Returns
    /// -------
    /// list[str]
    pub fn superclasses(&self, node: &str) -> Vec<String> {
        let Ok(id) = node.parse::<NodeId>() else { return vec![]; };
        self.inner.superclasses(id).iter().map(|i| i.to_string()).collect()
    }

    /// All subclasses of ``node`` — direct and inferred — as UUID strings.
    ///
    /// Parameters
    /// ----------
    /// node : str
    ///     UUID of the node.
    ///
    /// Returns
    /// -------
    /// list[str]
    pub fn subclasses(&mut self, node: &str) -> Vec<String> {
        let Ok(id) = node.parse::<NodeId>() else { return vec![]; };
        self.inner.subclasses(id).iter().map(|i| i.to_string()).collect()
    }

    /// Number of direct SubClassOf edges currently asserted in the taxonomy.
    pub fn axiom_count(&self) -> usize {
        self.inner.axiom_count()
    }

    /// Number of inferences derived so far (CR1 + CR2).
    pub fn derived_count(&self) -> usize {
        self.inner.derived_inferences().len()
    }

    /// All inferences derived so far as :class:`Inference` objects.
    ///
    /// Returns
    /// -------
    /// list[Inference]
    pub fn derived_inferences(&self) -> Vec<PyInference> {
        self.inner.derived_inferences().iter().map(inference_to_py).collect()
    }

    fn __repr__(&self) -> String {
        format!(
            "ELReasoner(axioms={}, derived={})",
            self.inner.axiom_count(),
            self.inner.derived_inferences().len(),
        )
    }
}

impl Default for PyELReasoner {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests (test the Python wrapper logic via Rust without GIL overhead)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn uid() -> String { Uuid::new_v4().to_string() }

    #[test]
    fn test_py_reasoner_register_and_assert() {
        let mut r = PyELReasoner::new();
        let a = uid(); let b = uid(); let c = uid();

        r.register_class(&a, "Animal").unwrap();
        r.register_class(&b, "Mammal").unwrap();
        r.register_class(&c, "Dog").unwrap();

        // Mammal ⊑ Animal, Dog ⊑ Mammal
        let _ = r.assert_subclass(&b, &a).unwrap();
        let _ = r.assert_subclass(&c, &b).unwrap();
        r.classify_all();

        // Dog ⊑ Animal (transitively)
        assert!(r.is_subclass_of(&c, &a), "Dog must be subclass of Animal");
        assert!(!r.is_subclass_of(&a, &c), "Animal must NOT be subclass of Dog");
    }

    #[test]
    fn test_py_reasoner_cr2() {
        let mut r = PyELReasoner::new();
        let a = uid(); let b = uid(); let c = uid(); let d = uid();

        r.register_class(&a, "A").unwrap();
        r.register_class(&b, "B").unwrap();
        r.register_class(&c, "C").unwrap();
        r.register_class(&d, "D").unwrap();

        r.assert_subclass(&a, &b).unwrap(); // A ⊑ B
        r.assert_subclass(&a, &c).unwrap(); // A ⊑ C
        let inferred = r.assert_conjunction(&b, &c, &d).unwrap(); // B ⊓ C ⊑ D → derive A ⊑ D

        assert!(inferred.iter().any(|i| i.sub == a && i.super_class == d && i.rule == "CR2"),
            "CR2 must derive A ⊑ D via Python API");
    }

    #[test]
    fn test_py_reasoner_invalid_uuid() {
        let mut r = PyELReasoner::new();
        let result = r.register_class("not-a-uuid", "Animal");
        assert!(result.is_err(), "invalid UUID must return error");
    }

    #[test]
    fn test_py_reasoner_superclasses_subclasses() {
        let mut r = PyELReasoner::new();
        let a = uid(); let b = uid(); let c = uid();

        r.register_class(&a, "A").unwrap();
        r.register_class(&b, "B").unwrap();
        r.register_class(&c, "C").unwrap();

        r.assert_subclass(&b, &a).unwrap(); // B ⊑ A
        r.assert_subclass(&c, &b).unwrap(); // C ⊑ B
        r.classify_all();

        let sup = r.superclasses(&c);
        assert!(sup.contains(&a), "superclasses(C) must contain A");
        assert!(sup.contains(&b), "superclasses(C) must contain B");

        let sub = r.subclasses(&a);
        assert!(sub.contains(&b), "subclasses(A) must contain B");
        assert!(sub.contains(&c), "subclasses(A) must contain C");
    }

    #[test]
    fn test_py_reasoner_derived_count() {
        let mut r = PyELReasoner::new();
        let a = uid(); let b = uid(); let c = uid();

        r.register_class(&a, "A").unwrap();
        r.register_class(&b, "B").unwrap();
        r.register_class(&c, "C").unwrap();

        r.assert_subclass(&b, &a).unwrap();
        r.assert_subclass(&c, &b).unwrap();
        r.classify_all();

        // C ⊑ A is derived (not direct)
        assert!(r.derived_count() > 0, "derived_count must be > 0 after classify_all");
        assert_eq!(r.axiom_count(), 2, "axiom_count must be 2 (direct edges only)");
    }

    #[test]
    fn test_py_reasoner_cr3_existential() {
        let mut r = PyELReasoner::new();
        let a = uid(); let b = uid(); let c = uid(); let d = uid();

        r.register_class(&a, "A").unwrap();
        r.register_class(&b, "B").unwrap();
        r.register_class(&c, "C").unwrap();
        r.register_class(&d, "D").unwrap();

        // B ⊑ C
        r.assert_subclass(&b, &c).unwrap();
        // A ⊑ ∃R.B
        r.assert_existential(&a, "R", &b).unwrap();
        // ∃R.C ⊑ D  => A ⊑ D via CR3
        let inferred = r.assert_existential_domain("R", &c, &d).unwrap();

        assert!(
            inferred.iter().any(|i| i.sub == a && i.super_class == d && i.rule == "CR3"),
            "CR3 must derive A ⊑ D via Python API"
        );
    }
}
