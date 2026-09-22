// src/python/bulk_loader.rs
//
// Python bindings for BulkLoader - High-performance bulk import API

use pyo3::prelude::*;
use pyo3::types::{PyAny, PyDict};
use crate::graph::BulkLoader as RustBulkLoader;
use crate::types::{Edge, Node};
use super::{pydict_to_props, to_py_result};
use uuid::Uuid;

/// Python wrapper for high-performance BulkLoader
///
/// BulkLoader buffers nodes and edges in memory and flushes them
/// in batches to the database, providing 100-1000x speedup.
///
/// Property values accept the same types as `Transaction.add_node`:
/// str, int, float, bool, bytes, None (stored as null), and nested
/// lists/tuples/dicts. They go through the one converter shared by every
/// write path, so a dict loaded here reads back exactly like one written
/// in a transaction or an upsert.
///
/// Example:
///     >>> with graph.bulk_loader(10_000) as loader:
///     ...     alice = loader.add_node("Person", {"name": "Alice"})
///     ...     bob = loader.add_node("Person", {"name": "Bob"})
///     ...     edge_id = loader.add_edge(alice, bob, "KNOWS", {"since": 2020})
///     >>> # or, without the context manager:
///     >>> loader = graph.bulk_loader(10_000)
///     >>> loader.add_node("Person", {"name": "Carol"})
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
        py: Python<'_>,
        label: &str,
        properties: &Bound<'_, PyDict>,
    ) -> PyResult<String> {
        let loader = self.active()?;

        let mut node = Node::new(label);
        node.properties.extend(pydict_to_props(properties)?);
        let node_id = node.id;

        to_py_result(crate::python::runtime::block_on(py, async {
            loader.add_node(node).await
        }))?;

        Ok(node_id.to_string())
    }

    /// Add an edge to the bulk load buffer
    ///
    /// Args:
    ///     source (str): Source node UUID
    ///     target (str): Target node UUID
    ///     edge_type (str): Edge type (e.g., "KNOWS")
    ///     properties (dict, optional): Edge properties, same value types as add_node
    ///
    /// Returns:
    ///     str: Edge UUID
    ///
    /// Example:
    ///     >>> edge_id = loader.add_edge(alice_id, bob_id, "KNOWS", {"since": 2020})
    #[pyo3(signature = (source, target, edge_type, properties=None))]
    fn add_edge(
        &mut self,
        py: Python<'_>,
        source: &str,
        target: &str,
        edge_type: &str,
        properties: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<String> {
        let loader = self.active()?;

        let source_id = Uuid::parse_str(source)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Invalid source UUID: {}", e)))?;
        let target_id = Uuid::parse_str(target)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Invalid target UUID: {}", e)))?;

        let mut edge = Edge::new(source_id, target_id, edge_type);
        if let Some(props) = properties {
            edge.properties.extend(pydict_to_props(props)?);
        }
        let edge_id = edge.id;

        to_py_result(crate::python::runtime::block_on(py, async {
            loader.add_edge(edge).await
        }))?;

        Ok(edge_id.to_string())
    }

    /// Finish bulk load and flush all pending data
    ///
    /// Called automatically when the loader is used as a context manager
    /// (the stats are discarded in that case). After it the loader cannot
    /// be used again: create a new one.
    ///
    /// Returns:
    ///     dict: Statistics with keys: nodes, edges, duration_secs, nodes_per_second
    ///
    /// Example:
    ///     >>> stats = loader.finish()
    ///     >>> print(f"Loaded {stats['nodes']:,} nodes")
    fn finish(&mut self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let loader = self.inner.take().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "BulkLoader already finished"
            )
        })?;

        let stats = to_py_result(crate::python::runtime::block_on(py, async {
            loader.finish().await
        }))?;

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
        py: Python<'_>,
        _exc_type: Option<&Bound<'_, PyAny>>,
        _exc_value: Option<&Bound<'_, PyAny>>,
        _traceback: Option<&Bound<'_, PyAny>>,
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

    fn active(&mut self) -> PyResult<&mut RustBulkLoader> {
        self.inner.as_mut().ok_or_else(|| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "BulkLoader already finished. Create a new one."
            )
        })
    }
}
