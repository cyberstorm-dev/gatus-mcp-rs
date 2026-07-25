# Read-Only Mode Design

## Objective

Add a fail-closed read-only access mode for safe use as the Relax.gg
`gatus_ro` MCP server. Read-write behavior remains the default so existing
users and tests retain their current behavior.

This repository change covers the server, CLI, tests, Docker invocation
contract, and user documentation. Publishing a pinned image and changing
infra-registry are follow-up work after this implementation is available as a
git-SHA-tagged image.

## Access Policy

Read-only mode allows these tools:

- `manage_resources`
- `get_metrics`

Every action currently advertised or dispatched by these two tools is
read-only and remains available. `manage_resources` currently contains:

- `list-services`
- `list-groups`
- `list-endpoints`
- `get-config`
- `get-health`
- `list-expiring-certificates`
- `get-alert-rules`
- `get-suite-health`

`get_metrics` currently contains:

- `system-stats`
- `service-details`
- `service-history`
- `get-raw-results`
- `group-summary`
- `uptime`
- `uptime-granular`
- `response-time`
- `alert-history`
- `get-badge`
- `get-latency-badge`
- `get-latency-chart`
- `failure-summary`
- `performance-comparison`
- `group-stats`
- `alert-correlation`
- `flapping-services`
- `diagnostic-bundle`
- `certificate-audit`

Read-only mode disables these tools in full:

- `trigger_check`
- `test_alert`
- `reload_config`
- `push_result`
- `manage_endpoints`

Although `manage_endpoints` includes the read action `list-suites`, the product
contract explicitly disables the entire mixed-purpose tool. The allowed
`manage_resources` action `get-suite-health` can inspect a known suite ID but
cannot discover suite IDs, so read-only mode intentionally does not provide
suite discovery in this pass.

The policy fails closed. Any future top-level tool is hidden and blocked in
read-only mode until it is explicitly added to the allowlist. Any future action
added to an allowed multi-action tool must be explicitly classified before it
is permitted in read-only mode.

## Architecture

`McpHandler` is the single enforcement boundary because stdio, HTTP/SSE,
`list-tools`, and `call-tool` all route through it. The handler gains an
explicit access-mode field. Existing `McpHandler::new` and
`McpHandler::new_with_arc` constructors continue to create read-write handlers
for backward compatibility, while access-mode-aware constructors create
read-only handlers.

The handler uses the same policy for discovery and execution:

1. `tools/list` filters definitions through the read-only allowlist.
2. `tools/call` checks the requested tool and, for allowed multi-action tools,
   its action before dispatch.
3. A rejected request returns an MCP JSON-RPC error before the Gatus client is
   called.

This avoids transport-specific policy and prevents a hidden tool from remaining
callable by name.

## CLI and Configuration

The top-level CLI gains:

```text
--read-only
GATUS_MCP_READ_ONLY=true
```

The flag is global and is accepted before or after every subcommand, including
both:

```bash
gatus-mcp-rs --read-only stdio
gatus-mcp-rs stdio --read-only
```

The resolved rule is additive:

```text
read_only = cli_flag || environment_value
```

In practice Clap resolves the global boolean argument from the CLI and its
environment binding. The default is `false`; either `--read-only` or
`GATUS_MCP_READ_ONLY=true` makes it `true`. There is deliberately no CLI
argument that can downgrade an environment-enforced read-only process.
Unsupported boolean environment values cause normal CLI parsing failure rather
than silently selecting a mode.

The resolved access mode applies equally to:

```text
gatus-mcp-rs
├── stdio
├── http
├── list-tools
└── call-tool
```

Every invocation logs `read-only mode: enabled` or
`read-only mode: disabled` to stderr after logging is initialized, including
stdio, HTTP, `list-tools`, and `call-tool`. Startup output must not print the
configured Gatus URL, API keys, usernames, passwords, or other credentials;
removing the current raw URL log avoids leaking URL userinfo or secret query
parameters.

## Machine-Readable Contract

MCP stdio and HTTP responses retain the existing JSON-RPC 2.0 envelope. Direct
CLI commands retain their current pretty-printed JSON-RPC response. No new
prose-only or custom wrapper is introduced.

Successful discovery has this shape:

```json
{
  "jsonrpc": "2.0",
  "result": {
    "tools": []
  },
  "id": 1
}
```

A disabled tool or action has this shape:

```json
{
  "jsonrpc": "2.0",
  "error": {
    "code": -32601,
    "message": "tool/action disabled by read-only mode"
  },
  "id": 1
}
```

The stable error code denotes an unavailable method in the active server mode.
The stable message is suitable for agent branching and matches the product
contract. Rejected operations do not silently no-op and do not send an HTTP
request to Gatus.

Output-size behavior does not change. `tools/list` is naturally bounded by the
tool catalog, MCP calls retain the existing per-action formatting and limits,
and no logs or deep parser representations are added to protocol output.

Agents and users discover the mode through `--help`, README examples, startup
logs, and the filtered `tools/list` response.

Representative commands are:

```bash
gatus-mcp-rs --read-only stdio
gatus-mcp-rs stdio --read-only
GATUS_MCP_READ_ONLY=true gatus-mcp-rs list-tools
GATUS_MCP_READ_ONLY=true gatus-mcp-rs call-tool trigger_check '{}'
```

The last command returns the stable JSON-RPC error and does not contact Gatus.

## Data Flow

At startup, Clap resolves the global read-only option. `run_app_with_stdio`
loads normal Gatus settings, logs the non-secret access mode, constructs a
handler with that mode, and passes it to the selected entry point.

For HTTP mode, `create_app` receives the resolved access mode and installs a
handler with the same policy in Axum state. Background polling remains enabled
because it only reads endpoint statuses.

For an MCP request, the handler parses JSON-RPC as today. Discovery calls apply
the allowlist to tool definitions. Execution calls consult the same policy,
return the read-only error when denied, and only then dispatch allowed calls to
the existing Gatus client methods.

## Testing

Tests follow red-green-refactor and cover the access boundary rather than only
the displayed catalog.

CLI/configuration coverage verifies:

- default mode is read-write;
- `--read-only` before a subcommand enables read-only;
- `--read-only` after a subcommand enables read-only;
- `GATUS_MCP_READ_ONLY=true` enables read-only;
- CLI and environment combination is deterministic;
- the setting applies to stdio, HTTP, `list-tools`, and `call-tool`;
- a process invocation emits the resolved mode to stderr without echoing a
  credential-bearing Gatus URL.

Handler coverage verifies:

- default handlers still advertise and execute existing mutating tools;
- read-only `tools/list` contains exactly `manage_resources` and `get_metrics`;
- every explicitly disabled tool returns JSON-RPC error `-32601` with the
  stable message;
- rejected mutating calls do not reach a Wiremock server;
- an allowed `manage_resources` action succeeds against Wiremock;
- an allowed `get_metrics` action succeeds against Wiremock;
- each currently classified action is permitted;
- an unclassified or mutating action on an allowed multi-action tool is denied
  before dispatch.

Stdio integration coverage sends newline-delimited JSON-RPC through the server
loop and performs the complete smoke sequence:

1. `initialize`
2. `notifications/initialized`
3. `tools/list`
4. assert mutating tools are absent
5. call an allowed health/status action
6. call a disabled mutating tool and assert the error

The final verification commands are:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

Existing upstream Clippy failures, if any, are reported rather than suppressed.

After the Rust checks pass, build the repository Docker image with a local
pinned tag and run the same newline-delimited smoke probe through the container
entrypoint using the required trailing argument shape:

```bash
docker build -t gatus-mcp-rs:read-only-smoke .
docker run -i --rm \
  -e GATUS_API_URL=http://host.docker.internal:<mock-port> \
  --add-host host.docker.internal:host-gateway \
  gatus-mcp-rs:read-only-smoke \
  stdio --read-only
```

The probe must execute `initialize`, `notifications/initialized`, `tools/list`,
one allowed call against the mock Gatus server, and one disabled call. It must
assert that stdout contains only newline-delimited protocol responses, the
mutating tools are absent, the allowed call succeeds, the denied call returns
the stable error, and stderr identifies read-only mode without secrets. Record
the exact image smoke result in the implementation handoff. If Docker is
unavailable in the execution environment, report that as an unverified release
checkpoint rather than treating the in-process test as equivalent.

## Documentation and Docker

README documents:

- the `GATUS_MCP_READ_ONLY` variable;
- global `--read-only` usage;
- deterministic resolution and the inability to downgrade environment-enforced
  read-only mode;
- the allowed and disabled tool sets;
- the stable disabled-operation error;
- a Docker stdio example using a pinned git SHA rather than `latest`.

The existing Dockerfile entrypoint already forwards arguments to the binary, so
it requires no structural change for:

```bash
docker run -i --rm \
  -e GATUS_API_URL=http://100.123.0.63:3003 \
  ghcr.io/relax-dot-gg/gatus-mcp-rs:<git-sha> \
  stdio --read-only
```

## Out of Scope

- Implementing or deploying `gatus_rw`
- Adding Gatus authentication
- Publishing an image before this code is committed
- Changing infra-registry before a pinned image exists
- Redesigning MCP transport or JSON-RPC handling
- Unrelated refactors
