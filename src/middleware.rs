use crate::{task_local::*, tl_info};
use axum::{body::Body, http::Request, middleware::Next, response::Response};
use axum_client_ip::InsecureClientIp;
use chrono::Utc;

use crate::task_local::{generate_trace_id, TRACE_ID};
use crate::util::set_no_cache_if_not_exist;

pub async fn access_log(
    InsecureClientIp(ip): InsecureClientIp,
    req: Request<Body>,
    next: Next,
) -> Response {
    let started_at = Utc::now().timestamp_millis();
    let path = req.uri().path().to_string();
    let uri = req.uri().to_string();
    let method = req.method().to_string();
    let resp = next.run(req).await;
    if path != "/ping" {
        let status = resp.status().as_u16();
        let cost = Utc::now().timestamp_millis() - started_at;
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
