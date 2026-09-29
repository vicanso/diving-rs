//! End-to-end test of the `/mcp` endpoint over real HTTP.
//!
//! Serves `mcp::new_router` on an ephemeral loopback port and speaks
//! JSON-RPC to it the way an MCP client does: initialize, list tools, then
//! call them against the `file://` docker-save fixture (no network; local
//! images never touch the analysis cache).

mod common;

use common::build_fixture_tar;
use diving::mcp::new_router;
use reqwest::{Client, RequestBuilder, StatusCode};
use rustls::crypto::ring::default_provider;
use serde_json::{json, Value};
use tempfile::NamedTempFile;

const PROTOCOL_VERSION: &str = "2025-06-18";

async fn spawn(token: Option<&str>) -> String {
    let router = new_router(token.map(str::to_string));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{addr}/mcp")
}

/// reqwest is built with `rustls-no-provider`; install `ring` like
/// `main.rs` does (a second install from another test just errors).
fn client() -> Client {
    let _ = default_provider().install_default();
    Client::new()
}

fn post(client: &Client, url: &str, body: Value) -> RequestBuilder {
    client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", PROTOCOL_VERSION)
        .json(&body)
}

/// The JSON-RPC response to `id`, whether the server answered with plain
/// JSON or an SSE stream (it switches to SSE once it sends notifications).
async fn rpc_response(resp: reqwest::Response, id: u64) -> Value {
    assert_eq!(resp.status(), StatusCode::OK);
    let is_sse = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/event-stream"));
    let body = resp.text().await.unwrap();
    if !is_sse {
        return serde_json::from_str(&body).unwrap();
    }
    body.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .filter_map(|data| serde_json::from_str::<Value>(data.trim()).ok())
        .find(|msg| msg["id"] == id)
        .unwrap_or_else(|| panic!("no response for id {id} in: {body}"))
}

async fn call(client: &Client, url: &str, id: u64, method: &str, params: Value) -> Value {
    let body = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    rpc_response(post(client, url, body).send().await.unwrap(), id).await
}

async fn call_tool(client: &Client, url: &str, id: u64, name: &str, args: Value) -> Value {
    let resp = call(
        client,
        url,
        id,
        "tools/call",
        json!({"name": name, "arguments": args}),
    )
    .await;
    assert!(resp["error"].is_null(), "{name} failed: {resp}");
    resp["result"].clone()
}

fn tool_text(result: &Value) -> &str {
    result["content"][0]["text"].as_str().unwrap()
}

fn fixture_image() -> (NamedTempFile, String) {
    let tmp = tempfile::Builder::new().suffix(".tar").tempfile().unwrap();
    std::fs::write(tmp.path(), build_fixture_tar()).unwrap();
    let image = format!("file://{}", tmp.path().display());
    (tmp, image)
}

#[tokio::test(flavor = "multi_thread")]
async fn mcp_tools_analyze_local_image() {
    let url = spawn(None).await;
    let client = client();
    let (_tmp, image) = fixture_image();

    let init = call(
        &client,
        &url,
        1,
        "initialize",
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {"name": "test", "version": "1.0"},
        }),
    )
    .await;
    assert_eq!(init["result"]["serverInfo"]["name"], "diving");
    assert!(init["result"]["capabilities"]["tools"].is_object());

    let tools = call(&client, &url, 2, "tools/list", json!({})).await;
    let mut names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "analyze_image",
            "get_findings",
            "latest_images",
            "list_files",
            "read_file"
        ]
    );

    let report = call_tool(
        &client,
        &url,
        3,
        "analyze_image",
        json!({"image": image, "lang": "en", "skip_base": false}),
    )
    .await;
    assert_ne!(report["isError"], true, "{report}");
    assert!(tool_text(&report).contains("data/model.bin"));

    // Layer numbers are 1-based: the model file lives in the third history
    // entry (the second is an empty ENV layer).
    let page = call_tool(
        &client,
        &url,
        4,
        "list_files",
        json!({"image": image, "keyword": "MODEL"}),
    )
    .await;
    let page: Value = serde_json::from_str(tool_text(&page)).unwrap();
    assert_eq!(page["total"], 1);
    assert_eq!(page["files"][0]["layer"], 3);
    assert_eq!(page["files"][0]["path"], "data/model.bin");
    assert_eq!(page["files"][0]["op"], "added");

    let findings = call_tool(&client, &url, 5, "get_findings", json!({"image": image})).await;
    let findings: Value = serde_json::from_str(tool_text(&findings)).unwrap();
    let secret = findings["sensitiveFiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "app/secret.pem")
        .expect("secret.pem flagged");
    assert_eq!(secret["layer"], 1);
    assert!(secret.get("layerIndex").is_none());
    assert_eq!(findings["layers"].as_array().unwrap().len(), 3);

    // Tool failures come back as `isError` results the model can read.
    let err = call_tool(
        &client,
        &url,
        6,
        "read_file",
        json!({"image": image, "layer": 1, "path": "etc/config.yml"}),
    )
    .await;
    assert_eq!(err["isError"], true);
    assert!(tool_text(&err).contains("registry images"));

    let latest = call_tool(&client, &url, 7, "latest_images", json!({})).await;
    assert!(tool_text(&latest).contains(".tar"));
}

#[tokio::test]
async fn mcp_token_is_required_when_configured() {
    let url = spawn(Some("s3cret")).await;
    let client = client();
    let ping = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"});

    let resp = post(&client, &url, ping.clone()).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(resp.headers()["www-authenticate"], "Bearer");

    let resp = post(&client, &url, ping.clone())
        .bearer_auth("wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // With a token the Host check is off, so a public hostname is fine.
    let resp = post(&client, &url, ping)
        .bearer_auth("s3cret")
        .header("Host", "diving.example.com")
        .send()
        .await
        .unwrap();
    let resp = rpc_response(resp, 1).await;
    assert!(resp["error"].is_null(), "{resp}");
}

#[tokio::test]
async fn mcp_rejects_foreign_host_without_token() {
    let url = spawn(None).await;
    let resp = post(
        &client(),
        &url,
        json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}),
    )
    .header("Host", "attacker.example.com")
    .send()
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
