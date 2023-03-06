use axum::{http::Request, middleware::Next, response::Response};
use axum_client_ip::SecureClientIp;
use chrono::Utc;
use tracing::{event, Level};

use crate::util::set_no_cache_if_not_exist;

pub async fn access_log<B>(
    SecureClientIp(ip): SecureClientIp,
    req: Request<B>,
    next: Next<B>,
) -> Response {
    let started_at = Utc::now().timestamp_millis();
    let uri = req.uri().to_string();
    let method = req.method().to_string();
    let resp = next.run(req).await;
    let status = resp.status().as_u16();
    let cost = Utc::now().timestamp_millis() - started_at;
    event!(
        Level::INFO,
        category = "accessLog",
        ip = ip.to_string(),
        method,
        uri,
        status,
        cost,
    );
    resp
}

pub async fn entry<B>(req: Request<B>, next: Next<B>) -> Response {
    let mut resp = next.run(req).await;

    let headers = resp.headers_mut();
    set_no_cache_if_not_exist(headers);
    resp
}
