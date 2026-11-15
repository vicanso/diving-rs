use crate::task_local::{generate_trace_id, TRACE_ID};
use crate::util::set_no_cache_if_not_exist;
use crate::{task_local::*, tl_info};
use axum::extract::ConnectInfo;
use axum::{body::Body, http::Request, middleware::Next, response::Response};
use axum_client_ip::RightmostXForwardedFor as ClientIp;
use chrono::Utc;
use std::net::SocketAddr;

pub async fn access_log(
    client_ip: Result<ClientIp, axum_client_ip::Rejection>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let started_at = Utc::now().timestamp_millis();
    let path = req.uri().path().to_string();
    let uri = req.uri().to_string();
    let method = req.method().to_string();

    let resp = next.run(req).await;
    let ip = client_ip
        .map(|ip| ip.0.to_string())
        .unwrap_or_else(|_| addr.ip().to_string());
    if path != "/ping" {
        let status = resp.status().as_u16();
        let cost = Utc::now().timestamp_millis() - started_at;
        let uri = redact_uri(&uri);
        tl_info!(
            category = "accessLog",
            ip = ip.to_string(),
            method,
            uri,
            status,
            cost,
        );
    }
    resp
}

/// Mask sensitive query parameters (`token`, `secret`) before writing request
/// URIs to access logs so URL-based webhook secrets don't leak on disk.
fn redact_uri(uri: &str) -> String {
    let Some((path, query)) = uri.split_once('?') else {
        return uri.to_string();
    };
    let redacted: Vec<String> = query
        .split('&')
        .map(|pair| match pair.split_once('=') {
            Some((k, _)) if k.eq_ignore_ascii_case("token") || k.eq_ignore_ascii_case("secret") => {
                format!("{k}=***")
            }
            _ => pair.to_string(),
        })
        .collect();
    format!("{path}?{}", redacted.join("&"))
}

pub async fn entry(req: Request<Body>, next: Next) -> Response {
    TRACE_ID
        .scope(generate_trace_id(), async {
            let mut resp = next.run(req).await;

            let headers = resp.headers_mut();
            set_no_cache_if_not_exist(headers);
            resp
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_uri_masks_token_and_secret_query_params() {
        assert_eq!(
            redact_uri("/api/analyze?image=redis:alpine"),
            "/api/analyze?image=redis:alpine"
        );
        assert_eq!(
            redact_uri("/api/webhook?token=topsecret&arch=amd64"),
            "/api/webhook?token=***&arch=amd64"
        );
        assert_eq!(
            redact_uri("/api/webhook?lang=zh&Secret=abc123"),
            "/api/webhook?lang=zh&Secret=***"
        );
    }
}
