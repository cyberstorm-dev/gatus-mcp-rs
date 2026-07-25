use crate::mcp::McpHandler;
use clap::{Parser, Subcommand};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub mcp_handler: Arc<McpHandler>,
    pub notification_sender: tokio::sync::broadcast::Sender<serde_json::Value>,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Config file path
    #[arg(short, long, env = "GATUS_CONFIG_FILE")]
    pub config: Option<String>,

    /// Gatus API base URL
    #[arg(long, env = "GATUS_API_URL")]
    pub gatus_url: Option<String>,

    /// Gatus API Key
    #[arg(long, env = "GATUS_API_KEY")]
    pub api_key: Option<String>,

    /// Logging level
    #[arg(short, long, env = "LOG_LEVEL", default_value = "info")]
    pub log_level: String,

    /// Log format (text or json)
    #[arg(long, env = "LOG_FORMAT", default_value = "text")]
    pub log_format: String,

    /// Disable tools and actions that mutate Gatus
    #[arg(long, env = "GATUS_MCP_READ_ONLY", global = true)]
    pub read_only: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run the MCP server over Stdio (default)
    Stdio,
    /// Run the MCP server over HTTP/SSE
    Http {
        /// Port to listen on
        #[arg(short, long, env = "GATUS_SERVER_PORT", default_value_t = 8080)]
        port: u16,
        /// Host to bind to
        #[arg(long, env = "GATUS_SERVER_HOST", default_value = "127.0.0.1")]
        host: String,
    },
    /// List available tools
    ListTools,
    /// Call a tool directly
    CallTool {
        /// Name of the tool to call
        name: String,
        /// Tool arguments as JSON
        #[arg(default_value = "{}")]
        arguments: String,
    },
}
