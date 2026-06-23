// src/query/nql/mod.rs
//
// Nopal Query Language (NQL)

pub mod executor;
pub mod parser;
mod validator;

// Re-exports
pub use executor::Executor;
pub use executor::result::{NqlResult, ProfileResult, QueryResult, Row, WriteResult};
pub use parser::ast::{Query, Statement};
pub use parser::{parse, parse_query};
pub use validator::{SemanticValidator, ValidationWarning};
