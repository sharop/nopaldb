// src/ml/pyg.rs
//
// PyTorch Geometric integration
// Provides Arrow-based graph conversion

use crate::error::{NopalError, Result};
use crate::graph::Graph;
use crate::ml::arrow_tensor::MLTensor;
use crate::types::{Edge, Node, NodeId};

/// PyTorch Geometric compatible graph data
#[derive(Debug, Clone)]
pub struct PyGData {
    /// Node feature tensors — one per numeric property column
    pub x: Vec<MLTensor>,

    /// Edge index [2, num_edges] - COO format
    pub edge_index: EdgeIndex,

    /// Edge attributes (optional) — one tensor per numeric edge property
    pub edge_attr: Option<Vec<MLTensor>>,

    /// Number of nodes
    pub num_nodes: usize,

    /// Number of edges
    pub num_edges: usize,
}

/// Edge index in COO (Coordinate) format
#[derive(Debug, Clone)]
pub struct EdgeIndex {
    /// Source node indices
    pub source: Vec<usize>,

    /// Target node indices
    pub target: Vec<usize>,
}

impl PyGData {
    /// Create PyG-compatible data from NopalDB graph
    ///
    /// # Arguments
    /// * `graph` - The graph instance
    /// * `node_label` - Label to filter nodes (e.g., "Person", "Transaction")
    /// * `edge_type` - Optional edge type filter
    ///
    /// # Returns
    /// PyGData structure ready for PyTorch Geometric
    pub async fn from_graph(
        graph: &Graph,
        node_label: &str,
        edge_type: Option<&str>,
    ) -> Result<Self> {
        // 1. Get all nodes with specified label
        let nodes = graph.get_nodes_by_label(node_label).await?;

        if nodes.is_empty() {
            return Err(NopalError::Custom(format!(
                "No nodes found with label '{}'",
                node_label
            )));
        }

        let num_nodes = nodes.len();

        // 2. Create node ID to index mapping
        let mut node_to_idx: std::collections::HashMap<NodeId, usize> =
            std::collections::HashMap::new();

        for (idx, node) in nodes.iter().enumerate() {
            node_to_idx.insert(node.id, idx);
        }

        // 3. Extract node features via Arrow RecordBatch
        let x = Self::extract_node_features(&nodes)?;

        // 4. Build edge index (returns COO + raw edges for attr extraction)
        let (edge_index, edges) = Self::build_edge_index(
            graph,
            &nodes,
            &node_to_idx,
            edge_type,
        ).await?;

        let num_edges = edge_index.source.len();

        // 5. Extract edge attributes if any edges are present
        let edge_attr = if edges.is_empty() {
            None
        } else {
            Self::extract_edge_features(&edges).ok()
        };

        Ok(PyGData {
            x,
            edge_index,
            edge_attr,
            num_nodes,
            num_edges,
        })
    }

    /// Versión extendida de `from_graph` que opcionalmente concatena embeddings
    /// almacenados como columnas adicionales en `x`.
    ///
    /// Si `embedding_model` es `Some("model")`, carga los embeddings de cada nodo
    /// para ese modelo y los añade como columnas al tensor de features. Los nodos
    /// sin embedding para ese modelo reciben un vector de ceros de la misma dimensión.
    #[cfg(feature = "embeddings")]
    pub async fn from_graph_with_embeddings(
        graph: &Graph,
        node_label: &str,
        edge_type: Option<&str>,
        embedding_model: Option<&str>,
    ) -> Result<Self> {
        // Construir PyGData base (sin embeddings)
        let mut base = Self::from_graph(graph, node_label, edge_type).await?;

        let model = match embedding_model {
            Some(m) => m,
            None => return Ok(base),
        };

        // Cargar todos los nodos del label para obtener sus IDs en orden
        let nodes = graph.get_nodes_by_label(node_label).await?;

        // Determinar dimensión del embedding (primer nodo que lo tenga)
        let mut dim = 0usize;
        for node in &nodes {
            if let Ok(emb) = graph.get_node_embedding(node.id, model).await {
                dim = emb.vector.len();
                break;
            }
        }

        if dim == 0 {
            // Ningún nodo tiene embedding para este modelo — devolver base sin modificar
            return Ok(base);
        }

        // Construir tensor de embeddings: un f32 por elemento, shape [num_nodes * dim]
        let mut emb_data: Vec<f32> = Vec::with_capacity(nodes.len() * dim);
        for node in &nodes {
            match graph.get_node_embedding(node.id, model).await {
                Ok(emb) if emb.vector.len() == dim => emb_data.extend_from_slice(&emb.vector),
                _ => emb_data.extend(std::iter::repeat_n(0.0f32, dim)),
            }
        }

        // Convertir Vec<f32> a bytes little-endian para MLTensor
        let emb_bytes: Vec<u8> = emb_data.iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();

        // Agregar como MLTensor adicional en x con nombre "embedding_<model>"
        base.x.push(crate::ml::arrow_tensor::MLTensor {
            shape: vec![nodes.len(), dim],
            dtype: crate::ml::arrow_tensor::TensorDType::Float32,
            data: emb_bytes,
        });

        Ok(base)
    }

    /// Extrae features numéricas de nodos via Arrow RecordBatch.
    /// Retorna un MLTensor por columna numérica (Float32/Float64/Int32/Int64).
    pub(crate) fn extract_node_features(
        nodes: &[Node],
    ) -> Result<Vec<MLTensor>> {
        if nodes.is_empty() {
            return Ok(vec![]);
        }

        let batch = crate::arrow_export::nodes_to_arrow_with_properties(nodes, None)?;

        let mut tensors = Vec::new();
        for i in 0..batch.num_columns() {
            let col = batch.column(i);
            match col.data_type() {
                arrow::datatypes::DataType::Float32
                | arrow::datatypes::DataType::Float64
                | arrow::datatypes::DataType::Int32
                | arrow::datatypes::DataType::Int64 => {
                    let tensor = MLTensor::from_arrow_array(col.as_ref())?;
                    tensors.push(tensor);
                }
                _ => {} // Ignorar columnas no numéricas (strings, etc.)
            }
        }

        Ok(tensors)
    }

    /// Extrae features numéricas de aristas directamente desde sus propiedades.
    /// Retorna un MLTensor (Float64) por propiedad numérica encontrada en las aristas.
    ///
    /// Nota: `edges_to_arrow_with_properties` almacena propiedades como Utf8,
    /// por lo que la extracción se hace directamente desde los valores de Properties.
    pub(crate) fn extract_edge_features(edges: &[Edge]) -> Result<Vec<MLTensor>> {
        if edges.is_empty() {
            return Ok(vec![]);
        }

        // Descubrir propiedades numéricas presentes en alguna arista
        let numeric_keys: Vec<String> = {
            let mut seen = std::collections::HashSet::new();
            for edge in edges {
                for (key, value) in &edge.properties {
                    match value {
                        crate::types::PropertyValue::Int(_)
                        | crate::types::PropertyValue::Float(_) => {
                            seen.insert(key.clone());
                        }
                        _ => {}
                    }
                }
            }
            let mut v: Vec<String> = seen.into_iter().collect();
            v.sort();
            v
        };

        if numeric_keys.is_empty() {
            return Ok(vec![]);
        }

        // Por cada clave numérica, construir un tensor Float64
        let mut tensors = Vec::new();
        for key in &numeric_keys {
            let mut data = Vec::with_capacity(edges.len() * 8);
            for edge in edges {
                let v = match edge.properties.get(key.as_str()) {
                    Some(crate::types::PropertyValue::Int(i)) => *i as f64,
                    Some(crate::types::PropertyValue::Float(f)) => *f,
                    _ => 0.0f64,
                };
                data.extend_from_slice(&v.to_le_bytes());
            }
            tensors.push(MLTensor {
                shape: vec![edges.len()],
                dtype: crate::ml::arrow_tensor::TensorDType::Float64,
                data,
            });
        }

        Ok(tensors)
    }

    /// Build edge index in COO format, returns (EdgeIndex, raw edges) para extraer edge_attr.
    async fn build_edge_index(
        graph: &Graph,
        nodes: &[Node],
        node_to_idx: &std::collections::HashMap<NodeId, usize>,
        edge_type: Option<&str>,
    ) -> Result<(EdgeIndex, Vec<Edge>)> {
        let mut source = Vec::new();
        let mut target = Vec::new();
        let mut collected_edges = Vec::new();

        for node in nodes {
            // Get outgoing edges
            let edges = graph.get_outgoing_edges(node.id).await?;

            for edge in edges {
                // Filter by edge type if specified
                if let Some(et) = edge_type && edge.edge_type != et {
                    continue;
                }

                // Only include if target is in our node set
                if let (Some(&target_idx), Some(&source_idx)) = (
                    node_to_idx.get(&edge.target),
                    node_to_idx.get(&edge.source),
                ) {
                    source.push(source_idx);
                    target.push(target_idx);
                    collected_edges.push(edge);
                }
            }
        }

        Ok((EdgeIndex { source, target }, collected_edges))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Node, PropertyValue};

    #[tokio::test]
    async fn test_pyg_conversion_basic() {
        let graph = Graph::in_memory().await.unwrap();

        // Add test nodes
        for i in 0..10 {
            let node = Node::new("User")
                .with_property("id", PropertyValue::Int(i))
                .with_property("age", PropertyValue::Int(20 + i));
            graph.add_node(node).await.unwrap();
        }

        // Convert to PyG
        let pyg_data = PyGData::from_graph(&graph, "User", None).await.unwrap();

        assert_eq!(pyg_data.num_nodes, 10);
        assert!(!pyg_data.x.is_empty(), "Should have at least one numeric tensor");
    }

    #[tokio::test]
    async fn test_extract_node_features_uses_arrow() {
        // 3 nodos con propiedades numéricas
        let nodes = vec![
            Node::new("Item")
                .with_property("weight", PropertyValue::Float(1.5))
                .with_property("count", PropertyValue::Int(10)),
            Node::new("Item")
                .with_property("weight", PropertyValue::Float(2.0))
                .with_property("count", PropertyValue::Int(20)),
            Node::new("Item")
                .with_property("weight", PropertyValue::Float(3.0))
                .with_property("count", PropertyValue::Int(30)),
        ];

        let tensors = PyGData::extract_node_features(&nodes).unwrap();

        // Debe haber al menos 1 tensor por columna numérica
        assert!(!tensors.is_empty(), "Should extract at least one numeric column");

        // Cada tensor debe tener 3 elementos (uno por nodo)
        for t in &tensors {
            assert_eq!(t.shape[0], 3, "Each tensor should have 3 elements");
        }
    }
}
