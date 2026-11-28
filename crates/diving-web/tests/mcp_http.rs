//! End-to-end test of the `/mcp` endpoint over real HTTP.
//!
//! Serves `mcp::new_router` on an ephemeral loopback port and speaks
//! JSON-RPC to it the way an MCP client does: initialize, list tools, then
//! call them against the `file://` docker-save fixture (no network; local
//! images never touch the analysis cache).

mod common;

use bytesize::ByteSize;
use common::{build_fixture_tar, build_fixture_v2_tar, test_web_state};
use diving_app::config::build_http_client;
use diving_app::logging::install_crypto_provider;
use diving_core::{Analyzer, CacheConfig, GateConfig};
use diving_web::controller;
use diving_web::mcp::new_router;
use diving_web::state::{McpStatus, WebState};
use reqwest::{Client, RequestBuilder, StatusCode, Url};
use serde_json::{json, Value};
use std::env;
use std::sync::Arc;
use tar::{Builder as TarBuilder, Header as TarHeader};
use tempfile::NamedTempFile;
use tokio::net::TcpListener;

const PROTOCOL_VERSION: &str = "2025-06-18";

async fn spawn(token: Option<&str>) -> String {
    let state = test_web_state(McpStatus {
        enabled: true,
        token_required: token.is_some(),
    });
    let router = new_router(state, token.map(str::to_string));
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
    install_crypto_provider();
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
    tar_image(build_fixture_tar())
}

fn tar_image(data: Vec<u8>) -> (NamedTempFile, String) {
    let tmp = tempfile::Builder::new().suffix(".tar").tempfile().unwrap();
    std::fs::write(tmp.path(), data).unwrap();
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
            "diff_images",
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

    // `read_file` works on local `file://` images (both cached sha256 blobs and tar entries)
    // and reports binary files by full size while returning tool errors as `isError`.
    let cfg = call_tool(
        &client,
        &url,
        6,
        "read_file",
        json!({"image": image, "layer": 1, "path": "etc/config.yml"}),
    )
    .await;
    assert_ne!(cfg["isError"], true, "{cfg}");
    assert_eq!(tool_text(&cfg), "a: 1");

    let bin = call_tool(
        &client,
        &url,
        7,
        "read_file",
        json!({"image": image, "layer": 3, "path": "data/model.bin"}),
    )
    .await;
    assert_ne!(bin["isError"], true, "{bin}");
    assert!(tool_text(&bin).contains("binary file"), "{bin}");
    assert!(
        tool_text(&bin).contains(&ByteSize(1_500_000).to_string()),
        "{bin}"
    );

    let err = call_tool(
        &client,
        &url,
        8,
        "read_file",
        json!({"image": image, "layer": 2, "path": "etc/config.yml"}),
    )
    .await;
    assert_eq!(err["isError"], true);
    assert!(tool_text(&err).contains("no file content"));

    let latest = call_tool(&client, &url, 9, "latest_images", json!({})).await;
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

    // With a token the Host check is off, so a public hostname is fine (case-insensitive Bearer).
    let resp = post(&client, &url, ping)
        .header("Authorization", "bearer s3cret")
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

/// The web UI's MCP button and setup dialog are driven by this field.
#[tokio::test]
async fn latest_images_reports_mcp_status() {
    let status = McpStatus {
        enabled: true,
        token_required: true,
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, controller::new_router(test_web_state(status)))
            .await
            .unwrap();
    });

    let resp: Value = client()
        .get(format!("http://{addr}/api/latest-images"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["mcp"], json!({"enabled": true, "tokenRequired": true}));
    assert!(resp["version"].is_string());
}

/// Web responses are gzipped for clients that ask, and left alone otherwise
/// (the test above reads the same endpoint as plain JSON).
#[tokio::test]
async fn api_responses_are_gzipped_on_request() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            controller::new_router(test_web_state(McpStatus::default())),
        )
        .await
        .unwrap();
    });

    let resp = client()
        .get(format!("http://{addr}/api/latest-images"))
        .header("Accept-Encoding", "gzip")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.headers()["content-encoding"], "gzip");
    let body = resp.bytes().await.unwrap();
    assert_eq!(&body[..2], [0x1f, 0x8b], "gzip magic bytes");
}

/// The same comparison over MCP (Markdown) and `/api/diff` (JSON).
#[tokio::test(flavor = "multi_thread")]
async fn diff_images_over_mcp_and_api() {
    let (_v1, base) = fixture_image();
    let (_v2, target) = tar_image(build_fixture_v2_tar());
    let client = client();

    let url = spawn(None).await;
    let md = call_tool(
        &client,
        &url,
        1,
        "diff_images",
        json!({"base": base, "target": target, "lang": "en"}),
    )
    .await;
    assert_ne!(md["isError"], true, "{md}");
    assert!(tool_text(&md).contains("`data/extra.bin`"));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            controller::new_router(test_web_state(McpStatus::default())),
        )
        .await
        .unwrap();
    });
    let diff: Value = client
        // temp-file paths need no escaping in a query string
        .get(format!(
            "http://{addr}/api/diff?base={base}&target={target}"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(diff["sharedLayers"], 1);
    assert_eq!(diff["added"]["count"], 2);
    assert_eq!(diff["changed"]["files"][0]["path"], "data/model.bin");
}

/// `/api/manifest` reads a registry manifest; a local source has none to
/// offer short of the full `docker save` / tar read, so it says so.
#[tokio::test]
async fn manifest_preview_is_registry_only() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            controller::new_router(test_web_state(McpStatus::default())),
        )
        .await
        .unwrap();
    });
    let resp = client()
        .get(format!(
            "http://{addr}/api/manifest?image=file:///tmp/image.tar"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: Value = resp.json().await.unwrap();
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("only available for registry images"));
}

fn multi_arch_test_tar(arch: &str, base_bytes: &[u8], bin_bytes: &[u8], user: &str) -> Vec<u8> {
    let mut l1 = TarBuilder::new(Vec::new());
    let mut h1 = TarHeader::new_gnu();
    h1.set_path("lib/libc.so.6").unwrap();
    h1.set_size(base_bytes.len() as u64);
    h1.set_mode(0o644);
    h1.set_uid(0);
    h1.set_gid(0);
    h1.set_cksum();
    l1.append(&h1, base_bytes).unwrap();
    let layer1 = l1.into_inner().unwrap();

    let mut l2 = TarBuilder::new(Vec::new());
    for (p, data, mode) in [
        ("usr/bin/app", bin_bytes, 0o755),
        ("etc/cfg.json", br#"{"ok":1}"#.as_slice(), 0o644),
    ] {
        let mut h = TarHeader::new_gnu();
        h.set_path(p).unwrap();
        h.set_size(data.len() as u64);
        h.set_mode(mode);
        h.set_uid(0);
        h.set_gid(0);
        h.set_cksum();
        l2.append(&h, data).unwrap();
    }
    let layer2 = l2.into_inner().unwrap();

    let cfg = format!(
        r#"{{
        "architecture": "{arch}",
        "created": "2024-06-01T00:00:00Z",
        "history": [
            {{"created": "2024-01-01T00:00:00Z", "created_by": "RUN apt-get install"}},
            {{"created": "2024-06-01T00:00:00Z", "created_by": "COPY app cfg.json /"}}
        ],
        "os": "linux",
        "rootfs": {{"type": "layers", "diff_ids": ["sha256:os-{arch}", "sha256:app-{arch}"]}},
        "config": {{"User": "{user}", "Entrypoint": ["/usr/bin/app"]}}
    }}"#
    );
    let manifest =
        br#"[{"Config":"config.json","RepoTags":["svc:latest"],"Layers":["l1.tar","l2.tar"]}]"#;
    let mut outer = TarBuilder::new(Vec::new());
    for (name, bytes) in [
        ("manifest.json", manifest.as_slice()),
        ("config.json", cfg.as_bytes()),
        ("l1.tar", &layer1),
        ("l2.tar", &layer2),
    ] {
        let mut h = TarHeader::new_gnu();
        h.set_path(name).unwrap();
        h.set_size(bytes.len() as u64);
        h.set_mode(0o644);
        h.set_uid(0);
        h.set_gid(0);
        h.set_cksum();
        outer.append(&h, bytes).unwrap();
    }
    outer.into_inner().unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn multi_arch_diff_supports_mode_override_over_http_and_mcp() {
    let (_amd, base) = tar_image(multi_arch_test_tar(
        "amd64",
        &[1u8; 4096],
        &[0x7f; 1000],
        "app",
    ));
    let (_arm, target) = tar_image(multi_arch_test_tar(
        "arm64",
        &[2u8; 8192],
        &[0x7f; 1600],
        "app",
    ));
    let client = client();

    let url = spawn(None).await;
    let md = call_tool(
        &client,
        &url,
        10,
        "diff_images",
        json!({"base": base, "target": target, "lang": "en"}),
    )
    .await;
    assert_ne!(md["isError"], true, "{md}");
    assert!(tool_text(&md).contains("Binary diffs only"));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            controller::new_router(test_web_state(McpStatus::default())),
        )
        .await
        .unwrap();
    });

    let auto_diff: Value = client
        .get(format!(
            "http://{addr}/api/diff?base={base}&target={target}"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(auto_diff["baseLayersIgnored"], true);
    assert_eq!(auto_diff["parity"]["status"], "binaryDiffsOnly");
    assert_eq!(auto_diff["contentVerified"], true);

    let full_diff: Value = client
        .get(format!(
            "http://{addr}/api/diff?base={base}&target={target}&baseLayerMode=off"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(full_diff["baseLayersIgnored"], false);
    assert_eq!(full_diff["changed"]["count"], 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn file_preview_download_and_error_statuses_on_local_tar() {
    let (_tmp, image) = fixture_image();
    let client = client();

    let state = test_web_state(McpStatus::default());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, controller::new_router(state))
            .await
            .unwrap();
    });

    // Missing file:// tar returns 404.
    let missing = client
        .get(format!(
            "http://{addr}/api/analyze?image=file:///nonexistent/diving-missing-12345.tar"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let report: Value = client
        .get(format!("http://{addr}/api/analyze?image={image}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let layer0 = &report["layers"][0];
    let layer2 = &report["layers"][2];

    let preview_url = Url::parse_with_params(
        &format!("http://{addr}/api/file-preview"),
        &[
            ("digest", layer0["digest"].as_str().unwrap()),
            ("mediaType", layer0["mediaType"].as_str().unwrap()),
            ("file", "etc/config.yml"),
            ("image", image.as_str()),
        ],
    )
    .unwrap();
    let preview: Value = client
        .get(preview_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(preview["binary"], false);
    assert_eq!(preview["content"], "a: 1");

    let download_url = Url::parse_with_params(
        &format!("http://{addr}/api/file"),
        &[
            ("digest", layer0["digest"].as_str().unwrap()),
            ("mediaType", layer0["mediaType"].as_str().unwrap()),
            ("file", "etc/config.yml"),
            ("image", image.as_str()),
        ],
    )
    .unwrap();
    let download = client.get(download_url).send().await.unwrap();
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(download.text().await.unwrap(), "a: 1");

    // 1.5 MB data/model.bin triggers 413 when max_download_file_size is 64 KiB.
    let small_cap_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let small_addr = small_cap_listener.local_addr().unwrap();
    let http_client = build_http_client().unwrap();
    let analyzer = Analyzer::builder()
        .cache(CacheConfig::ephemeral())
        .http_client(http_client.clone())
        .allow_local_file(true)
        .build()
        .unwrap();
    let small_state = Arc::new(WebState::new(
        analyzer,
        http_client,
        env::temp_dir(),
        GateConfig::default(),
        64 * 1024,
        McpStatus::default(),
        vec![],
    ));
    tokio::spawn(async move {
        axum::serve(small_cap_listener, controller::new_router(small_state))
            .await
            .unwrap();
    });
    let too_large_url = Url::parse_with_params(
        &format!("http://{small_addr}/api/file"),
        &[
            ("digest", layer2["digest"].as_str().unwrap()),
            ("mediaType", layer2["mediaType"].as_str().unwrap()),
            ("file", "data/model.bin"),
            ("image", image.as_str()),
        ],
    )
    .unwrap();
    let too_large = client.get(too_large_url).send().await.unwrap();
    assert_eq!(too_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
}
