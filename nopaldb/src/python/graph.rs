// src/python/graph.rs

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use crate::Graph as RustGraph;
use crate::types::PropertyValue;
use crate::{LinkSpec, StorageEngine, StorageOptions, StorageProfile, UpsertRequest};
use super::{PyNqlResult, PyTransaction, to_py_result};
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

fn parse_engine(engine: &str) -> PyResult<StorageEngine> {
    match engine.to_ascii_lowercase().as_str() {
        "sled" => Ok(StorageEngine::Sled),
        _ => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
            format!("Invalid engine '{}'. Use 'sled'", engine)
        )),
    }
}

/// Python wrapper for NopalDB Graph
#[pyclass(name = "Graph")]
pub struct PyGraph {
    // Mutex<Option<...>> permite que close() extraiga y suelte el Arc, liberando
    // el lock de Sled aunque el objeto Python siga vivo.
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
    /// engine: "sled"
    /// profile: "default" | "mobile" | "server"
    #[staticmethod]
    #[pyo3(signature = (path, engine="sled", profile="default"))]
    fn open_with_options(py: Python<'_>, path: &str, engine: &str, profile: &str) -> PyResult<Self> {
        let engine = parse_engine(engine)?;
        let profile = parse_profile(profile)?;
        let options = StorageOptions { engine, profile };

        let graph = crate::python::runtime::block_on(py, async { RustGraph::open_with_options(path, options).await });
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
    /// engine: "sled"
    /// profile: "default" | "mobile" | "server"
    #[staticmethod]
    #[pyo3(signature = (engine="sled", profile="default"))]
    fn in_memory_with_options(py: Python<'_>, engine: &str, profile: &str) -> PyResult<Self> {
        let engine = parse_engine(engine)?;
        let profile = parse_profile(profile)?;
        let options = StorageOptions { engine, profile };

        let graph = crate::python::runtime::block_on(py, async { RustGraph::in_memory_with_options(options).await });
        to_py_result(graph).map(|g| PyGraph {
            inner: Mutex::new(Some(Arc::new(g))),
        })
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

    /// Get node count
    fn node_count(&self, py: Python<'_>) -> PyResult<usize> {
        let graph = self.graph()?;

        let result = crate::python::runtime::block_on(py, async move {
            graph.get_all_nodes().await
        });

        to_py_result(result).map(|nodes| nodes.len())
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
    ///     index_type (str): 'hash', 'btree', or 'fulltext' (default: 'hash')
    ///
    /// Returns:
    ///     str: Index name
    ///
    /// Example:
    ///     >>> graph.create_index("Person", "name", "hash")
    ///     'Person_name'
    ///     >>> graph.create_index("Person", "age", "btree")
    ///     'Person_age'
    #[pyo3(signature = (label, property, index_type="hash"))]
    fn create_index(
        &self,
        py: Python<'_>,
        label: String,
        property: String,
        index_type: &str,
    ) -> PyResult<String> {
        use crate::index::IndexType;

        let graph = self.graph()?;
        let idx_type = match index_type {
            "hash" => IndexType::Hash,
            "btree" => IndexType::BTree,
            "fulltext" => IndexType::FullText,
            _ => return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                format!("Invalid index type '{}'. Use 'hash', 'btree', or 'fulltext'", index_type)
            )),
        };

        let result = crate::python::runtime::block_on(py, async move {
            graph.create_index(&label, &property, idx_type).await
        });

        to_py_result(result)
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

    /// Get query planner statistics
    ///
    /// Returns:
    ///     dict: Statistics about the graph
    ///
    /// Example:
    ///     >>> stats = graph.get_stats()
    ///     >>> print(f"Total nodes: {stats['total_nodes']}")
    ///     >>> print(f"Avg degree: {stats['avg_degree']}")
    fn get_stats(&self, py: Python<'_>) -> PyResult<std::collections::HashMap<String, String>> {
        let graph = self.graph()?;

        let stats = crate::python::runtime::block_on(py, async move {
            graph.get_stats().await
        });

        let stats = to_py_result(stats)?;

        let mut result = std::collections::HashMap::new();
        result.insert("total_nodes".to_string(), stats.total_nodes.to_string());
        result.insert("total_edges".to_string(), stats.total_edges.to_string());
        result.insert("avg_degree".to_string(), format!("{:.2}", stats.avg_degree));

        Ok(result)
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

    /// Busca los k nodos más cercanos en el espacio de embeddings.
    ///
    /// Construye el índice HNSW para el modelo dado (en memoria, sin caché),
    /// luego retorna los k vecinos más próximos al vector query.
    ///
    /// Args:
    ///     query_vector (list[float]): Vector de consulta.
    ///     k (int): Número de vecinos a retornar.
    ///     model (str): Nombre del modelo.
    ///
    /// Returns:
    ///     list[tuple[str, float]]: Lista de (node_id, distancia_coseno) ordenada por similitud.
    ///
    /// Example:
    ///     >>> results = graph.knn_nodes([0.1, 0.2, 0.3], k=5, model="minilm")
    ///     >>> for node_id, dist in results:
    ///     ...     print(node_id, dist)
    #[cfg(feature = "embeddings-index")]
    fn knn_nodes(&self, py: Python<'_>, query_vector: Vec<f32>, k: usize, model: &str) -> PyResult<Vec<(String, f32)>> {
        let graph = self.graph()?;
        let model = model.to_string();

        let idx = to_py_result(crate::python::runtime::block_on(py, async move {
            graph.build_embedding_index(&model).await
        }))?;

        to_py_result(idx.search_knn(&query_vector, k))
            .map(|hits| hits.into_iter().map(|(id, dist)| (id.to_string(), dist)).collect())
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
    ///     dict: {classes_added, subclass_edges_added, instances_added, triples_skipped}
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
    ///
    /// Returns:
    ///     list[dict]: {node_id, score, text_rank, vector_rank}, best first.
    #[cfg(feature = "hybrid")]
    #[pyo3(signature = (text=None, vector=None, model=None, k=10, ef=None, label=None, props=None, text_index=None, rrf_k=60.0))]
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
    ) -> PyResult<Vec<Py<PyDict>>> {
        let graph = self.graph()?;

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

        let hq = crate::HybridQuery {
            text,
            text_index,
            vector: embedding,
            k,
            ef_search: ef,
            rrf_k,
            overfetch: 4,
            filter,
        };

        let hits = to_py_result(crate::python::runtime::block_on(py, async move {
            graph.search_hybrid(hq).await
        }))?;

        hits.into_iter()
            .map(|h| {
                let d = PyDict::new(py);
                d.set_item("node_id", h.node_id.to_string())?;
                d.set_item("score", h.score)?;
                d.set_item("text_rank", h.text_rank)?;
                d.set_item("vector_rank", h.vector_rank)?;
                Ok(d.unbind())
            })
            .collect()
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

/// Convierte un valor Python a `PropertyValue` (bool antes que int: en Python
/// `bool` es subtipo de `int` y se extraería como 0/1 si se probara int primero).
fn pyany_to_property(value: &Bound<'_, PyAny>) -> PyResult<PropertyValue> {
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
    } else if let Ok(bytes) = value.extract::<Vec<u8>>() {
        Ok(PropertyValue::Bytes(bytes))
    } else {
        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "unsupported property value type (use str/int/float/bool/bytes/None)",
        ))
    }
}

fn pydict_to_props(dict: &Bound<'_, PyDict>) -> PyResult<HashMap<String, PropertyValue>> {
    let mut props = HashMap::new();
    for (k, v) in dict.iter() {
        let key: String = k.extract()?;
        props.insert(key, pyany_to_property(&v)?);
    }
    Ok(props)
}

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
