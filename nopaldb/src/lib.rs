//! NopalDB: an embedded property graph with native embeddings, MVCC
//! transactions and its own query language (NQL).
//!
//! One process opens a directory and owns it; everything else — nodes, edges,
//! full-text and vector indexes, transactions, the write-ahead log — lives in
//! that directory. There is no server to run.
//!
//! # Five-minute start
//!
//! ```toml
//! [dependencies]
//! nopaldb = { version = "0.5", features = ["core"] }
//! ```
//!
//! ```rust,no_run
//! use nopaldb::{Edge, Graph, Node, PropertyValue};
//!
//! #[tokio::main]
//! async fn main() -> nopaldb::Result<()> {
//!     let graph = Graph::open("./data.db").await?;
//!
//!     let mut tx = graph.begin_transaction().await?;
//!     let a = tx.add_node(Node::new("Person")
//!         .with_property("name", PropertyValue::String("Alice".into()))).await?;
//!     let b = tx.add_node(Node::new("Person")
//!         .with_property("name", PropertyValue::String("Bob".into()))).await?;
//!     tx.add_edge(Edge::new(a, b, "KNOWS"))?;
//!     tx.commit().await?;
//!
//!     let result = graph.execute_nql("find p.name from (p:Person)").await?;
//!     for row in result.rows() {
//!         println!("{:?}", row.get("p.name"));
//!     }
//!     Ok(())
//! }
//! ```
//!
//! # Feature tiers
//!
//! | Tier | What you get |
//! |------|--------------|
//! | *default* | Property graph + NQL + MVCC + WAL (sled storage) |
//! | `core` | + Arrow/Parquet export, graph algorithms, embeddings + HNSW, full-text search, ML helpers |
//! | `semantic` | + OWL-EL reasoner, Turtle import/export, SHACL validation |
//! | `full` | + `full-isolation`: isolation levels, per-node lock manager, deadlock detection |
//!
//! These docs are built with `full`, so every feature-gated module is listed;
//! an item's `cfg` badge says which feature you need.
//!
//! # Where to go next
//!
//! - [`Graph`] is the entry point: open, transactions, upsert, search, export.
//! - [`Graph::execute_nql`] runs NQL; the language reference lives in the
//!   repository under `docs/en/NQL_REFERENCE.md`.
//! - [`rdf_owl`] documents exactly what the Turtle bridge keeps and loses.
//! - Operational rules (one process per directory, durability, isolation,
//!   bulk loading) are in `docs/ADOPTION.md` in the repository.

// El storage necesita exactamente un motor KV compilado. Sin esto, un build
// sin backend produce cientos de errores crípticos en storage/ en vez de uno
// accionable. (Cuando exista más de un backend, esto pasa a `not(any(...))`.)
#[cfg(not(any(feature = "storage-sled", feature = "storage-redb")))]
compile_error!(
    "NopalDB requires a storage backend: enable `storage-sled` (default) or `storage-redb`."
);

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
pub use shacl::{ShaclValidator, Shape, ValidationReport, ConstraintViolation, ShapesReport, TargetMode};

// ML integrations (feature-gated)
#[cfg(feature = "ml")]
pub mod ml;

#[cfg(feature = "ml")]
pub use ml::PyGData;

// Re-exports
pub use error::{NopalError, Result, StorageError, StorageErrorKind};
pub use types::{Node, Edge, NodeId, EdgeId, PropertyValue, Properties};
pub use storage::{MigrationReport, Storage};
pub use storage::{StorageEngine, StorageOptions, StorageProfile, StorageTuning};
pub use graph::{Graph, Direction, BulkLoader, BulkLoadStats, AutoGcConfig, AutoGcStatus, GraphView, Subgraph, LinkSpec, UpsertOutcome, UpsertRequest};
#[cfg(feature = "hybrid")]
pub use graph::{
    BranchReport, ExplainedHit, HybridExplain, HybridFilter, HybridHit, HybridQuery, VectorPath,
};
pub use traversal::{TraversalResult, TraversalConfig, NodeFilter};
pub use query::{TraverseBuilder, SackBuilder, SackBlock, SackResult, SackItem, CycleMode, Truncation};
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
