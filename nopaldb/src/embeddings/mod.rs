// src/embeddings/mod.rs

pub mod edge;
#[cfg(feature = "embeddings-index")]
pub mod index;
pub mod node;
pub mod path_reference;
#[cfg(feature = "embeddings-index")]
pub mod persistence;

pub use edge::EdgeEmbedding;
#[cfg(feature = "embeddings-index")]
pub use index::EmbeddingIndex;
#[cfg(feature = "embeddings-index")]
pub use index::HnswIndex;
pub use node::Embedding;
pub use path_reference::PathReferenceEmbedding; // backward-compatible alias
