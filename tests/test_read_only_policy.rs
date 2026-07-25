use gatus_mcp_rs::client::GatusClient;
use gatus_mcp_rs::mcp::{AccessMode, McpHandler, READ_ONLY_ERROR_MESSAGE};
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn read_only_handler(url: &str) -> McpHandler {
    let client = GatusClient::new(url.to_string(), None, None, None);
    McpHandler::new_with_access_mode(client, AccessMode::ReadOnly)
}

fn tool_names(response: &Value) -> Vec<&str> {
    response["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn read_only_list_tools_exposes_only_safe_tools() {
    let handler = read_only_handler("http://localhost");
    let response = handler
        .handle(json!({
            "jsonrpc": "2.0",
            "method": "tools/list",
            "id": 1
        }))
        .await;

    let tools = response["result"]["tools"].as_array().unwrap();
    let names = tool_names(&response);

    assert_eq!(names, vec!["manage_resources", "get_metrics"]);
    assert_eq!(
        tools[0]["inputSchema"]["properties"]["action"]["enum"],
        json!([
            "list-services",
            "list-groups",
            "list-endpoints",
            "get-config",
            "get-health",
            "list-expiring-certificates",
            "get-alert-rules",
            "get-suite-health"
        ])
    );
    assert_eq!(
        tools[1]["inputSchema"]["properties"]["action"]["enum"],
        json!([
            "system-stats",
            "service-details",
            "service-history",
            "get-raw-results",
            "group-summary",
            "uptime",
            "uptime-granular",
            "response-time",
            "alert-history",
            "get-badge",
            "get-latency-badge",
            "get-latency-chart",
            "failure-summary",
            "performance-comparison",
            "group-stats",
            "alert-correlation",
            "flapping-services",
            "diagnostic-bundle",
            "certificate-audit"
        ])
    );
}

#[tokio::test]
async fn read_only_rejects_every_mutating_tool_before_http_dispatch() {
    let mock_server = MockServer::start().await;
    let handler = read_only_handler(&mock_server.uri());

    for (id, name) in [
        "trigger_check",
        "test_alert",
        "reload_config",
        "push_result",
        "manage_endpoints",
    ]
    .into_iter()
    .enumerate()
    {
        let response = handler
            .handle(json!({
                "jsonrpc": "2.0",
                "method": "tools/call",
                "params": {"name": name, "arguments": {}},
                "id": id
            }))
            .await;

        assert_eq!(response["error"]["code"], -32601);
        assert_eq!(response["error"]["message"], READ_ONLY_ERROR_MESSAGE);
    }

    assert!(mock_server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn read_only_rejects_unclassified_actions() {
    let handler = read_only_handler("http://localhost");

    for name in ["manage_resources", "get_metrics"] {
        let response = handler
            .handle(json!({
                "jsonrpc": "2.0",
                "method": "tools/call",
                "params": {
                    "name": name,
                    "arguments": {"action": "future-write-action"}
                },
                "id": 1
            }))
            .await;
        assert_eq!(response["error"]["code"], -32601);
        assert_eq!(response["error"]["message"], READ_ONLY_ERROR_MESSAGE);
    }
}

#[tokio::test]
async fn default_handler_remains_read_write() {
    let client = GatusClient::new("http://localhost".into(), None, None, None);
    let handler = McpHandler::new(client);
    let response = handler
        .handle(json!({"jsonrpc": "2.0", "method": "tools/list", "id": 1}))
        .await;
    let names: Vec<&str> = response["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"trigger_check"));
    assert!(names.contains(&"manage_endpoints"));
}

#[tokio::test]
async fn read_only_allows_representative_resource_and_metrics_calls() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/endpoints/statuses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .mount(&mock_server)
        .await;
    let handler = read_only_handler(&mock_server.uri());

    for (id, name, action) in [
        (1, "manage_resources", "get-health"),
        (2, "get_metrics", "system-stats"),
    ] {
        let response = handler
            .handle(json!({
                "jsonrpc": "2.0",
                "method": "tools/call",
                "params": {"name": name, "arguments": {"action": action}},
                "id": id
            }))
            .await;
        assert!(
            response["error"].is_null(),
            "{name}/{action} failed: {response}"
        );
    }
}
