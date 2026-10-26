// src/embeddings/mod.rs

pub mod node;
pub mod edge;
pub mod path_reference;
#[cfg(feature = "embeddings-index")]
pub(crate) mod hnsw_rs;
#[cfg(feature = "embeddings-index")]
pub mod index;
#[cfg(feature = "embeddings-index")]
pub mod persistence;

pub use node::Embedding;
pub use edge::EdgeEmbedding;
pub use path_reference::PathReferenceEmbedding;
#[cfg(feature = "embeddings-index")]
pub use index::HnswIndex;
#[cfg(feature = "embeddings-index")]
pub use index::{DEFAULT_EF_SEARCH, EXACT_SEARCH_THRESHOLD, MAX_FILTERED_EF_SEARCH, REBUILD_TOMBSTONE_MIN, REBUILD_TOMBSTONE_RATIO};
#[cfg(feature = "embeddings-index")]
pub use index::{exact_over_allowed_limit, rank_exact, FilteredSearchOutcome, FilteredSearchPath, EXACT_OVER_ALLOWED_FACTOR};
#[cfg(feature = "embeddings-index")]
pub use index::EmbeddingIndex; // backward-compatible alias