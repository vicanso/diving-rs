//! Inbound registry push webhook endpoint (`POST /api/webhook`).
//!
//! Accepts image push notifications from Harbor, Docker Hub, Aliyun ACR /
//! Tencent TCR, Docker Distribution Registry v2, or generic JSON triggers,
//! invalidates tag-keyed in-memory caches, analyzes the updated image in the
//! background, and pushes the result to all configured notification channels.

use axum::body::to_bytes;
use axum::extract::{ConnectInfo, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use diving_app::ai::{analyze_with_ai_opts, entrypoint_scripts_md, AiConfig};
use diving_app::notify::{build_report_url, send_all, NotifyConfig, NotifyContext};
use diving_app::summary::wecom_summary;
use diving_core::trace::{generate_trace_id, TRACE_ID};
use diving_core::{
    evaluate_gate, gate_verdict_md, parse_image_info, resolve_explicit_credentials, to_markdown,
    tr, AnalyzeOptions, Lang,
};
use ring::hmac;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::error::HTTPError;
use crate::http_util::{digests_equal, token_fingerprint};
use crate::state::{with_arch, WebState};

const MAX_WEBHOOK_BODY_BYTES: usize = 1024 * 1024;
const MAX_IMAGES_PER_WEBHOOK: usize = 16;

/// Configuration for the inbound `POST /api/webhook` endpoint.
#[derive(Debug, Clone)]
pub struct WebhookConfig {
    pub token: Option<String>,
    pub allow_public: bool,
    pub ai_cfg: Option<AiConfig>,
    pub no_ai_history: bool,
    pub notify_cfg: NotifyConfig,
    pub default_lang: Lang,
}

struct WebhookState {
    web: Arc<WebState>,
    token_digest: Option<Vec<u8>>,
    raw_token: Option<String>,
    allow_public: bool,
    ai_cfg: Option<AiConfig>,
    no_ai_history: bool,
    notify_cfg: NotifyConfig,
    default_lang: Lang,
}

#[derive(Debug, Default, Deserialize)]
pub struct WebhookQuery {
    pub token: Option<String>,
    pub registry: Option<String>,
    pub arch: Option<String>,
    pub lang: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum WebhookAction {
    Analyze {
        images: Vec<String>,
        lang: Option<String>,
    },
    Ignored {
        reason: String,
    },
}

/// Build the router serving `POST /api/webhook`.
pub fn new_router(web: Arc<WebState>, cfg: WebhookConfig) -> Router {
    let raw_token = cfg
        .token
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    let token_digest = raw_token.as_deref().map(token_fingerprint);
    let state = Arc::new(WebhookState {
        web,
        token_digest,
        raw_token,
        allow_public: cfg.allow_public,
        ai_cfg: cfg.ai_cfg,
        no_ai_history: cfg.no_ai_history,
        notify_cfg: cfg.notify_cfg,
        default_lang: cfg.default_lang,
    });
    Router::new()
        .route("/api/webhook", post(handle_webhook))
        .with_state(state)
}

fn json_error(status: StatusCode, category: &str, message: &str) -> Response {
    (
        status,
        Json(json!({
            "message": message,
            "category": category,
        })),
    )
        .into_response()
}

/// Whether `ip` belongs to loopback, RFC 1918 private networks, RFC 3927
/// link-local, or IPv6 ULA / link-local / IPv4-mapped equivalent ranges.
pub fn is_private_or_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_private_or_loopback(IpAddr::V4(mapped));
            }
            let seg0 = v6.segments()[0];
            v6.is_loopback() || (seg0 & 0xfe00) == 0xfc00 || (seg0 & 0xffc0) == 0xfe80
        }
    }
}

fn verify_token_or_hmac(
    expected_token: &str,
    expected_digest: &[u8],
    headers: &HeaderMap,
    query_token: Option<&str>,
    body: &[u8],
) -> bool {
    let matches_secret = |candidate: &str| {
        let candidate = candidate.trim();
        !candidate.is_empty() && digests_equal(&token_fingerprint(candidate), expected_digest)
    };

    if let Some(auth) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        let token = auth.strip_prefix("Bearer ").unwrap_or(auth);
        if matches_secret(token) {
            return true;
        }
    }
    for name in ["x-webhook-token", "x-gitlab-token"] {
        if let Some(val) = headers.get(name).and_then(|v| v.to_str().ok()) {
            if matches_secret(val) {
                return true;
            }
        }
    }
    if let Some(sig) = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
    {
        if let Some(hex_sig) = sig.trim().strip_prefix("sha256=") {
            if let Ok(got) = hex::decode(hex_sig.trim()) {
                let key = hmac::Key::new(hmac::HMAC_SHA256, expected_token.as_bytes());
                let expected = hmac::sign(&key, body);
                if digests_equal(&got, expected.as_ref()) {
                    return true;
                }
            }
        }
    }
    if let Some(qt) = query_token {
        if matches_secret(qt) {
            return true;
        }
    }
    false
}

fn verify_private_source(headers: &HeaderMap, peer: Option<SocketAddr>) -> bool {
    let Some(addr) = peer else {
        return false;
    };
    if !is_private_or_loopback(addr.ip()) {
        return false;
    }
    let forwarded_headers: Vec<_> = headers.get_all("x-forwarded-for").iter().collect();
    for hv in forwarded_headers {
        let Ok(raw) = hv.to_str() else {
            return false;
        };
        let parts: Vec<&str> = raw.split(',').collect();
        if parts.is_empty() {
            return false;
        }
        for part in parts {
            let trimmed = part.trim();
            let Ok(ip) = trimmed.parse::<IpAddr>() else {
                return false;
            };
            if !is_private_or_loopback(ip) {
                return false;
            }
        }
    }
    true
}

fn infer_report_base_url(headers: &HeaderMap) -> Option<String> {
    let host_raw = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())?;
    let host = host_raw.split(',').next()?.trim();
    if host.is_empty()
        || !host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'))
    {
        return None;
    }
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(str::trim)
        .filter(|p| p.eq_ignore_ascii_case("http") || p.eq_ignore_ascii_case("https"))
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "http".to_string());
    Some(format!("{proto}://{host}"))
}

async fn handle_webhook(
    State(state): State<Arc<WebhookState>>,
    Query(query): Query<WebhookQuery>,
    req: Request,
) -> Response {
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0);
    let headers = req.headers().clone();

    if state.token_digest.is_none() && !state.allow_public && !verify_private_source(&headers, peer)
    {
        return json_error(
            StatusCode::FORBIDDEN,
            "forbidden",
            "webhook endpoint only accepts private/loopback callers without a token",
        );
    }

    let body = match to_bytes(req.into_body(), MAX_WEBHOOK_BODY_BYTES).await {
        Ok(b) => b,
        Err(e) => {
            return json_error(
                StatusCode::BAD_REQUEST,
                "invalid",
                &format!("failed to read webhook body: {e}"),
            );
        }
    };

    if let (Some(raw_token), Some(expected_digest)) =
        (state.raw_token.as_deref(), state.token_digest.as_deref())
    {
        if !verify_token_or_hmac(
            raw_token,
            expected_digest,
            &headers,
            query.token.as_deref(),
            &body,
        ) {
            return json_error(StatusCode::UNAUTHORIZED, "unauthorized", "unauthorized");
        }
    }

    let action = match parse_webhook_payload(&body, &query) {
        Ok(a) => a,
        Err(msg) => return json_error(StatusCode::BAD_REQUEST, "invalid", &msg),
    };

    let (images, payload_lang) = match action {
        WebhookAction::Ignored { reason } => {
            return (
                StatusCode::OK,
                Json(json!({
                    "status": "ignored",
                    "reason": reason,
                })),
            )
                .into_response();
        }
        WebhookAction::Analyze { images, lang } => (images, lang),
    };

    for image in &images {
        if let Err(err) = state.web.analyzer.check_allowed(&parse_image_info(image)) {
            let http_err: HTTPError = err.into();
            return json_error(StatusCode::FORBIDDEN, &http_err.category, &http_err.message);
        }
    }

    let lang = match query
        .lang
        .as_deref()
        .or(payload_lang.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(explicit) => Lang::resolve(Some(explicit)),
        None => state.default_lang,
    };

    let report_base_url = state
        .notify_cfg
        .public_url
        .clone()
        .or_else(|| infer_report_base_url(&headers));

    let response_images = images.clone();
    let channels = state.notify_cfg.channel_names();
    let worker_state = state.clone();
    tokio::spawn(TRACE_ID.scope(generate_trace_id(), async move {
        for image in images {
            process_webhook_image(&worker_state, &image, lang, report_base_url.as_deref()).await;
        }
    }));

    (
        StatusCode::ACCEPTED,
        Json(json!({
            "status": "accepted",
            "images": response_images,
            "notifiers": channels,
        })),
    )
        .into_response()
}

async fn process_webhook_image(
    state: &WebhookState,
    image: &str,
    lang: Lang,
    report_base_url: Option<&str>,
) {
    let image_info = parse_image_info(image);
    state.web.analyzer.invalidate_info(&image_info);

    let opts = AnalyzeOptions::default()
        .lang(lang)
        .verify_dup(true)
        .fallback_to_last_known(false)
        .skip_memory_cache(true)
        .credentials(resolve_explicit_credentials(None, None, false));
    let result = match state.web.analyzer.analyze_info(image_info, opts).await {
        Ok(r) => r,
        Err(err) => {
            error!(
                image,
                err = err.to_string(),
                "webhook image analysis failed"
            );
            return;
        }
    };

    state.web.add_latest_image(image);

    if state.notify_cfg.is_empty() {
        info!(
            image,
            "webhook image analysis completed (no notification channels configured)"
        );
        return;
    }

    let summary = result.summary();
    let failures = evaluate_gate(&result, &summary, lang, &state.web.gate_config);
    let gate_passed = failures.is_empty();
    let verdict = gate_verdict_md(&failures, lang);
    let fail_label = tr(lang, "cli.fail");
    let failure_lines: Vec<String> = failures
        .iter()
        .map(|f| f.render(lang, fail_label))
        .collect();
    let summary_md = wecom_summary(&result, lang);
    let report_url = report_base_url.and_then(|base| build_report_url(base, image, &result.arch));

    let ai_report = if let Some(ai_cfg) = &state.ai_cfg {
        let mut md = to_markdown(&result, true, lang);
        let analyzer = state.web.analyzer.clone();
        let result_for_scripts = result.clone();
        let scripts = tokio::task::spawn_blocking(move || {
            entrypoint_scripts_md(&analyzer, &result_for_scripts, lang)
        })
        .await
        .unwrap_or_default();
        if !scripts.is_empty() {
            md.push_str("\n\n");
            md.push_str(&scripts);
        }
        match analyze_with_ai_opts(
            &state.web.http_client,
            &state.web.config_dir,
            image,
            &md,
            ai_cfg,
            lang,
            state.no_ai_history,
            true,
        )
        .await
        {
            Ok(report) => Some(report),
            Err(err) => {
                warn!(
                    image,
                    err, "webhook AI report failed, falling back to summary"
                );
                None
            }
        }
    } else {
        None
    };

    let ctx = NotifyContext {
        image,
        result: &result,
        summary: &summary,
        gate_passed,
        gate_verdict_md: Some(&verdict),
        gate_failures: &failure_lines,
        ai_report: ai_report.as_deref(),
        summary_md: &summary_md,
        report_url: report_url.as_deref(),
        lang,
    };

    match send_all(&state.web.http_client, &state.notify_cfg, &ctx).await {
        Ok(()) => info!(
            image,
            channels = ?state.notify_cfg.channel_names(),
            "webhook analysis result pushed"
        ),
        Err(errors) => error!(
            image,
            errors = ?errors,
            "webhook analysis notification failed"
        ),
    }
}

fn non_empty_str(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn clean_registry_host(raw: &str) -> String {
    raw.trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string()
}

fn looks_like_host(seg: &str) -> bool {
    seg == "localhost" || seg.starts_with("localhost:") || seg.contains('.') || seg.contains(':')
}

fn strip_scheme(s: &str) -> &str {
    let s = s.trim();
    s.strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s)
}

fn apply_registry_host(image_ref: &str, host: Option<&str>) -> String {
    let stripped = strip_scheme(image_ref);
    let Some(host) = host.map(clean_registry_host).filter(|h| !h.is_empty()) else {
        return stripped.to_string();
    };
    if stripped.starts_with("file://")
        || stripped.starts_with("docker://")
        || stripped.starts_with("podman://")
    {
        return stripped.to_string();
    }
    if let Some((first, rest)) = stripped.split_once('/') {
        if looks_like_host(first) {
            return format!("{host}/{rest}");
        }
    }
    format!("{host}/{stripped}")
}

fn expand_and_dedup(
    raw_images: Vec<String>,
    arches: &[Option<String>],
    lang: Option<String>,
) -> Result<WebhookAction, String> {
    let mut seen = HashSet::new();
    let mut out = vec![];
    for raw in raw_images {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        for arch in arches {
            let candidate = with_arch(trimmed, arch.as_deref());
            if seen.insert(candidate.clone()) {
                out.push(candidate);
                if out.len() >= MAX_IMAGES_PER_WEBHOOK {
                    return Ok(WebhookAction::Analyze { images: out, lang });
                }
            }
        }
    }
    if out.is_empty() {
        return Err("no valid image references found in webhook payload".to_string());
    }
    Ok(WebhookAction::Analyze { images: out, lang })
}

/// Parse Harbor, Docker Hub, Aliyun ACR / Tencent TCR, Docker Distribution v2,
/// or Generic JSON webhook payloads into deduplicated image references.
pub fn parse_webhook_payload(body: &[u8], query: &WebhookQuery) -> Result<WebhookAction, String> {
    let root: Value =
        serde_json::from_slice(body).map_err(|e| format!("invalid JSON webhook payload: {e}"))?;

    let payload_lang = non_empty_str(root.get("lang")).map(str::to_string);
    let mut arches: Vec<Option<String>> = vec![];
    if let Some(q_arch) = query
        .arch
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        arches.push(Some(q_arch.to_string()));
    } else if let Some(arr) = root.get("archs").and_then(Value::as_array) {
        for item in arr {
            if let Some(a) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                arches.push(Some(a.to_string()));
            }
        }
    } else if let Some(a) = non_empty_str(root.get("arch")) {
        arches.push(Some(a.to_string()));
    }
    if arches.is_empty() {
        arches.push(None);
    }

    let query_registry = query
        .registry
        .as_deref()
        .map(clean_registry_host)
        .filter(|s| !s.is_empty());

    // 1. Harbor webhook (`event_data`)
    if let Some(event_data) = root.get("event_data").and_then(Value::as_object) {
        if let Some(event_type) = non_empty_str(root.get("type")) {
            if !event_type.eq_ignore_ascii_case("PUSH_ARTIFACT")
                && !event_type.eq_ignore_ascii_case("pushImage")
            {
                return Ok(WebhookAction::Ignored {
                    reason: format!("ignored Harbor event type: {event_type}"),
                });
            }
        }
        let repo_full = event_data.get("repository").and_then(|r| {
            non_empty_str(r.get("repo_full_name"))
                .map(str::to_string)
                .or_else(|| {
                    let ns = non_empty_str(r.get("namespace"));
                    let name = non_empty_str(r.get("name"))?;
                    Some(match ns {
                        Some(ns) => format!("{ns}/{name}"),
                        None => name.to_string(),
                    })
                })
        });
        let mut raw_images = vec![];
        if let Some(resources) = event_data.get("resources").and_then(Value::as_array) {
            for res in resources {
                if let Some(url) = non_empty_str(res.get("resource_url")) {
                    raw_images.push(apply_registry_host(url, query_registry.as_deref()));
                } else if let Some(repo) = repo_full.as_deref() {
                    let suffix = non_empty_str(res.get("tag"))
                        .map(|t| format!(":{t}"))
                        .or_else(|| non_empty_str(res.get("digest")).map(|d| format!("@{d}")))
                        .unwrap_or_default();
                    let combined = format!("{repo}{suffix}");
                    raw_images.push(apply_registry_host(&combined, query_registry.as_deref()));
                }
            }
        }
        if raw_images.is_empty() {
            if let Some(repo) = repo_full {
                raw_images.push(apply_registry_host(&repo, query_registry.as_deref()));
            }
        }
        return expand_and_dedup(raw_images, &arches, payload_lang);
    }

    // 2. Docker Distribution Registry v2 notifications (`events` array)
    if let Some(events) = root.get("events").and_then(Value::as_array) {
        let mut raw_images = vec![];
        for ev in events {
            let Some(action) = non_empty_str(ev.get("action")) else {
                continue;
            };
            if !action.eq_ignore_ascii_case("push") {
                continue;
            }
            let Some(target) = ev.get("target") else {
                continue;
            };
            let Some(repo) = non_empty_str(target.get("repository")) else {
                continue;
            };
            let tag = non_empty_str(target.get("tag"));
            let digest = non_empty_str(target.get("digest"));
            let media_type = non_empty_str(target.get("mediaType"))
                .unwrap_or_default()
                .to_ascii_lowercase();
            let is_manifest = (media_type.is_empty() && tag.is_some())
                || media_type.contains("manifest")
                || media_type.contains("index");
            if !is_manifest {
                continue;
            }
            let suffix = tag
                .map(|t| format!(":{t}"))
                .or_else(|| digest.map(|d| format!("@{d}")))
                .unwrap_or_default();
            let req_host = ev
                .get("request")
                .and_then(|r| non_empty_str(r.get("host")))
                .map(clean_registry_host);
            let url_host = non_empty_str(target.get("url"))
                .map(strip_scheme)
                .and_then(|u| u.split('/').next())
                .filter(|h| !h.is_empty())
                .map(str::to_string);
            let host = query_registry.clone().or(req_host).or(url_host);
            let raw = match host {
                Some(h) if !h.is_empty() => format!("{h}/{repo}{suffix}"),
                _ => format!("{repo}{suffix}"),
            };
            raw_images.push(raw);
        }
        if raw_images.is_empty() {
            return Ok(WebhookAction::Ignored {
                reason: "no manifest push events".to_string(),
            });
        }
        return expand_and_dedup(raw_images, &arches, payload_lang);
    }

    // 3. Docker Hub / Aliyun ACR / Tencent TCR (`push_data` + `repository`)
    if let Some(repo_obj) = root.get("repository").and_then(Value::as_object) {
        if let Some(push_data) = root.get("push_data") {
            let repo = non_empty_str(repo_obj.get("repo_name"))
                .or_else(|| non_empty_str(repo_obj.get("repo_full_name")))
                .map(str::to_string)
                .or_else(|| {
                    let ns = non_empty_str(repo_obj.get("namespace"));
                    let name = non_empty_str(repo_obj.get("name"))?;
                    Some(match ns {
                        Some(ns) => format!("{ns}/{name}"),
                        None => name.to_string(),
                    })
                });
            let Some(repo) = repo else {
                return Err("missing repository name in push webhook payload".to_string());
            };
            let suffix = non_empty_str(push_data.get("tag"))
                .map(|t| format!(":{t}"))
                .or_else(|| non_empty_str(push_data.get("digest")).map(|d| format!("@{d}")))
                .unwrap_or_else(|| ":latest".to_string());
            let acr_host = non_empty_str(repo_obj.get("region"))
                .map(|region| format!("registry.{region}.aliyuncs.com"));
            let host = query_registry.clone().or(acr_host);
            let full = format!("{repo}{suffix}");
            let resolved = match host {
                Some(h) => apply_registry_host(&full, Some(&h)),
                None => full,
            };
            return expand_and_dedup(vec![resolved], &arches, payload_lang);
        }
    }

    // 4. Generic JSON (`image` / `images`)
    let mut generic_images = vec![];
    if let Some(single) = non_empty_str(root.get("image")) {
        generic_images.push(apply_registry_host(single, query_registry.as_deref()));
    }
    if let Some(arr) = root.get("images").and_then(Value::as_array) {
        for item in arr {
            if let Some(s) = item.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                generic_images.push(apply_registry_host(s, query_registry.as_deref()));
            }
        }
    }
    if !generic_images.is_empty() {
        return expand_and_dedup(generic_images, &arches, payload_lang);
    }

    Err(
        "unrecognized webhook payload: expected image, event_data, push_data, or events"
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_generic_single_multiple_and_archs() {
        let body = br#"{"image":"org/app:1.2","archs":["amd64","arm64"],"lang":"zh"}"#;
        let res = parse_webhook_payload(body, &WebhookQuery::default()).unwrap();
        assert_eq!(
            res,
            WebhookAction::Analyze {
                images: vec![
                    "org/app:1.2?arch=amd64".to_string(),
                    "org/app:1.2?arch=arm64".to_string(),
                ],
                lang: Some("zh".to_string()),
            }
        );

        let body_multi = br#"{"images":["redis:alpine","redis:alpine","nginx:alpine"]}"#;
        let query = WebhookQuery {
            arch: Some("arm64".to_string()),
            ..Default::default()
        };
        let res = parse_webhook_payload(body_multi, &query).unwrap();
        assert_eq!(
            res,
            WebhookAction::Analyze {
                images: vec![
                    "redis:alpine?arch=arm64".to_string(),
                    "nginx:alpine?arch=arm64".to_string(),
                ],
                lang: None,
            }
        );
    }

    #[test]
    fn parses_harbor_push_and_ignores_other_events() {
        let push_body = br#"{
            "type": "PUSH_ARTIFACT",
            "event_data": {
                "resources": [{
                    "digest": "sha256:abc",
                    "tag": "v2",
                    "resource_url": "hub.harbor.com/team/app:v2"
                }],
                "repository": {
                    "repo_full_name": "team/app"
                }
            }
        }"#;
        assert_eq!(
            parse_webhook_payload(push_body, &WebhookQuery::default()).unwrap(),
            WebhookAction::Analyze {
                images: vec!["hub.harbor.com/team/app:v2".to_string()],
                lang: None,
            }
        );

        let del_body = br#"{
            "type": "DELETE_ARTIFACT",
            "event_data": { "resources": [], "repository": { "repo_full_name": "team/app" } }
        }"#;
        assert!(matches!(
            parse_webhook_payload(del_body, &WebhookQuery::default()).unwrap(),
            WebhookAction::Ignored { .. }
        ));
    }

    #[test]
    fn parses_docker_hub_and_aliyun_acr_with_registry_override() {
        let docker_hub = br#"{
            "push_data": { "tag": "1.0.0" },
            "repository": { "repo_name": "vicanso/diving" }
        }"#;
        assert_eq!(
            parse_webhook_payload(docker_hub, &WebhookQuery::default()).unwrap(),
            WebhookAction::Analyze {
                images: vec!["vicanso/diving:1.0.0".to_string()],
                lang: None,
            }
        );

        let acr = br#"{
            "push_data": { "tag": "latest", "digest": "sha256:1234" },
            "repository": { "name": "api", "namespace": "prod", "region": "cn-hangzhou", "repo_full_name": "prod/api" }
        }"#;
        assert_eq!(
            parse_webhook_payload(acr, &WebhookQuery::default()).unwrap(),
            WebhookAction::Analyze {
                images: vec!["registry.cn-hangzhou.aliyuncs.com/prod/api:latest".to_string()],
                lang: None,
            }
        );

        let override_q = WebhookQuery {
            registry: Some("ccr.ccs.tencentyun.com".to_string()),
            ..Default::default()
        };
        assert_eq!(
            parse_webhook_payload(acr, &override_q).unwrap(),
            WebhookAction::Analyze {
                images: vec!["ccr.ccs.tencentyun.com/prod/api:latest".to_string()],
                lang: None,
            }
        );
    }

    #[test]
    fn parses_registry_v2_manifest_pushes_and_ignores_blobs_and_pulls() {
        let body = br#"{
            "events": [
                {
                    "action": "pull",
                    "target": { "mediaType": "application/vnd.docker.distribution.manifest.v2+json", "repository": "team/web", "tag": "v1" },
                    "request": { "host": "reg.internal" }
                },
                {
                    "action": "push",
                    "target": { "mediaType": "application/vnd.docker.image.rootfs.diff.tar.gzip", "repository": "team/web", "digest": "sha256:blob1" },
                    "request": { "host": "reg.internal" }
                },
                {
                    "action": "push",
                    "target": { "mediaType": "application/vnd.oci.image.manifest.v1+json", "repository": "team/web", "tag": "v2", "digest": "sha256:man2" },
                    "request": { "host": "reg.internal:5000" }
                }
            ]
        }"#;
        assert_eq!(
            parse_webhook_payload(body, &WebhookQuery::default()).unwrap(),
            WebhookAction::Analyze {
                images: vec!["reg.internal:5000/team/web:v2".to_string()],
                lang: None,
            }
        );

        let only_blob = br#"{
            "events": [{
                "action": "push",
                "target": { "mediaType": "application/octet-stream", "repository": "team/web", "digest": "sha256:blob1" }
            }]
        }"#;
        assert!(matches!(
            parse_webhook_payload(only_blob, &WebhookQuery::default()).unwrap(),
            WebhookAction::Ignored { .. }
        ));
    }

    #[test]
    fn private_network_detection_handles_v4_v6_and_mapped_addrs() {
        assert!(is_private_or_loopback("127.0.0.1".parse().unwrap()));
        assert!(is_private_or_loopback("10.20.30.40".parse().unwrap()));
        assert!(is_private_or_loopback("172.16.0.1".parse().unwrap()));
        assert!(is_private_or_loopback("192.168.1.5".parse().unwrap()));
        assert!(is_private_or_loopback("169.254.1.1".parse().unwrap()));
        assert!(!is_private_or_loopback("8.8.8.8".parse().unwrap()));

        assert!(is_private_or_loopback("::1".parse().unwrap()));
        assert!(is_private_or_loopback("fc00::1".parse().unwrap()));
        assert!(is_private_or_loopback("fe80::1".parse().unwrap()));
        assert!(is_private_or_loopback("::ffff:127.0.0.1".parse().unwrap()));
        assert!(!is_private_or_loopback("::ffff:8.8.8.8".parse().unwrap()));
        assert!(!is_private_or_loopback(
            "2001:4860:4860::8888".parse().unwrap()
        ));
    }

    #[test]
    fn token_and_hmac_verification_accepts_all_supported_auth_styles() {
        let secret = "whsec_demo123";
        let fp = token_fingerprint(secret);
        let body = br#"{"image":"redis:alpine"}"#;

        let mut h_bearer = HeaderMap::new();
        h_bearer.insert(
            header::AUTHORIZATION,
            "Bearer whsec_demo123".parse().unwrap(),
        );
        assert!(verify_token_or_hmac(secret, &fp, &h_bearer, None, body));

        let mut h_raw = HeaderMap::new();
        h_raw.insert(header::AUTHORIZATION, "whsec_demo123".parse().unwrap());
        assert!(verify_token_or_hmac(secret, &fp, &h_raw, None, body));

        let mut h_webhook = HeaderMap::new();
        h_webhook.insert("x-webhook-token", "whsec_demo123".parse().unwrap());
        assert!(verify_token_or_hmac(secret, &fp, &h_webhook, None, body));

        let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
        let sig = format!("sha256={}", hex::encode(hmac::sign(&key, body).as_ref()));
        let mut h_hmac = HeaderMap::new();
        h_hmac.insert("x-hub-signature-256", sig.parse().unwrap());
        assert!(verify_token_or_hmac(secret, &fp, &h_hmac, None, body));

        assert!(verify_token_or_hmac(
            secret,
            &fp,
            &HeaderMap::new(),
            Some("whsec_demo123"),
            body
        ));
        assert!(!verify_token_or_hmac(
            secret,
            &fp,
            &HeaderMap::new(),
            Some("wrong"),
            body
        ));
    }
}
