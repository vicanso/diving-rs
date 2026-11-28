//! End-to-end integration tests for per-domain private registry auth,
//! custom request headers, plain HTTP / SSRF exemptions, and strict
//! cross-origin redirect header stripping.

mod common;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderName, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use common::tar_bytes;
use diving_core::{sha256_hex, AnalyzeOptions, Analyzer, Error, RegistryRule};
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::json;
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;

const MANIFEST_TYPE: &str = "application/vnd.docker.distribution.manifest.v2+json";

struct ImageFixture {
    manifest: Vec<u8>,
    manifest_digest: String,
    blobs: HashMap<String, Vec<u8>>,
}

fn digest_of(data: &[u8]) -> String {
    format!("sha256:{}", sha256_hex(data))
}

fn build_fixture() -> ImageFixture {
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&tar_bytes(&[
        ("usr/bin/app", &[0x7f; 2048]),
        ("etc/app.conf", b"mode=prod\n"),
    ]))
    .unwrap();
    let layer = gz.finish().unwrap();
    let config = br#"{
        "architecture": "amd64",
        "created": "2025-01-01T00:00:00Z",
        "history": [{"created": "2025-01-01T00:00:00Z", "created_by": "/bin/sh -c #(nop) COPY . /"}],
        "os": "linux",
        "rootfs": {"type": "layers", "diff_ids": ["sha256:aaa"]},
        "config": {"User": "", "Env": ["PATH=/usr/bin"], "Entrypoint": ["/usr/bin/app"], "Cmd": []}
    }"#
    .to_vec();
    let manifest = serde_json::to_vec(&json!({
        "schemaVersion": 2,
        "mediaType": MANIFEST_TYPE,
        "config": {
            "mediaType": "application/vnd.docker.container.image.v1+json",
            "size": config.len(),
            "digest": digest_of(&config),
        },
        "layers": [{
            "mediaType": "application/vnd.docker.image.rootfs.diff.tar.gzip",
            "size": layer.len(),
            "digest": digest_of(&layer),
        }],
    }))
    .unwrap();
    ImageFixture {
        manifest_digest: digest_of(&manifest),
        manifest,
        blobs: HashMap::from([(digest_of(&config), config), (digest_of(&layer), layer)]),
    }
}

fn basic_auth_header(user: &str, pass: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let raw = format!("{user}:{pass}");
    let bytes = raw.as_bytes();
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        encoded.push(TABLE[((triple >> 18) & 0x3f) as usize] as char);
        encoded.push(TABLE[((triple >> 12) & 0x3f) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            TABLE[((triple >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            TABLE[(triple & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    format!("Basic {encoded}")
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn manifest_ok(fixture: &ImageFixture) -> Response {
    (
        [
            (header::CONTENT_TYPE, MANIFEST_TYPE.to_string()),
            (
                HeaderName::from_static("docker-content-digest"),
                fixture.manifest_digest.clone(),
            ),
        ],
        fixture.manifest.clone(),
    )
        .into_response()
}

fn blob_ok(fixture: &ImageFixture, digest: &str) -> Response {
    match fixture.blobs.get(digest) {
        Some(data) => data.clone().into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn spawn_router(router: Router) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    addr
}

fn rule_for(addr: SocketAddr) -> RegistryRule {
    RegistryRule {
        host: addr.to_string(),
        username: None,
        password: None,
        token: None,
        headers: BTreeMap::new(),
        insecure_http: true,
        insecure_skip_tls_verify: false,
        ca_cert: None,
    }
}

#[derive(Clone)]
struct BasicState {
    fixture: Arc<ImageFixture>,
    expected_auth: String,
    challenges: Arc<AtomicUsize>,
}

async fn basic_manifest(State(state): State<BasicState>, headers: HeaderMap) -> Response {
    if header_str(&headers, "x-gateway-token") != Some("gw-secret") {
        return StatusCode::FORBIDDEN.into_response();
    }
    if header_str(&headers, "authorization") != Some(state.expected_auth.as_str()) {
        state.challenges.fetch_add(1, Ordering::SeqCst);
        return (
            StatusCode::UNAUTHORIZED,
            [(
                header::WWW_AUTHENTICATE,
                "Basic realm=\"Mock Private Registry\"",
            )],
        )
            .into_response();
    }
    manifest_ok(&state.fixture)
}

async fn basic_blob(
    State(state): State<BasicState>,
    Path(digest): Path<String>,
    headers: HeaderMap,
) -> Response {
    if header_str(&headers, "x-gateway-token") != Some("gw-secret") {
        return StatusCode::FORBIDDEN.into_response();
    }
    if header_str(&headers, "authorization") != Some(state.expected_auth.as_str()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    blob_ok(&state.fixture, &digest)
}

#[tokio::test(flavor = "multi_thread")]
async fn basic_challenge_with_custom_header_and_insecure_http() {
    let challenges = Arc::new(AtomicUsize::new(0));
    let state = BasicState {
        fixture: Arc::new(build_fixture()),
        expected_auth: basic_auth_header("alice", "s3cr3t"),
        challenges: challenges.clone(),
    };
    let router = Router::new()
        .route("/v2/team/app/manifests/{reference}", get(basic_manifest))
        .route("/v2/team/app/blobs/{digest}", get(basic_blob))
        .with_state(state);
    let addr = spawn_router(router).await;

    let mut rule = rule_for(addr);
    rule.username = Some("alice".to_owned());
    rule.password = Some("s3cr3t".to_owned());
    rule.headers
        .insert("X-Gateway-Token".to_owned(), "gw-secret".to_owned());

    let analyzer = Analyzer::builder().registries([rule]).build().unwrap();
    let result = analyzer
        .analyze(
            &format!("{addr}/team/app:latest"),
            AnalyzeOptions::default().verify_dup(false),
        )
        .await
        .expect("Basic auth + custom header analysis should succeed");

    assert_eq!(result.layers.len(), 1);
    assert!(
        challenges.load(Ordering::SeqCst) >= 1,
        "expected initial 401 Basic challenge before credential retry"
    );
}

#[derive(Clone)]
struct BearerState {
    fixture: Arc<ImageFixture>,
    realm_url: String,
    expected_basic: String,
    token_hits: Arc<AtomicUsize>,
}

async fn bearer_manifest(State(state): State<BearerState>, headers: HeaderMap) -> Response {
    if header_str(&headers, "x-iap-key") != Some("iap-123") {
        return StatusCode::FORBIDDEN.into_response();
    }
    if header_str(&headers, "authorization") != Some("Bearer realm-tok-1") {
        let challenge = format!(
            "Bearer realm=\"{}\",service=\"mock\",scope=\"repository:team/app:pull\"",
            state.realm_url
        );
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, challenge)],
        )
            .into_response();
    }
    manifest_ok(&state.fixture)
}

async fn bearer_blob(
    State(state): State<BearerState>,
    Path(digest): Path<String>,
    headers: HeaderMap,
) -> Response {
    if header_str(&headers, "x-iap-key") != Some("iap-123") {
        return StatusCode::FORBIDDEN.into_response();
    }
    if header_str(&headers, "authorization") != Some("Bearer realm-tok-1") {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    blob_ok(&state.fixture, &digest)
}

async fn bearer_token(
    State(state): State<BearerState>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if header_str(&headers, "x-iap-key") != Some("iap-123") {
        return StatusCode::FORBIDDEN.into_response();
    }
    if header_str(&headers, "authorization") != Some(state.expected_basic.as_str()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if params.get("static").map(String::as_str) != Some("1")
        || params.get("service").map(String::as_str) != Some("mock")
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    state.token_hits.fetch_add(1, Ordering::SeqCst);
    Json(json!({ "token": "realm-tok-1" })).into_response()
}

#[tokio::test(flavor = "multi_thread")]
async fn bearer_realm_negotiation_with_harbor_dollar_user_and_custom_header() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token_hits = Arc::new(AtomicUsize::new(0));
    let state = BearerState {
        fixture: Arc::new(build_fixture()),
        realm_url: format!("http://{addr}/service/token?static=1"),
        expected_basic: basic_auth_header("robot$ci", "harbor-pass"),
        token_hits: token_hits.clone(),
    };
    let router = Router::new()
        .route("/v2/team/app/manifests/{reference}", get(bearer_manifest))
        .route("/v2/team/app/blobs/{digest}", get(bearer_blob))
        .route("/service/token", get(bearer_token))
        .with_state(state);
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let mut rule = rule_for(addr);
    rule.username = Some("robot$ci".to_owned());
    rule.password = Some("harbor-pass".to_owned());
    rule.headers
        .insert("X-IAP-Key".to_owned(), "iap-123".to_owned());

    let analyzer = Analyzer::builder().registries([rule]).build().unwrap();
    let result = analyzer
        .analyze(
            &format!("{addr}/team/app:latest"),
            AnalyzeOptions::default().verify_dup(false),
        )
        .await
        .expect("Bearer realm exchange with Harbor username should succeed");

    assert_eq!(result.layers.len(), 1);
    assert_eq!(token_hits.load(Ordering::SeqCst), 1);
}

#[derive(Clone)]
struct RedirectState {
    fixture: Arc<ImageFixture>,
    s3_addr: SocketAddr,
    s3_hits: Arc<AtomicUsize>,
}

async fn redirect_manifest(State(state): State<RedirectState>, headers: HeaderMap) -> Response {
    if header_str(&headers, "x-registry-secret") != Some("top-secret")
        || header_str(&headers, "authorization") != Some("Bearer static-jwt-token")
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    manifest_ok(&state.fixture)
}

async fn redirect_blob(
    State(state): State<RedirectState>,
    Path(digest): Path<String>,
    headers: HeaderMap,
) -> Response {
    if header_str(&headers, "x-registry-secret") != Some("top-secret")
        || header_str(&headers, "authorization") != Some("Bearer static-jwt-token")
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let target = format!("http://{}/signed/{digest}?sig=presigned", state.s3_addr);
    (StatusCode::TEMPORARY_REDIRECT, [(header::LOCATION, target)]).into_response()
}

async fn s3_blob(
    State(state): State<RedirectState>,
    Path(digest): Path<String>,
    Query(params): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if headers.contains_key("authorization") || headers.contains_key("x-registry-secret") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    if params.get("sig").map(String::as_str) != Some("presigned") {
        return StatusCode::FORBIDDEN.into_response();
    }
    state.s3_hits.fetch_add(1, Ordering::SeqCst);
    blob_ok(&state.fixture, &digest)
}

#[tokio::test(flavor = "multi_thread")]
async fn static_token_and_cross_origin_redirect_strips_auth_and_custom_headers() {
    let fixture = Arc::new(build_fixture());
    let s3_hits = Arc::new(AtomicUsize::new(0));
    let s3_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let s3_addr = s3_listener.local_addr().unwrap();

    let state = RedirectState {
        fixture,
        s3_addr,
        s3_hits: s3_hits.clone(),
    };
    let s3_router = Router::new()
        .route("/signed/{digest}", get(s3_blob))
        .with_state(state.clone());
    tokio::spawn(async move {
        axum::serve(s3_listener, s3_router).await.unwrap();
    });

    let reg_router = Router::new()
        .route("/v2/team/app/manifests/{reference}", get(redirect_manifest))
        .route("/v2/team/app/blobs/{digest}", get(redirect_blob))
        .with_state(state);
    let reg_addr = spawn_router(reg_router).await;

    let mut rule = rule_for(reg_addr);
    rule.token = Some("static-jwt-token".to_owned());
    rule.headers
        .insert("X-Registry-Secret".to_owned(), "top-secret".to_owned());

    let analyzer = Analyzer::builder().registries([rule]).build().unwrap();
    let result = analyzer
        .analyze(
            &format!("{reg_addr}/team/app:latest"),
            AnalyzeOptions::default().verify_dup(false),
        )
        .await
        .expect("cross-origin 307 redirect should succeed without leaking headers");

    assert_eq!(result.layers.len(), 1);
    assert_eq!(s3_hits.load(Ordering::SeqCst), 2);
}

#[derive(Clone)]
struct PlainState {
    fixture: Arc<ImageFixture>,
    redirect_to: Option<SocketAddr>,
}

async fn plain_manifest(State(state): State<PlainState>, headers: HeaderMap) -> Response {
    if header_str(&headers, "x-env") != Some("priv") {
        return StatusCode::FORBIDDEN.into_response();
    }
    manifest_ok(&state.fixture)
}

async fn plain_blob(
    State(state): State<PlainState>,
    Path(digest): Path<String>,
    headers: HeaderMap,
) -> Response {
    if header_str(&headers, "x-env") != Some("priv") {
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Some(target_addr) = state.redirect_to {
        let loc = format!("http://{target_addr}/signed/{digest}");
        return (StatusCode::TEMPORARY_REDIRECT, [(header::LOCATION, loc)]).into_response();
    }
    blob_ok(&state.fixture, &digest)
}

#[tokio::test(flavor = "multi_thread")]
async fn ssrf_blocks_unconfigured_private_host_and_redirect_while_allowing_configured_rule() {
    let fixture = Arc::new(build_fixture());
    let allowed_addr = spawn_router(
        Router::new()
            .route("/v2/team/app/manifests/{reference}", get(plain_manifest))
            .route("/v2/team/app/blobs/{digest}", get(plain_blob))
            .with_state(PlainState {
                fixture: fixture.clone(),
                redirect_to: None,
            }),
    )
    .await;

    let blocked_addr = spawn_router(
        Router::new()
            .route("/v2/team/app/manifests/{reference}", get(plain_manifest))
            .route("/v2/team/app/blobs/{digest}", get(plain_blob))
            .with_state(PlainState {
                fixture: fixture.clone(),
                redirect_to: None,
            }),
    )
    .await;

    let redirect_src_addr = spawn_router(
        Router::new()
            .route("/v2/team/app/manifests/{reference}", get(plain_manifest))
            .route("/v2/team/app/blobs/{digest}", get(plain_blob))
            .with_state(PlainState {
                fixture,
                redirect_to: Some(blocked_addr),
            }),
    )
    .await;

    let mut allowed_rule = rule_for(allowed_addr);
    allowed_rule
        .headers
        .insert("X-Env".to_owned(), "priv".to_owned());

    let mut redirect_src_rule = rule_for(redirect_src_addr);
    redirect_src_rule
        .headers
        .insert("X-Env".to_owned(), "priv".to_owned());

    let analyzer = Analyzer::builder()
        .block_private_registries(true)
        .registries([allowed_rule, redirect_src_rule.clone()])
        .build()
        .unwrap();

    let ok = analyzer
        .analyze(
            &format!("{allowed_addr}/team/app:latest"),
            AnalyzeOptions::default().verify_dup(false),
        )
        .await
        .expect("configured private registry should bypass SSRF check");
    assert_eq!(ok.layers.len(), 1);

    let err = analyzer
        .analyze(
            &format!("{blocked_addr}/team/app:latest"),
            AnalyzeOptions::default().verify_dup(false),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::DisallowedRegistry { .. }));
    assert!(err.is_forbidden());

    let err_scheme = analyzer
        .analyze(
            &format!("http://{blocked_addr}/team/app:latest"),
            AnalyzeOptions::default().verify_dup(false),
        )
        .await
        .unwrap_err();
    assert!(matches!(err_scheme, Error::DisallowedRegistry { .. }));

    let redirect_analyzer = Analyzer::builder()
        .block_private_registries(true)
        .registries([redirect_src_rule])
        .build()
        .unwrap();

    let err_redirect = redirect_analyzer
        .analyze(
            &format!("{redirect_src_addr}/team/app:latest"),
            AnalyzeOptions::default().verify_dup(false),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err_redirect, Error::DisallowedRegistry { .. }),
        "cross-origin redirect to unconfigured private host must be rejected, got {err_redirect:?}"
    );

    let allowlist_analyzer = Analyzer::builder()
        .registry_allowlist(["*.internal.example"])
        .build()
        .unwrap();
    let err_allowlist = allowlist_analyzer
        .analyze("evil.example/team/app:latest", AnalyzeOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err_allowlist, Error::DisallowedRegistry { .. }));
}
