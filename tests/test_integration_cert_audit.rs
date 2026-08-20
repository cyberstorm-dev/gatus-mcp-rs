use gatus_mcp_rs::client::GatusClient;
use gatus_mcp_rs::mcp::McpHandler;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_integration_certificate_audit_full_flow() {
    let mock_server = MockServer::start().await;
    let client = GatusClient::new(mock_server.uri(), None, None, None);
    let handler = McpHandler::new(client);

    let health_response = json!([
        {
            "name": "secure-api",
            "group": "prod",
            "results": [
                {
                    "timestamp": "2023-01-01T12:00:00Z",
                    "success": true,
                    "duration": 500000000u64,
                    "conditionResults": [],
                    "certificateExpiration": 1500000000000000000u64,
                    "certificateIssuer": "DigiCert",
                    "certificateAlgorithm": "ECC-384",
                    "certificateSans": ["api.example.com", "backup.example.com"]
                }
            ]
        }
    ]);

    Mock::given(method("GET"))
        .and(path("/api/v1/endpoints/prod_secure-api/statuses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(health_response))
        .mount(&mock_server)
        .await;

    let request = json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "params": {
            "name": "get_metrics",
            "arguments": {
                "action": "certificate-audit",
                "id": "prod_secure-api"
            }
        },
        "id": "test-cert"
    });

    let response = handler.handle(request).await;
    let text = response["result"]["content"][0]["text"].as_str().unwrap();

    assert!(text.contains("Certificate Audit: secure-api (prod)"));
    assert!(text.contains("DigiCert"));
    assert!(text.contains("ECC-384"));
    assert!(text.contains("api.example.com"));
    assert!(text.contains("backup.example.com"));
}
