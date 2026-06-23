// nopaldb-mcp — NopalDB MCP Server (stdio transport)
//
// Usage:
//   nopaldb-mcp --db /path/to/graph.db [--readonly] [--log-queries]
//
// Claude Desktop config (~/.config/claude/claude_desktop_config.json):
//   {
//     "mcpServers": {
//       "nopaldb": {
//         "command": "nopaldb-mcp",
//         "args": ["--db", "/path/to/graph.db", "--readonly"]
//       }
//     }
//   }
use std::sync::Arc;

use clap::Parser;
use nopaldb::Graph;
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::{self, EnvFilter};
mod server;
mod tools;

#[derive(Parser, Debug)]
#[command(
    name = "nopaldb-mcp",
    version,
    about = "NopalDB MCP Server — exposes graph queries as MCP tools"
)]
struct Cli {
    /// Path to the NopalDB database directory
    #[arg(long, default_value = "nopaldb.db")]
    db: String,

    /// Open database in read-only mode (disables ADD/UPDATE/DELETE/CREATE/DROP)
    #[arg(long)]
    readonly: bool,

    /// Log NQL queries to stderr (queries may contain sensitive data)
    #[arg(long)]
    log_queries: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Cli::parse();

    // All logging goes to stderr — stdout is reserved for the MCP stdio protocol.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    tracing::info!(db = %args.db, readonly = args.readonly, "Opening NopalDB");
    let graph = Graph::open(&args.db).await?;
    let graph = Arc::new(graph);

    let srv = server::NopalMcpServer::new(graph, args.readonly, args.log_queries);

    tracing::info!("NopalDB MCP server ready (stdio)");
    let service = srv.serve(stdio()).await.inspect_err(|e| {
        tracing::error!("MCP transport error: {:?}", e);
    })?;
    service.waiting().await?;
    Ok(())
}
