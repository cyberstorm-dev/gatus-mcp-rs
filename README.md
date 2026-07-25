[![task](https://img.shields.io/badge/Task-Enabled-brightgreen?style=for-the-badge&logo=task&logoColor=white)](https://taskfile.dev/#/)

# Gatus MCP Server (Rust) :robot:

> [!IMPORTANT]
> This project is currently in active development (v0.1.4) and is **not production-ready**. Features may change, and breaking changes may occur without notice.

A Model Context Protocol (MCP) server for [Gatus](https://gatus.io), the automated health check dashboard. This server enables Large Language Models (LLMs) to interact with Gatus APIs to monitor service health, retrieve check history, and diagnose issues.

> [!IMPORTANT]
> This MCP server currently only works with **self-hosted Gatus instances** and is not compatible with the `gatus.io` managed service.

## :sparkles: Features

- **Model Context Protocol (MCP):** Native support for MCP, allowing easy integration with AI tools like Claude Desktop.
- **Service Monitoring:** List all monitored services, groups, and endpoints with their current statuses (UP/DOWN/DEGRADED).
- **System Health Summary:** High-level overview of total, up, down, and degraded endpoint counts.
- **Detailed Diagnostics:** Fetch latest results (including detailed condition status ✅/❌), history (optimized with targeted API calls), raw non-truncated results, and granular performance metrics for specific health checks.
- **Alert & State Transitions:** Retrieve chronological alert history and configured alerting rules to identify incident root causes.
- **One-Turn Root Cause Analysis:** Aggregates failure summaries, recent results, and alert events into a single "Diagnostic Bundle" via the `get_metrics` tool, enabling rapid problem resolution.
- **Notification Verification:** Trigger test alert notifications for specific endpoints to verify provider integrations.
- **Status & Performance Visualization:** Surface health/uptime badges and latency charts (SVG) directly via MCP.
- **External Result Pushing:** Push health check results for external/asynchronous endpoints directly to Gatus.
- **Uptime Calculation:** Calculate success vs. failure ratios over 24h, 7d, and 30d timeframes.
- **Configuration Retrieval:** Retrieve the effective Gatus monitoring configuration (conditions, names, groups).
- **Endpoint Lifecycle Management:** Create, update, and delete endpoints and suites directly via MCP.
- **Multiple Transports:** Support for both Stdio and HTTP (SSE) transport layers.
- **Optimized for LLMs:** Returns "thinned" payloads to conserve token usage while providing high-signal information.
- **Flexible Configuration:** Configure via environment variables, `config.toml`, or CLI flags.

## :hammer_and_wrench: Installation

### Prerequisites

- Rust 1.8x or higher.
- A running Gatus instance.

### Homebrew

```bash
brew install nicholaswilde/tap/gatus-mcp-rs
```

### From Source

```bash
git clone https://github.com/nicholaswilde/gatus-mcp-rs.git
cd gatus-mcp-rs
cargo build --release
```

The binary will be available at `target/release/gatus-mcp-rs`.

## :gear: Configuration

You can configure the server using environment variables or a `config.toml` file.

### Environment Variables

- `GATUS_API_URL`: The base URL of your Gatus instance (e.g., `http://localhost:8080`).
- `GATUS_API_KEY`: (Optional) Your Gatus API key for authentication.
- `GATUS_SERVER_PORT`: Port for the HTTP server (default: `8080`).
- `GATUS_MCP_READ_ONLY`: Set to `true` to enable read-only mode (default: `false`).
- `LOG_LEVEL`: Logging level (`error`, `warn`, `info`, `debug`, `trace`).

### Example `config.toml`

```toml
[server]
port = 8080
host = "127.0.0.1"

[gatus]
api_url = "http://localhost:8080"
# api_key = "your-api-key"
```

## :rocket: Usage

### Stdio Mode (Default)

Ideal for integration with desktop LLM clients.

```bash
gatus-mcp-rs stdio
```

### HTTP (SSE) Mode

For remote clients or distributed setups.

```bash
gatus-mcp-rs http --port 8080 --host 0.0.0.0
```

### Command Line Interface

```bash
Usage: gatus-mcp-rs [OPTIONS] [COMMAND]

Commands:
  stdio       Run in stdio mode (default)
  http        Run in HTTP (SSE) mode
  list-tools  List available MCP tools
  call-tool   Call a specific tool directly
  help        Print this message or the help of the given subcommand(s)

Options:
  -u, --gatus-url <GATUS_URL>    Gatus API URL [env: GATUS_API_URL=]
  -k, --api-key <API_KEY>        Gatus API Key [env: GATUS_API_KEY]
  -l, --log-level <LOG_LEVEL>    Log level [default: info]
  -f, --log-format <LOG_FORMAT>  Log format (text, json) [default: text]
      --read-only                Disable tools and actions that mutate Gatus [env: GATUS_MCP_READ_ONLY=]
  -h, --help                     Print help
  -V, --version                  Print version
```

### Read-only Mode

The server runs in read-write mode by default. Read-only mode is enabled when either the global `--read-only` flag is present or `GATUS_MCP_READ_ONLY=true`. The flag only enables read-only mode: if the environment variable already enforces it, there is no CLI option that downgrades the process to read-write mode.

Because `--read-only` is global, it works before or after every subcommand and applies consistently to `stdio`, `http`, `list-tools`, and `call-tool`:

```bash
gatus-mcp-rs --read-only stdio
gatus-mcp-rs stdio --read-only
GATUS_MCP_READ_ONLY=true gatus-mcp-rs list-tools
gatus-mcp-rs call-tool trigger_check '{"id":"core_service-1"}' --read-only
```

In read-only mode, only `manage_resources` and `get_metrics` are advertised. Their allowed actions are the canonical action lists in the [MCP Tool Reference](#mcp-tool-reference) below. The `get-suite-health` action requires a known suite ID; it does not discover suites.

The tools `trigger_check`, `test_alert`, `reload_config`, `push_result`, and `manage_endpoints` are disabled. `manage_endpoints` is mixed-purpose and includes the non-mutating `list-suites` action, but read-only mode disables the entire tool, so suite discovery is intentionally unavailable. Calls to disabled tools or unclassified actions return JSON-RPC error `-32601` with the exact message `tool/action disabled by read-only mode`; they never silently no-op.

For container deployments, pin the image to a tested Git commit and pass the global option to the stdio command:

```bash
docker run --rm -i \
  -e GATUS_API_URL=http://host.docker.internal:8080 \
  ghcr.io/relax-dot-gg/gatus-mcp-rs:sha-<short-git-sha> \
  stdio --read-only
```

Use the exact published `sha-...` tag for the commit you tested, or pin the image by digest. Never use `latest` for a pinned deployment.

## MCP Tool Reference

### `manage_resources`
Discover and manage Gatus resources and instance state.
- **Arguments:**
  - `action`: `list-services`, `list-groups`, `list-endpoints`, `get-config`, `get-health`, `list-expiring-certificates`, `get-alert-rules`, or `get-suite-health`.
  - `id`: (Optional) Identifier (e.g., group name for `list-endpoints`, or suite ID for `get-suite-health`).
  - `status`: (Optional) Status filter for `list-services` (e.g., `DOWN`, `DEGRADED`).

### `get_metrics`
Retrieve status, metrics, and history for services and endpoints.
- **Arguments:**
  - `action`: `system-stats`, `service-details`, `service-history` (optimized), `get-raw-results` (non-truncated), `group-summary`, `uptime`, `uptime-granular`, `response-time`, `alert-history`, `get-badge`, `get-latency-badge`, `get-latency-chart`, `failure-summary`, `performance-comparison`, `group-stats`, `alert-correlation`, `flapping-services`, `diagnostic-bundle`, or `certificate-audit`.
  - `id`: (Optional) Identifier (e.g., service name for `service-details`, group name for `group-summary`, or endpoint key for `service-history`, `uptime-granular`, `response-time`, badge/chart actions, `failure-summary`, `performance-comparison`, `group-stats`, and `alert-correlation`).
  - `limit`: (Optional) Maximum number of results for history actions (default: 10 for `service-history` and `get-raw-results`, 5 for `alert-history`).
  - `timeframe`: (Optional) `1h`, `24h`, `7d`, or `30d` (default: `24h`) for `uptime`, `uptime-granular`, `response-time`, and badge/chart actions.

### `trigger_check`
Force an immediate health check for a specific endpoint.
- **Arguments:**
  - `id`: The service name or endpoint key to check.

### `test_alert`
Trigger a test alert notification for an endpoint.
- **Arguments:**
  - `id`: The service name or endpoint key to test.

### `reload_config`
Trigger a Gatus configuration reload.
- **Arguments:** None.

### `push_result`
Push a health check result for an external endpoint.
- **Arguments:**
  - `id`: The endpoint key (usually `group_name_endpoint_name`).
  - `success`: Whether the health check was successful (boolean).
  - `duration`: (Optional) Duration of the health check in milliseconds.
  - `error`: (Optional) Error message if the check was unsuccessful.

### `manage_endpoints`
Programmatically manage Gatus endpoints and suites.
- **Arguments:**
  - `action`: `list-suites`, `create-endpoint`, `update-endpoint`, or `delete-endpoint`.
  - `suite_id`: (Required for create/update/delete) The ID of the suite.
  - `endpoint_id`: (Required for update/delete) The ID of the endpoint.
  - `config`: (Required for create/update) Endpoint configuration object.
    - `name`: (Required) Display name for the endpoint.
    - `group`: (Optional) Group name.
    - `url`: (Required) Target URL/hostname.
    - `interval`: (Optional) Check interval (e.g., `30s`, `1m`).
    - `conditions`: (Required) List of Gatus condition strings.
    - `method`: (Optional) HTTP method (e.g., `POST`).
    - `body`: (Optional) Request body.

## :handshake: Contributing

Contributions are welcome! Please follow standard Rust coding conventions and ensure all tests pass (`task test:ci`) before submitting features.

## :balance_scale: License

[Apache License 2.0](LICENSE)

## :writing_hand: Author

This project was started in 2026 by [Nicholas Wilde](https://github.com/nicholaswilde/).
