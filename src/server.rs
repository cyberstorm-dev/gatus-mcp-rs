use crate::cli::AppState;
use crate::client::GatusClient;
use crate::mcp::{AccessMode, McpHandler};
use crate::settings::Settings;
use axum::{
    extract::State,
    response::sse::{Event, Sse},
    routing::{get, post},
    Json, Router,
};
use futures::stream::Stream;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::io::{self, AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio_stream::StreamExt as _;

pub fn create_app(settings: Settings) -> Router {
    create_app_with_access_mode(settings, AccessMode::ReadWrite)
}

pub fn create_app_with_access_mode(settings: Settings, access_mode: AccessMode) -> Router {
    let gatus_client = GatusClient::new(
        settings.gatus.api_url.clone(),
        settings.gatus.api_key.clone(),
        settings.gatus.username.clone(),
        settings.gatus.password.clone(),
    );
    let mcp_handler = McpHandler::new_with_access_mode(gatus_client.clone(), access_mode);
    let (tx, _) = broadcast::channel(100);

    let state = AppState {
        mcp_handler: Arc::new(mcp_handler),
        notification_sender: tx.clone(),
    };

    // Start background polling task
    let gatus_client_clone = Arc::new(gatus_client);
    let interval = settings.server.polling_interval;
    tokio::spawn(background_polling_task(gatus_client_clone, tx, interval));

    Router::new()
        .route("/sse", get(sse_handler))
        .route("/messages", post(messages_handler))
        .with_state(state)
}

async fn background_polling_task(
    client: Arc<GatusClient>,
    tx: broadcast::Sender<Value>,
    interval_secs: u64,
) {
    let mut last_statuses = std::collections::HashMap::new();
    let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));

    loop {
        interval.tick().await;
        tracing::debug!("Polling Gatus for state changes...");

        match client.list_services(true, None).await {
            Ok(services) => {
                for service in services {
                    let current_status = service.display_status();
                    let key = service.get_key();

                    if let Some(last_status) = last_statuses.get(&key) {
                        if *last_status != current_status {
                            tracing::info!(
                                "Service {} state changed: {} -> {}",
                                key,
                                last_status,
                                current_status
                            );

                            let notification = json!({
                                "jsonrpc": "2.0",
                                "method": "notifications/resources/updated",
                                "params": {
                                    "uri": "gatus://dashboard/status",
                                    "description": format!("Service '{}' in group '{}' is now {}", service.name, service.group, current_status)
                                }
                            });

                            let _ = tx.send(notification);
                        }
                    }
                    last_statuses.insert(key, current_status);
                }
            }
            Err(_) => tracing::error!("Failed to poll Gatus for state changes"),
        }
    }
}

pub async fn run_stdio_server(handler: McpHandler) -> anyhow::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    run_server_loop(handler, io::BufReader::new(stdin), stdout).await
}

pub async fn run_server_loop<R, W>(
    handler: McpHandler,
    mut reader: R,
    mut writer: W,
) -> anyhow::Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut line = String::new();

    tracing::info!("Ready to receive MCP messages");

    while reader.read_line(&mut line).await? > 0 {
        let request: Value = match serde_json::from_str(&line) {
            Ok(v) => {
                tracing::debug!("Received request: {}", v);
                v
            }
            Err(e) => {
                tracing::error!("Failed to parse JSON: {}", e);
                line.clear();
                continue;
            }
        };

        let response = handler.handle(request).await;
        let response_str = serde_json::to_string(&response)?;
        tracing::debug!("Sending response: {}", response_str);
        writer.write_all(response_str.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
        line.clear();
    }

    Ok(())
}

pub async fn run_http_server(settings: Settings, port: u16, host: String) -> anyhow::Result<()> {
    run_http_server_with_access_mode(settings, port, host, AccessMode::ReadWrite).await
}

pub async fn run_http_server_with_access_mode(
    settings: Settings,
    port: u16,
    host: String,
    access_mode: AccessMode,
) -> anyhow::Result<()> {
    let app = create_app_with_access_mode(settings, access_mode);
    let addr = format!("{}:{}", host, port).parse::<SocketAddr>()?;
    tracing::info!("Listening on {}", addr);
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn sse_handler(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.notification_sender.subscribe();

    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).map(|msg| match msg {
        Ok(v) => Ok(Event::default().json_data(v).unwrap()),
        Err(_) => Ok(Event::default().data("error")),
    });

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

async fn messages_handler(
    State(state): State<AppState>,
    Json(payload): Json<Value>,
) -> Json<Value> {
    let response = state.mcp_handler.handle(payload).await;
    Json(response)
}
