// src/lib.rs

pub mod error;
pub mod graph;
pub mod planner;
pub mod query;
pub mod storage;
pub mod transaction;
pub mod traversal;
pub mod types;

#[cfg(feature = "algorithms")]
pub mod algorithms;
pub mod index;
pub mod schema;
pub mod wal;

#[cfg(feature = "embeddings")]
pub mod embeddings;

#[doc(hidden)]
pub mod easter_eggs;

#[cfg(feature = "analytics")]
pub mod arrow_export;
pub mod mvcc;

// RDF está como referencia
pub mod rdf_owl;

// OWL-EL reasoner (feature-gated)
#[cfg(feature = "reasoner")]
pub mod reasoner;

#[cfg(feature = "reasoner")]
pub use reasoner::{Axiom, CompletionRule, ELReasoner, Inference};

// SHACL Core validator (feature-gated)
#[cfg(feature = "shacl")]
pub mod shacl;

#[cfg(feature = "shacl")]
pub use shacl::{ConstraintViolation, ShaclValidator, Shape, ValidationReport};

// ML integrations (feature-gated)
#[cfg(feature = "ml")]
pub mod ml;

#[cfg(feature = "ml")]
pub use ml::PyGData;

// Re-exports
pub use error::{NopalError, Result};
pub use graph::{
    AutoGcConfig, AutoGcStatus, BulkLoadStats, BulkLoader, Direction, Graph, GraphView, Subgraph,
};
pub use query::TraverseBuilder;
pub use storage::Storage;
pub use storage::{StorageBackend, StorageEngine, StorageOptions, StorageProfile, StorageTuning};
pub use transaction::Transaction;
pub use traversal::{NodeFilter, TraversalConfig, TraversalResult};
pub use types::{Edge, EdgeId, Node, NodeId, Properties, PropertyValue};

#[cfg(feature = "analytics")]
pub use arrow::record_batch::RecordBatch;

pub use query::nql::Executor;
pub use query::nql::parse;
pub use query::nql::parse_query;
pub use query::nql::parser::ast::Query as NQLQuery;
pub use query::nql::parser::ast::Statement as NQLStatement;
pub use query::nql::{NqlResult, ProfileResult, WriteResult};

// Python bindings (feature-gated)
#[cfg(feature = "python")]
pub mod python;

#[cfg(feature = "python")]
pub use python::*;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
