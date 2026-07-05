// src/python/arrow.rs
//
// Arrow export for Python

use pyo3::prelude::*;
use pyo3::types::PyBytes;
use std::sync::Arc;  // ← AGREGAR ESTO

//#[cfg(feature = "analytics")]
//use arrow::record_batch::RecordBatch;
#[cfg(feature = "analytics")]
use super::to_py_result;

use crate::Graph as RustGraph;



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
    // Export to Arrow RecordBatch with label filter
    let batch = crate::python::runtime::block_on(py, async {
        graph.to_arrow_with_label(label).await
    });

    let batch = to_py_result(batch)?;

    // Serialize to IPC format
    let mut writer = Vec::new();
    {
        use arrow::ipc::writer::StreamWriter;
        let mut stream_writer = StreamWriter::try_new(&mut writer, &batch.schema())
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                format!("Failed to create Arrow writer: {}", e)
            ))?;

        stream_writer.write(&batch)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                format!("Failed to write Arrow batch: {}", e)
            ))?;

        stream_writer.finish()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                format!("Failed to finish Arrow stream: {}", e)
            ))?;
    }

    // Return as Python bytes
    Ok(PyBytes::new(py, &writer))
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


/// Export edges to Arrow IPC stream
#[cfg(feature = "analytics")]
pub fn export_edges_to_arrow<'py>(
    py: Python<'py>,
    graph: &Arc<RustGraph>,  // ← CAMBIO: usar RustGraph directamente
) -> PyResult<Bound<'py, PyBytes>> {
    use arrow::ipc::writer::StreamWriter;
    use std::io::Cursor;

    let graph_clone = Arc::clone(graph);

    let batch = crate::python::runtime::block_on(py, async move {
        let edges = graph_clone.get_all_edges().await
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e)))?;

        if edges.is_empty() {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "No edges to export"
            ));
        }

        crate::arrow_export::edges_to_arrow_with_properties(&edges)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e)))
    })?;

    // Write to IPC stream
    let mut buffer = Cursor::new(Vec::new());
    {
        let mut writer = StreamWriter::try_new(&mut buffer, &batch.schema())
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e)))?;

        writer.write(&batch)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e)))?;

        writer.finish()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e)))?;
    }

    Ok(PyBytes::new(py, &buffer.into_inner()))  // ← CAMBIO: usar new() no new_bound()
}

/// Export complete graph to Arrow IPC streams
#[cfg(feature = "analytics")]
pub fn export_graph_to_arrow<'py>(
    py: Python<'py>,
    graph: &Arc<RustGraph>,  // ← CAMBIO: usar RustGraph directamente
    label_filter: Option<&str>,
) -> PyResult<(Bound<'py, PyBytes>, Bound<'py, PyBytes>)> {
    use arrow::ipc::writer::StreamWriter;
    use std::io::Cursor;

    let graph_clone = Arc::clone(graph);
    let label = label_filter.map(|s| s.to_string());

    let (nodes_batch, edges_batch) = crate::python::runtime::block_on(py, async move {
        crate::arrow_export::graph_to_arrow(&graph_clone, label.as_deref()).await
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e)))
    })?;

    // Write nodes
    let mut nodes_buffer = Cursor::new(Vec::new());
    {
        let mut writer = StreamWriter::try_new(&mut nodes_buffer, &nodes_batch.schema())
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e)))?;

        writer.write(&nodes_batch)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e)))?;

        writer.finish()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e)))?;
    }

    // Write edges
    let mut edges_buffer = Cursor::new(Vec::new());
    {
        let mut writer = StreamWriter::try_new(&mut edges_buffer, &edges_batch.schema())
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e)))?;

        writer.write(&edges_batch)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e)))?;

        writer.finish()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("{}", e)))?;
    }

    Ok((
        PyBytes::new(py, &nodes_buffer.into_inner()),  // ← CAMBIO: usar new() no new_bound()
        PyBytes::new(py, &edges_buffer.into_inner())   // ← CAMBIO: usar new() no new_bound()
    ))
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
