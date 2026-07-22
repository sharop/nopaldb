// src/lib.rs

pub mod error;
pub mod types;
pub mod storage;
pub mod graph;
pub mod query;
pub mod transaction;
pub mod traversal;
pub mod planner;

pub mod wal;
pub mod schema;
pub mod index;
#[cfg(feature = "algorithms")]
pub mod algorithms;

#[cfg(feature = "embeddings")]
pub mod embeddings;

#[doc(hidden)]
pub mod easter_eggs;


#[cfg(feature = "full-isolation")]
pub mod lock_manager;

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
pub use shacl::{ShaclValidator, Shape, ValidationReport, ConstraintViolation};

// ML integrations (feature-gated)
#[cfg(feature = "ml")]
pub mod ml;

#[cfg(feature = "ml")]
pub use ml::PyGData;

// Re-exports
pub use error::{NopalError, Result};
pub use types::{Node, Edge, NodeId, EdgeId, PropertyValue, Properties};
pub use storage::Storage;
pub use storage::{StorageBackend, StorageEngine, StorageOptions, StorageProfile, StorageTuning};
pub use graph::{Graph, Direction, BulkLoader, BulkLoadStats, AutoGcConfig, AutoGcStatus, GraphView, Subgraph, LinkSpec, UpsertOutcome, UpsertRequest};
#[cfg(feature = "hybrid")]
pub use graph::{HybridFilter, HybridHit, HybridQuery};
pub use traversal::{TraversalResult, TraversalConfig, NodeFilter};
pub use query::TraverseBuilder;
pub use transaction::Transaction;

#[cfg(feature = "full-isolation")]
pub use transaction::IsolationLevel;

#[cfg(feature = "full-isolation")]
pub use lock_manager::{LockManager, LockType};

#[cfg(feature = "analytics")]
pub use arrow::record_batch::RecordBatch;

pub use query::nql::parse;
pub use query::nql::parse_query;
pub use query::nql::parser::ast::Query as NQLQuery;
pub use query::nql::parser::ast::Statement as NQLStatement;
pub use query::nql::Executor;
pub use query::nql::{NqlResult, ProfileResult, WriteResult};

// Python bindings (feature-gated)
#[cfg(feature = "python")]
pub mod python;

#[cfg(feature = "python")]
pub use python::*;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
