use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use clap::Parser;
use gatus_mcp_rs::{
    cli::Cli,
    mcp::{AccessMode, READ_ONLY_ERROR_MESSAGE},
    server::create_app_with_access_mode,
    settings::Settings,
};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use tower::ServiceExt;

const BIN: &str = env!("CARGO_BIN_EXE_gatus-mcp-rs");
const SECRET_URL: &str = "http://cli-user:cli-password@127.0.0.1:9?token=cli-query-secret";

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn clean_command() -> Command {
    let mut command = Command::new(BIN);
    command
        .env_remove("RUST_LOG")
        .env("LOG_LEVEL", "info")
        .env_remove("GATUS_MCP_READ_ONLY")
        .env_remove("GATUS_API_URL");
    command
}

fn tool_names(output: &[u8]) -> Vec<String> {
    let response: Value = serde_json::from_slice(output).unwrap();
    response["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn read_only_flag_is_global_for_every_subcommand() {
    let subcommands: &[&[&str]] = &[
        &["stdio"],
        &["http"],
        &["list-tools"],
        &["call-tool", "trigger_check"],
    ];

    for subcommand in subcommands {
        let before = std::iter::once("gatus-mcp-rs")
            .chain(std::iter::once("--read-only"))
            .chain(subcommand.iter().copied());
        assert!(Cli::try_parse_from(before).unwrap().read_only);

        let after = std::iter::once("gatus-mcp-rs")
            .chain(subcommand.iter().copied())
            .chain(std::iter::once("--read-only"));
        assert!(Cli::try_parse_from(after).unwrap().read_only);
    }
}

#[test]
fn env_true_list_tools_is_read_only_and_does_not_log_credentials() {
    let output = clean_command()
        .arg("list-tools")
        .env("GATUS_MCP_READ_ONLY", "true")
        .env("GATUS_API_URL", SECRET_URL)
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        tool_names(&output.stdout),
        vec!["manage_resources", "get_metrics"]
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("read-only mode: enabled"), "{stderr}");
    assert!(!stderr.contains("cli-user"), "{stderr}");
    assert!(!stderr.contains("cli-password"), "{stderr}");
    assert!(!stderr.contains("cli-query-secret"), "{stderr}");
}

#[test]
fn flag_overrides_false_environment_value() {
    let output = clean_command()
        .args(["--read-only", "list-tools"])
        .env("GATUS_MCP_READ_ONLY", "false")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        tool_names(&output.stdout),
        vec!["manage_resources", "get_metrics"]
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("read-only mode: enabled"), "{stderr}");
}

#[test]
fn false_environment_value_preserves_read_write_default() {
    let output = clean_command()
        .arg("list-tools")
        .env("GATUS_MCP_READ_ONLY", "false")
        .output()
        .unwrap();

    assert!(output.status.success());
    let names = tool_names(&output.stdout);
    assert!(names.iter().any(|name| name == "trigger_check"));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("read-only mode: disabled"), "{stderr}");
}

#[test]
fn invalid_environment_value_is_rejected_by_clap() {
    let output = clean_command()
        .arg("list-tools")
        .env("GATUS_MCP_READ_ONLY", "definitely")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("invalid value"), "{stderr}");
    assert!(stderr.contains("--read-only"), "{stderr}");
}

#[test]
fn read_only_call_tool_rejects_mutation_without_http() {
    let output = clean_command()
        .args(["call-tool", "trigger_check", r#"{"id":"core_service-1"}"#])
        .env("GATUS_MCP_READ_ONLY", "true")
        .env("GATUS_API_URL", "http://127.0.0.1:9")
        .output()
        .unwrap();

    assert!(output.status.success());
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["error"]["code"], -32601);
    assert_eq!(response["error"]["message"], READ_ONLY_ERROR_MESSAGE);
}

#[tokio::test]
async fn read_only_http_messages_list_only_safe_tools() {
    let app = create_app_with_access_mode(Settings::default(), AccessMode::ReadOnly);
    let response = app
        .oneshot(
            Request::post("/messages")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"jsonrpc":"2.0","method":"tools/list","id":1}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();

    assert_eq!(tool_names(&body), vec!["manage_resources", "get_metrics"]);
}

#[test]
fn http_polling_error_and_mode_log_do_not_expose_credentials() {
    let child = clean_command()
        .args(["--read-only", "http", "--host", "127.0.0.1", "--port", "0"])
        .env("GATUS_API_URL", SECRET_URL)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child = ChildGuard(child);
    let stderr = child.0.stderr.take().unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || -> std::io::Result<String> {
        let mut observed = String::new();
        for line in BufReader::new(stderr).lines() {
            let line = line?;
            observed.push_str(&line);
            observed.push('\n');
            let _ = sender.send(line.contains("Failed to poll Gatus for state changes"));
        }
        Ok(observed)
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut saw_polling_error = false;
    while !saw_polling_error {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        match receiver.recv_timeout(remaining) {
            Ok(line_has_polling_error) => saw_polling_error = line_has_polling_error,
            Err(_) => break,
        }
    }

    let _ = child.0.kill();
    let wait_result = child.0.wait();
    let reader_result = reader.join();

    assert!(wait_result.is_ok(), "failed to wait for HTTP child process");
    let observed = reader_result
        .expect("stderr reader thread panicked")
        .expect("failed to read child stderr");

    assert!(saw_polling_error, "timed out waiting for polling error");
    assert!(observed.contains("read-only mode: enabled"), "{observed}");
    assert!(
        observed.contains("Failed to poll Gatus for state changes"),
        "{observed}"
    );
    assert!(!observed.contains("cli-user"), "{observed}");
    assert!(!observed.contains("cli-password"), "{observed}");
    assert!(!observed.contains("cli-query-secret"), "{observed}");
}
