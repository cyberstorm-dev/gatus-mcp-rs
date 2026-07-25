use gatus_mcp_rs::cli::{Cli, Commands};
use gatus_mcp_rs::run_app;
use gatus_mcp_rs::settings::Settings;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_run_app_list_tools() {
    let mock_server = MockServer::start().await;

    let cli = Cli {
        config: None,
        gatus_url: Some(mock_server.uri()),
        api_key: None,
        log_level: "info".to_string(),
        log_format: "text".to_string(),
        read_only: false,
        command: Some(Commands::ListTools),
    };

    run_app(cli).await.unwrap();
}

#[tokio::test]
async fn test_run_app_call_tool() {
    let mock_server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/v1/endpoints/statuses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&mock_server)
        .await;

    let cli = Cli {
        config: None,
        gatus_url: Some(mock_server.uri()),
        api_key: None,
        log_level: "info".to_string(),
        log_format: "text".to_string(),
        read_only: false,
        command: Some(Commands::CallTool {
            name: "manage_resources".to_string(),
            arguments: "{\"action\": \"list-endpoints\"}".to_string(),
        }),
    };

    run_app(cli).await.unwrap();
}

#[tokio::test]
async fn test_run_app_http() {
    let mut settings = Settings::new().unwrap();
    settings.gatus.api_url = "http://localhost:8080".to_string();

    let cli = Cli {
        config: None,
        gatus_url: Some("http://localhost:8080".to_string()),
        api_key: None,
        log_level: "info".to_string(),
        log_format: "text".to_string(),
        read_only: false,
        command: Some(Commands::Http {
            port: 8082,
            host: "127.0.0.1".to_string(),
        }),
    };

    let handle = tokio::spawn(async move { run_app(cli).await });

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    handle.abort();
}

#[tokio::test]
async fn test_run_app_with_stdio_wrapper() {
    let mock_server = MockServer::start().await;

    let cli = Cli {
        config: None,
        gatus_url: Some(mock_server.uri()),
        api_key: None,
        log_level: "info".to_string(),
        log_format: "text".to_string(),
        read_only: false,
        command: Some(Commands::Stdio),
    };

    let reader = tokio::io::empty();
    let writer = tokio::io::sink();

    gatus_mcp_rs::run_app_with_stdio(cli, reader, writer)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_run_app_stdio_json_log() {
    let mock_server = MockServer::start().await;

    let cli = Cli {
        config: None,
        gatus_url: Some(mock_server.uri()),
        api_key: None,
        log_level: "info".to_string(),
        log_format: "json".to_string(),
        read_only: false,
        command: Some(Commands::Stdio),
    };

    let reader = tokio::io::empty();
    let writer = tokio::io::sink();

    gatus_mcp_rs::run_app_with_stdio(cli, reader, writer)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_run_app_stdio_minimal() {
    let mock_server = MockServer::start().await;

    let _cli = Cli {
        config: None,
        gatus_url: Some(mock_server.uri()),
        api_key: None,
        log_level: "info".to_string(),
        log_format: "json".to_string(),
        read_only: false,
        command: Some(Commands::Stdio),
    };
}
