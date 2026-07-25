pub mod cli;
pub mod client;
pub mod fmt;
pub mod mcp;
pub mod server;
pub mod settings;

use crate::cli::{Cli, Commands};
use crate::client::GatusClient;
use crate::mcp::{AccessMode, McpHandler};
use crate::server::{run_http_server_with_access_mode, run_server_loop};
use crate::settings::Settings;
use tokio::io::{self, AsyncWrite};
use tracing_subscriber::{fmt as trace_fmt, prelude::*, EnvFilter};

pub async fn run_app(cli: Cli) -> anyhow::Result<()> {
    run_app_with_stdio(cli, io::stdin(), io::stdout()).await
}

pub async fn run_app_with_stdio<R, W>(cli: Cli, reader: R, writer: W) -> anyhow::Result<()>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let access_mode = if cli.read_only {
        AccessMode::ReadOnly
    } else {
        AccessMode::ReadWrite
    };

    // Initialize logging
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&cli.log_level));

    let registry = tracing_subscriber::registry().with(filter);

    let _ = if cli.log_format == "json" {
        registry
            .with(trace_fmt::layer().json().with_writer(std::io::stderr))
            .try_init()
    } else {
        registry
            .with(trace_fmt::layer().with_writer(std::io::stderr))
            .try_init()
    };
    tracing::info!("read-only mode: {}", access_mode.label());

    // Load settings
    let mut settings = Settings::new()?;

    // Override settings with CLI flags if provided
    if let Some(url) = cli.gatus_url {
        settings.gatus.api_url = url;
    }
    if let Some(key) = cli.api_key {
        settings.gatus.api_key = Some(key);
    }

    match cli.command.unwrap_or(Commands::Stdio) {
        Commands::Stdio => {
            tracing::info!(
                "Starting gatus-mcp-rs v{} in stdio mode",
                env!("CARGO_PKG_VERSION")
            );
            let client = GatusClient::new(
                settings.gatus.api_url,
                settings.gatus.api_key,
                settings.gatus.username,
                settings.gatus.password,
            );
            let handler = McpHandler::new_with_access_mode(client, access_mode);
            run_server_loop(handler, io::BufReader::new(reader), writer).await?;
        }
        Commands::Http { port, host } => {
            run_http_server_with_access_mode(settings, port, host, access_mode).await?;
        }
        Commands::ListTools => {
            let client = GatusClient::new(
                settings.gatus.api_url,
                settings.gatus.api_key,
                settings.gatus.username,
                settings.gatus.password,
            );
            let handler = McpHandler::new_with_access_mode(client, access_mode);
            let response = handler
                .handle(serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "tools/list",
                    "id": 1
                }))
                .await;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
        Commands::CallTool { name, arguments } => {
            let client = GatusClient::new(
                settings.gatus.api_url,
                settings.gatus.api_key,
                settings.gatus.username,
                settings.gatus.password,
            );
            let handler = McpHandler::new_with_access_mode(client, access_mode);
            let args: serde_json::Value = serde_json::from_str(&arguments)?;
            let response = handler
                .handle(serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "tools/call",
                    "params": {
                        "name": name,
                        "arguments": args
                    },
                    "id": 1
                }))
                .await;
            println!("{}", serde_json::to_string_pretty(&response)?);
        }
    }

    Ok(())
}
