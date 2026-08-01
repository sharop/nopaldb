// src/python/transaction.rs
//
// Python wrapper for Transaction

use pyo3::prelude::*;
use pyo3::types::PyDict;
use crate::Transaction as RustTransaction;
use crate::types::{Node, Edge};
use uuid::Uuid;
use super::to_py_result;

/// Python wrapper for Transaction
#[pyclass(name = "Transaction")]
pub struct PyTransaction {
    inner: Option<RustTransaction>,
}

impl PyTransaction {
    pub(crate) fn new(tx: RustTransaction) -> Self {
        PyTransaction { inner: Some(tx) }
    }

    fn take_inner(&mut self) -> PyResult<RustTransaction> {
        self.inner.take().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Transaction already committed or rolled back"
            )
        })
    }

    fn get_inner_mut(&mut self) -> PyResult<&mut RustTransaction> {
        self.inner.as_mut().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Transaction already committed or rolled back"
            )
        })
    }
}

#[pymethods]
impl PyTransaction {
    /// Add a node to the graph
    ///
    /// Args:
    ///     label: Node label (e.g. "Person")
    ///     properties: Dictionary of properties
    ///
    /// Returns:
    ///     str: Node ID (UUID)
    ///
    /// Example:
    ///     >>> tx = graph.begin_transaction()
    ///     >>> node_id = tx.add_node("Person", {"name": "Alice", "age": 30})
    fn add_node(
        &mut self,
        py: Python<'_>,
        label: &str,
        properties: &Bound<'_, PyDict>
    ) -> PyResult<String> {
        let tx = self.get_inner_mut()?;

        // Conversor compartido de la frontera (bool antes que int — las copias
        // inline anteriores convertían True en Int(1)).
        let props = super::pydict_to_props(properties)?;

        // Create node
        let mut node = Node::new(label);
        for(key, value) in props{
            node = node.with_property(key, value);
        }
        // Propagar el error del add: antes se descartaba con `let _ =` y el
        // caller recibía el id de un nodo que quizá nunca se agregó.
        let node_id = to_py_result(crate::python::runtime::block_on(py, async {
            tx.add_node(node).await
        }))?;

        Ok(node_id.to_string())
    }

    /// Add an edge between two nodes
    ///
    /// Args:
    ///     source: Source node ID (UUID string)
    ///     target: Target node ID (UUID string)
    ///     edge_type: Type of relationship (e.g. "KNOWS")
    ///     properties: Optional dictionary of edge properties
    ///
    /// Returns:
    ///     str: Edge ID (UUID)
    ///
    /// Example:
    ///     >>> tx.add_edge(alice_id, bob_id, "KNOWS")
    ///     >>> tx.add_edge(alice_id, bob_id, "KNOWS", {"since": 2019, "strength": "strong"})
    #[pyo3(signature = (source, target, edge_type, properties=None))]
    fn add_edge(
        &mut self,
        source: &str,
        target: &str,
        edge_type: &str,
        properties: Option<&Bound<'_, PyDict>>
    ) -> PyResult<String> {
        let tx = self.get_inner_mut()?;

        // Parse UUIDs
        let source_id = Uuid::parse_str(source)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(
                format!("Invalid source UUID: {}", e)
            ))?;

        let target_id = Uuid::parse_str(target)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(
                format!("Invalid target UUID: {}", e)
            ))?;

        // Create edge
        let mut edge = Edge::new(source_id, target_id, edge_type);

        // Add properties if provided (conversor compartido de la frontera)
        if let Some(props_dict) = properties {
            edge.properties.extend(super::pydict_to_props(props_dict)?);
        }

        // Propagar el error del add (mismo patrón que add_node)
        let edge_id = to_py_result(tx.add_edge(edge))?;

        Ok(edge_id.to_string())
    }

    /// Commit the transaction
    ///
    /// Example:
    ///     >>> tx.commit()
    fn commit(&mut self, py: Python<'_>) -> PyResult<()> {
        let tx = self.take_inner()?;

        let result = crate::python::runtime::block_on(py, async {
            tx.commit().await
        });

        to_py_result(result)
    }

    /// Rollback the transaction
    ///
    /// Example:
    ///     >>> tx.rollback()
    fn rollback(&mut self, py: Python<'_>) -> PyResult<()> {
        let tx = self.take_inner()?;

        let result = crate::python::runtime::block_on(py, async {
            tx.rollback()
        });

        to_py_result(result)
    }

    /// String representation
    fn __repr__(&self) -> String {
        if self.inner.is_some() {
            "<Transaction: active>".to_string()
        } else {
            "<Transaction: closed>".to_string()
        }
    }
}