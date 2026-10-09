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

/// Pista de capacidad que se pasa a `Hnsw::new`: 0, aunque el llamador sepa
/// cuántos puntos vienen (#206). hnsw_rs 0.3.4 reserva por capa
/// `frac · N` punteros con `frac = exp(-i/s) - (-(i+1)/s)`: al segundo
/// término le falta el `exp`, así que con M = 24 la reserva suma ~433·N
/// punteros en vez de N, unos 3.4 KB por punto que nunca se usan (330 MiB
/// para 100k). Con 0 las capas crecen al insertar; medido a 100k × 384, la
/// memoria baja de 651 a 318 MiB con el mismo recall y el mismo tiempo de
/// build. El índice cargado de un dump no lo sufre: hnsw_rs dimensiona las
/// capas con los puntos reales.
const HNSW_CAPACITY_HINT: usize = 0;

/// `ef_search` por defecto cuando el llamador no lo especifica
/// (`search_knn`). Controla el tamaño de la lista de candidatos que HNSW
/// explora: más alto = mejor recall, más lento. Solo aplica cuando la
/// búsqueda va por el grafo HNSW (N > `EXACT_SEARCH_THRESHOLD`); el camino
/// exacto lo ignora porque no hay aproximación que compensar.
pub const DEFAULT_EF_SEARCH: usize = 30;

/// Con `N ≤ EXACT_SEARCH_THRESHOLD` puntos, `search_knn*` responde con un
/// scan lineal exacto (distancia coseno contra todos los puntos) en lugar de
/// consultar el grafo HNSW.
///
/// Razón principal: determinismo. El grafo HNSW depende de la asignación
/// aleatoria de niveles por punto, así que dos builds con los mismos datos
/// pueden rankear distinto (y con recall < 1 hasta omitir vecinos), lo que
/// producía tests flaky en índices chicos. El costo es competitivo: el scan
/// lineal es cache-friendly y en el borde del umbral queda a la par de HNSW
/// (medido: ~69µs vs ~62µs por query con N=1000, dim=64, k=10, release);
/// muy por debajo del umbral el scan gana claramente.
///
/// Se descartó la alternativa de "no construir el grafo HNSW bajo el
/// umbral": cruzar el umbral exigiría un rebuild completo en medio de un
/// `insert`. El grafo se construye siempre; solo cambia el camino de LECTURA.
pub const EXACT_SEARCH_THRESHOLD: usize = 1024;

/// Debajo de este tamaño, `build_batch` inserta en serie para que la
/// construcción del grafo HNSW sea determinista (independiente del número de
/// threads de rayon). Por encima, la inserción paralela sí rinde.
const PARALLEL_INSERT_THRESHOLD: usize = 128;

/// Tope de la escalada adaptativa de `ef_search` en la búsqueda filtrada.
///
/// Con un filtro selectivo el grafo HNSW puede necesitar explorar mucho más
/// para juntar `k` vecinos permitidos. Escalar sin límite degeneraría en un
/// recorrido completo del índice por query, así que la escalada se corta
/// aquí y el resultado se marca como `underfilled` en vez de seguir pagando.
/// Para conjuntos permitidos chicos el llamador debería usar el camino
/// exacto ([`rank_exact`]), que además es más barato.
pub const MAX_FILTERED_EF_SEARCH: usize = 4096;

/// A partir de esta fracción de tombstones sobre puntos vivos, el índice pide
/// rebuild ([`HnswIndex::needs_rebuild`]).
pub const REBUILD_TOMBSTONE_RATIO: f64 = 0.20;
/// Mínimo absoluto de tombstones para pedir rebuild: reconstruir un índice
/// de 10 puntos por 3 borrados no vale el trabajo.
pub const REBUILD_TOMBSTONE_MIN: usize = 64;

/// Factor de crecimiento de `ef_search` entre intentos de la escalada.
const EF_ESCALATION_FACTOR: usize = 4;

/// Alcanzabilidad (#184, #201): tras construir o insertar, cada punto nuevo
/// se busca a sí mismo con `REACHABILITY_K` vecinos y este `ef`; el que no
/// aparece va a los huérfanos (ver [`HnswIndex::orphans`]). Medido sin esto:
/// ~0.5% de los puntos de un build de 3000 y ~0.8% de uno de 100k × 384 no
/// los encontraba ninguna búsqueda. `ef = 64` basta: a 100k, de 811 puntos
/// que no aparecen con 64, 781 tampoco aparecen con 4096 (no les llega
/// ningún enlace; no es un fallo de la búsqueda aproximada), y un falso
/// huérfano solo cuesta una distancia más por búsqueda.
const REACHABILITY_EF: usize = 64;
/// Vecinos pedidos al verificar: tolera puntos duplicados (otro con el mismo
/// vector puede salir primero).
const REACHABILITY_K: usize = 10;
/// Consultas por bloque en la verificación de un build grande: acota la
/// copia de vectores que `parallel_search` necesita.
const REACHABILITY_CHUNK: usize = 10_000;

/// Qué camino de lectura resolvió una búsqueda filtrada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilteredSearchPath {
    /// Scan lineal exacto sobre TODOS los puntos del índice
    /// (`N ≤ EXACT_SEARCH_THRESHOLD`). Recall = 1 por construcción.
    Exact,
    /// Grafo HNSW con el filtro aplicado DURANTE el recorrido
    /// (`Hnsw::search_filter`): los candidatos se recorren completos pero
    /// solo los permitidos entran al heap de resultados. Aproximado.
    HnswFiltered,
}

/// Resultado de una búsqueda filtrada con la traza de cómo se resolvió.
///
/// Existe porque el llamador no puede distinguir "no hay más nodos
/// permitidos" de "el índice aproximado no los encontró": ambos casos se ven
/// como menos de `k` resultados. `path`, `ef_used` y `attempts` hacen esa
/// diferencia observable (y alimentan el explain de la búsqueda híbrida).
#[derive(Debug, Clone)]
pub struct FilteredSearchOutcome {
    /// Vecinos permitidos, ordenados por distancia ascendente.
    pub hits: Vec<(NodeId, f32)>,
    /// Camino que resolvió la búsqueda.
    pub path: FilteredSearchPath,
    /// `ef_search` efectivo del último intento; `None` en el camino exacto,
    /// donde el parámetro no aplica.
    pub ef_used: Option<usize>,
    /// Intentos de la escalada adaptativa (1 = respondió al primero).
    pub attempts: usize,
    /// `true` si se devolvieron menos de `k` resultados. En el camino exacto
    /// significa que genuinamente hay menos de `k` nodos permitidos; en el
    /// camino HNSW puede significar eso O que la escalada llegó al tope.
    pub underfilled: bool,
}

/// Top-k exacto por distancia coseno sobre un conjunto de vectores dado.
///
/// Es la referencia compartida de los dos caminos exactos: el interno del
/// índice (bajo `EXACT_SEARCH_THRESHOLD`) y el que la búsqueda híbrida aplica
/// sobre un conjunto permitido chico. Vive en un solo lugar a propósito —
/// duplicar el cálculo dejaría que la distancia reportada o el desempate
/// divergieran entre caminos que el usuario percibe como el mismo.
///
/// Usa la misma `DistCosine` de `hnsw_rs` para que las distancias sean
/// idénticas a las del camino HNSW. Los empates se rompen por NodeId
/// ascendente: determinista entre corridas y entre builds.
pub fn rank_exact<'a, I>(query: &[f32], candidates: I, k: usize) -> Vec<(NodeId, f32)>
where
    I: IntoIterator<Item = (NodeId, &'a [f32])>,
{
    let dist = DistCosine {};
    let mut scored: Vec<(NodeId, f32)> = candidates
        .into_iter()
        .map(|(node_id, vec)| (node_id, dist.eval(query, vec)))
        .collect();
    scored.sort_unstable_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    scored.truncate(k);
    scored
}

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
    /// Copia de los vectores para el camino de búsqueda exacta.
    ///
    /// Se mantiene solo mientras `len() ≤ EXACT_SEARCH_THRESHOLD`; al cruzar
    /// el umbral se libera (la búsqueda pasa a HNSW y no vuelve atrás porque
    /// el índice no soporta remociones). El costo de memoria es acotado:
    /// a lo sumo `EXACT_SEARCH_THRESHOLD` vectores duplicados.
    exact_store: Vec<(NodeId, Vec<f32>)>,
    /// Puntos retirados del índice (`remove`). `hnsw_rs` no borra del grafo,
    /// así que el borrado es lógico: el `DataId` sale de `id_map` y las
    /// búsquedas lo descartan. Cada tombstone sigue ocupando un vecino en el
    /// recorrido, por eso las búsquedas piden `k + tombstones` y por eso,
    /// pasado [`Self::needs_rebuild`], conviene reconstruir.
    tombstones: usize,
    /// Puntos vivos a los que el grafo no llega: tras insertarse no se
    /// encontraban a sí mismos (#184). Cada búsqueda los compara uno por uno
    /// y los mezcla con lo que devuelve el grafo (#201). Ver
    /// [`Self::orphans`].
    orphans: Vec<(NodeId, Vec<f32>)>,
    /// `true` cuando el estado en memoria difiere de lo que hay (o no hay)
    /// en disco: recién construido, o con `insert`/`remove` desde el último
    /// dump. Lo limpia [`crate::embeddings::persistence`] al escribir/cargar.
    dirty: bool,
    /// Cuánto tardó cargarlo desde el dump (`None` si se construyó desde
    /// storage). Solo informativo: alimenta `EmbeddingIndexStats`.
    loaded_in: Option<std::time::Duration>,
}

impl HnswIndex {
    /// Crea un índice vacío para el modelo y dimensión dados.
    ///
    /// `max_elements` se acepta por compatibilidad y no se usa: ver
    /// `HNSW_CAPACITY_HINT` (#206).
    pub fn new(
        model: impl Into<String>,
        dimension: usize,
        _max_elements: usize,
    ) -> Self {
        let inner = Hnsw::<f32, DistCosine>::new(
            DEFAULT_MAX_NB_CONNECTION,
            HNSW_CAPACITY_HINT,
            DEFAULT_MAX_LAYER,
            DEFAULT_EF_CONSTRUCTION,
            DistCosine {},
        );
        // Sin `keep_pruned` (el default de hnsw_rs), a propósito (#201). Lo
        // activó #184 para que la poda no dejara puntos sin enlaces
        // entrantes, y a 3k puntos funcionaba; a escala hace lo contrario:
        // rellena cada lista de vecinos hasta 2M con sus candidatos más
        // cercanos, y el enlace inverso a cada punto nuevo, que se agrega a
        // esa lista llena y se poda por distancia, se descarta enseguida.
        // Medido a 100k × 384 tras `parallel_insert`: 32k puntos sin enlaces
        // entrantes con él, 0.8k sin él. Los que quedan van a los huérfanos
        // (`collect_orphans`).
        Self {
            inner,
            id_map: HashMap::new(),
            reverse_map: HashMap::new(),
            model: model.into(),
            dimension,
            next_data_id: 0,
            exact_store: Vec::new(),
            tombstones: 0,
            orphans: Vec::new(),
            dirty: true,
            loaded_in: None,
        }
    }

    /// Crea un índice con parámetros HNSW custom. `max_elements` no se usa:
    /// ver `HNSW_CAPACITY_HINT` (#206).
    pub fn with_params(
        model: impl Into<String>,
        dimension: usize,
        _max_elements: usize,
        max_nb_connection: usize,
        ef_construction: usize,
        max_layer: usize,
    ) -> Self {
        let inner = Hnsw::<f32, DistCosine>::new(
            max_nb_connection,
            HNSW_CAPACITY_HINT,
            max_layer,
            ef_construction,
            DistCosine {},
        );
        // Sin `keep_pruned`: ver `new` (#201).
        Self {
            inner,
            id_map: HashMap::new(),
            reverse_map: HashMap::new(),
            model: model.into(),
            dimension,
            next_data_id: 0,
            exact_store: Vec::new(),
            tombstones: 0,
            orphans: Vec::new(),
            dirty: true,
            loaded_in: None,
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
        drop(insert_data); // libera los préstamos sobre owned_vectors

        let node_ids: Vec<NodeId> = data_ids.iter().map(|data_id| index.id_map[data_id]).collect();
        let points: Vec<(NodeId, &[f32])> = node_ids
            .iter()
            .zip(&owned_vectors)
            .map(|(node_id, vector)| (*node_id, vector.as_slice()))
            .collect();
        index.collect_orphans(&points);
        drop(points);

        // Poblar el store del camino exacto solo si el índice queda bajo el
        // umbral (con N grande la copia sería memoria muerta: la lectura irá
        // por HNSW de todos modos).
        if nb_elements <= EXACT_SEARCH_THRESHOLD {
            index.exact_store = node_ids.into_iter().zip(owned_vectors).collect();
        }

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
        self.dirty = true;
        self.collect_orphans(&[(node_id, vector.as_slice())]);

        // Mantener el store exacto mientras estemos bajo el umbral; al
        // cruzarlo, liberarlo — la lectura pasa a HNSW y no hay vuelta atrás
        // (el índice no soporta remociones).
        if self.id_map.len() <= EXACT_SEARCH_THRESHOLD {
            self.exact_store.push((node_id, vector));
        } else if !self.exact_store.is_empty() {
            self.exact_store = Vec::new();
        }

        Ok(())
    }

    /// Busca cada uno de `points` (ya insertados) con su propio vector y
    /// pasa a los huérfanos los que no aparecen; devuelve cuántos.
    ///
    /// HNSW poda la lista de vecinos de cada nodo; un punto puede quedar con
    /// enlaces salientes pero sin entrantes, y entonces ninguna búsqueda lo
    /// alcanza aunque esté en el índice (#184). Se descartó reinsertarlo
    /// (#201): el punto vuelve a caer en la misma zona, sus vecinos tienen
    /// la lista llena de puntos más cercanos y el enlace inverso se poda
    /// otra vez (a 100k × 384, de 811 quedaban 749 tras cinco rondas), y
    /// cada intento deja una copia muerta en el grafo. Un huérfano no toca
    /// el grafo: la búsqueda lo compara directamente.
    ///
    /// Desde `PARALLEL_INSERT_THRESHOLD` consultas la verificación usa
    /// `parallel_search`.
    fn collect_orphans(&mut self, points: &[(NodeId, &[f32])]) -> usize {
        let mut found = 0;
        for block in points.chunks(REACHABILITY_CHUNK) {
            let queries: Vec<Vec<f32>> = block.iter().map(|(_, v)| v.to_vec()).collect();
            let results = if queries.len() >= PARALLEL_INSERT_THRESHOLD {
                self.inner.parallel_search(&queries, REACHABILITY_K, REACHABILITY_EF)
            } else {
                queries
                    .iter()
                    .map(|q| self.inner.search(q, REACHABILITY_K, REACHABILITY_EF))
                    .collect()
            };
            for ((node_id, vector), hits) in block.iter().zip(results) {
                let Some(&data_id) = self.reverse_map.get(node_id) else { continue };
                if !hits.iter().any(|n| n.d_id == data_id) {
                    self.orphans.push((*node_id, vector.to_vec()));
                    found += 1;
                }
            }
        }
        found
    }

    /// Inserta un lote de puntos de una vez (#175).
    ///
    /// Valida TODO antes de tocar el índice (dimensión de cada vector, ids
    /// repetidos dentro del lote): un lote inválido no deja el índice a
    /// medias. Los nodos que ya estaban se retiran primero (quedan como
    /// tombstone, igual que `remove` + `insert`). Desde
    /// `PARALLEL_INSERT_THRESHOLD` puntos inserta con `parallel_insert`, como
    /// `build_batch`: insertar 1000 vectores de 384 dims de uno en uno en un
    /// índice de 100k costaba ~9 ms cada uno. Después verifica que cada punto
    /// del lote sea alcanzable, igual que `insert` (#184, #201).
    pub fn insert_batch(&mut self, items: Vec<(NodeId, Vec<f32>)>) -> Result<(), NopalError> {
        let mut seen = std::collections::HashSet::with_capacity(items.len());
        for (node_id, vector) in &items {
            if vector.len() != self.dimension {
                return Err(NopalError::custom(format!(
                    "HnswIndex({}): node {} has dimension {}, expected {}",
                    self.model, node_id, vector.len(), self.dimension
                )));
            }
            if !seen.insert(*node_id) {
                return Err(NopalError::custom(format!(
                    "HnswIndex({}): node {} appears twice in the batch",
                    self.model, node_id
                )));
            }
        }
        for (node_id, _) in &items {
            self.remove(*node_id);
        }
        let mut data_ids = Vec::with_capacity(items.len());
        for (node_id, _) in &items {
            let data_id = self.next_data_id;
            self.next_data_id += 1;
            self.id_map.insert(data_id, *node_id);
            self.reverse_map.insert(*node_id, data_id);
            data_ids.push(data_id);
        }
        let insert_data: Vec<(&Vec<f32>, usize)> =
            items.iter().zip(&data_ids).map(|((_, v), &id)| (v, id)).collect();
        if insert_data.len() >= PARALLEL_INSERT_THRESHOLD {
            self.inner.parallel_insert(&insert_data);
        } else {
            for &(vector, data_id) in &insert_data {
                self.inner.insert((vector, data_id));
            }
        }
        drop(insert_data);
        self.dirty = true;
        // Como `insert`: cada punto del lote tiene que ser alcanzable (#184).
        let points: Vec<(NodeId, &[f32])> = items.iter().map(|(id, v)| (*id, v.as_slice())).collect();
        self.collect_orphans(&points);
        drop(points);
        // Mismo criterio que `insert`: el store exacto vive solo bajo el umbral.
        if self.id_map.len() <= EXACT_SEARCH_THRESHOLD {
            self.exact_store.extend(items);
        } else if !self.exact_store.is_empty() {
            self.exact_store = Vec::new();
        }
        Ok(())
    }

    /// Retira un punto del índice. Devuelve `false` si no estaba.
    ///
    /// Borrado lógico: `hnsw_rs` no elimina puntos del grafo, así que el
    /// `DataId` queda como tombstone (sigue en el recorrido, ya no se
    /// devuelve). Actualizar el vector de un nodo = `remove` + `insert`.
    pub fn remove(&mut self, node_id: NodeId) -> bool {
        let Some(data_id) = self.reverse_map.remove(&node_id) else { return false };
        self.id_map.remove(&data_id);
        if !self.exact_store.is_empty() {
            self.exact_store.retain(|(id, _)| *id != node_id);
        }
        self.orphans.retain(|(id, _)| *id != node_id);
        self.tombstones += 1;
        self.dirty = true;
        true
    }

    /// `true` si `node_id` está indexado (y no retirado).
    pub fn contains(&self, node_id: NodeId) -> bool {
        self.reverse_map.contains_key(&node_id)
    }

    /// Puntos retirados que siguen ocupando sitio en el grafo HNSW.
    pub fn tombstones(&self) -> usize {
        self.tombstones
    }

    /// Cuántos puntos vivos no alcanza el grafo y compara cada búsqueda por
    /// fuerza bruta (#201). Lo normal es menos del 1% tras un build.
    ///
    /// Tras construir o insertar, cada punto nuevo se busca con su propio
    /// vector; el que no aparece queda aquí. La garantía es la de #184: un
    /// punto recién insertado se encuentra. Una inserción posterior aún
    /// puede podar el último enlace que llegaba a un punto anterior, que
    /// entonces no es huérfano ni lo alcanza el grafo; medido: 0 de 10 000
    /// inserciones (384 dims).
    pub fn orphans(&self) -> usize {
        self.orphans.len()
    }

    /// Siempre 0 desde 0.6.11: la verificación ya no reinserta puntos.
    #[deprecated(since = "0.6.11", note = "la verificación ya no reinserta puntos; ver `orphans` (#201)")]
    pub fn repaired(&self) -> usize {
        0
    }

    /// `DataId` muertos que siguen en el grafo: los tombstones del llamador.
    pub fn dead_points(&self) -> usize {
        self.tombstones
    }

    /// `true` si el estado en memoria no está reflejado en un dump en disco
    /// (índice recién construido, o con inserciones/retiros desde el último
    /// dump). Ver [`crate::embeddings::persistence`].
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Cuánto tardó cargar este índice desde disco; `None` si se construyó
    /// desde los embeddings de storage.
    pub fn loaded_in(&self) -> Option<std::time::Duration> {
        self.loaded_in
    }

    pub(crate) fn mark_clean(&mut self) {
        self.dirty = false;
    }

    /// Rearma un índice a partir de un grafo HNSW recargado de disco y los
    /// mapas persistidos junto a él. Solo lo usa `persistence::load`, que ya
    /// verificó que el grafo y los mapas son coherentes entre sí.
    ///
    /// El índice queda en modo búsqueda y sin `exact_store`: un dump solo se
    /// escribe por encima de [`EXACT_SEARCH_THRESHOLD`], donde el camino
    /// exacto no aplica.
    // Un solo llamador (la carga del dump), que pasa campo a campo lo que
    // leyó del `.meta`: un struct intermedio solo movería la lista.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        mut inner: Hnsw<'static, f32, DistCosine>,
        model: String,
        dimension: usize,
        id_map: HashMap<usize, NodeId>,
        next_data_id: usize,
        tombstones: usize,
        orphans: Vec<(NodeId, Vec<f32>)>,
        loaded_in: std::time::Duration,
    ) -> Self {
        inner.set_searching_mode(true);
        let reverse_map = id_map.iter().map(|(d, n)| (*n, *d)).collect();
        Self {
            inner,
            id_map,
            reverse_map,
            model,
            dimension,
            next_data_id,
            exact_store: Vec::new(),
            tombstones,
            orphans,
            dirty: false,
            loaded_in: Some(loaded_in),
        }
    }

    /// `true` cuando los tombstones superan [`REBUILD_TOMBSTONE_RATIO`] de los
    /// puntos vivos (y al menos [`REBUILD_TOMBSTONE_MIN`]): cada búsqueda paga
    /// vecinos muertos y el recall se degrada; reconstruir desde storage
    /// devuelve un grafo limpio. Quien tiene el índice en caché decide cuándo.
    pub fn needs_rebuild(&self) -> bool {
        self.tombstones >= REBUILD_TOMBSTONE_MIN
            && self.tombstones as f64 > self.id_map.len() as f64 * REBUILD_TOMBSTONE_RATIO
    }

    /// `k` a pedir al grafo para devolver `k` vivos: los tombstones pueden
    /// ocupar hasta `tombstones` de los vecinos devueltos.
    fn k_with_tombstones(&self, k: usize) -> usize {
        k.saturating_add(self.tombstones).min(self.id_map.len().max(k))
    }

    /// Mezcla con `hits` (ordenados por distancia) los huérfanos que pasan
    /// `filter` y deja los `k` más cercanos, con el mismo desempate que
    /// [`rank_exact`].
    fn merge_orphans<F>(&self, query: &[f32], hits: &mut Vec<(NodeId, f32)>, k: usize, filter: F)
    where
        F: Fn(&NodeId) -> bool,
    {
        if self.orphans.is_empty() {
            return;
        }
        let seen: std::collections::HashSet<NodeId> = hits.iter().map(|(id, _)| *id).collect();
        let candidates = self
            .orphans
            .iter()
            .filter(|(id, _)| !seen.contains(id) && filter(id))
            .map(|(id, v)| (*id, v.as_slice()));
        hits.extend(rank_exact(query, candidates, k));
        hits.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        hits.truncate(k);
    }

    /// Busca los `k` nodos más cercanos al vector `query` en el espacio de embeddings.
    ///
    /// Retorna `Vec<(NodeId, f32)>` ordenado por distancia ascendente (más cercano primero).
    /// La distancia es coseno: 0 = idénticos, 1 = ortogonales, 2 = opuestos.
    ///
    /// Con `N ≤ EXACT_SEARCH_THRESHOLD` la búsqueda es exacta y determinista
    /// (scan lineal); por encima usa HNSW con `ef_search = DEFAULT_EF_SEARCH`.
    /// Para controlar `ef_search`, usar [`Self::search_knn_with_ef`].
    pub fn search_knn(
        &self,
        query: &[f32],
        k: usize,
    ) -> Result<Vec<(NodeId, f32)>, NopalError> {
        self.search_knn_with_ef(query, k, DEFAULT_EF_SEARCH)
    }

    /// Busca KNN con parámetro `ef_search` custom (controla calidad vs velocidad).
    ///
    /// `ef_search` solo tiene efecto cuando la búsqueda va por el grafo HNSW
    /// (`N > EXACT_SEARCH_THRESHOLD`); bajo el umbral el resultado es exacto
    /// y el parámetro es irrelevante.
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

        if self.uses_exact_path() {
            return Ok(self.search_exact(query, k));
        }
        // Los tombstones siguen en el grafo y ocupan vecinos: pedir de más y
        // descartar los retirados (no están en `id_map`).
        let k_eff = self.k_with_tombstones(k);
        let neighbors = self.inner.search(query, k_eff, ef_search.max(k_eff));
        let mut results = Vec::with_capacity(k);
        for neighbor in neighbors {
            if let Some(&node_id) = self.id_map.get(&neighbor.d_id) {
                results.push((node_id, neighbor.distance));
                if results.len() == k {
                    break;
                }
            }
        }
        self.merge_orphans(query, &mut results, k, |_| true);
        Ok(results)
    }

    /// `true` si la próxima búsqueda irá por el camino exacto (scan lineal).
    ///
    /// Requiere que el store cubra TODOS los puntos del índice: si por
    /// cualquier razón está incompleto (p. ej. un índice grande que nunca lo
    /// pobló), se cae al camino HNSW en vez de responder con resultados
    /// parciales.
    fn uses_exact_path(&self) -> bool {
        self.id_map.len() <= EXACT_SEARCH_THRESHOLD
            && self.exact_store.len() == self.id_map.len()
    }

    /// Top-k exacto por scan lineal con distancia coseno (la misma
    /// `DistCosine` de hnsw_rs, para que las distancias reportadas sean
    /// idénticas entre ambos caminos). Empates se rompen por NodeId
    /// ascendente — determinista entre corridas y entre builds.
    fn search_exact(&self, query: &[f32], k: usize) -> Vec<(NodeId, f32)> {
        rank_exact(
            query,
            self.exact_store.iter().map(|(id, v)| (*id, v.as_slice())),
            k,
        )
    }

    /// Busca KNN filtrando por un predicado sobre NodeId.
    ///
    /// Útil para combinar búsqueda vectorial con predicados de grafo:
    /// solo retorna vecinos cuyo NodeId pasa el filtro.
    ///
    /// Ver [`Self::search_knn_filtered_explained`] para el detalle de cómo se
    /// resolvió la búsqueda (camino, `ef_search` efectivo, underfill).
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
        Ok(self
            .search_knn_filtered_explained(query, k, ef_search, filter)?
            .hits)
    }

    /// Igual que [`Self::search_knn_filtered`], pero devuelve además cómo se
    /// resolvió la búsqueda.
    ///
    /// # Garantía
    ///
    /// Ningún NodeId que no pase `filter` aparece en el resultado — en los dos
    /// caminos, y con un post-filtro propio como red de seguridad
    /// independiente del filtro nativo del motor HNSW.
    ///
    /// # Caminos
    ///
    /// - **Exacto** (`N ≤ EXACT_SEARCH_THRESHOLD`): distancia contra todos los
    ///   puntos, filtro, top-k. Recall = 1.
    /// - **HNSW filtrado**: `Hnsw::search_filter` aplica el predicado DURANTE
    ///   el recorrido — los candidatos se exploran completos pero solo los
    ///   permitidos entran al heap de resultados. Si aun así se juntan menos
    ///   de `k`, se reintenta con `ef_search` multiplicado por
    ///   `EF_ESCALATION_FACTOR` hasta [`MAX_FILTERED_EF_SEARCH`].
    ///
    /// Se descartó el diseño anterior —pedir `k × 4` vecinos GLOBALES y
    /// filtrarlos después— porque con un predicado selectivo los vecinos
    /// permitidos simplemente no estaban entre esos candidatos: el resultado
    /// seguía siendo fail-closed, pero perdía recall silenciosamente. El
    /// filtro nativo existe en la API pública de `hnsw_rs` desde 0.3
    /// (`search_filter`); el comentario que decía lo contrario estaba obsoleto.
    ///
    /// # Cuándo NO usar este método
    ///
    /// Con un conjunto permitido chico dentro de un índice grande, el recorrido
    /// filtrado puede acercarse a explorar el grafo entero sin garantía de
    /// recall. Ahí conviene el camino exacto sobre el conjunto permitido
    /// ([`rank_exact`]), que es a la vez exacto y más barato; el índice no
    /// puede tomar esa decisión solo porque `filter` es opaco — no conoce la
    /// cardinalidad de lo permitido. `Graph::search_hybrid` sí la conoce y
    /// elige por ella.
    pub fn search_knn_filtered_explained<F>(
        &self,
        query: &[f32],
        k: usize,
        ef_search: usize,
        filter: F,
    ) -> Result<FilteredSearchOutcome, NopalError>
    where
        F: Fn(&NodeId) -> bool,
    {
        if query.len() != self.dimension {
            return Err(NopalError::custom(format!(
                "HnswIndex({}): query dimension {} != index dimension {}",
                self.model, query.len(), self.dimension
            )));
        }

        // Camino exacto: filtrar sobre TODOS los puntos y tomar top-k — sin
        // over-fetch, resultado exacto y determinista.
        if self.uses_exact_path() {
            let mut hits = self.search_exact(query, self.exact_store.len());
            hits.retain(|(node_id, _)| filter(node_id));
            hits.truncate(k);
            return Ok(FilteredSearchOutcome {
                underfilled: hits.len() < k,
                hits,
                path: FilteredSearchPath::Exact,
                ef_used: None,
                attempts: 1,
            });
        }

        // Camino HNSW con filtro nativo + escalada adaptativa de ef_search.
        let mut ef = ef_search.max(k).min(MAX_FILTERED_EF_SEARCH);
        let mut attempts = 0usize;
        let mut hits;
        loop {
            attempts += 1;
            // El filtro nativo razona sobre DataId; traducimos a NodeId.
            // Un DataId sin entrada en id_map no puede autorizarse.
            let by_data_id = |data_id: &usize| {
                self.id_map.get(data_id).is_some_and(&filter)
            };
            let neighbors = self
                .inner
                .search_filter(query, k, ef, Some(&by_data_id as &dyn FilterT));

            hits = Vec::with_capacity(neighbors.len());
            for neighbor in neighbors {
                if let Some(&node_id) = self.id_map.get(&neighbor.d_id) {
                    // Red de seguridad: el contrato "nada fuera del filtro"
                    // no debe depender del filtrado interno del motor.
                    if filter(&node_id) {
                        hits.push((node_id, neighbor.distance));
                    }
                }
            }
            hits.truncate(k);
            self.merge_orphans(query, &mut hits, k, &filter);

            if hits.len() >= k || ef >= MAX_FILTERED_EF_SEARCH {
                break;
            }
            ef = ef
                .saturating_mul(EF_ESCALATION_FACTOR)
                .min(MAX_FILTERED_EF_SEARCH);
        }

        Ok(FilteredSearchOutcome {
            underfilled: hits.len() < k,
            hits,
            path: FilteredSearchPath::HnswFiltered,
            ef_used: Some(ef),
            attempts,
        })
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

    pub(crate) fn orphan_points(&self) -> &[(NodeId, Vec<f32>)] {
        &self.orphans
    }

    /// Marca un punto vivo como huérfano, para probar la persistencia sin
    /// depender de qué puntos deja sueltos un build concreto.
    #[cfg(test)]
    pub(crate) fn force_orphan(&mut self, node_id: NodeId, vector: Vec<f32>) {
        assert!(self.contains(node_id));
        self.orphans.push((node_id, vector));
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
        let allowed = [id_b, id_c];
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

    /// Vectores pseudoaleatorios reproducibles para tests de camino exacto.
    fn seeded_vectors(n: usize, dim: usize, seed: u64) -> Vec<(Uuid, Vec<f32>)> {
        use rand::{Rng, SeedableRng};
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
        (0..n)
            .map(|_| {
                let v: Vec<f32> = (0..dim).map(|_| rng.gen_range(-1.0f32..1.0)).collect();
                (Uuid::new_v4(), v)
            })
            .collect()
    }

    #[test]
    fn test_small_batch_uses_exact_path() {
        let index = HnswIndex::build_batch(seeded_vectors(100, 4, 1), "test", 4).unwrap();
        assert!(index.uses_exact_path());
        assert_eq!(index.exact_store.len(), 100);
    }

    #[test]
    fn test_build_batch_above_threshold_uses_hnsw() {
        let n = EXACT_SEARCH_THRESHOLD + 1;
        let index = HnswIndex::build_batch(seeded_vectors(n, 4, 2), "test", 4).unwrap();
        assert!(!index.uses_exact_path());
        assert!(index.exact_store.is_empty(), "store no debe poblarse sobre el umbral");
        let results = index.search_knn(&[0.5, -0.5, 0.5, -0.5], 3).unwrap();
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_incremental_insert_crosses_threshold() {
        let mut index = HnswIndex::new("test", 4, EXACT_SEARCH_THRESHOLD + 8);
        let vectors = seeded_vectors(EXACT_SEARCH_THRESHOLD + 1, 4, 3);

        for (i, (id, v)) in vectors.into_iter().enumerate() {
            index.insert(id, v).unwrap();
            if i < EXACT_SEARCH_THRESHOLD {
                assert!(index.uses_exact_path(), "bajo el umbral debe usar camino exacto");
            }
        }
        // Al cruzar el umbral: HNSW y store liberado.
        assert_eq!(index.len(), EXACT_SEARCH_THRESHOLD + 1);
        assert!(!index.uses_exact_path());
        assert!(index.exact_store.is_empty(), "el store debe liberarse al cruzar el umbral");

        // Y sigue respondiendo vía HNSW sin rebuild.
        let results = index.search_knn(&[0.1, 0.2, 0.3, 0.4], 5).unwrap();
        assert_eq!(results.len(), 5);
    }

    /// Benchmark manual (no corre en CI):
    /// `cargo test --features embeddings-index bench_exact_vs_hnsw_n1000 -- --ignored --nocapture`
    #[test]
    #[ignore = "benchmark manual: correr con --ignored --nocapture"]
    fn bench_exact_vs_hnsw_n1000() {
        use std::time::Instant;

        let dim = 64;
        let n = 1000;
        let vectors = seeded_vectors(n, dim, 42);
        let query = vectors[500].1.clone();
        let index = HnswIndex::build_batch(vectors, "bench", dim).unwrap();
        assert!(index.uses_exact_path());

        let iters = 200u32;
        // Warmup
        for _ in 0..20 {
            let _ = index.search_exact(&query, 10);
            let _ = index.inner.search(&query, 10, DEFAULT_EF_SEARCH);
        }

        let t0 = Instant::now();
        for _ in 0..iters {
            let _ = index.search_exact(&query, 10);
        }
        let exact = t0.elapsed();

        let t1 = Instant::now();
        for _ in 0..iters {
            let _ = index.inner.search(&query, 10, DEFAULT_EF_SEARCH);
        }
        let hnsw = t1.elapsed();

        println!(
            "N={n} dim={dim} k=10 iters={iters}: exacto={:?} ({:?}/query) vs hnsw(ef={})={:?} ({:?}/query)",
            exact,
            exact / iters,
            DEFAULT_EF_SEARCH,
            hnsw,
            hnsw / iters,
        );
    }
}
