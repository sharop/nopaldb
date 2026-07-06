// src/python/bulk_loader.rs
//
// Python bindings for BulkLoader - High-performance bulk import API

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyAny};
use crate::graph::BulkLoader as RustBulkLoader;
use crate::types::{Node, Edge, PropertyValue};
use super::to_py_result;
use uuid::Uuid;

/// Python wrapper for high-performance BulkLoader
///
/// BulkLoader buffers nodes and edges in memory and flushes them
/// in batches to the database, providing 100-1000x speedup.
///
/// Example:
///     >>> loader = graph.bulk_loader(10000)
///     >>> node_id = loader.add_node("Person", {"name": "Alice"})
///     >>> stats = loader.finish()
#[pyclass(name = "BulkLoader")]
pub struct PyBulkLoader {
    inner: Option<RustBulkLoader>,
}

#[pymethods]
impl PyBulkLoader {
    /// Add a node to the bulk load buffer
    ///
    /// Args:
    ///     label (str): Node label (e.g., "Person", "Company")
    ///     properties (dict): Node properties as key-value pairs
    ///
    /// Returns:
    ///     str: Node UUID (can be used for building edges)
    ///
    /// Example:
    ///     >>> alice_id = loader.add_node("Person", {
    ///     ...     "name": "Alice",
    ///     ...     "age": 30
    ///     ... })
    fn add_node(
        &mut self,
        label: &str,
        properties: &Bound<PyDict>,
        py: Python,
    ) -> PyResult<String> {
        // Check if already finished
        let loader = self.inner.as_mut().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "BulkLoader already finished. Create a new one."
            )
        })?;

        // Create node with label
        let mut node = Node::new(label);

        // Add properties one by one
        for (key, value) in properties {
            let key_str = key.extract::<String>()?;
            let prop_value = python_to_property_value(py, value.into())?;
            node = node.with_property(key_str, prop_value);
        }

        let node_id = node.id;

        // Add to buffer (async)
        let result = crate::python::runtime::block_on(py, async {
            loader.add_node(node).await
        });

        to_py_result(result)?;

        // Return UUID as string
        Ok(node_id.to_string())
    }

    /// Add an edge to the bulk load buffer
    ///
    /// Args:
    ///     source_id (str): Source node UUID
    ///     target_id (str): Target node UUID
    ///     label (str): Edge label
    ///
    /// Example:
    ///     >>> loader.add_edge(alice_id, bob_id, "KNOWS")
    fn add_edge(
        &mut self,
        py: Python<'_>,
        source_id: &str,
        target_id: &str,
        label: &str,
    ) -> PyResult<()> {
        let loader = self.inner.as_mut().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "BulkLoader already finished"
            )
        })?;

        // Parse UUIDs
        let source = Uuid::parse_str(source_id)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(
                format!("Invalid source UUID: {}", e)
            ))?;

        let target = Uuid::parse_str(target_id)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(
                format!("Invalid target UUID: {}", e)
            ))?;

        // Create edge
        let edge = Edge::new(source, target, label);

        // Add to buffer
        let result = crate::python::runtime::block_on(py, async {
            loader.add_edge(edge).await
        });

        to_py_result(result)
    }

    /// Finish bulk load and flush all pending data
    ///
    /// Returns:
    ///     dict: Statistics with keys: nodes, edges, duration_secs, nodes_per_second
    ///
    /// Example:
    ///     >>> stats = loader.finish()
    ///     >>> print(f"Loaded {stats['nodes']:,} nodes")
    fn finish(&mut self, py: Python) -> PyResult<Py<PyAny>> {
        let loader = self.inner.take().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "BulkLoader already finished"
            )
        })?;

        // Finish bulk load
        let stats = crate::python::runtime::block_on(py, async {
            loader.finish().await
        });

        let stats = to_py_result(stats)?;

        // Create Python dict (compatible with pyo3 0.27)
        let dict = PyDict::new(py);
        dict.set_item("nodes", stats.nodes_inserted)?;
        dict.set_item("edges", stats.edges_inserted)?;
        dict.set_item("duration_secs", stats.duration.as_secs_f64())?;
        dict.set_item("nodes_per_second", stats.nodes_per_second)?;

        Ok(dict.into())
    }

    /// String representation
    fn __repr__(&self) -> String {
        if self.inner.is_some() {
            "<BulkLoader: active>".to_string()
        } else {
            "<BulkLoader: finished>".to_string()
        }
    }

    /// Enter context manager
    fn __enter__(slf: Py<Self>) -> Py<Self> {
        slf
    }

    /// Exit context manager
    fn __exit__(
        &mut self,
        py: Python,
        _exc_type: &Bound<PyAny>,
        _exc_value: &Bound<PyAny>,
        _traceback: &Bound<PyAny>,
    ) -> PyResult<bool> {
        if self.inner.is_some() {
            self.finish(py)?;
        }
        Ok(false)
    }
}

impl PyBulkLoader {
    /// Create from Rust BulkLoader
    pub(crate) fn new(loader: RustBulkLoader) -> PyResult<Self> {
        Ok(PyBulkLoader {
            inner: Some(loader),
        })
    }
}

/// Convert Python object to PropertyValue
fn python_to_property_value(py: Python, obj: Py<PyAny>) -> PyResult<PropertyValue> {
    // Try string first
    if let Ok(s) = obj.extract::<String>(py) {
        return Ok(PropertyValue::String(s));
    }

    // Try bool before int
    if let Ok(b) = obj.extract::<bool>(py) {
        return Ok(PropertyValue::Bool(b));
    }

    // Try integer
    if let Ok(i) = obj.extract::<i64>(py) {
        return Ok(PropertyValue::Int(i));
    }

    // Try float
    if let Ok(f) = obj.extract::<f64>(py) {
        return Ok(PropertyValue::Float(f));
    }

    // None → empty string
    if obj.is_none(py) {
        return Ok(PropertyValue::String(String::new()));
    }

    // Get type name for error
    let type_name = obj.bind(py).get_type().name()
        .map(|n| n.to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
        format!(
            "Unsupported property type: {}. Supported: str, int, float, bool",
            type_name
        )
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_property_conversion() {
        Python::attach(|py| {
            // String
            let s: Py<PyAny> = "hello"
                .into_pyobject(py)
                .expect("string conversion must work")
                .into_any()
                .unbind();
            let prop = python_to_property_value(py, s).unwrap();
            assert!(matches!(prop, PropertyValue::String(_)));

            // Integer
            let i: Py<PyAny> = 42_i64
                .into_pyobject(py)
                .expect("int conversion must work")
                .into_any()
                .unbind();
            let prop = python_to_property_value(py, i).unwrap();
            assert!(matches!(prop, PropertyValue::Int(42)));

            // Float
            let f: Py<PyAny> = 3.14_f64
                .into_pyobject(py)
                .expect("float conversion must work")
                .into_any()
                .unbind();
            let prop = python_to_property_value(py, f).unwrap();
            assert!(matches!(prop, PropertyValue::Float(_)));

            // Bool
            let b: Py<PyAny> = true
                .into_pyobject(py)
                .expect("bool conversion must work")
                .to_owned()
                .into_any()
                .unbind();
            let prop = python_to_property_value(py, b).unwrap();
            assert!(matches!(prop, PropertyValue::Bool(true)));
        });
    }
}
