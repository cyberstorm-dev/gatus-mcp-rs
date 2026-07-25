use gatus_mcp_rs::client::GatusClient;
use gatus_mcp_rs::mcp::{AccessMode, McpHandler, PROTOCOL_VERSION, READ_ONLY_ERROR_MESSAGE};
use gatus_mcp_rs::server::run_server_loop;
use serde_json::{json, Value};
use std::io::Cursor;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn read_only_stdio_exercises_the_complete_boundary() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
        .mount(&mock_server)
        .await;
    let client = GatusClient::new(mock_server.uri(), None, None, None);
    let handler = McpHandler::new_with_access_mode(client, AccessMode::ReadOnly);
    let input = [
        json!({"jsonrpc":"2.0","method":"initialize","params":{},"id":1}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","method":"tools/list","id":2}),
        json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"manage_resources","arguments":{"action":"get-health"}},"id":3}),
        json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"trigger_check","arguments":{"id":"core_service-1"}},"id":4}),
    ]
    .into_iter()
    .map(|request| format!("{request}\n"))
    .collect::<String>();
    let reader = Cursor::new(input.into_bytes());
    let mut writer = Cursor::new(Vec::new());

    run_server_loop(handler, reader, &mut writer).await.unwrap();

    let output = String::from_utf8(writer.into_inner()).unwrap();
    let responses: Vec<Value> = output
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let by_id = |id: i64| {
        responses
            .iter()
            .find(|response| response["id"] == id)
            .unwrap()
    };

    assert_eq!(by_id(1)["result"]["protocolVersion"], PROTOCOL_VERSION);
    let tool_names: Vec<&str> = by_id(2)["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(tool_names, vec!["manage_resources", "get_metrics"]);
    assert!(by_id(3)["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("OK"));
    assert_eq!(by_id(4)["error"]["code"], -32601);
    assert_eq!(by_id(4)["error"]["message"], READ_ONLY_ERROR_MESSAGE);
}
