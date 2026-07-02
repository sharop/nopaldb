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
mod ipc_export;

#[derive(Parser, Debug)]
#[command(
    name = "nopaldb-mcp",
    version,
    about = "NopalDB MCP Server — exposes graph queries as MCP tools",
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

    /// Transport mode: "stdio" or "sse"
    #[arg(long, default_value = "stdio")]
    transport: String,

    /// Port for SSE transport
    #[arg(long, default_value = "8080")]
    port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Cli::parse();

    // All logging goes to stderr — stdout is reserved for the MCP stdio protocol.
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    tracing::info!(db = %args.db, readonly = args.readonly, "Opening NopalDB");
    let graph = Graph::open(&args.db).await?;
    let graph = Arc::new(graph);

    let srv = server::NopalMcpServer::new(graph, args.readonly, args.log_queries);

    match args.transport.as_str() {
        "stdio" => {
            tracing::info!("NopalDB MCP server ready (stdio)");
            let service = srv.serve(stdio()).await.inspect_err(|e| {
                tracing::error!("MCP transport error: {:?}", e);
            })?;
            service.waiting().await?;
        }
        "sse" => {
            use axum::Router;
            use tower_http::cors::{Any, CorsLayer};
            use rmcp::transport::streamable_http_server::{
                session::local::LocalSessionManager,
                tower::{StreamableHttpServerConfig, StreamableHttpService},
            };

            tracing::info!("NopalDB MCP server ready (SSE on port {})", args.port);
            
            let config = StreamableHttpServerConfig::default().disable_allowed_hosts();
            let session_manager = Arc::new(LocalSessionManager::default());
            
            let service_factory = {
                let srv = srv.clone();
                move || Ok(srv.clone())
            };
            
            let mcp_service = StreamableHttpService::new(
                service_factory,
                session_manager,
                config,
            );

            let cors = CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any);

            let app = Router::new()
                .fallback_service(mcp_service)
                .layer(cors);

            let addr = format!("0.0.0.0:{}", args.port);
            let listener = tokio::net::TcpListener::bind(&addr).await?;
            axum::serve(listener, app).await?;
        }
        _ => {
            tracing::error!("Unknown transport: {}", args.transport);
            std::process::exit(1);
        }
    }

    Ok(())
}
