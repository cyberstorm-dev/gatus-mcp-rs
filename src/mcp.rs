use crate::client::{GatusClient, HealthResult};
use crate::fmt::{
    format_alert_correlation, format_alert_rules, format_certificate_audit, format_config_summary,
    format_diagnostic_bundle, format_endpoint_status, format_endpoints_summary,
    format_expiring_certificates, format_failure_summary, format_flapping_services,
    format_group_stats, format_performance_comparison, format_suite_health, format_suites,
    format_system_stats,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

pub const PROTOCOL_VERSION: &str = "2024-11-05";
pub const READ_ONLY_ERROR_MESSAGE: &str = "tool/action disabled by read-only mode";

const READ_ONLY_TOOL_NAMES: &[&str] = &["manage_resources", "get_metrics"];
const MANAGE_RESOURCES_READ_ONLY_ACTIONS: &[&str] = &[
    "list-services",
    "list-groups",
    "list-endpoints",
    "get-config",
    "get-health",
    "list-expiring-certificates",
    "get-alert-rules",
    "get-suite-health",
];
const GET_METRICS_READ_ONLY_ACTIONS: &[&str] = &[
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
    "certificate-audit",
];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AccessMode {
    #[default]
    ReadWrite,
    ReadOnly,
}

impl AccessMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::ReadWrite => "disabled",
            Self::ReadOnly => "enabled",
        }
    }

    fn allows_tool(self, name: &str) -> bool {
        self == Self::ReadWrite || READ_ONLY_TOOL_NAMES.contains(&name)
    }

    fn allows_call(self, name: &str, arguments: &Value) -> bool {
        if self == Self::ReadWrite {
            return true;
        }

        let allowed_actions = match name {
            "manage_resources" => MANAGE_RESOURCES_READ_ONLY_ACTIONS,
            "get_metrics" => GET_METRICS_READ_ONLY_ACTIONS,
            _ => return false,
        };

        match arguments.get("action") {
            Some(Value::String(action)) => allowed_actions.contains(&action.as_str()),
            Some(_) | None => true,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    pub params: Option<Value>,
    pub id: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    pub id: Option<Value>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

pub struct McpHandler {
    gatus_client: Arc<GatusClient>,
    access_mode: AccessMode,
}

impl McpHandler {
    pub fn new(gatus_client: GatusClient) -> Self {
        Self::new_with_access_mode(gatus_client, AccessMode::ReadWrite)
    }

    pub fn new_with_access_mode(gatus_client: GatusClient, access_mode: AccessMode) -> Self {
        Self {
            gatus_client: Arc::new(gatus_client),
            access_mode,
        }
    }

    pub fn new_with_arc(gatus_client: Arc<GatusClient>) -> Self {
        Self::new_with_arc_and_access_mode(gatus_client, AccessMode::ReadWrite)
    }

    pub fn new_with_arc_and_access_mode(
        gatus_client: Arc<GatusClient>,
        access_mode: AccessMode,
    ) -> Self {
        Self {
            gatus_client,
            access_mode,
        }
    }

    #[tracing::instrument(skip(self, request))]
    pub async fn handle(&self, request: Value) -> Value {
        let req: JsonRpcRequest = match serde_json::from_value(request) {
            Ok(r) => r,
            Err(_) => return self.error_response(Value::Null, -32600, "Invalid Request"),
        };

        let id = req.id.unwrap_or(Value::Null);

        match req.method.as_str() {
            "initialize" => self.handle_initialize(id).await,
            "tools/list" => self.handle_list_tools(id).await,
            "tools/call" => self.handle_call_tool(id, req.params).await,
            "prompts/list" => self.handle_list_prompts(id).await,
            "prompts/get" => self.handle_get_prompt(id, req.params).await,
            "resources/list" => self.handle_list_resources(id).await,
            "resources/read" => self.handle_read_resource(id, req.params).await,
            "notifications/initialized" => json!(null),
            _ => self.error_response(id, -32601, "Method not found"),
        }
    }

    async fn handle_initialize(&self, id: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "result": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {
                    "tools": {},
                    "prompts": {},
                    "resources": {}
                },
                "serverInfo": {
                    "name": "gatus-mcp-rs",
                    "version": env!("CARGO_PKG_VERSION")
                }
            },
            "id": id
        })
    }

    async fn handle_list_resources(&self, id: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "result": {
                "resources": [
                    {
                        "uri": "gatus://system/config",
                        "name": "Gatus Configuration",
                        "description": "The active Gatus monitoring configuration.",
                        "mimeType": "application/json"
                    },
                    {
                        "uri": "gatus://dashboard/status",
                        "name": "Gatus Dashboard Status",
                        "description": "High-level summary of current endpoint statuses.",
                        "mimeType": "text/markdown"
                    }
                ]
            },
            "id": id
        })
    }

    async fn handle_read_resource(&self, id: Value, params: Option<Value>) -> Value {
        let params = params.unwrap_or(Value::Null);
        let uri = match params.get("uri").and_then(|u| u.as_str()) {
            Some(u) => u,
            None => return self.error_response(id, -32602, "Missing 'uri' parameter"),
        };

        match uri {
            "gatus://system/config" => match self.gatus_client.get_config().await {
                Ok(config) => self.success_response(
                    id,
                    json!({
                        "contents": [
                            {
                                "uri": uri,
                                "mimeType": "application/json",
                                "text": serde_json::to_string_pretty(&config).unwrap()
                            }
                        ]
                    }),
                ),
                Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
            },
            "gatus://dashboard/status" => match self.gatus_client.get_system_stats().await {
                Ok(stats) => self.success_response(
                    id,
                    json!({
                        "contents": [
                            {
                                "uri": uri,
                                "mimeType": "text/markdown",
                                "text": format_system_stats(&stats)
                            }
                        ]
                    }),
                ),
                Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
            },
            _ => self.error_response(id, -32602, &format!("Unknown resource URI '{}'", uri)),
        }
    }

    async fn handle_list_tools(&self, id: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "result": {
                "tools": self.get_tool_definitions()
            },
            "id": id
        })
    }

    fn get_tool_definitions(&self) -> Vec<Value> {
        let tools = vec![
            json!({
                "name": "manage_resources",
                "description": "Discover and manage Gatus resources and instance state.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": MANAGE_RESOURCES_READ_ONLY_ACTIONS,
                            "description": "Action to perform."
                        },
                        "id": {
                            "type": "string",
                            "description": "Optional identifier (e.g., group name for list-endpoints, or suite ID for get-suite-health)."
                        },
                        "status": {
                            "type": "string",
                            "description": "Optional status filter for list-services (e.g., DOWN, DEGRADED)."
                        }
                    },
                    "required": ["action"]
                }
            }),
            json!({
                "name": "get_metrics",
                "description": "Retrieve status, metrics, and history for services and endpoints.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": GET_METRICS_READ_ONLY_ACTIONS,
                            "description": "Action to perform."
                        },
                        "id": {
                            "type": "string",
                            "description": "Identifier (e.g., service name, group name, or endpoint key)."
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of results for history actions."
                        },
                        "timeframe": {
                            "type": "string",
                            "enum": ["1h", "24h", "7d", "30d"],
                            "description": "Timeframe for uptime/response-time calculation."
                        }
                    },
                    "required": ["action"]
                }
            }),
            json!({
                "name": "trigger_check",
                "description": "Force an immediate health check for a specific endpoint.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "The service name or endpoint key to check."
                        }
                    },
                    "required": ["id"]
                }
            }),
            json!({
                "name": "reload_config",
                "description": "Trigger a Gatus configuration reload.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            }),
            json!({
                "name": "test_alert",
                "description": "Trigger a test alert notification for an endpoint.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "The service name or endpoint key to test."
                        }
                    },
                    "required": ["id"]
                }
            }),
            json!({
                "name": "push_result",
                "description": "Push a health check result for an external endpoint.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "string",
                            "description": "The endpoint key (usually group_name_endpoint_name)."
                        },
                        "success": {
                            "type": "boolean",
                            "description": "Whether the health check was successful."
                        },
                        "duration": {
                            "type": "integer",
                            "description": "Duration of the health check in milliseconds."
                        },
                        "error": {
                            "type": "string",
                            "description": "Error message if the check was unsuccessful."
                        }
                    },
                    "required": ["id", "success"]
                }
            }),
            json!({
                "name": "manage_endpoints",
                "description": "Programmatically manage Gatus endpoints and suites.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["list-suites", "create-endpoint", "update-endpoint", "delete-endpoint"],
                            "description": "Action to perform."
                        },
                        "suite_id": {
                            "type": "string",
                            "description": "The ID of the suite (required for create/update/delete)."
                        },
                        "endpoint_id": {
                            "type": "string",
                            "description": "The ID of the endpoint (required for update/delete)."
                        },
                        "config": {
                            "type": "object",
                            "description": "Endpoint configuration (required for create/update).",
                            "properties": {
                                "name": { "type": "string" },
                                "group": { "type": "string" },
                                "url": { "type": "string" },
                                "interval": { "type": "string" },
                                "conditions": { "type": "array", "items": { "type": "string" } },
                                "method": { "type": "string" },
                                "body": { "type": "string" }
                            },
                            "required": ["name", "url", "conditions"]
                        }
                    },
                    "required": ["action"]
                }
            }),
        ];

        self.filter_tool_definitions(tools)
    }

    fn filter_tool_definitions(&self, tools: Vec<Value>) -> Vec<Value> {
        tools
            .into_iter()
            .filter(|tool| {
                tool["name"]
                    .as_str()
                    .is_some_and(|name| self.access_mode.allows_tool(name))
            })
            .collect()
    }
    async fn handle_list_prompts(&self, id: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "result": {
                "prompts": vec![
                    json!({
                        "name": "analyze-outage",
                        "description": "Assist in diagnosing a specific service outage.",
                        "arguments": [
                            {
                                "name": "id",
                                "description": "The service name to analyze.",
                                "required": true
                            }
                        ]
                    }),
                    json!({
                        "name": "daily-health-report",
                        "description": "Generate a high-level summary of the system's health."
                    })
                ]
            },
            "id": id
        })
    }

    async fn handle_get_prompt(&self, id: Value, params: Option<Value>) -> Value {
        let params = params.unwrap_or(Value::Null);
        let name = match params.get("name").and_then(|n| n.as_str()) {
            Some(n) => n,
            None => return self.error_response(id, -32602, "Missing prompt name"),
        };

        let arguments = params.get("arguments").unwrap_or(&Value::Null);

        match name {
            "analyze-outage" => {
                let service_id = match arguments.get("id").and_then(|i| i.as_str()) {
                    Some(i) => i,
                    None => return self.error_response(id, -32602, "Missing 'id' argument"),
                };

                let prompt = format!(
                    "You are an expert SRE. Analyze the outage for the service '{}'. \
                     1. Use `get_metrics` with `action: 'service-history'` and `id: '{}'` to see recent check results. \
                     2. Use `get_metrics` with `action: 'alert-history'` to see if any alerts were triggered. \
                     3. Cross-reference the check failures with the alert timing. \
                     4. Provide a root cause hypothesis and suggest remediation steps.",
                    service_id, service_id
                );

                self.success_response(
                    id,
                    json!({
                        "messages": [
                            {
                                "role": "user",
                                "content": {
                                    "type": "text",
                                    "text": prompt
                                }
                            }
                        ]
                    }),
                )
            }
            "daily-health-report" => {
                let prompt = "Generate a daily health report for the infrastructure. \
                             1. Use `get_metrics` with `action: 'system-stats'` to get an overview. \
                             2. Use `manage_resources` with `action: 'list-groups'` to see all groups. \
                             3. For each group, use `get_metrics` with `action: 'group-summary'` to see the status of endpoints. \
                             4. Summarize the overall health, highlighting any down or degraded services.";

                self.success_response(
                    id,
                    json!({
                        "messages": [
                            {
                                "role": "user",
                                "content": {
                                    "type": "text",
                                    "text": prompt
                                }
                            }
                        ]
                    }),
                )
            }
            _ => self.error_response(id, -32601, "Prompt not found"),
        }
    }

    #[tracing::instrument(skip(self, id, params))]
    async fn handle_call_tool(&self, id: Value, params: Option<Value>) -> Value {
        let params = params.unwrap_or(Value::Null);
        let name = match params.get("name").and_then(|n| n.as_str()) {
            Some(n) => n,
            None => return self.error_response(id, -32602, "Missing tool name"),
        };

        let arguments = params.get("arguments").unwrap_or(&Value::Null);

        if !self.access_mode.allows_call(name, arguments) {
            return self.error_response(id, -32601, READ_ONLY_ERROR_MESSAGE);
        }

        match name {
            "manage_resources" => self.handle_manage_resources_tool(id, arguments).await,
            "get_metrics" => self.handle_get_metrics_tool(id, arguments).await,
            "trigger_check" => self.handle_trigger_check_tool(id, arguments).await,
            "reload_config" => self.handle_reload_config_tool(id, arguments).await,
            "test_alert" => self.handle_test_alert_tool(id, arguments).await,
            "push_result" => self.handle_push_result_tool(id, arguments).await,
            "manage_endpoints" => self.handle_manage_endpoints_tool(id, arguments).await,
            _ => self.error_response(id, -32601, "Tool not found"),
        }
    }

    async fn handle_test_alert_tool(&self, id: Value, arguments: &Value) -> Value {
        let id_arg = match arguments.get("id").and_then(|i| i.as_str()) {
            Some(i) => i,
            None => return self.error_response(id, -32602, "Missing 'id' argument"),
        };

        let key = self.gatus_client.sanitize_key(id_arg);

        match self.gatus_client.test_alert_notification(&key).await {
            Ok(_) => self.success_response(
                id,
                json!({
                    "content": [
                        {
                            "type": "text",
                            "text": format!("Successfully triggered test alert for '{}'", key)
                        }
                    ]
                }),
            ),
            Err(e) => {
                self.error_response(id, -32000, &format!("Error triggering test alert: {}", e))
            }
        }
    }

    async fn handle_manage_endpoints_tool(&self, id: Value, arguments: &Value) -> Value {
        let action = match arguments.get("action").and_then(|a| a.as_str()) {
            Some(a) => a,
            None => return self.error_response(id, -32602, "Missing 'action' argument"),
        };

        match action {
            "list-suites" => match self.gatus_client.list_suites().await {
                Ok(pages) => {
                    let text = format_suites(&pages);
                    self.success_response(
                        id,
                        json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": text
                                }
                            ]
                        }),
                    )
                }
                Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
            },
            "create-endpoint" => {
                let suite_id = match arguments.get("suite_id").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => return self.error_response(id, -32602, "Missing 'suite_id' argument"),
                };
                let config: crate::client::EndpointConfig = match arguments.get("config").cloned() {
                    Some(c) => match serde_json::from_value(c) {
                        Ok(conf) => conf,
                        Err(e) => {
                            return self.error_response(
                                id,
                                -32602,
                                &format!("Invalid 'config' argument: {}", e),
                            )
                        }
                    },
                    None => return self.error_response(id, -32602, "Missing 'config' argument"),
                };

                match self.gatus_client.create_endpoint(suite_id, config).await {
                    Ok(_) => self.success_response(
                        id,
                        json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": "Successfully created endpoint"
                                }
                            ]
                        }),
                    ),
                    Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
                }
            }
            "update-endpoint" => {
                let suite_id = match arguments.get("suite_id").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => return self.error_response(id, -32602, "Missing 'suite_id' argument"),
                };
                let endpoint_id = match arguments.get("endpoint_id").and_then(|e| e.as_str()) {
                    Some(e) => e,
                    None => {
                        return self.error_response(id, -32602, "Missing 'endpoint_id' argument")
                    }
                };
                let config: crate::client::EndpointConfig = match arguments.get("config").cloned() {
                    Some(c) => match serde_json::from_value(c) {
                        Ok(conf) => conf,
                        Err(e) => {
                            return self.error_response(
                                id,
                                -32602,
                                &format!("Invalid 'config' argument: {}", e),
                            )
                        }
                    },
                    None => return self.error_response(id, -32602, "Missing 'config' argument"),
                };

                match self
                    .gatus_client
                    .update_endpoint(suite_id, endpoint_id, config)
                    .await
                {
                    Ok(_) => self.success_response(
                        id,
                        json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": "Successfully updated endpoint"
                                }
                            ]
                        }),
                    ),
                    Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
                }
            }
            "delete-endpoint" => {
                let suite_id = match arguments.get("suite_id").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => return self.error_response(id, -32602, "Missing 'suite_id' argument"),
                };
                let endpoint_id = match arguments.get("endpoint_id").and_then(|e| e.as_str()) {
                    Some(e) => e,
                    None => {
                        return self.error_response(id, -32602, "Missing 'endpoint_id' argument")
                    }
                };

                match self
                    .gatus_client
                    .delete_endpoint(suite_id, endpoint_id)
                    .await
                {
                    Ok(_) => self.success_response(
                        id,
                        json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": "Successfully deleted endpoint"
                                }
                            ]
                        }),
                    ),
                    Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
                }
            }
            _ => self.error_response(
                id,
                -32602,
                &format!("Unknown action '{}' for manage_endpoints", action),
            ),
        }
    }

    async fn handle_trigger_check_tool(&self, id: Value, arguments: &Value) -> Value {
        let id_arg = match arguments.get("id").and_then(|i| i.as_str()) {
            Some(i) => i,
            None => return self.error_response(id, -32602, "Missing 'id' argument"),
        };

        let key = self.gatus_client.sanitize_key(id_arg);

        match self.gatus_client.trigger_check(&key).await {
            Ok(_) => self.success_response(
                id,
                json!({
                    "content": [
                        {
                            "type": "text",
                            "text": format!("Successfully triggered check for '{}'", key)
                        }
                    ]
                }),
            ),
            Err(e) => self.error_response(id, -32000, &format!("Error triggering check: {}", e)),
        }
    }

    async fn handle_reload_config_tool(&self, id: Value, _arguments: &Value) -> Value {
        match self.gatus_client.reload_config().await {
            Ok(_) => self.success_response(
                id,
                json!({
                    "content": [
                        {
                            "type": "text",
                            "text": "Successfully triggered configuration reload"
                        }
                    ]
                }),
            ),
            Err(e) => self.error_response(id, -32000, &format!("Error reloading config: {}", e)),
        }
    }

    async fn handle_push_result_tool(&self, id: Value, arguments: &Value) -> Value {
        let key = match arguments.get("id").and_then(|i| i.as_str()) {
            Some(i) => i,
            None => return self.error_response(id, -32602, "Missing 'id' argument"),
        };

        let success = match arguments.get("success").and_then(|s| s.as_bool()) {
            Some(s) => s,
            None => return self.error_response(id, -32602, "Missing 'success' argument"),
        };

        let duration = arguments
            .get("duration")
            .and_then(|d| d.as_u64())
            .unwrap_or(0);
        let error = arguments.get("error").and_then(|e| e.as_str());

        let result = HealthResult {
            timestamp: chrono::Utc::now().to_rfc3339(),
            success,
            hostname: None,
            ip: None,
            duration,
            errors: error.map(|e| vec![e.to_string()]).unwrap_or_default(),
            status: if success { Some(200) } else { Some(500) },
            condition_results: vec![],
            body: None,
            headers: None,
            certificate_expiration: None,
            certificate_issuer: None,
            certificate_algorithm: None,
            certificate_sans: None,
        };

        match self.gatus_client.push_endpoint_result(key, result).await {
            Ok(_) => self.success_response(
                id,
                json!({
                    "content": [
                        {
                            "type": "text",
                            "text": format!("Successfully pushed result for '{}'", key)
                        }
                    ]
                }),
            ),
            Err(e) => self.error_response(id, -32000, &format!("Error pushing result: {}", e)),
        }
    }

    async fn handle_manage_resources_tool(&self, id: Value, arguments: &Value) -> Value {
        let action = match arguments.get("action").and_then(|a| a.as_str()) {
            Some(a) => a,
            None => return self.error_response(id, -32602, "Missing 'action' argument"),
        };

        match action {
            "list-services" => {
                let status_filter = arguments.get("status").and_then(|s| s.as_str());
                match self.gatus_client.list_services(false, status_filter).await {
                    Ok(services) => {
                        let text = format_endpoints_summary(&services);
                        self.success_response(
                            id,
                            json!({
                                "content": [
                                    {
                                        "type": "text",
                                        "text": text
                                    }
                                ]
                            }),
                        )
                    }
                    Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
                }
            }
            "list-expiring-certificates" => {
                match self.gatus_client.get_expiring_certificates(30).await {
                    Ok(certs) => {
                        let text = format_expiring_certificates(&certs);
                        self.success_response(
                            id,
                            json!({
                                "content": [
                                    {
                                        "type": "text",
                                        "text": text
                                    }
                                ]
                            }),
                        )
                    }
                    Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
                }
            }
            "list-groups" => match self.gatus_client.list_services(false, None).await {
                Ok(services) => {
                    let mut groups: Vec<_> = services.into_iter().map(|s| s.group).collect();
                    groups.sort();
                    groups.dedup();
                    let text = format!("Available groups:\n- {}", groups.join("\n- "));

                    self.success_response(
                        id,
                        json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": text
                                }
                            ]
                        }),
                    )
                }
                Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
            },
            "list-endpoints" => {
                let group_filter = arguments.get("id").and_then(|g| g.as_str());
                match self.gatus_client.list_services(false, None).await {
                    Ok(services) => {
                        let endpoints: Vec<String> = services
                            .into_iter()
                            .filter(|s| {
                                group_filter.is_none_or(|g| s.group.eq_ignore_ascii_case(g))
                            })
                            .map(|s| s.name)
                            .collect();

                        let text = format!("Available endpoints:\n- {}", endpoints.join("\n- "));
                        self.success_response(
                            id,
                            json!({
                                "content": [
                                    {
                                        "type": "text",
                                        "text": text
                                    }
                                ]
                            }),
                        )
                    }
                    Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
                }
            }
            "get-config" => self.handle_get_config_tool(id, arguments).await,
            "get-health" => self.handle_get_instance_health_tool(id, arguments).await,
            "get-alert-rules" => match self.gatus_client.get_alert_rules().await {
                Ok(rules) => {
                    let text = format_alert_rules(&rules);
                    self.success_response(
                        id,
                        json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": text
                                }
                            ]
                        }),
                    )
                }
                Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
            },
            "get-suite-health" => {
                let suite_id = match arguments.get("id").and_then(|i| i.as_str()) {
                    Some(i) => i,
                    None => {
                        return self.error_response(
                            id,
                            -32602,
                            "Missing 'id' argument for get-suite-health",
                        )
                    }
                };
                match self.gatus_client.get_suite_health(suite_id).await {
                    Ok(health) => {
                        let text = format_suite_health(&health);
                        self.success_response(
                            id,
                            json!({
                                "content": [
                                    {
                                        "type": "text",
                                        "text": text
                                    }
                                ]
                            }),
                        )
                    }
                    Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
                }
            }
            _ => self.error_response(
                id,
                -32602,
                &format!("Unknown action '{}' for manage_resources", action),
            ),
        }
    }

    async fn handle_get_metrics_tool(&self, id: Value, arguments: &Value) -> Value {
        let action = match arguments.get("action").and_then(|a| a.as_str()) {
            Some(a) => a,
            None => return self.error_response(id, -32602, "Missing 'action' argument"),
        };

        match action {
            "system-stats" => {
                let new_args = json!({});
                self.handle_get_system_stats_tool(id, &new_args).await
            }
            "service-details" => {
                let service_name = match arguments.get("id").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => {
                        return self.error_response(
                            id,
                            -32602,
                            "Missing 'id' argument for service-details",
                        )
                    }
                };
                let new_args = json!({"service": service_name, "action": "details"});
                self.handle_get_service_info_tool(id, &new_args).await
            }
            "service-history" => {
                let service_id = match arguments.get("id").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => {
                        return self.error_response(
                            id,
                            -32602,
                            "Missing 'id' argument for service-history",
                        )
                    }
                };
                let limit = arguments.get("limit").cloned().unwrap_or(json!(10));
                self.handle_get_service_history_tool(id, service_id, limit)
                    .await
            }
            "get-raw-results" => {
                let service_id = match arguments.get("id").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => {
                        return self.error_response(
                            id,
                            -32602,
                            "Missing 'id' argument for get-raw-results",
                        )
                    }
                };
                let limit = arguments.get("limit").cloned().unwrap_or(json!(10));
                self.handle_get_raw_results_tool(id, service_id, limit)
                    .await
            }
            "group-summary" => {
                let group_name = match arguments.get("id").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => {
                        return self.error_response(
                            id,
                            -32602,
                            "Missing 'id' argument for group-summary",
                        )
                    }
                };
                let new_args = json!({"group": group_name});
                self.handle_get_group_summary_tool(id, &new_args).await
            }
            "uptime" => {
                let service_name = match arguments.get("id").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => {
                        return self.error_response(id, -32602, "Missing 'id' argument for uptime")
                    }
                };
                let timeframe = arguments.get("timeframe").cloned().unwrap_or(json!("24h"));
                let new_args = json!({"service": service_name, "timeframe": timeframe});
                self.handle_get_uptime_tool(id, &new_args).await
            }
            "uptime-granular" => {
                let service_name = match arguments.get("id").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => {
                        return self.error_response(
                            id,
                            -32602,
                            "Missing 'id' argument for uptime-granular",
                        )
                    }
                };
                let timeframe = arguments.get("timeframe").cloned().unwrap_or(json!("24h"));
                let new_args =
                    json!({"key": service_name, "type": "uptime", "duration": timeframe});
                self.handle_get_endpoint_stats_tool(id, &new_args).await
            }
            "response-time" => {
                let service_name = match arguments.get("id").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => {
                        return self.error_response(
                            id,
                            -32602,
                            "Missing 'id' argument for response-time",
                        )
                    }
                };
                let timeframe = arguments.get("timeframe").cloned().unwrap_or(json!("24h"));
                let new_args =
                    json!({"key": service_name, "type": "response-time", "duration": timeframe});
                self.handle_get_endpoint_stats_tool(id, &new_args).await
            }
            "alert-history" => {
                let limit = arguments.get("limit").cloned().unwrap_or(json!(5));
                let new_args = json!({"limit": limit});
                self.handle_get_alert_history_tool(id, &new_args).await
            }
            "get-badge" => {
                let id_arg = match arguments.get("id").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => {
                        return self.error_response(
                            id,
                            -32602,
                            "Missing 'id' argument for get-badge",
                        )
                    }
                };
                let key = self.gatus_client.sanitize_key(id_arg);
                let timeframe = arguments.get("timeframe").and_then(|t| t.as_str());

                let markdown = if let Some(tf) = timeframe {
                    let url = self.gatus_client.get_uptime_badge_url(&key, tf);
                    format!("![Uptime Badge ({})]({})", tf, url)
                } else {
                    let url = self.gatus_client.get_badge_url(&key);
                    format!("![Health Badge]({})", url)
                };

                self.success_response(
                    id,
                    json!({
                        "content": [
                            {
                                "type": "text",
                                "text": markdown
                            }
                        ]
                    }),
                )
            }
            "get-latency-badge" => {
                let id_arg = match arguments.get("id").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => {
                        return self.error_response(
                            id,
                            -32602,
                            "Missing 'id' argument for get-latency-badge",
                        )
                    }
                };
                let key = self.gatus_client.sanitize_key(id_arg);
                let timeframe = arguments
                    .get("timeframe")
                    .and_then(|t| t.as_str())
                    .unwrap_or("24h");

                let url = self.gatus_client.get_latency_badge_url(&key, timeframe);
                let markdown = format!("![Latency Badge ({})]({})", timeframe, url);

                self.success_response(
                    id,
                    json!({
                        "content": [
                            {
                                "type": "text",
                                "text": markdown
                            }
                        ]
                    }),
                )
            }
            "get-latency-chart" => {
                let id_arg = match arguments.get("id").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => {
                        return self.error_response(
                            id,
                            -32602,
                            "Missing 'id' argument for get-latency-chart",
                        )
                    }
                };
                let key = self.gatus_client.sanitize_key(id_arg);
                let timeframe = arguments
                    .get("timeframe")
                    .and_then(|t| t.as_str())
                    .unwrap_or("24h");

                let url = self.gatus_client.get_latency_chart_url(&key, timeframe);
                let markdown = format!("![Latency Chart ({})]({})", timeframe, url);

                self.success_response(
                    id,
                    json!({
                        "content": [
                            {
                                "type": "text",
                                "text": markdown
                            }
                        ]
                    }),
                )
            }
            "failure-summary" => {
                let id_arg = match arguments.get("id").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => {
                        return self.error_response(
                            id,
                            -32602,
                            "Missing 'id' argument for failure-summary",
                        )
                    }
                };
                let key = self.gatus_client.sanitize_key(id_arg);
                match self.gatus_client.get_failure_summary(&key).await {
                    Ok(summary) => {
                        let text = format_failure_summary(&summary);
                        self.success_response(
                            id,
                            json!({
                                "content": [
                                    {
                                        "type": "text",
                                        "text": text
                                    }
                                ]
                            }),
                        )
                    }
                    Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
                }
            }
            "performance-comparison" => {
                let id_arg = match arguments.get("id").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => {
                        return self.error_response(
                            id,
                            -32602,
                            "Missing 'id' argument for performance-comparison",
                        )
                    }
                };
                let key = self.gatus_client.sanitize_key(id_arg);
                match self.gatus_client.compare_performance(&key).await {
                    Ok(comparison) => {
                        let text = format_performance_comparison(&comparison);
                        self.success_response(
                            id,
                            json!({
                                "content": [
                                    {
                                        "type": "text",
                                        "text": text
                                    }
                                ]
                            }),
                        )
                    }
                    Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
                }
            }
            "group-stats" => {
                let group_name = match arguments.get("id").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => {
                        return self.error_response(
                            id,
                            -32602,
                            "Missing 'id' argument (group name) for group-stats",
                        )
                    }
                };
                match self.gatus_client.get_group_stats(group_name).await {
                    Ok(stats) => {
                        let text = format_group_stats(&stats);
                        self.success_response(
                            id,
                            json!({
                                "content": [
                                    {
                                        "type": "text",
                                        "text": text
                                    }
                                ]
                            }),
                        )
                    }
                    Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
                }
            }
            "alert-correlation" => {
                let id_arg = match arguments.get("id").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => {
                        return self.error_response(
                            id,
                            -32602,
                            "Missing 'id' argument for alert-correlation",
                        )
                    }
                };
                let key = self.gatus_client.sanitize_key(id_arg);
                match self.gatus_client.get_notification_events(&key).await {
                    Ok(events) => {
                        let text = format_alert_correlation(&events);
                        self.success_response(
                            id,
                            json!({
                                "content": [
                                    {
                                        "type": "text",
                                        "text": text
                                    }
                                ]
                            }),
                        )
                    }
                    Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
                }
            }
            "flapping-services" => match self.gatus_client.get_flapping_services().await {
                Ok(services) => {
                    let text = format_flapping_services(&services);
                    self.success_response(
                        id,
                        json!({
                            "content": [
                                {
                                    "type": "text",
                                    "text": text
                                }
                            ]
                        }),
                    )
                }
                Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
            },
            "diagnostic-bundle" => {
                let id_arg = match arguments.get("id").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => {
                        return self.error_response(
                            id,
                            -32602,
                            "Missing 'id' argument for diagnostic-bundle",
                        )
                    }
                };
                let key = self.gatus_client.sanitize_key(id_arg);
                match self.gatus_client.get_diagnostic_bundle(&key).await {
                    Ok(bundle) => {
                        let text = format_diagnostic_bundle(&bundle);
                        self.success_response(
                            id,
                            json!({
                                "content": [
                                    {
                                        "type": "text",
                                        "text": text
                                    }
                                ]
                            }),
                        )
                    }
                    Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
                }
            }
            "certificate-audit" => {
                let id_arg = match arguments.get("id").and_then(|s| s.as_str()) {
                    Some(s) => s,
                    None => {
                        return self.error_response(
                            id,
                            -32602,
                            "Missing 'id' argument for certificate-audit",
                        )
                    }
                };
                let key = self.gatus_client.sanitize_key(id_arg);
                match self.gatus_client.get_certificate_audit(&key).await {
                    Ok(audit) => {
                        let text = format_certificate_audit(&audit);
                        self.success_response(
                            id,
                            json!({
                                "content": [
                                    {
                                        "type": "text",
                                        "text": text
                                    }
                                ]
                            }),
                        )
                    }
                    Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
                }
            }
            _ => self.error_response(
                id,
                -32602,
                &format!("Unknown action '{}' for get_metrics", action),
            ),
        }
    }

    async fn handle_get_service_history_tool(&self, id: Value, key: &str, limit: Value) -> Value {
        let limit = limit.as_u64().unwrap_or(10) as usize;

        match self.gatus_client.get_endpoint_statuses(key).await {
            Ok(services) => {
                let service = services.into_iter().next();
                match service {
                    Some(s) => {
                        let mut history: Vec<_> = s.results.into_iter().take(limit).collect();
                        // Strip body and headers from successful results to save tokens
                        // Truncate body and strip headers for failed results
                        for result in &mut history {
                            if result.success {
                                result.body = None;
                                result.headers = None;
                            } else {
                                if let Some(ref body) = result.body {
                                    if body.len() > 100 {
                                        result.body = Some(format!("{}...", &body[..100]));
                                    }
                                }
                                result.headers = None;
                            }
                        }
                        self.success_response(
                            id,
                            json!({
                                "content": [
                                    {
                                        "type": "text",
                                        "text": serde_json::to_string_pretty(&history).unwrap()
                                    }
                                ]
                            }),
                        )
                    }
                    None => {
                        self.error_response(id, -32602, &format!("Service '{}' not found", key))
                    }
                }
            }
            Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
        }
    }

    async fn handle_get_raw_results_tool(&self, id: Value, key: &str, limit: Value) -> Value {
        let limit = limit.as_u64().unwrap_or(10) as usize;

        match self.gatus_client.get_raw_results(key, limit).await {
            Ok(history) => self.success_response(
                id,
                json!({
                    "content": [
                        {
                            "type": "text",
                            "text": serde_json::to_string_pretty(&history).unwrap()
                        }
                    ]
                }),
            ),
            Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
        }
    }

    async fn handle_get_alert_history_tool(&self, id: Value, arguments: &Value) -> Value {
        let limit = arguments.get("limit").and_then(|l| l.as_u64()).unwrap_or(5) as usize;

        match self.gatus_client.get_alert_history(limit).await {
            Ok(events) => {
                let mut text = String::from(
                    "| Service | Group | Event | Timestamp |\n| :--- | :--- | :--- | :--- |\n",
                );
                for event in events {
                    text.push_str(&format!(
                        "| {} | {} | {} | {} |\n",
                        event.service, event.group, event.event_type, event.timestamp
                    ));
                }

                self.success_response(
                    id,
                    json!({
                        "content": [
                            {
                                "type": "text",
                                "text": text
                            }
                        ]
                    }),
                )
            }
            Err(e) => {
                self.error_response(id, -32000, &format!("Error getting alert history: {}", e))
            }
        }
    }

    async fn handle_get_endpoint_stats_tool(&self, id: Value, arguments: &Value) -> Value {
        let key = match arguments.get("key").and_then(|k| k.as_str()) {
            Some(k) => k,
            None => return self.error_response(id, -32602, "Missing 'key' argument"),
        };

        let stat_type = match arguments.get("type").and_then(|t| t.as_str()) {
            Some(t) => t,
            None => return self.error_response(id, -32602, "Missing 'type' argument"),
        };

        let duration = arguments
            .get("duration")
            .and_then(|d| d.as_str())
            .unwrap_or("24h");

        match stat_type {
            "uptime" => match self.gatus_client.get_endpoint_uptimes(key, duration).await {
                Ok(uptimes) => {
                    let mut text = format!("Uptime statistics for {} over {}:\n", key, duration);
                    let mut sorted_keys: Vec<_> = uptimes.keys().collect();
                    sorted_keys.sort();
                    for k in sorted_keys {
                        text.push_str(&format!("- {}: {:.2}%\n", k, uptimes[k] * 100.0));
                    }
                    self.success_response(
                        id,
                        json!({ "content": [{ "type": "text", "text": text }] }),
                    )
                }
                Err(e) => {
                    self.error_response(id, -32000, &format!("Error getting uptime stats: {}", e))
                }
            },
            "response-time" => match self
                .gatus_client
                .get_endpoint_response_times(key, duration)
                .await
            {
                Ok(times) => {
                    if times.is_empty() {
                        return self.success_response(id, json!({ "content": [{ "type": "text", "text": "No response time data found." }] }));
                    }
                    let avg: f64 =
                        times.iter().map(|p| p.value as f64).sum::<f64>() / times.len() as f64;
                    let max = times.iter().map(|p| p.value).max().unwrap_or(0);
                    let min = times.iter().map(|p| p.value).min().unwrap_or(0);
                    let text = format!(
                        "Response time statistics for {} over {}:\n- Average: {:.2}ms\n- Min: {}ms\n- Max: {}ms\n- Data points: {}",
                        key, duration, avg, min, max, times.len()
                    );
                    self.success_response(
                        id,
                        json!({ "content": [{ "type": "text", "text": text }] }),
                    )
                }
                Err(e) => self.error_response(
                    id,
                    -32000,
                    &format!("Error getting response time stats: {}", e),
                ),
            },
            _ => self.error_response(id, -32602, &format!("Unknown stat type '{}'", stat_type)),
        }
    }

    async fn handle_get_instance_health_tool(&self, id: Value, _arguments: &Value) -> Value {
        match self.gatus_client.get_instance_health().await {
            Ok(health) => self.success_response(
                id,
                json!({
                    "content": [
                        {
                            "type": "text",
                            "text": format!("Gatus instance health: {}", health)
                        }
                    ]
                }),
            ),
            Err(e) => {
                self.error_response(id, -32000, &format!("Error getting instance health: {}", e))
            }
        }
    }

    async fn handle_get_uptime_tool(&self, id: Value, arguments: &Value) -> Value {
        let service_name = match arguments.get("service").and_then(|s| s.as_str()) {
            Some(s) => s,
            None => return self.error_response(id, -32602, "Missing 'service' argument"),
        };

        let timeframe = arguments
            .get("timeframe")
            .and_then(|t| t.as_str())
            .unwrap_or("24h");

        match self.gatus_client.get_uptime(service_name, timeframe).await {
            Ok(uptime) => self.success_response(
                id,
                json!({
                    "content": [
                        {
                            "type": "text",
                            "text": format!("Uptime for {} ({}): {:.2}%", service_name, timeframe, uptime)
                        }
                    ]
                }),
            ),
            Err(e) => self.error_response(id, -32000, &format!("Error getting uptime: {}", e)),
        }
    }

    async fn handle_get_group_summary_tool(&self, id: Value, arguments: &Value) -> Value {
        let group_name = match arguments.get("group").and_then(|g| g.as_str()) {
            Some(g) => g,
            None => return self.error_response(id, -32602, "Missing 'group' argument"),
        };

        match self.gatus_client.list_services(false, None).await {
            Ok(services) => {
                let filtered: Vec<_> = services
                    .into_iter()
                    .filter(|s| s.group.to_lowercase() == group_name.to_lowercase())
                    .collect();

                if filtered.is_empty() {
                    return self.error_response(
                        id,
                        -32602,
                        &format!("Group '{}' not found or has no services", group_name),
                    );
                }

                self.success_response(
                    id,
                    json!({
                        "content": [
                            {
                                "type": "text",
                                "text": format_endpoints_summary(&filtered)
                            }
                        ]
                    }),
                )
            }
            Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
        }
    }

    async fn handle_get_config_tool(&self, id: Value, _arguments: &Value) -> Value {
        match self.gatus_client.list_services(false, None).await {
            Ok(services) => self.success_response(
                id,
                json!({
                    "content": [
                        {
                            "type": "text",
                            "text": format_config_summary(&services)
                        }
                    ]
                }),
            ),
            Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
        }
    }

    async fn handle_get_system_stats_tool(&self, id: Value, _arguments: &Value) -> Value {
        match self.gatus_client.get_system_stats().await {
            Ok(stats) => self.success_response(
                id,
                json!({
                    "content": [
                        {
                            "type": "text",
                            "text": format_system_stats(&stats)
                        }
                    ]
                }),
            ),
            Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
        }
    }

    async fn handle_get_service_info_tool(&self, id: Value, arguments: &Value) -> Value {
        let service_name = match arguments.get("service").and_then(|s| s.as_str()) {
            Some(s) => s,
            None => return self.error_response(id, -32602, "Missing 'service' argument"),
        };

        let action = match arguments.get("action").and_then(|a| a.as_str()) {
            Some(a) => a,
            None => return self.error_response(id, -32602, "Missing 'action' argument"),
        };

        match self.gatus_client.list_services(false, None).await {
            Ok(services) => {
                let service = services.into_iter().find(|s| s.name == service_name);
                match service {
                    Some(s) => match action {
                        "details" => {
                            let key = s.get_key();
                            let badge_url = self.gatus_client.get_badge_url(&key);
                            self.success_response(
                                id,
                                json!({
                                    "content": [
                                        {
                                            "type": "text",
                                            "text": format_endpoint_status(&s, Some(&badge_url))
                                        }
                                    ]
                                }),
                            )
                        }
                        _ => self.error_response(
                            id,
                            -32602,
                            &format!("Unknown action '{}' for get_service_info", action),
                        ),
                    },
                    None => self.error_response(
                        id,
                        -32602,
                        &format!("Service '{}' not found", service_name),
                    ),
                }
            }
            Err(e) => self.error_response(id, -32000, &format!("Gatus API error: {}", e)),
        }
    }

    fn success_response(&self, id: Value, result: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "result": result,
            "id": id
        })
    }

    fn error_response(&self, id: Value, code: i32, message: &str) -> Value {
        json!({
            "jsonrpc": "2.0",
            "error": {
                "code": code,
                "message": message
            },
            "id": id
        })
    }
}

#[cfg(test)]
mod read_only_policy_tests {
    use super::*;

    #[test]
    fn every_classified_action_is_allowed_and_unknown_actions_are_denied() {
        for (tool, actions) in [
            ("manage_resources", MANAGE_RESOURCES_READ_ONLY_ACTIONS),
            ("get_metrics", GET_METRICS_READ_ONLY_ACTIONS),
        ] {
            for action in actions {
                assert!(AccessMode::ReadOnly.allows_call(tool, &json!({"action": action})));
            }
            assert!(!AccessMode::ReadOnly.allows_call(tool, &json!({"action": "unclassified"})));
        }
    }
}
