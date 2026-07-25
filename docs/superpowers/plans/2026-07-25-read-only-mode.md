# Read-Only Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a global, fail-closed read-only mode that exposes only safe Gatus MCP operations and supports the Relax.gg Docker stdio contract.

**Architecture:** Resolve read-only mode once in the global Clap parser and pass an `AccessMode` into every `McpHandler`. Keep the complete read-only allowlist beside MCP tool definitions, and consult the same policy for both `tools/list` and `tools/call` so hidden operations cannot be called by name.

**Tech Stack:** Rust 2021, Clap 4, Tokio, serde/serde_json, Axum, Wiremock, JSON-RPC 2.0, Docker

---

## File Structure

- Create `tests/test_read_only_policy.rs`: handler-level discovery, allowlist, denial, and no-upstream-call coverage.
- Create `tests/test_read_only_stdio.rs`: full newline-delimited MCP smoke sequence over the stdio server loop.
- Create `tests/test_read_only_cli.rs`: global argument placement, environment resolution, safe logging, and direct CLI coverage.
- Modify `src/mcp.rs`: define `AccessMode`, central action allowlists, filtered discovery, and pre-dispatch denial.
- Modify `src/cli.rs`: add the global CLI/environment boolean.
- Modify `src/lib.rs`: log the resolved mode safely and construct mode-aware handlers for every command.
- Modify `src/server.rs`: preserve existing read-write APIs, add access-mode-aware HTTP construction, and remove credential-bearing polling errors from logs.
- Modify `tests/test_run_app.rs`: initialize the new `Cli` field in existing struct literals.
- Modify `README.md`: document configuration, precedence, policy, errors, and pinned Docker usage.
- Do not modify `Dockerfile`: its `ENTRYPOINT` already forwards trailing `stdio --read-only` arguments.

### Task 1: Enforce the MCP Access Policy

**Files:**

- Create: `tests/test_read_only_policy.rs`
- Create: `tests/test_read_only_stdio.rs`
- Modify: `src/mcp.rs`

- [ ] **Step 1: Write failing handler policy tests**

Create `tests/test_read_only_policy.rs` with these imports and helpers:

```rust
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
```

At the bottom of `src/mcp.rs`, add the exhaustive policy test before adding
`AccessMode`. This tests every classified action without invoking unrelated
Gatus APIs:

```rust
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
                assert!(AccessMode::ReadOnly
                    .allows_call(tool, &json!({"action": action})));
            }
            assert!(!AccessMode::ReadOnly
                .allows_call(tool, &json!({"action": "unclassified"})));
        }
    }
}
```

- [ ] **Step 2: Write the failing full stdio boundary test**

Create `tests/test_read_only_stdio.rs`:

```rust
use gatus_mcp_rs::client::GatusClient;
use gatus_mcp_rs::mcp::{
    AccessMode, McpHandler, PROTOCOL_VERSION, READ_ONLY_ERROR_MESSAGE,
};
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
```

- [ ] **Step 3: Run the focused tests and verify RED**

Run:

```bash
cargo test --lib read_only_policy_tests
cargo test --test test_read_only_policy --test test_read_only_stdio
```

Expected: compilation fails because `AccessMode` and
`McpHandler::new_with_access_mode` do not exist.

- [ ] **Step 4: Implement the minimal central policy**

In `src/mcp.rs`, add:

```rust
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
```

Add `access_mode: AccessMode` to `McpHandler`. Preserve existing defaults and
add explicit constructors:

```rust
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
```

In the existing `manage_resources` definition, replace its literal action
array with:

```rust
"enum": MANAGE_RESOURCES_READ_ONLY_ACTIONS,
```

In the existing `get_metrics` definition, replace its literal action array
with:

```rust
"enum": GET_METRICS_READ_ONLY_ACTIONS,
```

Change the first expression in `get_tool_definitions` from `vec![` to
`let tools = vec![`. After the existing closing `];`, return the filtered
catalog:

```rust
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
```

Call `self.filter_tool_definitions(tools)` at the end of
`get_tool_definitions`. Keeping filtering separate makes the full catalog
construction unchanged and keeps the policy transformation testable.

In `handle_call_tool`, immediately after extracting `arguments`, deny before
the existing match:

```rust
if !self.access_mode.allows_call(name, arguments) {
    return self.error_response(id, -32601, READ_ONLY_ERROR_MESSAGE);
}
```

Malformed or missing actions still reach existing `-32602` validation, while
any unclassified string action fails closed.

- [ ] **Step 5: Run focused and regression tests and verify GREEN**

Run:

```bash
cargo test --lib read_only_policy_tests
cargo test --test test_read_only_policy --test test_read_only_stdio
cargo test --test test_mcp --test test_mcp_mutative --test test_mcp_manage_resources
```

Expected: all selected tests pass.

- [ ] **Step 6: Commit the MCP policy**

```bash
git add src/mcp.rs tests/test_read_only_policy.rs tests/test_read_only_stdio.rs
git commit -m "feat: enforce read-only MCP policy"
```

### Task 2: Resolve and Propagate Read-Only Mode

**Files:**

- Create: `tests/test_read_only_cli.rs`
- Modify: `src/cli.rs`
- Modify: `src/lib.rs`
- Modify: `src/server.rs`
- Modify: `tests/test_run_app.rs`

- [ ] **Step 1: Write failing CLI placement tests**

Create `tests/test_read_only_cli.rs` with these imports and helpers:

```rust
use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use clap::Parser;
use gatus_mcp_rs::cli::Cli;
use gatus_mcp_rs::mcp::AccessMode;
use gatus_mcp_rs::server::create_app_with_access_mode;
use gatus_mcp_rs::settings::Settings;
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use tower::ServiceExt;

fn binary() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_gatus-mcp-rs"));
    command.env_remove("RUST_LOG").env("LOG_LEVEL", "info");
    command
}

fn names_from_stdout(stdout: &[u8]) -> Vec<String> {
    let response: Value = serde_json::from_slice(stdout).unwrap();
    response["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn read_only_is_global_for_every_subcommand() {
    for args in [
        vec!["gatus-mcp-rs", "--read-only", "stdio"],
        vec!["gatus-mcp-rs", "stdio", "--read-only"],
        vec!["gatus-mcp-rs", "--read-only", "http"],
        vec!["gatus-mcp-rs", "http", "--read-only"],
        vec!["gatus-mcp-rs", "--read-only", "list-tools"],
        vec!["gatus-mcp-rs", "list-tools", "--read-only"],
        vec![
            "gatus-mcp-rs",
            "--read-only",
            "call-tool",
            "trigger_check",
            "{}",
        ],
        vec![
            "gatus-mcp-rs",
            "call-tool",
            "trigger_check",
            "{}",
            "--read-only",
        ],
    ] {
        let cli = Cli::try_parse_from(args).unwrap();
        assert!(cli.read_only);
    }
}
```

- [ ] **Step 2: Write failing process-level environment and logging test**

Continue the same file with fresh-process tests so environment and the global
tracing subscriber cannot race with other tests:

```rust
#[test]
fn environment_enables_read_only_and_logs_without_secrets() {
    let output = binary()
        .arg("list-tools")
        .env("GATUS_MCP_READ_ONLY", "true")
        .env(
            "GATUS_API_URL",
            "http://user:super-secret@example.invalid/api?token=hidden",
        )
        .output()
        .unwrap();

    assert!(output.status.success());

    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    let names: Vec<&str> = response["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["manage_resources", "get_metrics"]);

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("read-only mode: enabled"));
    assert!(!stderr.contains("super-secret"));
    assert!(!stderr.contains("token=hidden"));
    assert!(!stderr.contains("user:"));
}

#[test]
fn cli_flag_cannot_downgrade_environment_policy() {
    let output = binary()
        .args(["--read-only", "list-tools"])
        .env("GATUS_MCP_READ_ONLY", "false")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        names_from_stdout(&output.stdout),
        vec!["manage_resources".to_string(), "get_metrics".to_string()]
    );
}

#[test]
fn default_mode_remains_read_write() {
    let output = binary()
        .arg("list-tools")
        .env("GATUS_MCP_READ_ONLY", "false")
        .output()
        .unwrap();
    assert!(output.status.success());
    let names = names_from_stdout(&output.stdout);
    assert!(names.iter().any(|name| name == "trigger_check"));
    assert!(names.iter().any(|name| name == "manage_endpoints"));
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("read-only mode: disabled"));
}

#[test]
fn invalid_environment_value_fails_clap_parsing() {
    let output = binary()
        .arg("list-tools")
        .env("GATUS_MCP_READ_ONLY", "sometimes")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("invalid value"));
    assert!(stderr.contains("--read-only"));
}

#[test]
fn direct_mutating_call_returns_the_stable_mcp_error() {
    let output = binary()
        .args(["call-tool", "trigger_check", "{}"])
        .env("GATUS_MCP_READ_ONLY", "true")
        .env("GATUS_API_URL", "http://127.0.0.1:9")
        .output()
        .unwrap();
    assert!(output.status.success());
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["error"]["code"], -32601);
    assert_eq!(
        response["error"]["message"],
        "tool/action disabled by read-only mode"
    );
}
```

- [ ] **Step 3: Write the failing HTTP propagation test**

Continue `tests/test_read_only_cli.rs`:

```rust
#[tokio::test]
async fn http_messages_use_the_read_only_handler() {
    let app = create_app_with_access_mode(Settings::default(), AccessMode::ReadOnly);
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/messages")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"jsonrpc":"2.0","method":"tools/list","id":1}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let response: Value = serde_json::from_slice(&bytes).unwrap();
    let names: Vec<&str> = response["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["manage_resources", "get_metrics"]);
}

#[test]
fn http_startup_logs_mode_without_credential_bearing_urls() {
    let mut child = binary()
        .args(["http", "--host", "127.0.0.1", "--port", "0", "--read-only"])
        .env("GATUS_MCP_READ_ONLY", "false")
        .env(
            "GATUS_API_URL",
            "http://user:super-secret@127.0.0.1:9/?token=hidden",
        )
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let stderr = child.stderr.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            sender.send(line.unwrap()).unwrap();
        }
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut lines = Vec::new();
    let mut saw_poll_error = false;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(remaining) {
            Ok(line) => {
                saw_poll_error |= line.contains("Failed to poll Gatus for state changes");
                lines.push(line);
                if saw_poll_error {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = child.kill();
    child.wait().unwrap();
    reader.join().unwrap();
    lines.extend(receiver.try_iter());
    let stderr = lines.join("\n");

    assert!(saw_poll_error, "polling error was not observed: {stderr}");
    assert!(stderr.contains("read-only mode: enabled"));
    assert!(!stderr.contains("super-secret"));
    assert!(!stderr.contains("token=hidden"));
    assert!(!stderr.contains("user:"));
}
```

- [ ] **Step 4: Run the focused CLI tests and verify RED**

Run:

```bash
cargo test --test test_read_only_cli
```

Expected: compilation fails because `Cli::read_only` and the access-mode-aware
HTTP constructor do not exist.

- [ ] **Step 5: Add the global Clap/environment option**

In `src/cli.rs`, add this top-level field before `command`:

```rust
/// Disable tools and actions that can mutate Gatus state
#[arg(long, env = "GATUS_MCP_READ_ONLY", global = true)]
pub read_only: bool,
```

`global = true` is required for `stdio --read-only`; Clap's boolean environment
parser rejects invalid values. Add `read_only: false` to every existing `Cli`
literal in `tests/test_run_app.rs`.

- [ ] **Step 6: Propagate the access mode and remove raw URL logging**

In `src/lib.rs`, resolve once:

```rust
let access_mode = if cli.read_only {
    AccessMode::ReadOnly
} else {
    AccessMode::ReadWrite
};
tracing::info!("read-only mode: {}", access_mode.label());
```

Delete the current `Using Gatus API URL` log. Construct handlers with
`McpHandler::new_with_access_mode(client, access_mode)` in stdio, `list-tools`,
and `call-tool`.

Import `AccessMode` beside `McpHandler` in `src/server.rs`. Replace the current
`create_app` with the backward-compatible wrapper and complete mode-aware
constructor:

```rust
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

    let gatus_client_clone = Arc::new(gatus_client);
    let interval = settings.server.polling_interval;
    tokio::spawn(background_polling_task(gatus_client_clone, tx, interval));

    Router::new()
        .route("/sse", get(sse_handler))
        .route("/messages", post(messages_handler))
        .with_state(state)
}
```

Replace the polling error arm with a message that cannot contain the client
URL or credential material:

```rust
Err(_) => tracing::error!("Failed to poll Gatus for state changes"),
```

Replace the current `run_http_server` with:

```rust
pub async fn run_http_server(
    settings: Settings,
    port: u16,
    host: String,
) -> anyhow::Result<()> {
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
```

Import and call `run_http_server_with_access_mode` from `run_app_with_stdio`.
The polling request remains read-only, but its error details are intentionally
not logged because `reqwest` errors can embed the credential-bearing URL.

- [ ] **Step 7: Run focused and regression tests and verify GREEN**

Run:

```bash
cargo test --test test_read_only_cli
cargo test --test test_run_app --test test_http_server --test test_server_loop
```

Expected: all selected tests pass, stdout remains protocol/JSON only, and mode
logging is on stderr.

- [ ] **Step 8: Commit CLI and transport propagation**

```bash
git add src/cli.rs src/lib.rs src/server.rs tests/test_read_only_cli.rs tests/test_run_app.rs
git commit -m "feat: expose global read-only mode"
```

### Task 3: Document the Read-Only Contract

**Files:**

- Modify: `README.md`

- [ ] **Step 1: Update configuration and usage documentation**

Add `GATUS_MCP_READ_ONLY` to the environment variable list. Add
`--read-only` to the CLI options and document that it is global, accepted
before or after subcommands, and applies to stdio, HTTP, `list-tools`, and
`call-tool`.

State the deterministic rule:

```text
Read-write is the default. Either --read-only or
GATUS_MCP_READ_ONLY=true enables read-only mode. There is no CLI option that
can downgrade an environment-enforced read-only process.
```

- [ ] **Step 2: Document policy, error behavior, and Docker invocation**

Add a "Read-only mode" section containing:

```text
Allowed tools:
- manage_resources (all currently advertised actions)
- get_metrics (all currently advertised actions)

Disabled tools:
- trigger_check
- test_alert
- reload_config
- push_result
- manage_endpoints
```

Document that disabled or unclassified operations return JSON-RPC code
`-32601` and message `tool/action disabled by read-only mode`, and are not
silently ignored.

Include the pinned deployment shape:

```bash
docker run -i --rm \
  -e GATUS_API_URL=http://100.123.0.63:3003 \
  ghcr.io/relax-dot-gg/gatus-mcp-rs:<git-sha> \
  stdio --read-only
```

Explicitly warn not to use `latest` for deployment.

- [ ] **Step 3: Check rendered text and commit**

Run:

```bash
git diff --check
rg -n "GATUS_MCP_READ_ONLY|read-only|git-sha|latest" README.md
```

Expected: no whitespace errors, and each contract term appears in the relevant
configuration/usage section.

Commit:

```bash
git add README.md
git commit -m "docs: document read-only MCP usage"
```

### Task 4: Verify the Release Boundary

**Files:**

- No source changes expected.

- [ ] **Step 1: Run formatting and the complete test suite**

Run:

```bash
cargo fmt --all
cargo fmt --check
cargo test
```

Expected: formatting check and all tests pass.

- [ ] **Step 2: Run strict Clippy**

Run:

```bash
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: pass. If failures predate this work, capture the exact diagnostics
and verify with the parent commit before reporting them; do not suppress
warnings.

- [ ] **Step 3: Build the Docker image with a local pinned tag**

Run:

```bash
docker build -t gatus-mcp-rs:read-only-smoke .
```

Expected: the runtime image builds and retains
`ENTRYPOINT ["/app/gatus-mcp-rs"]`.

- [ ] **Step 4: Run the full newline-delimited Docker smoke probe**

Use the scope's reachable internal Gatus URL and capture protocol and logs
separately:

```bash
smoke_dir="$(mktemp -d)"
printf '%s\n' \
  '{"jsonrpc":"2.0","method":"initialize","params":{},"id":1}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","method":"tools/list","id":2}' \
  '{"jsonrpc":"2.0","method":"tools/call","params":{"name":"manage_resources","arguments":{"action":"get-health"}},"id":3}' \
  '{"jsonrpc":"2.0","method":"tools/call","params":{"name":"trigger_check","arguments":{"id":"core_service-1"}},"id":4}' |
docker run -i --rm \
  -e GATUS_API_URL=http://100.123.0.63:3003 \
  gatus-mcp-rs:read-only-smoke \
  stdio --read-only \
  >"${smoke_dir}/stdout.jsonl" \
  2>"${smoke_dir}/stderr.log"
python3 - "${smoke_dir}/stdout.jsonl" "${smoke_dir}/stderr.log" <<'PY'
import json
import pathlib
import sys

responses = [
    json.loads(line)
    for line in pathlib.Path(sys.argv[1]).read_text().splitlines()
    if line.strip()
]
by_id = {response.get("id"): response for response in responses if isinstance(response, dict)}
assert by_id[1]["result"]["protocolVersion"] == "2024-11-05"
assert [tool["name"] for tool in by_id[2]["result"]["tools"]] == [
    "manage_resources",
    "get_metrics",
]
assert "UP" in by_id[3]["result"]["content"][0]["text"]
assert by_id[4]["error"]["code"] == -32601
assert by_id[4]["error"]["message"] == "tool/action disabled by read-only mode"
stderr = pathlib.Path(sys.argv[2]).read_text()
assert "read-only mode: enabled" in stderr
assert "100.123.0.63" not in stderr
assert "GATUS_API_URL" not in stderr
PY
```

The script asserts:

- every non-empty stdout line is JSON and responses with IDs 1–4 are present;
- initialization succeeds;
- `tools/list` exposes only `manage_resources` and `get_metrics`;
- `manage_resources/get-health` reports the live instance is up;
- `trigger_check` returns code `-32601` and the stable message;
- stderr contains `read-only mode: enabled`;
- stderr contains no Gatus URL or environment-variable echo.

The notification may produce no output or the current implementation's
`null`; neither behavior is made part of the access-control contract.

If Docker is unavailable, record this checkpoint as unverified rather than
substituting the in-process stdio test.

- [ ] **Step 5: Record final repository state**

Run:

```bash
git status --short
git log --oneline -6
```

Expected: only the user-owned untracked `RELAXGG_SCOPE.md` remains, and the
implementation commits are visible. Do not add or modify that scope file.
