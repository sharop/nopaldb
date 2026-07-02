// src/python/mod.rs

use pyo3::prelude::*;
use pyo3::{BoundObject, IntoPyObject};
use crate::error::Result as NopalResult;
use crate::types::PropertyValue;

mod graph;
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

/// Helper: Convert Result<T> to PyResult<T>
pub(crate) fn to_py_result<T>(result: NopalResult<T>) -> PyResult<T> {
    result.map_err(|e| {
        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e))
    })
}
