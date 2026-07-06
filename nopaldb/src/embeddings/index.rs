// src/embeddings/index.rs
//
// HnswIndex — índice HNSW para búsqueda ANN de nodos por similitud semántica.
//
// Backed by `hnsw_rs 0.3` — soporta inserciones incrementales, búsqueda paralela,
// SIMD opcional, y persistencia nativa a disco.
//
// El NodeId (UUID) se mapea a DataId (usize) via tablas bidireccionales.
//
// Ciclo de vida:
//   Bulk: `HnswIndex::build_batch(vectors, model, M, ef_c)` → `search_knn()`
//   Incremental: `new()` → `insert()` repetido → `set_searching_mode()` → `search_knn()`
//
// Persistencia: ver `persistence.rs` para save/load a disco.

use crate::error::NopalError;
use crate::types::NodeId;

use hnsw_rs::prelude::*; // incluye Hnsw, HnswIo, Neighbour, DistCosine, etc.

use std::collections::HashMap;

/// Parámetros por defecto para construcción HNSW.
const DEFAULT_MAX_NB_CONNECTION: usize = 24;
const DEFAULT_EF_CONSTRUCTION: usize = 400;
const DEFAULT_MAX_LAYER: usize = 16;
const DEFAULT_EF_SEARCH: usize = 30;

/// Debajo de este tamaño, `build_batch` inserta en serie para que la
/// construcción del grafo HNSW sea determinista (independiente del número de
/// threads de rayon). Por encima, la inserción paralela sí rinde.
const PARALLEL_INSERT_THRESHOLD: usize = 128;

/// Índice HNSW para búsqueda aproximada de vecinos más cercanos (ANN) sobre embeddings.
///
/// Usa `hnsw_rs` con distancia coseno. Soporta inserciones incrementales (sin rebuild),
/// búsqueda paralela, y persistencia a disco.
pub struct HnswIndex {
    /// El grafo HNSW interno.
    inner: Hnsw<'static, f32, DistCosine>,
    /// Mapeo DataId → NodeId (para traducir resultados de búsqueda).
    id_map: HashMap<usize, NodeId>,
    /// Mapeo inverso NodeId → DataId (para detectar duplicados / upsert).
    reverse_map: HashMap<NodeId, usize>,
    /// Modelo al que corresponde este índice (ej: "minilm", "bert-base").
    model: String,
    /// Dimensión de los vectores (validación en insert).
    dimension: usize,
    /// Siguiente DataId disponible para asignar.
    next_data_id: usize,
}

impl HnswIndex {
    /// Crea un índice vacío para el modelo y dimensión dados.
    ///
    /// `max_elements` es un hint de capacidad inicial (no un límite duro).
    pub fn new(
        model: impl Into<String>,
        dimension: usize,
        max_elements: usize,
    ) -> Self {
        let inner = Hnsw::<f32, DistCosine>::new(
            DEFAULT_MAX_NB_CONNECTION,
            max_elements,
            DEFAULT_MAX_LAYER,
            DEFAULT_EF_CONSTRUCTION,
            DistCosine {},
        );
        Self {
            inner,
            id_map: HashMap::new(),
            reverse_map: HashMap::new(),
            model: model.into(),
            dimension,
            next_data_id: 0,
        }
    }

    /// Crea un índice con parámetros HNSW custom.
    pub fn with_params(
        model: impl Into<String>,
        dimension: usize,
        max_elements: usize,
        max_nb_connection: usize,
        ef_construction: usize,
        max_layer: usize,
    ) -> Self {
        let inner = Hnsw::<f32, DistCosine>::new(
            max_nb_connection,
            max_elements,
            max_layer,
            ef_construction,
            DistCosine {},
        );
        Self {
            inner,
            id_map: HashMap::new(),
            reverse_map: HashMap::new(),
            model: model.into(),
            dimension,
            next_data_id: 0,
        }
    }

    /// Construye un índice en batch a partir de un vector de (NodeId, vector).
    ///
    /// Usa `parallel_insert` para construcción eficiente y activa modo búsqueda al final.
    pub fn build_batch(
        vectors: Vec<(NodeId, Vec<f32>)>,
        model: impl Into<String>,
        dimension: usize,
    ) -> Result<Self, NopalError> {
        if vectors.is_empty() {
            return Err(NopalError::custom("HnswIndex::build_batch: no vectors provided"));
        }

        // Validar dimensiones
        for (node_id, vec) in &vectors {
            if vec.len() != dimension {
                return Err(NopalError::custom(format!(
                    "HnswIndex::build_batch: node {} has dimension {}, expected {}",
                    node_id, vec.len(), dimension
                )));
            }
        }

        let model_str = model.into();
        let nb_elements = vectors.len();

        let mut index = Self::new(&model_str, dimension, nb_elements);

        // Preparar datos para parallel_insert: Vec<(&Vec<f32>, usize)>
        let mut owned_vectors: Vec<Vec<f32>> = Vec::with_capacity(nb_elements);
        let mut data_ids: Vec<usize> = Vec::with_capacity(nb_elements);

        for (node_id, vec) in vectors {
            let data_id = index.next_data_id;
            index.next_data_id += 1;
            index.id_map.insert(data_id, node_id);
            index.reverse_map.insert(node_id, data_id);
            owned_vectors.push(vec);
            data_ids.push(data_id);
        }

        // parallel_insert espera &[(&Vec<T>, usize)]
        let insert_data: Vec<(&Vec<f32>, usize)> = owned_vectors
            .iter()
            .zip(data_ids.iter())
            .map(|(v, &id)| (v, id))
            .collect();

        // Para lotes pequeños se inserta en serie: `parallel_insert` no aporta
        // (el overhead de rayon supera el trabajo) y su comportamiento depende
        // del número de threads, lo que produce grafos HNSW ligeramente
        // distintos entre entornos (p. ej. un test que pasa local y falla en un
        // runner con más cores). Serial = determinista para N pequeño.
        if insert_data.len() >= PARALLEL_INSERT_THRESHOLD {
            index.inner.parallel_insert(&insert_data);
        } else {
            for &(vec, data_id) in &insert_data {
                index.inner.insert((vec, data_id));
            }
        }
        index.inner.set_searching_mode(true);

        Ok(index)
    }

    /// Inserta un punto de forma incremental (no requiere rebuild).
    ///
    /// Si el NodeId ya existe en el índice, retorna error (usar `remove` + `insert` para upsert).
    pub fn insert(&mut self, node_id: NodeId, vector: Vec<f32>) -> Result<(), NopalError> {
        if vector.len() != self.dimension {
            return Err(NopalError::custom(format!(
                "HnswIndex({}): expected dimension {}, got {}",
                self.model, self.dimension, vector.len()
            )));
        }

        if self.reverse_map.contains_key(&node_id) {
            return Err(NopalError::custom(format!(
                "HnswIndex({}): node {} already indexed — remove first to update",
                self.model, node_id
            )));
        }

        let data_id = self.next_data_id;
        self.next_data_id += 1;

        self.inner.insert((&vector, data_id));
        self.id_map.insert(data_id, node_id);
        self.reverse_map.insert(node_id, data_id);

        Ok(())
    }

    /// Busca los `k` nodos más cercanos al vector `query` en el espacio de embeddings.
    ///
    /// Retorna `Vec<(NodeId, f32)>` ordenado por distancia ascendente (más cercano primero).
    /// La distancia es coseno: 0 = idénticos, 1 = ortogonales, 2 = opuestos.
    pub fn search_knn(
        &self,
        query: &[f32],
        k: usize,
    ) -> Result<Vec<(NodeId, f32)>, NopalError> {
        self.search_knn_with_ef(query, k, DEFAULT_EF_SEARCH)
    }

    /// Busca KNN con parámetro `ef_search` custom (controla calidad vs velocidad).
    pub fn search_knn_with_ef(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
    ) -> Result<Vec<(NodeId, f32)>, NopalError> {
        if query.len() != self.dimension {
            return Err(NopalError::custom(format!(
                "HnswIndex({}): query dimension {} != index dimension {}",
                self.model, query.len(), self.dimension
            )));
        }

        if self.id_map.is_empty() {
            return Ok(Vec::new());
        }

        let neighbors = self.inner.search(query, k, ef_search);

        let mut results = Vec::with_capacity(neighbors.len());
        for neighbor in neighbors {
            let data_id = neighbor.d_id;
            if let Some(&node_id) = self.id_map.get(&data_id) {
                results.push((node_id, neighbor.distance));
            }
        }

        Ok(results)
    }

    /// Busca KNN filtrando por un predicado sobre NodeId.
    ///
    /// Útil para combinar búsqueda vectorial con predicados de grafo:
    /// solo retorna vecinos cuyo NodeId pasa el filtro.
    pub fn search_knn_filtered<F>(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
        filter: F,
    ) -> Result<Vec<(NodeId, f32)>, NopalError>
    where
        F: Fn(&NodeId) -> bool,
    {
        // hnsw_rs no tiene search_with_filter nativo en la API pública,
        // así que hacemos over-fetch y filtramos.
        // Pedimos más candidatos para compensar los filtrados.
        let over_fetch = k * 4;
        let mut results = self.search_knn_with_ef(query, over_fetch, ef_search)?;
        results.retain(|(node_id, _)| filter(node_id));
        results.truncate(k);
        Ok(results)
    }

    /// Retorna cuántos puntos hay en el índice.
    pub fn len(&self) -> usize {
        self.id_map.len()
    }

    /// Retorna `true` si el índice no tiene puntos.
    pub fn is_empty(&self) -> bool {
        self.id_map.is_empty()
    }

    /// Retorna el nombre del modelo asociado a este índice.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Retorna la dimensión de los vectores en este índice.
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Acceso interno al grafo HNSW (para persistencia).
    #[allow(dead_code)] // se usará en persistence.rs
    pub(crate) fn inner(&self) -> &Hnsw<'static, f32, DistCosine> {
        &self.inner
    }

    /// Acceso al mapeo DataId → NodeId (para persistencia).
    #[allow(dead_code)] // se usará en persistence.rs
    pub(crate) fn id_map(&self) -> &HashMap<usize, NodeId> {
        &self.id_map
    }

    /// Acceso al mapeo inverso (para persistencia).
    #[allow(dead_code)] // se usará en persistence.rs
    pub(crate) fn reverse_map(&self) -> &HashMap<NodeId, usize> {
        &self.reverse_map
    }

    /// Retorna el siguiente DataId disponible (para restaurar estado post-load).
    #[allow(dead_code)] // se usará en persistence.rs
    pub(crate) fn next_data_id(&self) -> usize {
        self.next_data_id
    }
}

// Backward-compatible alias
pub type EmbeddingIndex = HnswIndex;

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_build_batch_and_search() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let id_c = Uuid::new_v4();

        let vectors = vec![
            (id_a, vec![1.0, 0.0, 0.0]),
            (id_b, vec![0.0, 1.0, 0.0]),
            (id_c, vec![0.9, 0.1, 0.0]),
        ];

        let index = HnswIndex::build_batch(vectors, "test", 3).unwrap();

        // Query cerca de id_a — coseno: [1,0,0] vs [0.9,0.1,0] es muy cercano
        let results = index.search_knn(&[1.0, 0.0, 0.0], 1).unwrap();
        assert_eq!(results.len(), 1);
        // El más cercano a [1,0,0] debe ser id_a (distancia 0)
        assert_eq!(results[0].0, id_a);
        assert!(results[0].1 < 0.01, "self-distance should be ~0, got {}", results[0].1);
    }

    #[test]
    fn test_incremental_insert() {
        let mut index = HnswIndex::new("test", 2, 10);
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();

        index.insert(id_a, vec![1.0, 0.0]).unwrap();
        index.insert(id_b, vec![0.0, 1.0]).unwrap();

        assert_eq!(index.len(), 2);

        let results = index.search_knn(&[0.9, 0.1], 1).unwrap();
        assert_eq!(results[0].0, id_a);
    }

    #[test]
    fn test_duplicate_insert_returns_error() {
        let mut index = HnswIndex::new("test", 2, 10);
        let id = Uuid::new_v4();

        index.insert(id, vec![1.0, 0.0]).unwrap();
        let result = index.insert(id, vec![0.0, 1.0]);
        assert!(result.is_err());
    }

    #[test]
    fn test_search_top_k() {
        let ids: Vec<Uuid> = (0..10).map(|_| Uuid::new_v4()).collect();
        let vectors: Vec<(Uuid, Vec<f32>)> = ids
            .iter()
            .enumerate()
            .map(|(i, &id)| {
                // Vectores unitarios a lo largo del eje 0
                let mut v = vec![0.0; 4];
                v[0] = 1.0 - (i as f32 * 0.1);
                v[1] = i as f32 * 0.1;
                (id, v)
            })
            .collect();

        let index = HnswIndex::build_batch(vectors, "test", 4).unwrap();
        let results = index.search_knn(&[1.0, 0.0, 0.0, 0.0], 3).unwrap();

        assert_eq!(results.len(), 3);
        // El primer resultado debe ser el más cercano a [1,0,0,0]
        assert_eq!(results[0].0, ids[0]);
    }

    #[test]
    fn test_dimension_mismatch_on_insert() {
        let mut index = HnswIndex::new("test", 3, 10);
        let result = index.insert(Uuid::new_v4(), vec![1.0, 2.0]); // dim 2 != 3
        assert!(result.is_err());
    }

    #[test]
    fn test_dimension_mismatch_on_search() {
        let index = HnswIndex::build_batch(
            vec![(Uuid::new_v4(), vec![1.0, 0.0])],
            "test",
            2,
        )
        .unwrap();
        let result = index.search_knn(&[1.0, 0.0, 0.0], 1); // dim 3 != 2
        assert!(result.is_err());
    }

    #[test]
    fn test_build_batch_empty_returns_error() {
        let result = HnswIndex::build_batch(Vec::new(), "test", 2);
        assert!(result.is_err());
    }

    #[test]
    fn test_len_and_is_empty() {
        let index = HnswIndex::new("test", 2, 10);
        assert_eq!(index.len(), 0);
        assert!(index.is_empty());

        let index = HnswIndex::build_batch(
            vec![(Uuid::new_v4(), vec![1.0, 0.0])],
            "test",
            2,
        )
        .unwrap();
        assert_eq!(index.len(), 1);
        assert!(!index.is_empty());
    }

    #[test]
    fn test_search_empty_index_returns_empty() {
        let index = HnswIndex::new("test", 2, 10);
        let results = index.search_knn(&[1.0, 0.0], 5).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_filtered_search() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let id_c = Uuid::new_v4();

        let vectors = vec![
            (id_a, vec![1.0, 0.0, 0.0]),
            (id_b, vec![0.9, 0.1, 0.0]),
            (id_c, vec![0.0, 1.0, 0.0]),
        ];

        let index = HnswIndex::build_batch(vectors, "test", 3).unwrap();

        // Filtrar: solo id_b y id_c. ef alto para forzar exploración exhaustiva
        // en un grafo diminuto (independiente de la asignación de niveles HNSW).
        let allowed = vec![id_b, id_c];
        let results = index
            .search_knn_filtered(&[1.0, 0.0, 0.0], 2, 200, |nid| allowed.contains(nid))
            .unwrap();

        // El filtro debe excluir id_a (el más cercano, no permitido)
        assert!(
            results.iter().all(|(id, _)| *id != id_a),
            "filtered-out node id_a must not appear in results"
        );
        // Debe encontrar id_b (el permitido más cercano a [1,0,0])
        assert!(
            results.iter().any(|(id, _)| *id == id_b),
            "closest allowed vector id_b must be returned"
        );
        // id_b es más cercano que id_c → primero en el ranking
        assert_eq!(results[0].0, id_b);
    }

    #[test]
    fn test_model_and_dimension_accessors() {
        let index = HnswIndex::new("minilm", 384, 100);
        assert_eq!(index.model(), "minilm");
        assert_eq!(index.dimension(), 384);
    }
}
