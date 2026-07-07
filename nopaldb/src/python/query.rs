// src/python/query.rs

use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::query::nql::{NqlResult as RustNqlResult, QueryResult as RustQueryResult};
use crate::query::nql::executor::result::{ProfileResult as RustProfileResult, WriteResult as RustWriteResult};

use super::property_to_py;

/// Python wrapper for QueryResult
#[pyclass(name = "QueryResult")]
pub struct PyQueryResult {
    inner: RustQueryResult,
}

impl PyQueryResult {
    pub fn new(result: RustQueryResult) -> Self {
        PyQueryResult { inner: result }
    }
}

#[pymethods]
impl PyQueryResult {
    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __iter__(slf: PyRef<Self>) -> PyResult<PyResultIterator> {
        Ok(PyResultIterator {
            inner: Some(slf.inner.clone()),
            index: 0,
        })
    }

    fn __getitem__<'py>(&self, py: Python<'py>, index: usize) -> PyResult<Bound<'py, PyDict>> {
        if index >= self.inner.len() {
            return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                "Index out of range"
            ));
        }

        row_to_pydict(py, &self.inner, index)
    }

    fn __repr__(&self) -> String {
        format!(
            "<QueryResult: {} rows, {} columns>",
            self.inner.len(),
            self.inner.columns.len()
        )
    }

    #[getter]
    fn columns(&self) -> Vec<String> {
        self.inner.columns.clone()
    }
}

/// Python wrapper for PROFILE result
#[pyclass(name = "ProfileResult")]
pub struct PyProfileResult {
    inner: RustProfileResult,
}

impl PyProfileResult {
    pub fn new(result: RustProfileResult) -> Self {
        Self { inner: result }
    }
}

#[pymethods]
impl PyProfileResult {
    fn __repr__(&self) -> String {
        format!(
            "<ProfileResult: {} rows, {:.3} ms>",
            self.inner.rows_returned, self.inner.execution_ms
        )
    }

    #[getter]
    fn plan(&self) -> String {
        self.inner.plan.clone()
    }

    #[getter]
    fn statement_type(&self) -> String {
        self.inner.statement_type.clone()
    }

    #[getter]
    fn execution_ms(&self) -> f64 {
        self.inner.execution_ms
    }

    #[getter]
    fn rows_returned(&self) -> i64 {
        self.inner.rows_returned
    }

    #[getter]
    fn columns(&self) -> Vec<String> {
        self.inner.columns.clone()
    }

    #[getter]
    fn path_query(&self) -> bool {
        self.inner.path_query
    }

    #[getter]
    fn path_metrics<'py>(&self, py: Python<'py>) -> PyResult<Option<Py<PyAny>>> {
        match &self.inner.path_metrics {
            Some(value) => Ok(Some(property_to_py(py, value)?.unbind())),
            None => Ok(None),
        }
    }
}

/// Python wrapper for unified NQL result
#[pyclass(name = "NqlResult")]
pub struct PyNqlResult {
    inner: RustNqlResult,
}

impl PyNqlResult {
    pub fn new(result: RustNqlResult) -> Self {
        Self { inner: result }
    }
}

#[pymethods]
impl PyNqlResult {
    fn __repr__(&self) -> String {
        format!("<NqlResult: {}>", self.kind())
    }

    /// Itera las filas del resultado de lectura (como documenta el README:
    /// `for row in graph.execute_nql(...)`). Para resultados sin filas
    /// (write/index/message/export) itera vacío — usar `.write`/`.summary`.
    fn __iter__(slf: PyRef<Self>) -> PyResult<PyResultIterator> {
        let inner = match &slf.inner {
            RustNqlResult::Query(result) => Some(result.clone()),
            _ => None,
        };
        Ok(PyResultIterator { inner, index: 0 })
    }

    fn __len__(&self) -> usize {
        match &self.inner {
            RustNqlResult::Query(result) => result.len(),
            _ => 0,
        }
    }

    fn __getitem__<'py>(&self, py: Python<'py>, index: usize) -> PyResult<Bound<'py, PyDict>> {
        match &self.inner {
            RustNqlResult::Query(result) => {
                if index >= result.len() {
                    return Err(PyErr::new::<pyo3::exceptions::PyIndexError, _>(
                        "Index out of range",
                    ));
                }
                row_to_pydict(py, result, index)
            }
            _ => Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(format!(
                "NqlResult of kind '{}' has no rows — use .write / .summary",
                self.kind()
            ))),
        }
    }

    #[getter]
    fn kind(&self) -> String {
        match &self.inner {
            RustNqlResult::Query(_) => "query".to_string(),
            RustNqlResult::Write(_) => "write".to_string(),
            RustNqlResult::Index(_) => "index".to_string(),
            RustNqlResult::Explain(_) => "explain".to_string(),
            RustNqlResult::Profile(_) => "profile".to_string(),
            RustNqlResult::Export { .. } => "export".to_string(),
            RustNqlResult::Message(_) => "message".to_string(),
        }
    }

    #[getter]
    fn summary(&self) -> String {
        self.inner.summary()
    }

    #[getter]
    fn query(&self) -> Option<PyQueryResult> {
        match &self.inner {
            RustNqlResult::Query(result) => Some(PyQueryResult::new(result.clone())),
            _ => None,
        }
    }

    #[getter]
    fn write<'py>(&self, py: Python<'py>) -> PyResult<Option<Py<PyAny>>> {
        match &self.inner {
            RustNqlResult::Write(result) => Ok(Some(write_result_to_py(py, result)?.into_any().unbind())),
            _ => Ok(None),
        }
    }

    #[getter]
    fn explain(&self) -> Option<String> {
        match &self.inner {
            RustNqlResult::Explain(plan) => Some(plan.clone()),
            _ => None,
        }
    }

    #[getter]
    fn profile(&self) -> Option<PyProfileResult> {
        match &self.inner {
            RustNqlResult::Profile(result) => Some(PyProfileResult::new(result.clone())),
            _ => None,
        }
    }

    #[getter]
    fn message(&self) -> Option<String> {
        match &self.inner {
            RustNqlResult::Message(msg) => Some(msg.clone()),
            RustNqlResult::Index(msg) => Some(msg.clone()),
            RustNqlResult::Export { format, rows_exported, .. } => {
                Some(format!("Exported {} rows as {}", rows_exported, format))
            }
            _ => None,
        }
    }
}

#[pyclass]
struct PyResultIterator {
    /// None = resultado sin filas (writes, index, message…): iterador vacío.
    inner: Option<RustQueryResult>,
    index: usize,
}

#[pymethods]
impl PyResultIterator {
    fn __iter__(slf: PyRef<Self>) -> PyRef<Self> {
        slf
    }

    fn __next__<'py>(
        mut slf: PyRefMut<'py, Self>,
        py: Python<'py>
    ) -> PyResult<Option<Bound<'py, PyDict>>> {
        let Some(inner) = &slf.inner else {
            return Ok(None);
        };
        if slf.index >= inner.len() {
            return Ok(None);
        }

        let dict = row_to_pydict(py, inner, slf.index)?;
        slf.index += 1;
        Ok(Some(dict))
    }
}

fn row_to_pydict<'py>(
    py: Python<'py>,
    result: &RustQueryResult,
    index: usize,
) -> PyResult<Bound<'py, PyDict>> {
    let row = &result.rows()[index];
    let dict = PyDict::new(py);

    if result.columns.len() == 1 && result.columns[0] == "*" {
        for key in row.keys() {
            if let Some(value) = row.get(key) {
                let py_value = property_to_py(py, value)?;
                dict.set_item(key, py_value)?;
            }
        }
    } else {
        for column in &result.columns {
            if let Some(value) = row.get(column) {
                let py_value = property_to_py(py, value)?;
                dict.set_item(column, py_value)?;
            }
        }
    }

    Ok(dict)
}

fn write_result_to_py<'py>(py: Python<'py>, result: &RustWriteResult) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item("nodes_created", result.nodes_created)?;
    dict.set_item("edges_created", result.edges_created)?;
    dict.set_item("nodes_deleted", result.nodes_deleted)?;
    dict.set_item("edges_deleted", result.edges_deleted)?;
    dict.set_item("nodes_updated", result.nodes_updated)?;
    dict.set_item("edges_updated", result.edges_updated)?;
    dict.set_item("properties_changed", result.properties_changed)?;
    dict.set_item("created_ids", result.created_ids.clone())?;
    Ok(dict)
}
