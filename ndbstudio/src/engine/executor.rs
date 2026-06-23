// Query executor
// Will interface with NopalDB to execute NQL queries

use anyhow::Result;

pub struct QueryExecutor {
    // TODO: Hold reference to NopalDB instance
}

impl QueryExecutor {
    pub fn new() -> Self {
        Self {}
    }

    pub fn execute(&self, _query: &str) -> Result<QueryResult> {
        // TODO: Execute query through NopalDB
        Ok(QueryResult::default())
    }
}

#[derive(Default)]
pub struct QueryResult {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub execution_time_ms: f64,
}
