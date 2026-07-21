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
mod auth;

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

    /// Interface to bind for SSE transport. Defaults to loopback; set a
    /// non-loopback address (e.g. 0.0.0.0) only together with an auth token.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Bearer token required for SSE requests. Overridden by the
    /// NOPALDB_MCP_TOKEN environment variable when set.
    #[arg(long)]
    auth_token: Option<String>,

    /// Allowed CORS origin for SSE (repeatable). Empty = same-origin only;
    /// pass "*" to allow any origin (not recommended).
    #[arg(long = "cors-origin")]
    cors_origin: Vec<String>,

    /// Allow binding a non-loopback interface without an auth token.
    /// Unsafe: exposes the database to the network unauthenticated.
    #[arg(long)]
    insecure_no_auth: bool,
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
            use rmcp::transport::streamable_http_server::{
                session::local::LocalSessionManager,
                tower::{StreamableHttpServerConfig, StreamableHttpService},
            };

            // Resolve the bearer token: env var wins over the CLI flag.
            let token = std::env::var("NOPALDB_MCP_TOKEN")
                .ok()
                .filter(|t| !t.is_empty())
                .or_else(|| args.auth_token.clone().filter(|t| !t.is_empty()));

            let loopback = auth::is_loopback(&args.host);

            // Fail-closed: never expose the database to the network without a
            // token unless the operator explicitly opts out.
            if !loopback && token.is_none() && !args.insecure_no_auth {
                eprintln!(
                    "refusing to bind non-loopback host {host} without authentication.\n\
                     Set NOPALDB_MCP_TOKEN (or --auth-token) to require a bearer token,\n\
                     or pass --insecure-no-auth to expose the database unauthenticated.",
                    host = args.host
                );
                std::process::exit(1);
            }
            if !loopback && token.is_none() {
                tracing::warn!(
                    host = %args.host,
                    "SSE transport bound to a non-loopback interface WITHOUT authentication (--insecure-no-auth)"
                );
            }

            tracing::info!(
                host = %args.host,
                port = args.port,
                auth = token.is_some(),
                "NopalDB MCP server ready (SSE)"
            );

            let config = StreamableHttpServerConfig::default();
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

            let mut app = Router::new().fallback_service(mcp_service);

            // Require a bearer token on every request when one is configured.
            if let Some(tok) = token {
                app = app.layer(axum::middleware::from_fn_with_state(
                    Arc::new(tok),
                    auth::require_bearer,
                ));
            }

            // Restrict CORS to the configured origins (no layer = same-origin).
            if let Some(cors) = auth::cors_layer(&args.cors_origin) {
                app = app.layer(cors);
            }

            let addr = format!("{}:{}", args.host, args.port);
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
