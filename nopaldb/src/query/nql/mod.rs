// src/query/nql/mod.rs
//
// Nopal Query Language (NQL)

pub mod parser;
pub mod executor;
mod validator;

// Re-exports
pub use parser::ast::{Query, Statement};
pub use parser::{parse, parse_query};
pub use executor::Executor;
pub use executor::result::{ProfileResult, QueryResult, Row, NqlResult, WriteResult};
pub use validator::{SemanticValidator, ValidationWarning};
