// src/python/graph.rs

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use crate::Graph as RustGraph;
use crate::{LinkSpec, StorageEngine, StorageOptions, StorageProfile, UpsertRequest};
use super::{PyNqlResult, PyTransaction, to_py_result};
use super::{edge_to_pydict, node_to_pydict, parse_direction, parse_uuid};
use super::PyBulkLoader;

fn parse_profile(profile: &str) -> PyResult<StorageProfile> {
    match profile.to_ascii_lowercase().as_str() {
        "default" => Ok(StorageProfile::Default),
        "mobile" => Ok(StorageProfile::Mobile),
        "server" => Ok(StorageProfile::Server),
        _ => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            format!(
                "Invalid profile '{}'. Use 'default', 'mobile', or 'server'",
                profile
            ),
        )),
    }
}

/// Traduce el nombre del engine, rechazando los que ESTE build no trae.
///
/// El engine se valida contra lo compilado, no contra lo que el enum sabe
/// nombrar: el rechazo ocurre en la llamada, con un `ValueError` que dice qué
/// backends tiene ESTE build. `"auto"` (default desde 0.6.0) elige el motor
/// del directorio si ya hay una base y, si no, el del build.
fn parse_engine(engine: &str) -> PyResult<StorageEngine> {
    let available = {
        let mut v = vec!["auto"];
        if cfg!(feature = "storage-redb") {
            v.push("redb");
        }
        if cfg!(feature = "storage-sled") {
            v.push("sled");
        }
        v.join(", ")
    };
    match engine.to_ascii_lowercase().as_str() {
        "auto" => Ok(StorageEngine::Auto),
        #[cfg(feature = "storage-redb")]
        "redb" => Ok(StorageEngine::Redb),
        #[cfg(feature = "storage-sled")]
        "sled" => Ok(StorageEngine::Sled),
        #[allow(unreachable_patterns)]
        known @ ("redb" | "sled") => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
            "engine '{known}' is not available in this build (compiled without `storage-{known}`). \
             This build supports: {available}. The wheels published on PyPI ship both engines."
        ))),
        _ => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
            "Invalid engine '{engine}'. Use one of: {available}"
        ))),
    }
}

fn engine_str(name: &str) -> &'static str {
    match name {
        "redb" => "redb",
        "sled" => "sled",
        _ => "unknown",
    }
}

/// Envuelve un callable Python como callback de progreso del motor (#158).
///
/// El motor lo invoca desde el hilo del runtime Tokio mientras el hilo
/// Python que llamó está en `block_on` con el GIL SOLTADO (`py.detach`), así
/// que aquí se vuelve a tomar con `Python::attach`. Un error del callable no
/// puede propagarse a la operación (sería abortar un replay por un `print`
/// roto): se escribe en el log y se sigue.
fn progress_callback(callable: Py<PyAny>) -> crate::ProgressCallback {
    Arc::new(move |p: crate::Progress| {
        Python::attach(|py| {
            let event = PyDict::new(py);
            let ok = event
                .set_item("phase", p.phase)
                .and_then(|_| event.set_item("done", p.done))
                .and_then(|_| event.set_item("total", p.total))
                .and_then(|_| callable.bind(py).call1((event,)).map(|_| ()));
            if let Err(e) = ok {
                log::warn!("progress callback raised and was ignored: {e}");
            }
        })
    })
}

/// `EmbeddingIndexStats` como dict; lo comparten `embedding_index_stats` y la
/// sección `hnsw` de `get_stats`.
fn hnsw_stats_dict<'py>(py: Python<'py>, st: &crate::EmbeddingIndexStats) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item("model", &st.model)?;
    dict.set_item("size", st.size)?;
    dict.set_item("tombstones", st.tombstones)?;
    dict.set_item("dimension", st.dimension)?;
    dict.set_item("needs_rebuild", st.needs_rebuild)?;
    dict.set_item("persisted", st.persisted)?;
    dict.set_item("loaded_from_disk_ms", st.loaded_from_disk_ms)?;
    Ok(dict)
}

/// Python wrapper for NopalDB Graph
#[pyclass(name = "Graph")]
pub struct PyGraph {
    // Mutex<Option<...>> permite que close() extraiga y suelte el Arc, liberando
    // el lock del motor aunque el objeto Python siga vivo.
    inner: Mutex<Option<Arc<RustGraph>>>,
}

impl PyGraph {
    /// Devuelve un clone del Arc interno, o error si el grafo ya fue cerrado.
    fn graph(&self) -> PyResult<Arc<RustGraph>> {
        self.inner
            .lock()
            .map_err(|_| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Graph mutex poisoned"
            ))?
            .as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Graph is closed"
            ))
            .map(Arc::clone)
    }
}

#[pymethods]
impl PyGraph {
    /// Open a graph database
    #[staticmethod]
    fn open(py: Python<'_>, path: &str) -> PyResult<Self> {
        let graph = crate::python::runtime::block_on(py, async {
            RustGraph::open(path).await
        });

        to_py_result(graph).map(|g| PyGraph {
            inner: Mutex::new(Some(Arc::new(g))),
        })
    }

    /// Open a graph database with runtime profile.
    ///
    /// profile: "default" | "mobile" | "server"
    #[staticmethod]
    #[pyo3(signature = (path, profile="default"))]
    fn open_with_profile(py: Python<'_>, path: &str, profile: &str) -> PyResult<Self> {
        let profile = parse_profile(profile)?;
        let graph = crate::python::runtime::block_on(py, async { RustGraph::open_with_profile(path, profile).await });
        to_py_result(graph).map(|g| PyGraph {
            inner: Mutex::new(Some(Arc::new(g))),
        })
    }

    /// Open a graph database with explicit storage options.
    ///
    /// engine: "auto" (default: the engine of the database already in
    ///     `path`, or redb for a new one), "redb" or "sled". The wheels on
    ///     PyPI ship both engines; asking for one this build does not have
    ///     raises ValueError. Use `storage_engine()` to see which one opened.
    /// profile: "default" | "mobile" | "server"
    /// on_progress: optional callable receiving a dict
    ///     `{"phase": str, "done": int, "total": int | None}` while the open
    ///     replays the WAL or rebuilds adjacency/indexes, and afterwards for
    ///     every long operation on this graph (same as
    ///     `set_progress_callback`). Called from a worker thread every ~1000
    ///     items or ~250 ms; keep it cheap.
    ///
    /// Example:
    ///     >>> g = nopaldb.Graph.open_with_options("big.db",
    ///     ...         on_progress=lambda e: print(e["phase"], e["done"], e["total"]))
    #[staticmethod]
    #[pyo3(signature = (path, engine="auto", profile="default", on_progress=None))]
    fn open_with_options(
        py: Python<'_>,
        path: &str,
        engine: &str,
        profile: &str,
        on_progress: Option<Py<PyAny>>,
    ) -> PyResult<Self> {
        let engine = parse_engine(engine)?;
        let profile = parse_profile(profile)?;
        let options = StorageOptions { engine, profile, ..Default::default() };
        let progress = on_progress.map(progress_callback);

        let graph = crate::python::runtime::block_on(py, async {
            RustGraph::open_with_progress(path, options, progress).await
        });
        to_py_result(graph).map(|g| PyGraph {
            inner: Mutex::new(Some(Arc::new(g))),
        })
    }

    /// Create in-memory graph
    #[staticmethod]
    fn in_memory(py: Python<'_>, ) -> PyResult<Self> {
        let graph = crate::python::runtime::block_on(py, async {
            RustGraph::in_memory().await
        });

        to_py_result(graph).map(|g| PyGraph {
            inner: Mutex::new(Some(Arc::new(g))),
        })
    }

    /// Create in-memory graph with runtime profile.
    ///
    /// profile: "default" | "mobile" | "server"
    #[staticmethod]
    #[pyo3(signature = (profile="default"))]
    fn in_memory_with_profile(py: Python<'_>, profile: &str) -> PyResult<Self> {
        let profile = parse_profile(profile)?;
        let graph = crate::python::runtime::block_on(py, async { RustGraph::in_memory_with_profile(profile).await });
        to_py_result(graph).map(|g| PyGraph {
            inner: Mutex::new(Some(Arc::new(g))),
        })
    }

    /// Create in-memory graph with explicit storage options.
    ///
    /// engine: same values as `open_with_options`; "auto" means the build's
    ///     default engine (redb).
    /// profile: "default" | "mobile" | "server"
    #[staticmethod]
    #[pyo3(signature = (engine="auto", profile="default"))]
    fn in_memory_with_options(py: Python<'_>, engine: &str, profile: &str) -> PyResult<Self> {
        let engine = parse_engine(engine)?;
        let profile = parse_profile(profile)?;
        let options = StorageOptions { engine, profile, ..Default::default() };

        let graph = crate::python::runtime::block_on(py, async { RustGraph::in_memory_with_options(options).await });
        to_py_result(graph).map(|g| PyGraph {
            inner: Mutex::new(Some(Arc::new(g))),
        })
    }

    // -------------------------------------------------------------------------
    // Retrieval: fetch by id and expand the neighbourhood (GraphRAG, 0.6.8)
    // -------------------------------------------------------------------------

    /// Node by id: `{"id", "label", "properties"}`, or None if it does not exist.
    ///
    /// A point read, no scan. Until 0.6.7 the only way from Python was NQL
    /// `where n.id = "…"`, which scanned the whole label (327 ms with 100k
    /// nodes). Raises ValueError for a string that is not a UUID.
    fn get_node(&self, py: Python<'_>, id: &str) -> PyResult<Option<Py<PyDict>>> {
        let graph = self.graph()?;
        let id = parse_uuid(id, "node")?;
        let nodes = to_py_result(crate::python::runtime::block_on(py, async move { graph.get_nodes(&[id]).await }))?;
        nodes.into_iter().next().flatten().map(|n| node_to_pydict(py, &n).map(|d| d.unbind())).transpose()
    }

    /// Nodes by id, in input order; None in the position of an id that does
    /// not exist. Use it to hydrate the hits of `knn_nodes`/`search_hybrid`
    /// in one call (or pass `hydrate=True` to them).
    fn get_nodes(&self, py: Python<'_>, ids: Vec<String>) -> PyResult<Vec<Option<Py<PyDict>>>> {
        let graph = self.graph()?;
        let ids = ids.iter().map(|s| parse_uuid(s, "node")).collect::<PyResult<Vec<_>>>()?;
        let nodes = to_py_result(crate::python::runtime::block_on(py, async move { graph.get_nodes(&ids).await }))?;
        nodes
            .iter()
            .map(|n| n.as_ref().map(|n| node_to_pydict(py, n).map(|d| d.unbind())).transpose())
            .collect()
    }

    /// Edge by id: `{"id", "source", "target", "type", "properties"}`, or None.
    fn get_edge(&self, py: Python<'_>, id: &str) -> PyResult<Option<Py<PyDict>>> {
        let graph = self.graph()?;
        let id = parse_uuid(id, "edge")?;
        let edges = to_py_result(crate::python::runtime::block_on(py, async move { graph.get_edges(&[id]).await }))?;
        edges.into_iter().next().flatten().map(|e| edge_to_pydict(py, &e).map(|d| d.unbind())).transpose()
    }

    /// Edges by id, in input order; None where the id does not exist.
    fn get_edges(&self, py: Python<'_>, ids: Vec<String>) -> PyResult<Vec<Option<Py<PyDict>>>> {
        let graph = self.graph()?;
        let ids = ids.iter().map(|s| parse_uuid(s, "edge")).collect::<PyResult<Vec<_>>>()?;
        let edges = to_py_result(crate::python::runtime::block_on(py, async move { graph.get_edges(&ids).await }))?;
        edges
            .iter()
            .map(|e| e.as_ref().map(|e| edge_to_pydict(py, e).map(|d| d.unbind())).transpose())
            .collect()
    }

    /// Neighbouring nodes of `id` at one hop, as node dicts.
    ///
    /// direction: "out" (default), "in" or "both". edge_types: keep only
    /// these relationship types. A neighbour reachable through several
    /// edges appears once.
    #[pyo3(signature = (id, direction="out", edge_types=None))]
    fn neighbors(&self, py: Python<'_>, id: &str, direction: &str, edge_types: Option<Vec<String>>) -> PyResult<Vec<Py<PyDict>>> {
        let graph = self.graph()?;
        let id = parse_uuid(id, "node")?;
        let opts = crate::ExpandOptions { direction: parse_direction(direction)?, edge_types, labels: None, max_nodes: usize::MAX, max_edges_per_node: None };
        let nb = to_py_result(crate::python::runtime::block_on(py, async move { graph.neighborhood(&[id], 1, &opts).await }))?;
        nb.nodes
            .iter()
            .filter(|n| nb.depth_of.get(&n.id) == Some(&1))
            .map(|n| node_to_pydict(py, n).map(|d| d.unbind()))
            .collect()
    }

    /// Number of edges incident to `id`: "out", "in" or "both" (default).
    #[pyo3(signature = (id, direction="both"))]
    fn degree(&self, py: Python<'_>, id: &str, direction: &str) -> PyResult<usize> {
        let graph = self.graph()?;
        let id = parse_uuid(id, "node")?;
        let direction = parse_direction(direction)?;
        to_py_result(crate::python::runtime::block_on(py, async move { graph.degree(id, direction).await }))
    }

    /// The neighbourhood of `ids` up to `depth` hops, in one call: the
    /// context subgraph of a GraphRAG.
    ///
    /// BFS by node: a node reachable by several paths appears once, at its
    /// minimum depth; seeds are depth 0. Edge ids come from the in-memory
    /// adjacency, edges are filtered by type BEFORE their target node is
    /// read, `labels` keeps only nodes with those labels (a filtered node is
    /// neither returned nor expanded; seeds are not filtered), `max_nodes`
    /// caps the result (then `truncated` is True) and `max_edges_per_node`
    /// caps how many edges of one node are considered (the real brake on a
    /// super-node). Runs with the GIL released.
    ///
    /// Returns:
    ///     dict: {"nodes": [node dicts, seeds first], "edges": [edge dicts
    ///     whose both endpoints are in "nodes"], "depth": {id: int},
    ///     "truncated": bool}
    ///
    /// Example:
    ///     >>> hits = graph.search_hybrid(text=q, vector=v, model="m", k=10)
    ///     >>> ctx = graph.neighborhood([h["node_id"] for h in hits], depth=1,
    ///     ...                          edge_types=["MENTIONS"], max_nodes=200)
    #[pyo3(signature = (ids, depth=1, direction="out", edge_types=None, labels=None, max_nodes=1000, max_edges_per_node=None))]
    #[allow(clippy::too_many_arguments)]
    fn neighborhood(
        &self,
        py: Python<'_>,
        ids: Vec<String>,
        depth: usize,
        direction: &str,
        edge_types: Option<Vec<String>>,
        labels: Option<Vec<String>>,
        max_nodes: usize,
        max_edges_per_node: Option<usize>,
    ) -> PyResult<Py<PyDict>> {
        let graph = self.graph()?;
        let ids = ids.iter().map(|s| parse_uuid(s, "node")).collect::<PyResult<Vec<_>>>()?;
        let opts = crate::ExpandOptions { direction: parse_direction(direction)?, edge_types, labels, max_nodes, max_edges_per_node };
        let nb = to_py_result(crate::python::runtime::block_on(py, async move { graph.neighborhood(&ids, depth, &opts).await }))?;
        let out = PyDict::new(py);
        let nodes = PyList::empty(py);
        for n in &nb.nodes {
            nodes.append(node_to_pydict(py, n)?)?;
        }
        let edges = PyList::empty(py);
        for e in &nb.edges {
            edges.append(edge_to_pydict(py, e)?)?;
        }
        let depth_map = PyDict::new(py);
        for (id, d) in &nb.depth_of {
            depth_map.set_item(id.to_string(), *d)?;
        }
        out.set_item("nodes", nodes)?;
        out.set_item("edges", edges)?;
        out.set_item("depth", depth_map)?;
        out.set_item("truncated", nb.truncated)?;
        Ok(out.into())
    }

    /// Execute any NQL statement and return a unified result.
    fn execute_nql(&self, py: Python<'_>, query: &str) -> PyResult<PyNqlResult> {
        let graph = self.graph()?;
        let query_str = query.to_string();


        let result = crate::python::runtime::block_on(py, async move { graph.execute_statement(&query_str).await });

        to_py_result(result).map(PyNqlResult::new)
    }

    /// Begin a transaction
    ///
    /// Returns:
    ///     Transaction: Active transaction
    ///
    /// Example:
    ///     >>> tx = graph.begin_transaction()
    ///     >>> node_id = tx.add_node("Person", {"name": "Alice"})
    ///     >>> tx.commit()
    /// Begin a transaction.
    ///
    /// isolation: None (default ReadCommitted) | "read_uncommitted" |
    ///            "read_committed" | "repeatable_read" | "serializable"
    ///            (requires the `full-isolation` feature)
    #[pyo3(signature = (isolation=None))]
    fn begin_transaction(&self, py: Python<'_>, isolation: Option<&str>) -> PyResult<PyTransaction> {
        let graph = self.graph()?;

        let tx = crate::python::runtime::block_on(py, async move {
            graph.begin_transaction().await
        });
        let tx = to_py_result(tx)?;

        let tx = match isolation {
            None => tx,
            Some(level) => {
                #[cfg(feature = "full-isolation")]
                {
                    use crate::transaction::IsolationLevel;
                    let level = match level.to_ascii_lowercase().as_str() {
                        "read_uncommitted" => IsolationLevel::ReadUncommitted,
                        "read_committed" => IsolationLevel::ReadCommitted,
                        "repeatable_read" => IsolationLevel::RepeatableRead,
                        "serializable" => IsolationLevel::Serializable,
                        other => {
                            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                                "Invalid isolation level '{}'. Use 'read_uncommitted' | 'read_committed' | 'repeatable_read' | 'serializable'",
                                other
                            )))
                        }
                    };
                    tx.with_isolation(level)
                }
                #[cfg(not(feature = "full-isolation"))]
                {
                    let _ = level;
                    return Err(PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                        "Isolation levels require building with the 'full-isolation' feature",
                    ));
                }
            }
        };

        Ok(PyTransaction::new(tx))
    }

    /// Number of nodes.
    ///
    /// Counts storage keys without deserializing any node: exact, and it
    /// does not depend on the schema cache. O(N) in keys but allocation-free
    /// (until 0.6.5 it materialized every node). For per-label counts use
    /// `get_stats()["graph"]["nodes_per_label"]` or `get_label_count`.
    fn node_count(&self, py: Python<'_>) -> PyResult<usize> {
        let graph = self.graph()?;
        to_py_result(crate::python::runtime::block_on(py, async move { graph.node_count().await }))
    }

    /// Number of edges; same cost and guarantees as `node_count`.
    fn edge_count(&self, py: Python<'_>) -> PyResult<usize> {
        let graph = self.graph()?;
        to_py_result(crate::python::runtime::block_on(py, async move { graph.edge_count().await }))
    }

    /// Export graph to Apache Arrow format
    ///
    /// Returns bytes in Arrow IPC stream format.
    /// Load in Python with: pyarrow.ipc.open_stream(bytes)
    ///
    /// Returns:
    ///     bytes: Arrow IPC stream
    ///
    /// Example:
    ///     >>> import pyarrow as pa
    ///     >>> arrow_bytes = graph.to_arrow()
    ///     >>> reader = pa.ipc.open_stream(arrow_bytes)
    ///     >>> batch = reader.read_next_batch()
    ///     >>> df = batch.to_pandas()
    #[pyo3(signature = (label=None))]
    fn to_arrow<'py>(
        &self,
        py: Python<'py>,
        label: Option<&str>
    ) -> PyResult<Bound<'py, PyBytes>> {
        use crate::python::arrow::export_to_arrow;
        let graph = self.graph()?;
        export_to_arrow(py, &graph, label)
    }

    /// Export edges to Arrow format
    #[pyo3(signature = ())]
    fn edges_to_arrow<'py>(
        &self,
        py: Python<'py>
    ) -> PyResult<Bound<'py, PyBytes>> {
        use crate::python::arrow::export_edges_to_arrow;
        let graph = self.graph()?;
        export_edges_to_arrow(py, &graph)
    }

    /// Export complete graph (nodes + edges) to Arrow format
    ///
    /// Returns: (nodes_bytes, edges_bytes)
    #[pyo3(signature = (label=None))]
    fn to_arrow_complete<'py>(
        &self,
        py: Python<'py>,
        label: Option<&str>
    ) -> PyResult<(Bound<'py, PyBytes>, Bound<'py, PyBytes>)> {
        use crate::python::arrow::export_graph_to_arrow;
        let graph = self.graph()?;
        export_graph_to_arrow(py, &graph, label)
    }

    /// Create a BulkLoader for high-throughput ingestion.
    ///
    /// The loader buffers up to `batch_size` nodes (and, separately, edges)
    /// and writes each buffer in one batch; `finish()` flushes the rest and
    /// makes the load durable. Use it as a context manager so `finish()`
    /// runs on exit. Bulk batches bypass the transactional path: nodes are
    /// not indexed by user indexes until `rebuild_indexes()`.
    ///
    /// Args:
    ///     batch_size (int): rows buffered before each flush (e.g. 10_000).
    ///
    /// Example:
    ///     >>> with graph.bulk_loader(10_000) as loader:
    ///     ...     a = loader.add_node("Person", {"name": "Alice"})
    ///     ...     b = loader.add_node("Person", {"name": "Bob"})
    ///     ...     loader.add_edge(a, b, "KNOWS", {"since": 2020})
    fn bulk_loader(
        &self,
        batch_size: usize
    ) -> PyResult<PyBulkLoader> {
        let graph = self.graph()?;
        let loader = graph.bulk_loader(batch_size);
        PyBulkLoader::new(loader)
    }

    /// Get all node labels in the graph
    ///
    /// Returns:
    ///     list[str]: List of unique node labels
    ///
    /// Example:
    ///     >>> labels = graph.get_labels()
    ///     >>> print(labels)
    ///     ['Person', 'Entity', 'Address']
    fn get_labels(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        let graph = self.graph()?;

        let result = crate::python::runtime::block_on(py, async move {
            graph.get_labels().await
        });

        to_py_result(result)
    }

    /// Get all edge types in the graph
    ///
    /// Returns:
    ///     list[str]: List of unique edge types
    ///
    /// Example:
    ///     >>> types = graph.get_edge_types()
    ///     >>> print(types)
    ///     ['KNOWS', 'OFFICER_OF']
    fn get_edge_types(&self, py: Python<'_>) -> PyResult<Vec<String>> {
        let graph = self.graph()?;

        let result = crate::python::runtime::block_on(py, async move {
            graph.get_edge_types().await
        });

        to_py_result(result)
    }

    /// Get complete schema information
    ///
    /// Returns:
    ///     dict: Schema information including labels, types, properties, and counts
    ///
    /// Example:
    ///     >>> schema = graph.get_schema()
    ///     >>> print(schema['node_labels'])
    ///     ['Person', 'Entity']
    ///     >>> print(schema['node_counts'])
    ///     {'Person': 100, 'Entity': 50}
    fn get_schema(&self, py: Python) -> PyResult<Py<pyo3::types::PyDict>> {
        let graph = self.graph()?;

        let schema = crate::python::runtime::block_on(py, async move {
            graph.get_schema().await
        });

        let schema = to_py_result(schema)?;

        // Convert to Python dict
        let dict = pyo3::types::PyDict::new(py);
        dict.set_item("node_labels", schema.node_labels)?;
        dict.set_item("edge_types", schema.edge_types)?;
        dict.set_item("node_counts", schema.node_counts)?;
        dict.set_item("edge_counts", schema.edge_counts)?;
        dict.set_item("total_nodes", schema.total_nodes)?;
        dict.set_item("total_edges", schema.total_edges)?;

        // Convert properties HashSets to lists
        let node_props = pyo3::types::PyDict::new(py);
        for (label, props) in schema.node_properties {
            let props_list: Vec<String> = props.into_iter().collect();
            node_props.set_item(label, props_list)?;
        }
        dict.set_item("node_properties", node_props)?;

        let edge_props = pyo3::types::PyDict::new(py);
        for (etype, props) in schema.edge_properties {
            let props_list: Vec<String> = props.into_iter().collect();
            edge_props.set_item(etype, props_list)?;
        }
        dict.set_item("edge_properties", edge_props)?;

        Ok(dict.into())
    }

    /// Get properties for a specific node label
    ///
    /// Args:
    ///     label (str): The node label
    ///
    /// Returns:
    ///     list[str]: List of property names
    ///
    /// Example:
    ///     >>> props = graph.get_label_properties("Person")
    ///     >>> print(props)
    ///     ['name', 'age', 'email']
    #[pyo3(signature = (label))]
    fn get_label_properties(&self, py: Python<'_>, label: &str) -> PyResult<Vec<String>> {
        let graph = self.graph()?;
        let label_str = label.to_string();

        let result = crate::python::runtime::block_on(py, async move {
            graph.get_label_properties(&label_str).await
        });

        to_py_result(result)
    }

    /// Get node count for a specific label
    ///
    /// Args:
    ///     label (str): The node label
    ///
    /// Returns:
    ///     int: Number of nodes with this label
    ///
    /// Example:
    ///     >>> count = graph.get_label_count("Person")
    ///     >>> print(f"Total Person nodes: {count}")
    ///     Total Person nodes: 100
    #[pyo3(signature = (label))]
    fn get_label_count(&self, py: Python<'_>, label: &str) -> PyResult<usize> {
        let graph = self.graph()?;
        let label_str = label.to_string();

        let result = crate::python::runtime::block_on(py, async move {
            graph.get_label_count(&label_str).await
        });

        to_py_result(result)
    }

    /// Get properties for a specific edge type
    ///
    /// Args:
    ///     edge_type (str): The edge type
    ///
    /// Returns:
    ///     list[str]: List of property names
    ///
    /// Example:
    ///     >>> props = graph.get_edge_type_properties("KNOWS")
    ///     >>> print(props)
    ///     ['since', 'strength']
    #[pyo3(signature = (edge_type))]
    fn get_edge_type_properties(&self, py: Python<'_>, edge_type: &str) -> PyResult<Vec<String>> {
        let graph = self.graph()?;
        let type_str = edge_type.to_string();

        let result = crate::python::runtime::block_on(py, async move {
            graph.get_edge_type_properties(&type_str).await
        });

        to_py_result(result)
    }

    /// Get edge count for a specific type
    ///
    /// Args:
    ///     edge_type (str): The edge type
    ///
    /// Returns:
    ///     int: Number of edges with this type
    ///
    /// Example:
    ///     >>> count = graph.get_edge_type_count("KNOWS")
    ///     >>> print(f"Total KNOWS edges: {count}")
    ///     Total KNOWS edges: 42
    #[pyo3(signature = (edge_type))]
    fn get_edge_type_count(&self, py: Python<'_>, edge_type: &str) -> PyResult<usize> {
        let graph = self.graph()?;
        let type_str = edge_type.to_string();

        let result = crate::python::runtime::block_on(py, async move {
            graph.get_edge_type_count(&type_str).await
        });

        to_py_result(result)
    }

    /// Rebuild schema cache
    ///
    /// Use after bulk imports or major changes.
    ///
    /// Example:
    ///     >>> graph.rebuild_schema()
    fn rebuild_schema(&self, py: Python<'_>) -> PyResult<()> {
        let graph = self.graph()?;

        let result = crate::python::runtime::block_on(py, async move {
            graph.rebuild_schema().await
        });

        to_py_result(result)
    }

    /// Create an index on a property
    ///
    /// Args:
    ///     label (str): Node label
    ///     property (str): Property name
    ///     index_type (str): 'hash', 'btree', 'fulltext' or 'taxonomy' (default: 'hash')
    ///     analyzer (dict | None): full-text only. Keys: `language` (tantivy
    ///         language name, e.g. "spanish"), `stemming`, `stopwords`,
    ///         `ascii_folding` (bools). `language` alone turns the three on.
    ///         The same analyzer is applied to every query on the index;
    ///         changing it requires `drop_index` + `create_index`.
    ///
    /// Returns:
    ///     str: Index name
    ///
    /// Example:
    ///     >>> graph.create_index("Person", "name", "hash")
    ///     'Person_name'
    ///     >>> graph.create_index("Nota", "cuerpo", "fulltext", analyzer={"language": "spanish"})
    ///     'Nota_cuerpo'
    #[pyo3(signature = (label, property, index_type="hash", analyzer=None))]
    fn create_index(
        &self,
        py: Python<'_>,
        label: String,
        property: String,
        index_type: &str,
        analyzer: Option<&Bound<'_, pyo3::types::PyDict>>,
    ) -> PyResult<String> {
        use crate::index::{FullTextAnalyzer, IndexOptions, IndexType};

        let graph = self.graph()?;
        let idx_type = match index_type {
            "hash" => IndexType::Hash,
            "btree" => IndexType::BTree,
            "fulltext" => IndexType::FullText,
            "taxonomy" => IndexType::Taxonomy,
            _ => return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                format!("Invalid index type '{}'. Use 'hash', 'btree', 'fulltext' or 'taxonomy'", index_type)
            )),
        };
        let analyzer = match analyzer {
            None => None,
            Some(dict) => {
                let language: Option<String> = match dict.get_item("language")? {
                    Some(v) if !v.is_none() => Some(v.extract::<String>()?),
                    _ => None,
                };
                let mut a = match &language {
                    Some(lang) => FullTextAnalyzer::for_language(lang),
                    None => FullTextAnalyzer::default(),
                };
                for (key, slot) in [
                    ("stemming", &mut a.stemming),
                    ("stopwords", &mut a.stopwords),
                    ("ascii_folding", &mut a.ascii_folding),
                ] {
                    if let Some(v) = dict.get_item(key)? && !v.is_none() {
                        *slot = v.extract::<bool>()?;
                    }
                }
                for key in dict.keys().iter() {
                    let k: String = key.extract()?;
                    if !matches!(k.as_str(), "language" | "stemming" | "stopwords" | "ascii_folding") {
                        return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(format!(
                            "analyzer: unknown key '{k}'; valid keys are language, stemming, stopwords, ascii_folding"
                        )));
                    }
                }
                Some(a)
            }
        };
        let options = IndexOptions { analyzer };

        let result = crate::python::runtime::block_on(py, async move {
            graph.create_index_with(&label, &property, idx_type, options).await
        });

        to_py_result(result)
    }

    /// Describe one index: metadata plus, for a full-text index, its analyzer.
    ///
    /// Returns:
    ///     dict | None: {name, label, property, type, analyzer} where `analyzer`
    ///     is {language, stemming, stopwords, ascii_folding} for a full-text
    ///     index and None otherwise. None if no index has that name.
    #[pyo3(signature = (index_name))]
    fn describe_index(&self, py: Python<'_>, index_name: String) -> PyResult<Option<Py<pyo3::types::PyDict>>> {
        let graph = self.graph()?;
        let info = crate::python::runtime::block_on(py, async move { graph.describe_index(&index_name).await });
        let Some(info) = info else { return Ok(None) };
        let dict = pyo3::types::PyDict::new(py);
        dict.set_item("name", info.metadata.name)?;
        dict.set_item("label", info.metadata.label)?;
        dict.set_item("property", info.metadata.property)?;
        dict.set_item("type", format!("{:?}", info.metadata.index_type))?;
        match info.analyzer {
            Some(a) => {
                let an = pyo3::types::PyDict::new(py);
                an.set_item("language", a.language)?;
                an.set_item("stemming", a.stemming)?;
                an.set_item("stopwords", a.stopwords)?;
                an.set_item("ascii_folding", a.ascii_folding)?;
                dict.set_item("analyzer", an)?;
            }
            None => dict.set_item("analyzer", py.None())?,
        }
        Ok(Some(dict.into()))
    }

    /// Drop an index
    ///
    /// Args:
    ///     index_name (str): Name of the index to drop
    ///
    /// Example:
    ///     >>> graph.drop_index("Person_name")
    #[pyo3(signature = (index_name))]
    fn drop_index(&self, py: Python<'_>, index_name: String) -> PyResult<()> {
        let graph = self.graph()?;

        let result = crate::python::runtime::block_on(py, async move {
            graph.drop_index(&index_name).await
        });

        to_py_result(result)
    }

    /// List all indexes
    ///
    /// Returns:
    ///     list[tuple]: List of (name, label, property, type) tuples
    ///
    /// Example:
    ///     indexes = graph.list_indexes()
    ///     for name, label, prop, type in indexes:
    ///     ...     print(f"{name}: {label}.{prop} [{type}]")
    ///     Person_name: Person.name [Hash]
    ///     Person_age: Person.age [BTree]
    fn list_indexes(&self, py: Python<'_>) -> PyResult<Vec<(String, String, String, String)>> {
        let graph = self.graph()?;

        let indexes = crate::python::runtime::block_on(py, async move {
            graph.list_indexes().await
        });

        Ok(indexes.into_iter().map(|meta| {
            let type_str = match meta.index_type {
                crate::index::IndexType::Hash => "Hash",
                crate::index::IndexType::BTree => "BTree",
                crate::index::IndexType::FullText => "FullText",
                crate::index::IndexType::Taxonomy => "Taxonomy",
            };
            (
                meta.name,
                meta.label,
                meta.property,
                type_str.to_string(),
            )
        }).collect())
    }

    /// Operational state of the database in one call (#158): what to look at
    /// when something is slow or seems stuck. See docs/OPERATIONS.md.
    ///
    /// Returns a nested dict with native types:
    ///     graph:    total_nodes, total_edges, avg_degree, nodes_per_label,
    ///               edges_per_type
    ///     storage:  engine, profile, data_dir (None in memory), read_only
    ///     wal:      bytes, checkpoint_threshold_bytes, checkpoints_this_session,
    ///               last_checkpoint_unix_ms (None if none yet),
    ///               direct_write_durability
    ///     recovery: what the open that created this handle did —
    ///               wal_records_read, operations_replayed,
    ///               uncommitted_txs_discarded, crash_recovery,
    ///               adjacency_rebuilt, open_ms {storage, wal_replay,
    ///               adjacency, indexes, total}
    ///     indexes:  [{name, label, property, type, size, analyzer}] — the
    ///               analyzer is a string ("default", "spanish+stemming+…")
    ///               for full-text indexes, None otherwise
    ///     hnsw:     [{model, size, tombstones, dimension, needs_rebuild,
    ///               persisted, loaded_from_disk_ms}] per model whose index
    ///               is in cache (built or loaded on the first search)
    ///     gc:       auto_running, auto (None or {interval_secs,
    ///               cutoff_timestamp, min_versions_to_keep,
    ///               max_nodes_per_cycle, dry_run, use_active_horizon}),
    ///               last_run (None or {unix_ms, nodes_scanned,
    ///               versions_removed, bytes_freed, duration_ms, dry_run})
    ///
    /// Deprecated flat keys, kept at the top level for one more minor so
    /// 0.6.x readers keep working — all of them strings, as before:
    /// "total_nodes", "total_edges", "avg_degree", "storage_engine",
    /// "wal_bytes". Read the sections instead.
    ///
    /// Cheap: nothing is scanned; it reads what open/checkpoint/gc already
    /// recorded plus the in-memory index catalog.
    ///
    /// Example:
    ///     >>> s = graph.get_stats()
    ///     >>> s["recovery"]["operations_replayed"], s["recovery"]["open_ms"]["total"]
    ///     (0, 12)
    ///     >>> [(ix["name"], ix["size"], ix["analyzer"]) for ix in s["indexes"]]
    ///     [('Doc_texto', 1200, 'spanish+stemming+stopwords+ascii_folding')]
    fn get_stats(&self, py: Python<'_>) -> PyResult<Py<PyDict>> {
        let graph = self.graph()?;
        let report = to_py_result(crate::python::runtime::block_on(py, async move { graph.stats().await }))?;

        let out = PyDict::new(py);

        let g = PyDict::new(py);
        g.set_item("total_nodes", report.graph.total_nodes)?;
        g.set_item("total_edges", report.graph.total_edges)?;
        g.set_item("avg_degree", report.graph.avg_degree)?;
        g.set_item("nodes_per_label", &report.graph.nodes_per_label)?;
        g.set_item("edges_per_type", &report.graph.edges_per_type)?;
        out.set_item("graph", g)?;

        let s = PyDict::new(py);
        s.set_item("engine", report.storage.engine)?;
        s.set_item("profile", report.storage.profile)?;
        s.set_item("data_dir", report.storage.data_dir.as_ref().map(|p| p.display().to_string()))?;
        s.set_item("read_only", report.storage.read_only)?;
        out.set_item("storage", s)?;

        let w = PyDict::new(py);
        w.set_item("bytes", report.wal.bytes)?;
        w.set_item("checkpoint_threshold_bytes", report.wal.checkpoint_threshold_bytes)?;
        w.set_item("checkpoints_this_session", report.wal.checkpoints_this_session)?;
        w.set_item("last_checkpoint_unix_ms", report.wal.last_checkpoint_unix_ms)?;
        w.set_item("direct_write_durability", report.wal.direct_write_durability)?;
        out.set_item("wal", w)?;

        let r = PyDict::new(py);
        r.set_item("wal_records_read", report.recovery.wal_records_read)?;
        r.set_item("operations_replayed", report.recovery.operations_replayed)?;
        r.set_item("uncommitted_txs_discarded", report.recovery.uncommitted_txs_discarded)?;
        r.set_item("crash_recovery", report.recovery.crash_recovery)?;
        r.set_item("adjacency_rebuilt", report.recovery.adjacency_rebuilt)?;
        let ms = PyDict::new(py);
        ms.set_item("storage", report.recovery.open_ms.storage)?;
        ms.set_item("wal_replay", report.recovery.open_ms.wal_replay)?;
        ms.set_item("adjacency", report.recovery.open_ms.adjacency)?;
        ms.set_item("indexes", report.recovery.open_ms.indexes)?;
        ms.set_item("total", report.recovery.open_ms.total)?;
        r.set_item("open_ms", ms)?;
        out.set_item("recovery", r)?;

        let indexes = PyList::empty(py);
        for ix in &report.indexes {
            let d = PyDict::new(py);
            d.set_item("name", &ix.name)?;
            d.set_item("label", &ix.label)?;
            d.set_item("property", &ix.property)?;
            d.set_item("type", &ix.kind)?;
            d.set_item("size", ix.size)?;
            d.set_item("analyzer", ix.analyzer.as_deref())?;
            indexes.append(d)?;
        }
        out.set_item("indexes", indexes)?;

        let hnsw = PyList::empty(py);
        for st in &report.hnsw {
            hnsw.append(hnsw_stats_dict(py, st)?)?;
        }
        out.set_item("hnsw", hnsw)?;

        let gc = PyDict::new(py);
        gc.set_item("auto_running", report.gc.auto_running)?;
        match &report.gc.auto {
            Some(a) => {
                let d = PyDict::new(py);
                d.set_item("interval_secs", a.interval_secs)?;
                d.set_item("cutoff_timestamp", a.cutoff_timestamp)?;
                d.set_item("min_versions_to_keep", a.min_versions_to_keep)?;
                d.set_item("max_nodes_per_cycle", a.max_nodes_per_cycle)?;
                d.set_item("dry_run", a.dry_run)?;
                d.set_item("use_active_horizon", a.use_active_horizon)?;
                gc.set_item("auto", d)?;
            }
            None => gc.set_item("auto", py.None())?,
        }
        match &report.gc.last_run {
            Some(run) => {
                let d = PyDict::new(py);
                d.set_item("unix_ms", run.unix_ms)?;
                d.set_item("nodes_scanned", run.nodes_scanned)?;
                d.set_item("versions_removed", run.versions_removed)?;
                d.set_item("bytes_freed", run.bytes_freed)?;
                d.set_item("duration_ms", run.duration_ms)?;
                d.set_item("dry_run", run.dry_run)?;
                gc.set_item("last_run", d)?;
            }
            None => gc.set_item("last_run", py.None())?,
        }
        out.set_item("gc", gc)?;

        // Claves planas de 0.6.x (strings, como siempre fueron). Un minor más.
        out.set_item("total_nodes", report.graph.total_nodes.to_string())?;
        out.set_item("total_edges", report.graph.total_edges.to_string())?;
        out.set_item("avg_degree", format!("{:.2}", report.graph.avg_degree))?;
        out.set_item("storage_engine", report.storage.engine)?;
        out.set_item("wal_bytes", report.wal.bytes.to_string())?;

        Ok(out.into())
    }

    /// Register (or replace) the progress callback for long operations:
    /// `create_index`, `upsert_many`, `BulkLoader`, and the WAL replay and
    /// rebuilds of an open made with `open_with_options(on_progress=...)`.
    /// The callable receives `{"phase": str, "done": int, "total": int | None}`
    /// every ~1000 items or ~250 ms, from a worker thread: keep it cheap and
    /// do not call back into the graph from it. Pass `None` to remove it.
    /// Without a callback the cost is one comparison per batch. The existing
    /// log lines stay; this complements them for whoever does not read logs.
    ///
    /// Example:
    ///     >>> graph.set_progress_callback(lambda e: print(e))
    ///     >>> graph.upsert_many(rows)          # prints upsert_batch 0/5000, 1000/5000, …
    ///     >>> graph.set_progress_callback(None)
    #[pyo3(signature = (callback))]
    fn set_progress_callback(&self, callback: Option<Py<PyAny>>) -> PyResult<()> {
        let graph = self.graph()?;
        match callback {
            Some(cb) => graph.set_progress_callback(progress_callback(cb)),
            None => graph.clear_progress_callback(),
        }
        Ok(())
    }

    /// Name of the storage engine this handle opened: "redb" or "sled".
    ///
    /// With `engine="auto"` (the default) an existing database keeps the
    /// engine it was created with, so a 0.5.x database reports "sled" until
    /// it is migrated with `Graph.migrate`.
    fn storage_engine(&self) -> PyResult<&'static str> {
        let graph = self.graph()?;
        Ok(engine_str(graph.storage().backend_name()))
    }

    /// Copy a whole database directory to another engine, verifying the copy.
    ///
    /// Byte-for-byte copy of every keyspace (nodes, MVCC versions, edges,
    /// adjacency, indexes, clocks, embeddings) followed by a re-scan of the
    /// destination that checks counts and checksums. Time-travel and indexes
    /// survive intact because nothing is reinterpreted.
    ///
    /// Preconditions: both directories closed (no open Graph on them); the
    /// source opened and closed cleanly at least once with NopalDB (so its
    /// WAL is applied); the destination directory empty or absent. Both
    /// engines must be present in the build (the PyPI wheels have both).
    ///
    /// Args:
    ///     src (str): source directory.
    ///     dst (str): destination directory (empty or absent).
    ///     src_engine (str): "auto" (detect from the directory), "sled" or "redb".
    ///     dst_engine (str): "auto" (the build's default, redb), "sled" or "redb".
    ///     profile (str): tuning profile used to open both.
    ///
    /// Returns:
    ///     dict: {"keyspaces": [{"name", "pairs", "bytes"}, ...], "verified": bool}.
    ///     A failed verification is raised as an error and the destination
    ///     must not be used.
    ///
    /// Example:
    ///     >>> report = nopaldb.Graph.migrate("old_sled.db", "new_redb.db")
    ///     >>> report["verified"]
    ///     True
    ///     >>> [ix["name"] for ix in report["indexes"]]   # user indexes that travelled
    ///     ['idx_Doc_texto']
    ///     >>> report["hnsw_copied"]                       # False = rebuilt on first search
    ///     True
    #[staticmethod]
    #[pyo3(signature = (src, dst, src_engine="auto", dst_engine="auto", profile="default"))]
    fn migrate(
        py: Python<'_>,
        src: &str,
        dst: &str,
        src_engine: &str,
        dst_engine: &str,
        profile: &str,
    ) -> PyResult<Py<PyDict>> {
        let profile = parse_profile(profile)?;
        let src_opts = StorageOptions { engine: parse_engine(src_engine)?, profile, ..Default::default() };
        let dst_opts = StorageOptions { engine: parse_engine(dst_engine)?, profile, ..Default::default() };
        let (src, dst) = (src.to_string(), dst.to_string());
        let report = crate::python::runtime::block_on(py, async move {
            crate::Storage::copy_database(&src, src_opts, &dst, dst_opts).await
        });
        let report = to_py_result(report)?;
        let dict = PyDict::new(py);
        let keyspaces = PyList::empty(py);
        for (name, pairs, bytes) in &report.keyspaces {
            let ks = PyDict::new(py);
            ks.set_item("name", name)?;
            ks.set_item("pairs", *pairs)?;
            ks.set_item("bytes", *bytes)?;
            keyspaces.append(ks)?;
        }
        dict.set_item("keyspaces", keyspaces)?;
        dict.set_item("verified", report.verified)?;
        let sidecars = PyList::empty(py);
        for sc in &report.sidecars {
            let d = PyDict::new(py);
            d.set_item("dir", &sc.dir)?;
            d.set_item("files", sc.files)?;
            d.set_item("bytes", sc.bytes)?;
            sidecars.append(d)?;
        }
        dict.set_item("sidecars", sidecars)?;
        let indexes = PyList::empty(py);
        for ix in &report.indexes {
            let d = PyDict::new(py);
            d.set_item("name", &ix.name)?;
            d.set_item("label", &ix.label)?;
            d.set_item("property", &ix.property)?;
            d.set_item("type", &ix.kind)?;
            d.set_item("analyzer", ix.analyzer.as_deref())?;
            indexes.append(d)?;
        }
        dict.set_item("indexes", indexes)?;
        dict.set_item("hnsw_copied", report.hnsw_copied)?;
        Ok(dict.into())
    }

    /// Rebuild the user indexes from `<dir>/indexes/metadata.bin` and the
    /// current nodes, as `Graph.open` does. Returns how many indexes are
    /// loaded. For databases copied by hand (not with `Graph.migrate`, which
    /// already carries the catalog): put `indexes/` in place, then call this.
    fn rebuild_indexes(&self, py: Python<'_>) -> PyResult<usize> {
        let graph = self.graph()?;
        to_py_result(crate::python::runtime::block_on(py, async move {
            graph.rebuild_indexes().await
        }))
    }

    /// Checkpoint: make everything applied so far durable in the storage
    /// engine and truncate the WAL, so the next `open` replays nothing.
    ///
    /// Runs on its own when the WAL grows past 16 MiB and on `close()`; call
    /// it after a big load if you want the reopen to be instant. Every
    /// acknowledged `upsert`, `add_node`, commit or bulk load is recoverable
    /// with or without it; what it changes is how long the next open takes
    /// (and, for direct writes in the default mode, it is also an fsync).
    /// Raises on a read-only graph.
    ///
    /// Example:
    ///     >>> graph.upsert_many(rows)
    ///     >>> graph.checkpoint()
    ///     >>> graph.get_stats()["wal_bytes"]   # back to ~'100'
    fn checkpoint(&self, py: Python<'_>) -> PyResult<()> {
        let graph = self.graph()?;
        to_py_result(crate::python::runtime::block_on(py, async move { graph.checkpoint().await }))
    }

    /// Close the database
    ///
    /// Extrae el Arc interno y lo suelta, permitiendo que Sled libere su lock
    /// aunque el objeto Python todavía esté vivo. Llamadas posteriores a cualquier
    /// método devolverán RuntimeError("Graph is closed").
    ///
    /// Example:
    ///     >>> graph = nopaldb.Graph.open("my.db")
    ///     >>> # ... use graph ...
    ///     >>> graph.close()
    fn close(&self, py: Python<'_>) -> PyResult<()> {
        let arc = self.inner
            .lock()
            .map_err(|_| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(
                "Graph mutex poisoned"
            ))?
            .take(); // extrae el Arc<RustGraph>, deja None en su lugar

        if let Some(graph) = arc {
            // graph se mueve al bloque — cuando termine, el Arc se suelta aquí.
            // Si este era el último clone, Sled libera el lock en este punto.
            let result = crate::python::runtime::block_on(py, async move { graph.close().await });
            to_py_result(result)
        } else {
            Ok(()) // ya cerrado — idempotente
        }
    }

    /// Context manager: __enter__
    fn __enter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    /// Context manager: __exit__
    fn __exit__<'py>(
        &self,
        _py: Python<'py>,
        _exc_type: Option<&Bound<'py, PyAny>>,
        _exc_value: Option<&Bound<'py, PyAny>>,
        _traceback: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<bool> {
        self.close(_py)?;
        Ok(false)
    }



    // -------------------------------------------------------------------------
    // Embeddings API
    // -------------------------------------------------------------------------

    /// Agrega un embedding vectorial a un nodo existente.
    ///
    /// Args:
    ///     node_id (str): UUID del nodo.
    ///     vector (list[float]): Vector de embedding (debe ser consistente en dimensión por modelo).
    ///     model (str): Nombre del modelo, e.g. "minilm", "openai-ada-002".
    ///
    /// Example:
    ///     >>> graph.add_node_embedding(node_id, [0.1, 0.2, 0.3], "minilm")
    #[cfg(feature = "embeddings")]
    fn add_node_embedding(&self, py: Python<'_>, node_id: &str, vector: Vec<f32>, model: &str) -> PyResult<()> {
        let graph = self.graph()?;
        let model = model.to_string();
        let node_id: uuid::Uuid = node_id.parse().map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Invalid node_id UUID: {}", e))
        })?;

        to_py_result(crate::python::runtime::block_on(py, async move {
            graph.add_node_embedding(node_id, vector, &model).await
        }))
    }

    /// Agrega un embedding vectorial a una arista existente.
    ///
    /// Args:
    ///     edge_id (str): UUID de la arista (devuelto por `tx.add_edge()`).
    ///     vector (list[float]): Vector de embedding.
    ///     model (str): Nombre del modelo, e.g. "relbert", "openai-ada-002".
    ///
    /// Example:
    ///     >>> graph.add_edge_embedding(edge_id, [0.1, 0.2, 0.3], "relbert")
    #[cfg(feature = "embeddings")]
    fn add_edge_embedding(&self, py: Python<'_>, edge_id: &str, vector: Vec<f32>, model: &str) -> PyResult<()> {
        let graph = self.graph()?;
        let model = model.to_string();
        let edge_id: uuid::Uuid = edge_id.parse().map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Invalid edge_id UUID: {}", e))
        })?;

        to_py_result(crate::python::runtime::block_on(py, async move {
            graph.add_edge_embedding(edge_id, vector, &model).await
        }))
    }

    /// Persiste un embedding de referencia de path para usar con
    /// `path_embedding_similarity`, `path_knn_references` y `path_anomaly_score`.
    ///
    /// El vector debe tener dimensión = dim(node_model) * 2 + dim(edge_model) * 2
    /// (concatenación media-nodos || media-aristas, formato E-7).
    ///
    /// Args:
    ///     name (str): Identificador único de la referencia (e.g. "baseline_normal").
    ///     node_model (str): Nombre del modelo de nodos.
    ///     edge_model (str): Nombre del modelo de aristas.
    ///     vector (list[float]): Vector de referencia.
    ///
    /// Example:
    ///     >>> graph.add_path_reference_embedding("normal_tx", "minilm", "relbert", ref_vec)
    #[cfg(feature = "embeddings")]
    fn add_path_reference_embedding(
        &self,
        py: Python<'_>,
        name: &str,
        node_model: &str,
        edge_model: &str,
        vector: Vec<f32>,
    ) -> PyResult<()> {
        let graph = self.graph()?;
        let name = name.to_string();
        let node_model = node_model.to_string();
        let edge_model = edge_model.to_string();

        to_py_result(crate::python::runtime::block_on(py, async move {
            graph.add_path_reference_embedding(name, node_model, edge_model, vector).await
        }))
    }

    /// Recupera el vector de embedding de un nodo.
    ///
    /// Args:
    ///     node_id (str): UUID del nodo.
    ///     model (str): Nombre del modelo usado al insertar.
    ///
    /// Returns:
    ///     list[float]: Vector de embedding.
    ///
    /// Example:
    ///     >>> vec = graph.get_node_embedding(node_id, "minilm")
    #[cfg(feature = "embeddings")]
    fn get_node_embedding(&self, py: Python<'_>, node_id: &str, model: &str) -> PyResult<Vec<f32>> {
        let graph = self.graph()?;
        let model = model.to_string();
        let node_id: uuid::Uuid = node_id.parse().map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyValueError, _>(format!("Invalid node_id UUID: {}", e))
        })?;

        to_py_result(crate::python::runtime::block_on(py, async move {
            graph.get_node_embedding(node_id, &model).await
        })).map(|emb| emb.vector)
    }

    /// Find the k nearest nodes in embedding space.
    ///
    /// Uses the HNSW index the graph caches per model: built (or loaded
    /// from disk) on the first search and updated in place by later
    /// `add_node_embedding` calls; see `embedding_index_stats(model)`. The
    /// search itself runs with the GIL released.
    ///
    /// Args:
    ///     query_vector (list[float]): Query vector.
    ///     k (int): Number of neighbours to return.
    ///     model (str): Model name.
    ///     ef_search (int, optional): HNSW candidate list size (higher =
    ///         better recall, slower). Default: 30. Only matters above the
    ///         exact-search threshold (1024 points); below it the search is
    ///         exact and the parameter is irrelevant.
    ///
    ///     hydrate (bool): if True, return `[{"node_id", "distance", "node"}]`
    ///         with each node read in the same call (`"node"` is None if the
    ///         node vanished); if False (default) return `(node_id, distance)`
    ///         tuples, as before.
    ///
    /// Returns:
    ///     list[tuple[str, float]]: (node_id, cosine distance) pairs, nearest first
    ///     (or list[dict] with hydrate=True).
    ///
    /// Example:
    ///     >>> results = graph.knn_nodes([0.1, 0.2, 0.3], k=5, model="minilm")
    ///     >>> for node_id, dist in results:
    ///     ...     print(node_id, dist)
    ///     >>> for hit in graph.knn_nodes(q, k=5, model="minilm", hydrate=True):
    ///     ...     print(hit["distance"], hit["node"]["properties"]["text"])
    #[cfg(feature = "embeddings-index")]
    #[pyo3(signature = (query_vector, k, model, ef_search=None, hydrate=false))]
    fn knn_nodes(&self, py: Python<'_>, query_vector: Vec<f32>, k: usize, model: &str, ef_search: Option<usize>, hydrate: bool) -> PyResult<Py<PyAny>> {
        let graph = self.graph()?;
        let graph_for_hydrate = graph.clone();
        let model = model.to_string();

        // Índice cacheado del Graph (antes se reconstruía COMPLETO en cada
        // llamada: 90 s por consulta con 100k vectores).
        let idx = to_py_result(crate::python::runtime::block_on(py, async move {
            graph.get_or_build_embedding_index(&model).await
        }))?;

        let ef = ef_search.unwrap_or(crate::embeddings::DEFAULT_EF_SEARCH);
        // La búsqueda es la parte cara: fuera del GIL (hasta 0.6.5 corría
        // con él tomado y paraba a los demás hilos Python). El guard del
        // RwLock nace y muere dentro del closure.
        let hits = to_py_result(py.detach(move || {
            let guard = idx.read().unwrap_or_else(|e| e.into_inner());
            guard.search_knn_with_ef(&query_vector, k, ef)
        }))?;
        if !hydrate {
            let tuples: Vec<(String, f32)> = hits.into_iter().map(|(id, dist)| (id.to_string(), dist)).collect();
            return Ok(tuples.into_pyobject(py)?.into_any().unbind());
        }
        let ids: Vec<crate::NodeId> = hits.iter().map(|(id, _)| *id).collect();
        let nodes = to_py_result(crate::python::runtime::block_on(py, async move { graph_for_hydrate.get_nodes(&ids).await }))?;
        let out = PyList::empty(py);
        for ((id, dist), node) in hits.into_iter().zip(nodes) {
            let d = PyDict::new(py);
            d.set_item("node_id", id.to_string())?;
            d.set_item("distance", dist)?;
            d.set_item("node", node.as_ref().map(|n| node_to_pydict(py, n)).transpose()?)?;
            out.append(d)?;
        }
        Ok(out.into_any().unbind())
    }

    /// State of the cached HNSW index for `model`.
    ///
    /// Returns:
    ///     dict | None: {model, size, tombstones, dimension, needs_rebuild,
    ///     persisted, loaded_from_disk_ms} (the same entry `get_stats()["hnsw"]`
    ///     lists), or None if the index has not been built yet (it is built
    ///     or loaded on the first search).
    #[cfg(feature = "embeddings-index")]
    fn embedding_index_stats(&self, py: Python<'_>, model: &str) -> PyResult<Option<Py<pyo3::types::PyDict>>> {
        let graph = self.graph()?;
        let model = model.to_string();
        let stats = crate::python::runtime::block_on(py, async move { graph.embedding_index_stats(&model).await });
        let Some(st) = stats else { return Ok(None) };
        Ok(Some(hnsw_stats_dict(py, &st)?.into()))
    }

    /// Importa una fuente Turtle (OWL/RDF) en el grafo.
    ///
    /// Registra clases (`owl:Class`), jerarquía de subclases (`rdfs:subClassOf`) e individuos
    /// (`rdf:type SomeClass`) en el grafo persistido y actualiza el TaxonomyIndex, habilitando
    /// las predicados NQL `instanceOf` y `subClassOf` con inferencia transitiva.
    ///
    /// Args:
    ///     ttl_source (str): Contenido Turtle como string.
    ///
    /// Returns:
    ///     dict: {classes_added, subclass_edges_added, instances_added, edges_created,
    ///           placeholders_created, triples_skipped, warnings}.
    ///     `triples_skipped` cuenta solo los triples que no dejaron nada en el grafo
    ///     (metadatos de clases, tipos desconocidos, axiomas no modelados); las data
    ///     properties de individuos se importan y no cuentan.
    ///
    /// Requires:
    ///     Wheel compilado con `--features python-owl` (incluido en el tier `semantic`).
    #[cfg(feature = "python-owl")]
    fn import_turtle(&self, py: Python<'_>, ttl_source: &str) -> PyResult<Py<pyo3::types::PyDict>> {
        let graph = self.graph()?;
        let source = ttl_source.to_string();
        let report = to_py_result(
            crate::python::runtime::block_on(py, async move { graph.import_turtle(&source).await })
        )?;
        let dict = pyo3::types::PyDict::new(py);
        dict.set_item("classes_added",        report.classes_added)?;
        dict.set_item("subclass_edges_added", report.subclass_edges_added)?;
        dict.set_item("instances_added",      report.instances_added)?;
        dict.set_item("triples_skipped",      report.triples_skipped)?;
        dict.set_item("edges_created",        report.edges_created)?;
        dict.set_item("placeholders_created", report.placeholders_created)?;
        dict.set_item("warnings",             report.warnings.clone())?;
        Ok(dict.into())
    }

    /// Exporta el contenido RDF del grafo como Turtle.
    ///
    /// Returns:
    ///     tuple[str, dict]: el documento Turtle y el reporte
    ///     {classes, subclass_edges, individuals, type_triples, edges, literals,
    ///     triples_written, skipped}. `skipped` es la lista de valores y aristas
    ///     sin representación en Turtle (Bytes, Object, NaN, propiedades de
    ///     arista, aristas hacia nodos sin `iri`), una línea por cada uno con la
    ///     razón; vacía significa que el export es fiel.
    ///
    /// Requires:
    ///     Wheel compilado con `--features python-owl` (incluido en el tier `semantic`).
    #[cfg(feature = "python-owl")]
    fn export_turtle(&self, py: Python<'_>) -> PyResult<(String, Py<pyo3::types::PyDict>)> {
        let graph = self.graph()?;
        let export = to_py_result(
            crate::python::runtime::block_on(py, async move { graph.export_turtle().await })
        )?;
        Ok((export.turtle, export_report_dict(py, &export.report)?))
    }

    /// Exporta el contenido RDF del grafo a un archivo Turtle (.ttl).
    ///
    /// Returns:
    ///     dict: el mismo reporte que `export_turtle`.
    #[cfg(feature = "python-owl")]
    fn export_owl_file(&self, py: Python<'_>, path: &str) -> PyResult<Py<pyo3::types::PyDict>> {
        let graph = self.graph()?;
        let path = path.to_string();
        let report = to_py_result(
            crate::python::runtime::block_on(py, async move { graph.export_owl_file(&path).await })
        )?;
        export_report_dict(py, &report)
    }

    /// Valida el grafo contra shapes SHACL escritas en Turtle.
    ///
    /// Returns:
    ///     dict: {conforms, violations, notes, shapes, property_shapes,
    ///     constraints, ignored, warnings}. Cada violación es
    ///     {focus_node, shape, constraint, path, value, message, severity}.
    ///     `ignored` lista, con razón, cada término `sh:*` que este validador
    ///     no comprueba; vacío significa que las shapes se aplican completas.
    ///     Un Turtle malformado levanta con línea y columna. El grafo no se
    ///     modifica.
    ///
    /// Requires:
    ///     Wheel compilado con `--features python-shacl` (incluido en `python-full`).
    #[cfg(feature = "python-shacl")]
    fn validate_shapes(&self, py: Python<'_>, shapes_turtle: &str) -> PyResult<Py<pyo3::types::PyDict>> {
        let graph = self.graph()?;
        let source = shapes_turtle.to_string();
        let (report, shapes) = to_py_result(
            crate::python::runtime::block_on(py, async move { graph.validate_shapes(&source).await })
        )?;
        let dict = pyo3::types::PyDict::new(py);
        dict.set_item("conforms", report.conforms)?;
        dict.set_item("violations", violations_to_py(py, &report.violations)?)?;
        dict.set_item("notes", report.notes.clone())?;
        dict.set_item("shapes", shapes.shapes)?;
        dict.set_item("property_shapes", shapes.property_shapes)?;
        dict.set_item("constraints", shapes.constraints)?;
        dict.set_item("ignored", shapes.ignored.clone())?;
        dict.set_item("warnings", shapes.warnings.clone())?;
        Ok(dict.into())
    }

    /// Prefijos declarados por los documentos Turtle importados en este grafo
    /// (`{prefijo: namespace}`), fusionados entre imports. Vacío si nunca se
    /// importó Turtle.
    #[cfg(feature = "python-owl")]
    fn rdf_prefixes(&self, py: Python<'_>) -> PyResult<Py<pyo3::types::PyDict>> {
        let graph = self.graph()?;
        let map = to_py_result(
            crate::python::runtime::block_on(py, async move { graph.rdf_prefixes().await })
        )?;
        let dict = pyo3::types::PyDict::new(py);
        for (k, v) in map {
            dict.set_item(k, v)?;
        }
        Ok(dict.into())
    }

    /// String representation
    fn __repr__(&self) -> String {
        let closed = self.inner.lock()
            .map(|g| g.is_none())
            .unwrap_or(true);
        if closed {
            "<NopalDB Graph (closed)>".to_string()
        } else {
            "<NopalDB Graph>".to_string()
        }
    }

    /// Idempotently write the desired state of a node keyed by `(label, key)`.
    ///
    /// Re-running the same upsert over unchanged data performs no writes.
    ///
    /// Args:
    ///     label (str): node label.
    ///     key (str): name of the identity property (must be present in `props`).
    ///     props (dict): full desired property map (includes the key property).
    ///     vector (list[float], optional): embedding vector (requires `model`).
    ///     model (str, optional): embedding model name (requires `vector`).
    ///     links (list[dict], optional): outgoing edges to reconcile. Each dict:
    ///         {"type": str, "target_label": str, "target_key": str,
    ///          "target_key_value": Any, "props": dict?, "stub": bool?}.
    ///
    /// Returns:
    ///     tuple[str, str]: (outcome, node_id) where outcome is
    ///     "created" | "updated" | "unchanged".
    ///
    /// Example:
    ///     >>> graph.upsert("Chunk", "key", {"key": "note:a", "path": "a.md"})
    ///     ('created', '…uuid…')
    #[pyo3(signature = (label, key, props, vector=None, model=None, links=None))]
    fn upsert(
        &self,
        py: Python<'_>,
        label: &str,
        key: &str,
        props: &Bound<'_, PyDict>,
        vector: Option<Vec<f32>>,
        model: Option<String>,
        links: Option<&Bound<'_, PyList>>,
    ) -> PyResult<(String, String)> {
        let graph = self.graph()?;
        let req = build_upsert_request(label, key, props, vector, model, links)?;
        let (outcome, id) = to_py_result(crate::python::runtime::block_on(py, async move {
            graph.upsert_node(req).await
        }))?;
        Ok((outcome.as_str().to_string(), id.to_string()))
    }

    /// Upsert many nodes. Each item is a dict with the same fields as `upsert`:
    /// {"label", "key", "props", "vector"?, "model"?, "links"?}.
    ///
    /// One transaction (one fsync) per chunk of 1 024 items. A chunk is
    /// atomic: if an item fails, nothing of its chunk is written and the error
    /// is raised; earlier chunks stay committed. A key repeated inside the
    /// list updates the node created by the earlier item, and a link to
    /// another item of the list resolves to that item's node.
    ///
    /// Returns:
    ///     list[tuple[str, str]]: (outcome, node_id) per item, in order.
    fn upsert_many(
        &self,
        py: Python<'_>,
        requests: &Bound<'_, PyList>,
    ) -> PyResult<Vec<(String, String)>> {
        let graph = self.graph()?;
        let mut reqs = Vec::with_capacity(requests.len());
        for item in requests.iter() {
            let dict = item.cast::<PyDict>().map_err(|_| {
                PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                    "upsert_many: each request must be a dict",
                )
            })?;
            reqs.push(build_upsert_request_from_dict(dict)?);
        }
        let out = to_py_result(crate::python::runtime::block_on(py, async move {
            graph.upsert_batch(reqs).await
        }))?;
        Ok(out
            .into_iter()
            .map(|(o, id)| (o.as_str().to_string(), id.to_string()))
            .collect())
    }

    /// Hybrid search: Reciprocal Rank Fusion of full-text (tantivy) and vector
    /// (HNSW) retrieval, with an optional label/property filter.
    ///
    /// Args:
    ///     text (str, optional): full-text query (needs a fulltext index).
    ///     vector (list[float], optional): query vector (requires `model`).
    ///     model (str, optional): embedding model name (requires `vector`).
    ///     k (int): number of fused results (default 10).
    ///     ef (int, optional): HNSW ef_search (default 30).
    ///     label (str, optional): restrict to this node label.
    ///     props (dict, optional): restrict to these property equalities (AND).
    ///     text_index (str, optional): fulltext index name; auto-discovered if omitted.
    ///     rrf_k (float): RRF constant (default 60.0).
    ///     hydrate (bool): if True each hit also carries `"node"` (the node
    ///         dict, read in the same call; None if it vanished). Default False.
    ///
    /// Returns:
    ///     list[dict]: {node_id, score, text_rank, vector_rank[, node]}, best first.
    #[cfg(feature = "hybrid")]
    #[pyo3(signature = (text=None, vector=None, model=None, k=10, ef=None, label=None, props=None, text_index=None, rrf_k=60.0, hydrate=false))]
    #[allow(clippy::too_many_arguments)]
    fn search_hybrid(
        &self,
        py: Python<'_>,
        text: Option<String>,
        vector: Option<Vec<f32>>,
        model: Option<String>,
        k: usize,
        ef: Option<usize>,
        label: Option<String>,
        props: Option<&Bound<'_, PyDict>>,
        text_index: Option<String>,
        rrf_k: f32,
        hydrate: bool,
    ) -> PyResult<Vec<Py<PyDict>>> {
        let graph = self.graph()?;
        let hq = build_hybrid_query(text, vector, model, k, ef, label, props, text_index, rrf_k)?;

        let graph_for_hydrate = graph.clone();
        let hits = to_py_result(crate::python::runtime::block_on(py, async move {
            graph.search_hybrid(hq).await
        }))?;
        let nodes: Vec<Option<crate::Node>> = if hydrate {
            let ids: Vec<crate::NodeId> = hits.iter().map(|h| h.node_id).collect();
            to_py_result(crate::python::runtime::block_on(py, async move { graph_for_hydrate.get_nodes(&ids).await }))?
        } else {
            Vec::new()
        };

        hits.into_iter()
            .enumerate()
            .map(|(i, h)| {
                let d = PyDict::new(py);
                d.set_item("node_id", h.node_id.to_string())?;
                d.set_item("score", h.score)?;
                d.set_item("text_rank", h.text_rank)?;
                d.set_item("vector_rank", h.vector_rank)?;
                if hydrate {
                    d.set_item("node", nodes.get(i).and_then(|n| n.as_ref()).map(|n| node_to_pydict(py, n)).transpose()?)?;
                }
                Ok(d.unbind())
            })
            .collect()
    }

    /// Same search as `search_hybrid`, plus why each hit ranked where it did.
    ///
    /// The RRF score orders results but does not explain them: it is a sum of
    /// reciprocal *ranks*, so it cannot tell you whether a document rose
    /// through text, through vectors, or both. This returns the raw numbers
    /// the fusion consumes and discards.
    ///
    /// Args: same as `search_hybrid`.
    ///
    /// Returns:
    ///     dict with:
    ///       - `hits`: list[dict] {node_id, score, text_rank, text_score,
    ///         vector_rank, vector_distance}, best first. `text_score` is the
    ///         raw BM25 score and `vector_distance` the cosine distance
    ///         (0 = identical); each is None when the hit did not come
    ///         through that branch.
    ///       - `k`, `rrf_k`, `overfetch`, `candidates`: effective config.
    ///       - `ef_search`, `text_index`, `allowed_set_size`: what was
    ///         actually used — including values the caller did not pick.
    ///       - `text`, `vector`: {requested, returned, underfilled} per
    ///         branch, or None if that branch did not run.
    ///       - `vector_path`: "unfiltered" | "exact_over_allowed" |
    ///         "hnsw_filtered". Only the last is approximate, which is what
    ///         decides how to read a short result.
    ///
    /// Example:
    ///     >>> e = graph.search_hybrid_explain(text="cactus", k=5)
    ///     >>> e["hits"][0]["text_score"], e["vector_path"]
    #[cfg(feature = "hybrid")]
    #[pyo3(signature = (text=None, vector=None, model=None, k=10, ef=None, label=None, props=None, text_index=None, rrf_k=60.0))]
    #[allow(clippy::too_many_arguments)]
    fn search_hybrid_explain(
        &self,
        py: Python<'_>,
        text: Option<String>,
        vector: Option<Vec<f32>>,
        model: Option<String>,
        k: usize,
        ef: Option<usize>,
        label: Option<String>,
        props: Option<&Bound<'_, PyDict>>,
        text_index: Option<String>,
        rrf_k: f32,
    ) -> PyResult<Py<PyDict>> {
        let graph = self.graph()?;
        let hq = build_hybrid_query(text, vector, model, k, ef, label, props, text_index, rrf_k)?;

        let e = to_py_result(crate::python::runtime::block_on(py, async move {
            graph.search_hybrid_explain(hq).await
        }))?;

        let branch = |b: Option<crate::BranchReport>| -> PyResult<Option<Py<PyDict>>> {
            b.map(|b| {
                let d = PyDict::new(py);
                d.set_item("requested", b.requested)?;
                d.set_item("returned", b.returned)?;
                d.set_item("underfilled", b.underfilled())?;
                Ok(d.unbind())
            })
            .transpose()
        };

        let hits = pyo3::types::PyList::empty(py);
        for h in &e.hits {
            let d = PyDict::new(py);
            d.set_item("node_id", h.node_id.to_string())?;
            d.set_item("score", h.score)?;
            d.set_item("text_rank", h.text_rank)?;
            d.set_item("text_score", h.text_score)?;
            d.set_item("vector_rank", h.vector_rank)?;
            d.set_item("vector_distance", h.vector_distance)?;
            hits.append(d)?;
        }

        let out = PyDict::new(py);
        out.set_item("hits", hits)?;
        out.set_item("k", e.k)?;
        out.set_item("rrf_k", e.rrf_k)?;
        out.set_item("overfetch", e.overfetch)?;
        out.set_item("candidates", e.candidates)?;
        out.set_item("ef_search", e.ef_search)?;
        out.set_item("text_index", e.text_index.clone())?;
        out.set_item("allowed_set_size", e.allowed_set_size)?;
        out.set_item("text", branch(e.text)?)?;
        out.set_item("vector", branch(e.vector)?)?;
        out.set_item(
            "vector_path",
            e.vector_path.map(|p| match p {
                crate::VectorPath::Unfiltered => "unfiltered",
                crate::VectorPath::ExactOverAllowed => "exact_over_allowed",
                crate::VectorPath::HnswFiltered => "hnsw_filtered",
            }),
        )?;
        Ok(out.unbind())
    }

    /// Delete the node identified by a business key `(label, key, value)` — the
    /// counterpart of `upsert` for incremental reconciliation.
    ///
    /// Returns the deleted node id, or None if no node matched (idempotent).
    /// Raises if more than one node matches the key.
    ///
    /// Example:
    ///     >>> graph.delete("Note", "key", "note:intro")
    ///     '…uuid…'
    fn delete(
        &self,
        py: Python<'_>,
        label: &str,
        key: &str,
        value: &Bound<'_, PyAny>,
    ) -> PyResult<Option<String>> {
        let graph = self.graph()?;
        let value = pyany_to_property(value)?;
        let label = label.to_string();
        let key = key.to_string();
        let id = to_py_result(crate::python::runtime::block_on(py, async move {
            graph.delete_node_by_key(&label, &key, &value).await
        }))?;
        Ok(id.map(|i| i.to_string()))
    }
}

// ─── Conversión Python → tipos de upsert ────────────────────────────────────
// Conversor único en super:: (python/mod.rs) — ver su doc para el orden de
// despacho (bool antes que int, PyBytes por downcast).

use super::{pyany_to_property, pydict_to_props};

fn build_link(dict: &Bound<'_, PyDict>) -> PyResult<LinkSpec> {
    let get_str = |name: &str| -> PyResult<String> {
        dict.get_item(name)?
            .ok_or_else(|| {
                PyErr::new::<pyo3::exceptions::PyKeyError, _>(format!("link missing '{name}'"))
            })?
            .extract::<String>()
    };
    let edge_type = get_str("type")?;
    let target_label = get_str("target_label")?;
    let target_key = get_str("target_key")?;
    let target_key_value = pyany_to_property(&dict.get_item("target_key_value")?.ok_or_else(
        || PyErr::new::<pyo3::exceptions::PyKeyError, _>("link missing 'target_key_value'"),
    )?)?;
    let props = match dict.get_item("props")? {
        Some(p) => pydict_to_props(p.cast::<PyDict>().map_err(|_| {
            PyErr::new::<pyo3::exceptions::PyTypeError, _>("link 'props' must be a dict")
        })?)?,
        None => HashMap::new(),
    };
    let create_target_stub = match dict.get_item("stub")? {
        Some(b) => b.extract::<bool>().unwrap_or(false),
        None => false,
    };
    Ok(LinkSpec {
        edge_type,
        target_label,
        target_key,
        target_key_value,
        props,
        create_target_stub,
    })
}

/// Arma la `HybridQuery` desde los argumentos de Python.
///
/// Compartida por `search_hybrid` y `search_hybrid_explain`: si cada uno
/// armara la suya, podrían divergir en un default —el `overfetch`, por
/// ejemplo— y la explicación describiría una búsqueda distinta de la que el
/// usuario obtiene.
#[cfg(feature = "hybrid")]
#[allow(clippy::too_many_arguments)]
fn build_hybrid_query(
    text: Option<String>,
    vector: Option<Vec<f32>>,
    model: Option<String>,
    k: usize,
    ef: Option<usize>,
    label: Option<String>,
    props: Option<&Bound<'_, PyDict>>,
    text_index: Option<String>,
    rrf_k: f32,
) -> PyResult<crate::HybridQuery> {
    let embedding = build_embedding(vector, model)?;
    let filter = if label.is_some() || props.is_some() {
        let mut f = crate::HybridFilter { label, props: Vec::new() };
        if let Some(p) = props {
            for (key, value) in p.iter() {
                f.props.push((key.extract()?, pyany_to_property(&value)?));
            }
        }
        Some(f)
    } else {
        None
    };
    Ok(crate::HybridQuery {
        text,
        text_index,
        vector: embedding,
        k,
        ef_search: ef,
        rrf_k,
        overfetch: 4,
        filter,
    })
}

fn build_embedding(
    vector: Option<Vec<f32>>,
    model: Option<String>,
) -> PyResult<Option<(Vec<f32>, String)>> {
    match (vector, model) {
        (Some(v), Some(m)) => Ok(Some((v, m))),
        (None, None) => Ok(None),
        _ => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            "upsert: provide both 'vector' and 'model', or neither",
        )),
    }
}

fn build_links(links: Option<&Bound<'_, PyList>>) -> PyResult<Vec<LinkSpec>> {
    let mut out = Vec::new();
    if let Some(list) = links {
        for item in list.iter() {
            let dict = item.cast::<PyDict>().map_err(|_| {
                PyErr::new::<pyo3::exceptions::PyTypeError, _>("each link must be a dict")
            })?;
            out.push(build_link(dict)?);
        }
    }
    Ok(out)
}

fn build_upsert_request(
    label: &str,
    key: &str,
    props: &Bound<'_, PyDict>,
    vector: Option<Vec<f32>>,
    model: Option<String>,
    links: Option<&Bound<'_, PyList>>,
) -> PyResult<UpsertRequest> {
    Ok(UpsertRequest {
        label: label.to_string(),
        key: key.to_string(),
        props: pydict_to_props(props)?,
        embedding: build_embedding(vector, model)?,
        links: build_links(links)?,
    })
}

/// Parse a full request dict for `upsert_many`.
fn build_upsert_request_from_dict(dict: &Bound<'_, PyDict>) -> PyResult<UpsertRequest> {
    let label: String = dict
        .get_item("label")?
        .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyKeyError, _>("request missing 'label'"))?
        .extract()?;
    let key: String = dict
        .get_item("key")?
        .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyKeyError, _>("request missing 'key'"))?
        .extract()?;
    let props_item = dict
        .get_item("props")?
        .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyKeyError, _>("request missing 'props'"))?;
    let props = pydict_to_props(props_item.cast::<PyDict>().map_err(|_| {
        PyErr::new::<pyo3::exceptions::PyTypeError, _>("'props' must be a dict")
    })?)?;
    let vector: Option<Vec<f32>> = match dict.get_item("vector")? {
        Some(v) if !v.is_none() => Some(v.extract()?),
        _ => None,
    };
    let model: Option<String> = match dict.get_item("model")? {
        Some(m) if !m.is_none() => Some(m.extract()?),
        _ => None,
    };
    let links = match dict.get_item("links")? {
        Some(l) if !l.is_none() => build_links(Some(l.cast::<PyList>().map_err(|_| {
            PyErr::new::<pyo3::exceptions::PyTypeError, _>("'links' must be a list")
        })?))?,
        _ => Vec::new(),
    };
    Ok(UpsertRequest {
        label,
        key,
        props,
        embedding: build_embedding(vector, model)?,
        links,
    })
}

#[cfg(feature = "python-owl")]
fn export_report_dict(
    py: Python<'_>,
    report: &crate::rdf_owl::exporter::ExportReport,
) -> PyResult<Py<pyo3::types::PyDict>> {
    let dict = pyo3::types::PyDict::new(py);
    dict.set_item("classes",         report.classes)?;
    dict.set_item("subclass_edges",  report.subclass_edges)?;
    dict.set_item("individuals",     report.individuals)?;
    dict.set_item("type_triples",    report.type_triples)?;
    dict.set_item("edges",           report.edges)?;
    dict.set_item("literals",        report.literals)?;
    dict.set_item("triples_written", report.triples_written)?;
    dict.set_item("skipped",         report.skipped.clone())?;
    Ok(dict.into())
}

#[cfg(feature = "python-shacl")]
fn violations_to_py<'py>(
    py: Python<'py>,
    violations: &[crate::shacl::ConstraintViolation],
) -> PyResult<Bound<'py, pyo3::types::PyList>> {
    let list = pyo3::types::PyList::empty(py);
    for v in violations {
        let d = pyo3::types::PyDict::new(py);
        d.set_item("focus_node", v.focus_node.to_string())?;
        d.set_item("shape", v.shape_name.clone())?;
        d.set_item("constraint", v.constraint.clone())?;
        d.set_item("path", v.path.clone())?;
        match &v.value {
            Some(value) => d.set_item("value", crate::python::property_to_py(py, value)?)?,
            None => d.set_item("value", py.None())?,
        }
        d.set_item("message", v.message.clone())?;
        d.set_item("severity", format!("{:?}", v.severity))?;
        d.set_item("nested", violations_to_py(py, &v.nested)?)?;
        list.append(d)?;
    }
    Ok(list)
}
