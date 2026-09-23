// src/python/mod.rs

use pyo3::prelude::*;
use pyo3::{BoundObject, IntoPyObject};
use crate::error::Result as NopalResult;
use crate::types::PropertyValue;

mod graph;
mod runtime;
mod query;
mod transaction;
mod arrow;
mod bulk_loader;

#[cfg(feature = "python-reasoner")]
pub mod reasoner;

pub use graph::PyGraph;
pub use query::{PyNqlResult, PyProfileResult, PyQueryResult};
pub use transaction::PyTransaction;
pub use bulk_loader::PyBulkLoader;

#[cfg(feature = "python-reasoner")]
pub use reasoner::{PyELReasoner, PyInference};

/// NopalDB Python module
#[pymodule]
fn nopaldb(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyGraph>()?;
    m.add_class::<PyNqlResult>()?;
    m.add_class::<PyProfileResult>()?;
    m.add_class::<PyQueryResult>()?;
    m.add_class::<PyTransaction>()?;
    m.add_class::<PyBulkLoader>()?;

    #[cfg(feature = "python-reasoner")]
    {
        m.add_class::<PyELReasoner>()?;
        m.add_class::<PyInference>()?;
    }

    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add("__author__", "Sharop")?;

    Ok(())
}

/// Helper: Convert PropertyValue to Python object
pub(crate) fn property_to_py<'py>(
    py: Python<'py>,
    value: &PropertyValue
) -> PyResult<Bound<'py, PyAny>> {
    match value {
        PropertyValue::String(s) => {
            Ok(s.as_str().into_pyobject(py)?.into_any().into_bound())
        }
        PropertyValue::Int(i) => {
            Ok(i.into_pyobject(py)?.into_any().into_bound())
        }
        PropertyValue::Float(f) => {
            Ok(f.into_pyobject(py)?.into_any().into_bound())
        }
        PropertyValue::Bool(b) => {
            Ok(b.into_pyobject(py)?.into_any().into_bound())
        }
        PropertyValue::Null => {
            // Python None
            Ok(py.None().into_bound(py))
        }
        PropertyValue::Bytes(bytes) => {
            // Python bytes
            Ok(bytes.as_slice().into_pyobject(py)?.into_any().into_bound())
        }
        PropertyValue::List(values) => {
            let items: Vec<Py<PyAny>> = values
                .iter()
                .map(|value| property_to_py(py, value).map(|obj| obj.unbind()))
                .collect::<PyResult<_>>()?;
            Ok(items.into_pyobject(py)?.into_any().into_bound())
        }
        PropertyValue::Object(fields) => {
            let dict = pyo3::types::PyDict::new(py);
            for (key, value) in fields {
                let py_value = property_to_py(py, value)?;
                dict.set_item(key, py_value)?;
            }
            Ok(dict.into_any().into_bound())
        }
    }
}

/// A node as Python sees it: `{"id", "label", "properties"}`. Shared by
/// `get_node(s)`, `neighborhood`, `neighbors` and the `hydrate` option of the
/// searches, so a node always looks the same wherever it comes from.
pub(crate) fn node_to_pydict<'py>(py: Python<'py>, node: &crate::types::Node) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
    let d = pyo3::types::PyDict::new(py);
    d.set_item("id", node.id.to_string())?;
    d.set_item("label", &node.label)?;
    let props = pyo3::types::PyDict::new(py);
    for (k, v) in &node.properties {
        props.set_item(k, property_to_py(py, v)?)?;
    }
    d.set_item("properties", props)?;
    Ok(d)
}

/// An edge as Python sees it: `{"id", "source", "target", "type", "properties"}`.
pub(crate) fn edge_to_pydict<'py>(py: Python<'py>, edge: &crate::types::Edge) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
    let d = pyo3::types::PyDict::new(py);
    d.set_item("id", edge.id.to_string())?;
    d.set_item("source", edge.source.to_string())?;
    d.set_item("target", edge.target.to_string())?;
    d.set_item("type", &edge.edge_type)?;
    let props = pyo3::types::PyDict::new(py);
    for (k, v) in &edge.properties {
        props.set_item(k, property_to_py(py, v)?)?;
    }
    d.set_item("properties", props)?;
    Ok(d)
}

/// UUID from Python text; `ValueError` says which argument was wrong.
pub(crate) fn parse_uuid(text: &str, what: &str) -> PyResult<uuid::Uuid> {
    text.parse().map_err(|e| {
        PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Invalid {what} UUID '{text}': {e}"))
    })
}

/// `"out" | "in" | "both"` → `Direction`.
pub(crate) fn parse_direction(text: &str) -> PyResult<crate::graph::Direction> {
    match text.to_ascii_lowercase().as_str() {
        "out" | "outgoing" => Ok(crate::graph::Direction::Outgoing),
        "in" | "incoming" => Ok(crate::graph::Direction::Incoming),
        "both" => Ok(crate::graph::Direction::Both),
        other => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
            "direction must be 'out', 'in' or 'both' (got '{other}')"
        ))),
    }
}

/// Helper: Convert Result<T> to PyResult<T>
pub(crate) fn to_py_result<T>(result: NopalResult<T>) -> PyResult<T> {
    result.map_err(|e| {
        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e))
    })
}

/// Convierte un valor Python a `PropertyValue`.
///
/// Orden de despacho deliberado: `bool` va ANTES que `int` porque en Python
/// `bool` es subtipo de `int` y `extract::<i64>()` aceptaría `True` como `1`.
/// `bytes` se detecta por downcast explícito de `PyBytes` y no vía
/// `extract::<Vec<u8>>()`, porque ese `FromPyObject` también acepta una lista
/// de enteros y la convertiría en `Bytes` en vez de `List`.
///
/// Es el conversor único de la frontera Python→Rust: `graph.rs` (upsert,
/// links, delete-by-key) y `transaction.rs` (add_node/add_edge) deben usarlo
/// en vez de reimplementar el match — las copias inline anteriores tenían el
/// bug bool→Int(1).
pub(crate) fn pyany_to_property(value: &Bound<'_, PyAny>) -> PyResult<PropertyValue> {
    use pyo3::types::{PyBytes, PyDict, PyList, PyTuple};

    if value.is_none() {
        Ok(PropertyValue::Null)
    } else if let Ok(b) = value.extract::<bool>() {
        Ok(PropertyValue::Bool(b))
    } else if let Ok(i) = value.extract::<i64>() {
        Ok(PropertyValue::Int(i))
    } else if let Ok(f) = value.extract::<f64>() {
        Ok(PropertyValue::Float(f))
    } else if let Ok(s) = value.extract::<String>() {
        Ok(PropertyValue::String(s))
    } else if let Ok(bytes) = value.cast::<PyBytes>() {
        Ok(PropertyValue::Bytes(bytes.as_bytes().to_vec()))
    } else if let Ok(list) = value.cast::<PyList>() {
        let items = list
            .iter()
            .map(|v| pyany_to_property(&v))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(PropertyValue::List(items))
    } else if let Ok(tuple) = value.cast::<PyTuple>() {
        let items = tuple
            .iter()
            .map(|v| pyany_to_property(&v))
            .collect::<PyResult<Vec<_>>>()?;
        Ok(PropertyValue::List(items))
    } else if let Ok(dict) = value.cast::<PyDict>() {
        let mut fields = Vec::with_capacity(dict.len());
        for (k, v) in dict.iter() {
            fields.push((k.extract::<String>()?, pyany_to_property(&v)?));
        }
        Ok(PropertyValue::Object(fields))
    } else {
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "unsupported property value type (use str/int/float/bool/bytes/None/list/tuple/dict)",
        ))
    }
}

/// Convierte un `dict` Python completo a propiedades de nodo/arista.
pub(crate) fn pydict_to_props(
    dict: &Bound<'_, pyo3::types::PyDict>,
) -> PyResult<std::collections::HashMap<String, PropertyValue>> {
    let mut props = std::collections::HashMap::new();
    for (k, v) in dict.iter() {
        let key: String = k.extract()?;
        props.insert(key, pyany_to_property(&v)?);
    }
    Ok(props)
}
