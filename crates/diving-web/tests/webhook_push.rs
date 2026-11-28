//! End-to-end tests of `POST /api/webhook`:
//! 1. Auth & signature checks, private-network enforcement, and `?token=` support.
//! 2. Background image analysis via `file://` docker-save tar fixture.
//! 3. Concurrent delivery & signature verification across WeCom, Feishu, DingTalk,
//!    Slack, and custom HTTP JSON webhook channels.

mod common;

use axum::body::Bytes;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use common::{build_fixture_tar, test_web_state};
use diving_app::logging::install_crypto_provider;
use diving_app::notify::{
    dingtalk_signed_url, feishu_sign, webhook_signature, CustomWebhookConfig, DingtalkConfig,
    FeishuConfig, NotifyConfig, SlackConfig,
};
use diving_app::wecom::WecomConfig;
use diving_core::Lang;
use diving_web::state::McpStatus;
use diving_web::webhook::{new_router, WebhookConfig};
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::NamedTempFile;
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio::time::timeout;

#[derive(Clone)]
struct CapturedRequest {
    query: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

#[derive(Default)]
struct MockStore {
    hits: Mutex<HashMap<&'static str, Vec<CapturedRequest>>>,
    notify: Notify,
}

impl MockStore {
    fn record(
        &self,
        channel: &'static str,
        query: Option<String>,
        headers: &HeaderMap,
        body: Bytes,
    ) {
        let mut map = HashMap::new();
        for (k, v) in headers {
            if let Ok(s) = v.to_str() {
                map.insert(k.as_str().to_ascii_lowercase(), s.to_string());
            }
        }
        let req = CapturedRequest {
            query: query.unwrap_or_default(),
            headers: map,
            body: body.to_vec(),
        };
        self.hits
            .lock()
            .unwrap()
            .entry(channel)
            .or_default()
            .push(req);
        self.notify.notify_waiters();
    }

    fn take(&self, channel: &'static str) -> Option<CapturedRequest> {
        self.hits
            .lock()
            .unwrap()
            .get(channel)
            .and_then(|v| v.first().cloned())
    }

    fn channel_count(&self) -> usize {
        self.hits.lock().unwrap().len()
    }
}

async fn spawn_mock_notifiers() -> (String, Arc<MockStore>) {
    let store = Arc::new(MockStore::default());
    let router =
        Router::new()
            .route(
                "/wecom",
                post(
                    |State(s): State<Arc<MockStore>>,
                     RawQuery(q): RawQuery,
                     h: HeaderMap,
                     b: Bytes| async move {
                        s.record("wecom", q, &h, b);
                        Json(json!({ "errcode": 0, "errmsg": "ok" }))
                    },
                ),
            )
            .route(
                "/feishu",
                post(
                    |State(s): State<Arc<MockStore>>,
                     RawQuery(q): RawQuery,
                     h: HeaderMap,
                     b: Bytes| async move {
                        s.record("feishu", q, &h, b);
                        Json(json!({ "code": 0, "msg": "success" }))
                    },
                ),
            )
            .route(
                "/dingtalk",
                post(
                    |State(s): State<Arc<MockStore>>,
                     RawQuery(q): RawQuery,
                     h: HeaderMap,
                     b: Bytes| async move {
                        s.record("dingtalk", q, &h, b);
                        Json(json!({ "errcode": 0, "errmsg": "ok" }))
                    },
                ),
            )
            .route(
                "/slack",
                post(
                    |State(s): State<Arc<MockStore>>,
                     RawQuery(q): RawQuery,
                     h: HeaderMap,
                     b: Bytes| async move {
                        s.record("slack", q, &h, b);
                        "ok"
                    },
                ),
            )
            .route(
                "/hook",
                post(
                    |State(s): State<Arc<MockStore>>,
                     RawQuery(q): RawQuery,
                     h: HeaderMap,
                     b: Bytes| async move {
                        s.record("webhook", q, &h, b);
                        Json(json!({ "received": true }))
                    },
                ),
            )
            .with_state(store.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (format!("http://{addr}"), store)
}

async fn spawn_webhook_server(cfg: WebhookConfig) -> String {
    let router = new_router(test_web_state(McpStatus::default()), cfg);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    format!("http://{addr}/api/webhook")
}

fn client() -> Client {
    install_crypto_provider();
    Client::new()
}

fn fixture_image() -> (NamedTempFile, String) {
    let tmp = tempfile::Builder::new().suffix(".tar").tempfile().unwrap();
    fs::write(tmp.path(), build_fixture_tar()).unwrap();
    let image = format!("file://{}", tmp.path().display());
    (tmp, image)
}

#[tokio::test(flavor = "multi_thread")]
async fn webhook_requires_auth_and_pushes_to_all_channels() {
    let (mock_base, store) = spawn_mock_notifiers().await;
    let notify_cfg = NotifyConfig {
        wecom: Some(WecomConfig {
            webhook: format!("{mock_base}/wecom"),
        }),
        feishu: Some(FeishuConfig {
            webhook: format!("{mock_base}/feishu"),
            secret: Some("feishu-sec".to_string()),
        }),
        dingtalk: Some(DingtalkConfig {
            webhook: format!("{mock_base}/dingtalk?access_token=tok"),
            secret: Some("SEC-dt".to_string()),
        }),
        slack: Some(SlackConfig {
            webhook: format!("{mock_base}/slack"),
        }),
        webhook: Some(CustomWebhookConfig {
            webhook: format!("{mock_base}/hook"),
            secret: Some("hook-sec".to_string()),
        }),
        public_url: Some("https://diving.example.com".to_string()),
    };

    let webhook_url = spawn_webhook_server(WebhookConfig {
        token: Some("top-secret-token".to_string()),
        allow_public: false,
        ai_cfg: None,
        no_ai_history: false,
        notify_cfg,
        default_lang: Lang::En,
    })
    .await;

    let c = client();
    let (_tmp, image) = fixture_image();

    // Missing or wrong token returns 401.
    let unauth = c
        .post(&webhook_url)
        .json(&json!({ "image": image }))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);

    let wrong = c
        .post(&webhook_url)
        .header("Authorization", "Bearer wrong")
        .json(&json!({ "image": image }))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

    // Query token also works and ignored Harbor event answers 200 without analyzing.
    let ignored = c
        .post(format!("{webhook_url}?token=top-secret-token"))
        .json(&json!({
            "type": "DELETE_ARTIFACT",
            "event_data": { "resources": [], "repository": { "repo_full_name": "org/app" } }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(ignored.status(), StatusCode::OK);
    let ignored_json: Value = ignored.json().await.unwrap();
    assert_eq!(ignored_json["status"], "ignored");

    // Valid push returns 202 Accepted and notifies all 5 channels asynchronously.
    let accepted = c
        .post(&webhook_url)
        .header("Authorization", "Bearer top-secret-token")
        .json(&json!({ "image": image }))
        .send()
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    let acc_json: Value = accepted.json().await.unwrap();
    assert_eq!(acc_json["status"], "accepted");
    assert_eq!(acc_json["images"][0], image);
    assert_eq!(
        acc_json["notifiers"],
        json!(["wecom", "feishu", "dingtalk", "slack", "webhook"])
    );

    timeout(Duration::from_secs(15), async {
        while store.channel_count() < 5 {
            store.notify.notified().await;
        }
    })
    .await
    .expect("all 5 notification channels must receive a request");

    // 1. Custom HTTP JSON Webhook
    let hook_req = store.take("webhook").unwrap();
    let expected_sig = webhook_signature("hook-sec", &hook_req.body);
    assert_eq!(
        hook_req
            .headers
            .get("x-diving-signature")
            .map(String::as_str),
        Some(expected_sig.as_str())
    );
    let hook_json: Value = serde_json::from_slice(&hook_req.body).unwrap();
    assert_eq!(hook_json["event"], "analysis.completed");
    assert_eq!(hook_json["image"], image);
    let report_url = hook_json["reportUrl"].as_str().unwrap();
    assert!(
        report_url.starts_with("https://diving.example.com/?image="),
        "{report_url}"
    );
    let gate_passed = hook_json["gatePassed"].as_bool().unwrap();
    assert!(!hook_json["recommendations"].as_array().unwrap().is_empty());

    // 2. WeCom
    let wecom_req = store.take("wecom").unwrap();
    let wecom_json: Value = serde_json::from_slice(&wecom_req.body).unwrap();
    assert_eq!(wecom_json["msgtype"], "markdown");
    let wecom_content = wecom_json["markdown"]["content"].as_str().unwrap();
    assert!(wecom_content.contains(report_url), "{wecom_content}");
    assert!(wecom_content.contains("CI gate:"), "{wecom_content}");

    // 3. Feishu interactive card + signature
    let feishu_req = store.take("feishu").unwrap();
    let feishu_json: Value = serde_json::from_slice(&feishu_req.body).unwrap();
    assert_eq!(feishu_json["msg_type"], "interactive");
    let ts: i64 = feishu_json["timestamp"].as_str().unwrap().parse().unwrap();
    assert_eq!(
        feishu_json["sign"].as_str().unwrap(),
        feishu_sign(ts, "feishu-sec")
    );
    assert_eq!(
        feishu_json["card"]["header"]["template"].as_str().unwrap(),
        if gate_passed { "green" } else { "red" }
    );
    assert_eq!(
        feishu_json["card"]["elements"][1]["actions"][0]["url"]
            .as_str()
            .unwrap(),
        report_url
    );

    // 4. DingTalk signed query + markdown body
    let dt_req = store.take("dingtalk").unwrap();
    let ts_ms: i64 = dt_req
        .query
        .split('&')
        .find_map(|p| p.strip_prefix("timestamp="))
        .unwrap()
        .parse()
        .unwrap();
    let expected_url = dingtalk_signed_url(
        &format!("{mock_base}/dingtalk?access_token=tok"),
        ts_ms,
        "SEC-dt",
    );
    let expected_query = expected_url.split_once('?').unwrap().1;
    assert_eq!(dt_req.query, expected_query);
    let dt_json: Value = serde_json::from_slice(&dt_req.body).unwrap();
    assert_eq!(dt_json["msgtype"], "markdown");
    let dt_text = dt_json["markdown"]["text"].as_str().unwrap();
    assert!(dt_text.contains(report_url), "{dt_text}");

    // 5. Slack
    let slack_req = store.take("slack").unwrap();
    let slack_json: Value = serde_json::from_slice(&slack_req.body).unwrap();
    let slack_text = slack_json["text"].as_str().unwrap();
    assert!(slack_text.contains(report_url), "{slack_text}");
    assert!(slack_text.contains("CI gate:"), "{slack_text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn unauthenticated_webhook_allows_loopback_and_rejects_public_forwarded_ip() {
    let webhook_url = spawn_webhook_server(WebhookConfig {
        token: None,
        allow_public: false,
        ai_cfg: None,
        no_ai_history: false,
        notify_cfg: NotifyConfig::default(),
        default_lang: Lang::En,
    })
    .await;

    let c = client();
    let (_tmp, image) = fixture_image();

    let blocked = c
        .post(&webhook_url)
        .header("X-Forwarded-For", "8.8.8.8")
        .json(&json!({ "image": image }))
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), StatusCode::FORBIDDEN);

    let allowed = c
        .post(&webhook_url)
        .json(&json!({ "image": image }))
        .send()
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::ACCEPTED);
}

#[tokio::test(flavor = "multi_thread")]
async fn webhook_rejects_oversized_body_with_413_and_ignores_forwarded_host() {
    let (mock_base, store) = spawn_mock_notifiers().await;
    let notify_cfg = NotifyConfig {
        webhook: Some(CustomWebhookConfig {
            webhook: format!("{mock_base}/hook"),
            secret: None,
        }),
        public_url: None,
        ..Default::default()
    };
    let webhook_url = spawn_webhook_server(WebhookConfig {
        token: Some("sec".to_string()),
        allow_public: false,
        ai_cfg: None,
        no_ai_history: false,
        notify_cfg,
        default_lang: Lang::En,
    })
    .await;

    let c = client();

    // > 1 MiB body returns 413 Payload Too Large.
    let huge = vec![b'a'; 1024 * 1024 + 64];
    let resp = c
        .post(&webhook_url)
        .header("Authorization", "bearer sec")
        .header("Content-Type", "application/json")
        .body(huge)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);

    // Untrusted X-Forwarded-Host / Proto are ignored when public_url is unset.
    let (_tmp, image) = fixture_image();
    let accepted = c
        .post(&webhook_url)
        .header("Authorization", "bearer sec")
        .header("X-Forwarded-Host", "evil.attacker.example")
        .header("X-Forwarded-Proto", "https")
        .json(&json!({ "image": image }))
        .send()
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);

    timeout(Duration::from_secs(15), async {
        while store.channel_count() < 1 {
            store.notify.notified().await;
        }
    })
    .await
    .expect("custom webhook channel must receive notification");

    let hook_req = store.take("webhook").unwrap();
    let hook_json: Value = serde_json::from_slice(&hook_req.body).unwrap();
    let report_url = hook_json["reportUrl"].as_str().unwrap();
    assert!(report_url.starts_with("http://127.0.0.1:"), "{report_url}");
    assert!(
        !report_url.contains("evil.attacker.example"),
        "{report_url}"
    );
}
