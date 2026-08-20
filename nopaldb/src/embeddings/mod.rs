// src/embeddings/mod.rs

pub mod node;
pub mod edge;
pub mod path_reference;
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
pub use index::{DEFAULT_EF_SEARCH, EXACT_SEARCH_THRESHOLD, MAX_FILTERED_EF_SEARCH};
pub use index::{rank_exact, FilteredSearchOutcome, FilteredSearchPath};
#[cfg(feature = "embeddings-index")]
pub use index::EmbeddingIndex; // backward-compatible alias