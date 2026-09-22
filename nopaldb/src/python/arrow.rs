// src/python/arrow.rs
//
// Arrow export for Python.
//
// Everything that costs (reading the graph, building the RecordBatch,
// serializing it to the IPC stream) runs inside `runtime::block_on`, which
// releases the GIL; the only thing done with the GIL held is copying the
// finished bytes into a `PyBytes`. Until 0.6.5 the IPC serialization ran
// with the GIL held, so a large export stalled every other Python thread.

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use std::sync::Arc;

#[cfg(feature = "analytics")]
use super::to_py_result;

use crate::Graph as RustGraph;

/// Serialize one batch to an Arrow IPC stream (`pyarrow.ipc.open_stream`).
#[cfg(feature = "analytics")]
fn batch_to_ipc(batch: &arrow::record_batch::RecordBatch) -> crate::error::Result<Vec<u8>> {
    use arrow::ipc::writer::StreamWriter;
    let err = |what: &str, e: arrow::error::ArrowError| {
        crate::error::NopalError::Custom(format!("Arrow IPC {what}: {e}"))
    };
    let mut buf = Vec::new();
    let mut writer = StreamWriter::try_new(&mut buf, &batch.schema()).map_err(|e| err("writer", e))?;
    writer.write(batch).map_err(|e| err("write", e))?;
    writer.finish().map_err(|e| err("finish", e))?;
    Ok(buf)
}

/// Export graph to Apache Arrow RecordBatch
///
/// Returns raw bytes of Arrow IPC stream format
/// Can be loaded with: pyarrow.ipc.open_stream()
#[cfg(feature = "analytics")]
pub fn export_to_arrow<'py>(
    py: Python<'py>,
    graph: &RustGraph,
    label: Option<&str>,
) -> PyResult<Bound<'py, PyBytes>> {
    let bytes = to_py_result(crate::python::runtime::block_on(py, async {
        let batch = graph.to_arrow_with_label(label).await?;
        batch_to_ipc(&batch)
    }))?;
    Ok(PyBytes::new(py, &bytes))
}

/// Stub when analytics feature is disabled
#[cfg(not(feature = "analytics"))]
pub fn export_to_arrow<'py>(
    _py: Python<'py>,
    _graph: &RustGraph,
    _label: Option<&str>,
) -> PyResult<Bound<'py, PyBytes>> {
    Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
        "Arrow export requires 'analytics' feature. Rebuild with: maturin develop --features python,analytics"
    ))
}

/// Export edges to Arrow IPC stream.
///
/// A graph without edges yields an empty batch with the base columns
/// (`id`, `source`, `target`, `edge_type`), the same schema
/// `to_arrow_complete` returns for that case. Until 0.6.5 this raised
/// `ValueError("No edges to export")` while `to_arrow_complete` did not.
#[cfg(feature = "analytics")]
pub fn export_edges_to_arrow<'py>(
    py: Python<'py>,
    graph: &Arc<RustGraph>,
) -> PyResult<Bound<'py, PyBytes>> {
    let graph = Arc::clone(graph);
    let bytes = to_py_result(crate::python::runtime::block_on(py, async move {
        let edges = graph.get_all_edges().await?;
        let batch = if edges.is_empty() {
            crate::arrow_export::empty_edges_batch()
        } else {
            crate::arrow_export::edges_to_arrow_with_properties(&edges)?
        };
        batch_to_ipc(&batch)
    }))?;
    Ok(PyBytes::new(py, &bytes))
}

/// Export complete graph to Arrow IPC streams
#[cfg(feature = "analytics")]
pub fn export_graph_to_arrow<'py>(
    py: Python<'py>,
    graph: &Arc<RustGraph>,
    label_filter: Option<&str>,
) -> PyResult<(Bound<'py, PyBytes>, Bound<'py, PyBytes>)> {
    let graph = Arc::clone(graph);
    let label = label_filter.map(|s| s.to_string());
    let (nodes, edges) = to_py_result(crate::python::runtime::block_on(py, async move {
        let (nodes_batch, edges_batch) =
            crate::arrow_export::graph_to_arrow(&graph, label.as_deref()).await?;
        Ok::<_, crate::error::NopalError>((batch_to_ipc(&nodes_batch)?, batch_to_ipc(&edges_batch)?))
    }))?;
    Ok((PyBytes::new(py, &nodes), PyBytes::new(py, &edges)))
}

/// Stub when analytics feature is disabled
#[cfg(not(feature = "analytics"))]
pub fn export_edges_to_arrow<'py>(
    _py: Python<'py>,
    _graph: &Arc<RustGraph>,
) -> PyResult<Bound<'py, PyBytes>> {
    Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
        "Arrow export requires 'analytics' feature. Rebuild with: maturin develop --features python,analytics"
    ))
}

/// Stub when analytics feature is disabled
#[cfg(not(feature = "analytics"))]
pub fn export_graph_to_arrow<'py>(
    _py: Python<'py>,
    _graph: &Arc<RustGraph>,
    _label_filter: Option<&str>,
) -> PyResult<(Bound<'py, PyBytes>, Bound<'py, PyBytes>)>  {
    Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
        "Arrow export requires 'analytics' feature. Rebuild with: maturin develop --features python,analytics"
    ))
}
