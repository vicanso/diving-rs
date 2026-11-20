//! Outbound multi-channel notifications for completed image analyses.

use crate::config::DivingConfig;
use crate::wecom::{send_markdown as send_wecom_markdown, WecomConfig};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chrono::Utc;
use diving_core::{tr, DockerAnalyzeResult, DockerAnalyzeSummary, Lang, Recommendation};
use reqwest::Client;
use ring::hmac;
use serde::Serialize;
use serde_json::{json, Value};
use std::env;
use std::future::Future;
use std::time::Duration;

const FEISHU_HOOK_BASE: &str = "https://open.feishu.cn/open-apis/bot/v2/hook/";
const DINGTALK_HOOK_BASE: &str = "https://oapi.dingtalk.com/robot/send?access_token=";
const FEISHU_MAX_BYTES: usize = 24_000;
const DINGTALK_MAX_BYTES: usize = 18_000;
const SLACK_MAX_BYTES: usize = 30_000;
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct FeishuConfig {
    pub webhook: String,
    pub secret: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DingtalkConfig {
    pub webhook: String,
    pub secret: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SlackConfig {
    pub webhook: String,
}

#[derive(Debug, Clone)]
pub struct CustomWebhookConfig {
    pub webhook: String,
    pub secret: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NotifyInputs<'a> {
    pub wecom_webhook: Option<&'a str>,
    pub feishu_webhook: Option<&'a str>,
    pub feishu_secret: Option<&'a str>,
    pub dingtalk_webhook: Option<&'a str>,
    pub dingtalk_secret: Option<&'a str>,
    pub slack_webhook: Option<&'a str>,
    pub notify_webhook: Option<&'a str>,
    pub notify_webhook_secret: Option<&'a str>,
    pub public_url: Option<&'a str>,
}

#[derive(Debug, Clone, Default)]
pub struct NotifyConfig {
    pub wecom: Option<WecomConfig>,
    pub feishu: Option<FeishuConfig>,
    pub dingtalk: Option<DingtalkConfig>,
    pub slack: Option<SlackConfig>,
    pub webhook: Option<CustomWebhookConfig>,
    pub public_url: Option<String>,
}

impl NotifyConfig {
    pub fn resolve(inputs: NotifyInputs<'_>, file_cfg: &DivingConfig) -> Self {
        Self::resolve_with(inputs, |key| env::var(key).ok(), file_cfg)
    }

    pub fn resolve_with<F>(inputs: NotifyInputs<'_>, env_get: F, file_cfg: &DivingConfig) -> Self
    where
        F: Fn(&str) -> Option<String>,
    {
        let wecom = pick_value(
            inputs.wecom_webhook,
            "WECOM_WEBHOOK",
            file_cfg.wecom_webhook.as_deref(),
            &env_get,
        )
        .and_then(|v| WecomConfig::resolve(Some(&v)));

        let feishu_secret = pick_value(
            inputs.feishu_secret,
            "FEISHU_SECRET",
            file_cfg.feishu_secret.as_deref(),
            &env_get,
        );
        let feishu = pick_value(
            inputs.feishu_webhook,
            "FEISHU_WEBHOOK",
            file_cfg.feishu_webhook.as_deref(),
            &env_get,
        )
        .map(|raw| FeishuConfig {
            webhook: expand_url(&raw, FEISHU_HOOK_BASE),
            secret: feishu_secret,
        });

        let dingtalk_secret = pick_value(
            inputs.dingtalk_secret,
            "DINGTALK_SECRET",
            file_cfg.dingtalk_secret.as_deref(),
            &env_get,
        );
        let dingtalk = pick_value(
            inputs.dingtalk_webhook,
            "DINGTALK_WEBHOOK",
            file_cfg.dingtalk_webhook.as_deref(),
            &env_get,
        )
        .map(|raw| DingtalkConfig {
            webhook: expand_url(&raw, DINGTALK_HOOK_BASE),
            secret: dingtalk_secret,
        });

        let slack = pick_value(
            inputs.slack_webhook,
            "SLACK_WEBHOOK",
            file_cfg.slack_webhook.as_deref(),
            &env_get,
        )
        .map(|webhook| SlackConfig { webhook });

        let webhook_secret = pick_value(
            inputs.notify_webhook_secret,
            "NOTIFY_WEBHOOK_SECRET",
            file_cfg.notify_webhook_secret.as_deref(),
            &env_get,
        );
        let webhook = pick_value(
            inputs.notify_webhook,
            "NOTIFY_WEBHOOK",
            file_cfg.notify_webhook.as_deref(),
            &env_get,
        )
        .map(|webhook| CustomWebhookConfig {
            webhook,
            secret: webhook_secret,
        });

        let public_url = pick_value(
            inputs.public_url,
            "DIVING_PUBLIC_URL",
            file_cfg.public_url.as_deref(),
            &env_get,
        )
        .map(|u| u.trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty());

        Self {
            wecom,
            feishu,
            dingtalk,
            slack,
            webhook,
            public_url,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.wecom.is_none()
            && self.feishu.is_none()
            && self.dingtalk.is_none()
            && self.slack.is_none()
            && self.webhook.is_none()
    }

    pub fn channel_names(&self) -> Vec<&'static str> {
        let mut names = vec![];
        if self.wecom.is_some() {
            names.push("wecom");
        }
        if self.feishu.is_some() {
            names.push("feishu");
        }
        if self.dingtalk.is_some() {
            names.push("dingtalk");
        }
        if self.slack.is_some() {
            names.push("slack");
        }
        if self.webhook.is_some() {
            names.push("webhook");
        }
        names
    }
}

pub struct NotifyContext<'a> {
    pub image: &'a str,
    pub result: &'a DockerAnalyzeResult,
    pub summary: &'a DockerAnalyzeSummary,
    pub gate_passed: bool,
    pub gate_verdict_md: Option<&'a str>,
    pub gate_failures: &'a [String],
    pub ai_report: Option<&'a str>,
    pub summary_md: &'a str,
    pub report_url: Option<&'a str>,
    pub lang: Lang,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebhookNotifyPayload<'a> {
    event: &'static str,
    image: &'a str,
    arch: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    report_url: Option<&'a str>,
    size: u64,
    total_size: u64,
    efficiency_score: u64,
    wasted_size: u64,
    wasted_percent: f64,
    gate_passed: bool,
    gate_failures: &'a [String],
    recommendations: &'a [Recommendation],
    #[serde(skip_serializing_if = "Option::is_none")]
    ai_report: Option<&'a str>,
    summary_markdown: &'a str,
}

fn pick_value<F>(
    explicit: Option<&str>,
    env_key: &str,
    cfg_val: Option<&str>,
    env_get: &F,
) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    if let Some(v) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(v.to_string());
    }
    if let Some(v) = env_get(env_key) {
        let v = v.trim();
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    cfg_val
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn expand_url(raw: &str, base: &str) -> String {
    if raw.starts_with("http://") || raw.starts_with("https://") {
        raw.to_string()
    } else {
        format!("{base}{raw}")
    }
}

pub fn percent_encode_query(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0f) as usize] as char);
        }
    }
    out
}

pub fn build_report_url(base_url: &str, image: &str, arch: &str) -> Option<String> {
    let base = base_url.trim().trim_end_matches('/');
    if !(base.starts_with("http://") || base.starts_with("https://")) {
        return None;
    }
    let clean_image = image.split('?').next().unwrap_or(image).trim();
    if clean_image.is_empty() {
        return None;
    }
    Some(format!(
        "{base}/?image={}&arch={}",
        percent_encode_query(clean_image),
        percent_encode_query(arch.trim())
    ))
}

fn clamp_utf8(content: &str, max_bytes: usize) -> String {
    if content.len() <= max_bytes {
        return content.to_string();
    }
    const MARK: &str = "\n\n… (truncated)";
    let mut end = max_bytes.saturating_sub(MARK.len());
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{MARK}", &content[..end])
}

pub fn feishu_sign(timestamp_secs: i64, secret: &str) -> String {
    let string_to_sign = format!("{timestamp_secs}\n{secret}");
    let key = hmac::Key::new(hmac::HMAC_SHA256, string_to_sign.as_bytes());
    let tag = hmac::sign(&key, &[]);
    B64.encode(tag.as_ref())
}

pub fn dingtalk_signed_url(webhook: &str, timestamp_ms: i64, secret: &str) -> String {
    let string_to_sign = format!("{timestamp_ms}\n{secret}");
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let tag = hmac::sign(&key, string_to_sign.as_bytes());
    let sign = percent_encode_query(&B64.encode(tag.as_ref()));
    let sep = if webhook.contains('?') { '&' } else { '?' };
    format!("{webhook}{sep}timestamp={timestamp_ms}&sign={sign}")
}

pub fn webhook_signature(secret: &str, body: &[u8]) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
    let tag = hmac::sign(&key, body);
    format!("sha256={}", hex::encode(tag.as_ref()))
}

pub fn build_markdown_message(ctx: &NotifyContext<'_>) -> String {
    let mut parts = vec![format!("## {}: {}", tr(ctx.lang, "wecom.title"), ctx.image)];
    if let Some(verdict) = ctx.gate_verdict_md {
        parts.push(verdict.to_string());
    }
    if let Some(url) = ctx.report_url {
        parts.push(format!("[{}]({url})", tr(ctx.lang, "notify.report_link")));
    }
    parts.push(ctx.ai_report.unwrap_or(ctx.summary_md).to_string());
    parts.join("\n\n")
}

fn build_slack_message(ctx: &NotifyContext<'_>) -> String {
    let mut parts = vec![format!("*{}: {}*", tr(ctx.lang, "wecom.title"), ctx.image)];
    if let Some(verdict) = ctx.gate_verdict_md {
        parts.push(verdict.replace("**", "*"));
    }
    if let Some(url) = ctx.report_url {
        parts.push(format!("<{url}|{}>", tr(ctx.lang, "notify.report_link")));
    }
    parts.push(ctx.ai_report.unwrap_or(ctx.summary_md).to_string());
    clamp_utf8(&parts.join("\n\n"), SLACK_MAX_BYTES)
}

async fn send_feishu(
    http_client: &Client,
    cfg: &FeishuConfig,
    ctx: &NotifyContext<'_>,
) -> Result<(), String> {
    let mut body_parts = vec![];
    if let Some(verdict) = ctx.gate_verdict_md {
        body_parts.push(verdict.to_string());
    }
    body_parts.push(ctx.ai_report.unwrap_or(ctx.summary_md).to_string());
    let body_md = clamp_utf8(&body_parts.join("\n\n"), FEISHU_MAX_BYTES);

    let mut elements = vec![json!({
        "tag": "div",
        "text": {
            "tag": "lark_md",
            "content": body_md,
        }
    })];
    if let Some(url) = ctx.report_url {
        elements.push(json!({
            "tag": "action",
            "actions": [{
                "tag": "button",
                "text": {
                    "tag": "plain_text",
                    "content": tr(ctx.lang, "notify.report_link"),
                },
                "type": "primary",
                "url": url,
            }]
        }));
    }

    let mut payload = json!({
        "msg_type": "interactive",
        "card": {
            "config": { "wide_screen_mode": true },
            "header": {
                "title": {
                    "tag": "plain_text",
                    "content": format!("{}: {}", tr(ctx.lang, "wecom.title"), ctx.image),
                },
                "template": if ctx.gate_passed { "green" } else { "red" },
            },
            "elements": elements,
        }
    });
    if let Some(secret) = cfg.secret.as_deref() {
        let ts = Utc::now().timestamp();
        payload["timestamp"] = json!(ts.to_string());
        payload["sign"] = json!(feishu_sign(ts, secret));
    }

    let resp = http_client
        .post(&cfg.webhook)
        .timeout(HTTP_TIMEOUT)
        .json(&payload)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", text.trim()));
    }
    let parsed: Value = serde_json::from_str(&text).map_err(|e| format!("{e}: {}", text.trim()))?;
    let code = parsed
        .get("code")
        .or_else(|| parsed.get("StatusCode"))
        .and_then(Value::as_i64);
    if code != Some(0) {
        let msg = parsed
            .get("msg")
            .or_else(|| parsed.get("StatusMessage"))
            .and_then(Value::as_str)
            .unwrap_or(text.trim());
        return Err(format!("code {:?}: {msg}", code));
    }
    Ok(())
}

async fn send_dingtalk(
    http_client: &Client,
    cfg: &DingtalkConfig,
    ctx: &NotifyContext<'_>,
) -> Result<(), String> {
    let url = match cfg.secret.as_deref() {
        Some(secret) => dingtalk_signed_url(&cfg.webhook, Utc::now().timestamp_millis(), secret),
        None => cfg.webhook.clone(),
    };
    let text = clamp_utf8(&build_markdown_message(ctx), DINGTALK_MAX_BYTES);
    let payload = json!({
        "msgtype": "markdown",
        "markdown": {
            "title": format!("{}: {}", tr(ctx.lang, "wecom.title"), ctx.image),
            "text": text,
        }
    });

    let resp = http_client
        .post(&url)
        .timeout(HTTP_TIMEOUT)
        .json(&payload)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", text.trim()));
    }
    let parsed: Value = serde_json::from_str(&text).map_err(|e| format!("{e}: {}", text.trim()))?;
    let errcode = parsed.get("errcode").and_then(Value::as_i64);
    if errcode != Some(0) {
        let errmsg = parsed
            .get("errmsg")
            .and_then(Value::as_str)
            .unwrap_or(text.trim());
        return Err(format!("errcode {:?}: {errmsg}", errcode));
    }
    Ok(())
}

async fn send_slack(
    http_client: &Client,
    cfg: &SlackConfig,
    ctx: &NotifyContext<'_>,
) -> Result<(), String> {
    let payload = json!({ "text": build_slack_message(ctx) });
    let resp = http_client
        .post(&cfg.webhook)
        .timeout(HTTP_TIMEOUT)
        .json(&payload)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP {status}: {}", text.trim()));
    }
    Ok(())
}

async fn send_custom_webhook(
    http_client: &Client,
    cfg: &CustomWebhookConfig,
    ctx: &NotifyContext<'_>,
) -> Result<(), String> {
    let payload = WebhookNotifyPayload {
        event: "analysis.completed",
        image: ctx.image,
        arch: &ctx.result.arch,
        report_url: ctx.report_url,
        size: ctx.result.size,
        total_size: ctx.result.total_size,
        efficiency_score: ctx.summary.score,
        wasted_size: ctx.summary.wasted_size,
        wasted_percent: ctx.summary.wasted_percent,
        gate_passed: ctx.gate_passed,
        gate_failures: ctx.gate_failures,
        recommendations: &ctx.result.recommendations,
        ai_report: ctx.ai_report,
        summary_markdown: ctx.summary_md,
    };
    let bytes = serde_json::to_vec(&payload).map_err(|e| e.to_string())?;
    let mut req = http_client
        .post(&cfg.webhook)
        .timeout(HTTP_TIMEOUT)
        .header("Content-Type", "application/json");
    if let Some(secret) = cfg.secret.as_deref() {
        req = req.header("X-Diving-Signature", webhook_signature(secret, &bytes));
    }
    let resp = req.body(bytes).send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP {status}: {}", text.trim()));
    }
    Ok(())
}

async fn send_opt<'a, T, F, Fut>(
    name: &'static str,
    cfg: Option<&'a T>,
    sender: F,
) -> Option<String>
where
    F: FnOnce(&'a T) -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let c = cfg?;
    sender(c).await.err().map(|e| format!("{name}: {e}"))
}

pub async fn send_all(
    http_client: &Client,
    cfg: &NotifyConfig,
    ctx: &NotifyContext<'_>,
) -> Result<(), Vec<String>> {
    let wecom_msg = build_markdown_message(ctx);
    let (wecom_err, feishu_err, dingtalk_err, slack_err, webhook_err) = tokio::join!(
        send_opt("wecom", cfg.wecom.as_ref(), |c| send_wecom_markdown(
            http_client,
            c,
            &wecom_msg
        )),
        send_opt("feishu", cfg.feishu.as_ref(), |c| send_feishu(
            http_client,
            c,
            ctx
        )),
        send_opt("dingtalk", cfg.dingtalk.as_ref(), |c| send_dingtalk(
            http_client,
            c,
            ctx
        )),
        send_opt("slack", cfg.slack.as_ref(), |c| send_slack(
            http_client,
            c,
            ctx
        )),
        send_opt("webhook", cfg.webhook.as_ref(), |c| send_custom_webhook(
            http_client,
            c,
            ctx
        )),
    );
    let errors: Vec<String> = [wecom_err, feishu_err, dingtalk_err, slack_err, webhook_err]
        .into_iter()
        .flatten()
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn resolve_prioritizes_cli_then_env_then_config_and_expands_keys() {
        let file_cfg = DivingConfig {
            wecom_webhook: Some("cfg-wecom".to_string()),
            feishu_webhook: Some("cfg-feishu-hook".to_string()),
            feishu_secret: Some("cfg-feishu-sec".to_string()),
            dingtalk_webhook: Some("cfg-dingtalk-token".to_string()),
            dingtalk_secret: Some("SEC-cfg".to_string()),
            slack_webhook: Some("https://hooks.slack.com/services/A/B/C".to_string()),
            notify_webhook: Some("https://cfg.example.com/hook".to_string()),
            notify_webhook_secret: Some("cfg-hook-sec".to_string()),
            public_url: Some("https://cfg.diving.example.com/".to_string()),
            ..Default::default()
        };
        let envs = HashMap::from([
            ("FEISHU_WEBHOOK", "env-feishu-hook"),
            ("DIVING_PUBLIC_URL", "https://env.diving.example.com/"),
        ]);
        let resolved = NotifyConfig::resolve_with(
            NotifyInputs {
                wecom_webhook: Some("cli-wecom"),
                slack_webhook: Some("  "),
                ..Default::default()
            },
            |k| envs.get(k).map(|v| (*v).to_string()),
            &file_cfg,
        );

        assert_eq!(
            resolved.wecom.as_ref().unwrap().webhook,
            "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=cli-wecom"
        );
        assert_eq!(
            resolved.feishu.as_ref().unwrap().webhook,
            "https://open.feishu.cn/open-apis/bot/v2/hook/env-feishu-hook"
        );
        assert_eq!(
            resolved.feishu.as_ref().unwrap().secret.as_deref(),
            Some("cfg-feishu-sec")
        );
        assert_eq!(
            resolved.dingtalk.as_ref().unwrap().webhook,
            "https://oapi.dingtalk.com/robot/send?access_token=cfg-dingtalk-token"
        );
        assert_eq!(
            resolved.slack.as_ref().unwrap().webhook,
            "https://hooks.slack.com/services/A/B/C"
        );
        assert_eq!(
            resolved.webhook.as_ref().unwrap().webhook,
            "https://cfg.example.com/hook"
        );
        assert_eq!(
            resolved.public_url.as_deref(),
            Some("https://env.diving.example.com")
        );
        assert_eq!(
            resolved.channel_names(),
            vec!["wecom", "feishu", "dingtalk", "slack", "webhook"]
        );
    }

    #[test]
    fn report_url_percent_encodes_and_strips_inline_query() {
        assert_eq!(
            build_report_url(
                "https://diving.example.com/",
                "ghcr.io/org/app:v1.2?arch=arm64",
                "arm64"
            ),
            Some(
                "https://diving.example.com/?image=ghcr.io%2Forg%2Fapp%3Av1.2&arch=arm64"
                    .to_string()
            )
        );
        assert_eq!(build_report_url("ftp://bad.host", "redis", "amd64"), None);
        assert_eq!(
            build_report_url("https://diving.example.com", "", "amd64"),
            None
        );
    }

    #[test]
    fn signatures_match_known_vectors() {
        let key = hmac::Key::new(hmac::HMAC_SHA256, b"1599360473\ndemo-secret");
        let expected_feishu = B64.encode(hmac::sign(&key, &[]).as_ref());
        assert_eq!(feishu_sign(1599360473, "demo-secret"), expected_feishu);

        let dt_key = hmac::Key::new(hmac::HMAC_SHA256, b"SECdemo");
        let dt_raw = B64.encode(hmac::sign(&dt_key, b"1700000000000\nSECdemo").as_ref());
        let dt_url = dingtalk_signed_url(
            "https://oapi.dingtalk.com/robot/send?access_token=abc",
            1700000000000,
            "SECdemo",
        );
        assert_eq!(
            dt_url,
            format!(
                "https://oapi.dingtalk.com/robot/send?access_token=abc&timestamp=1700000000000&sign={}",
                percent_encode_query(&dt_raw)
            )
        );

        assert_eq!(
            webhook_signature("my-secret", br#"{"hello":"world"}"#),
            format!(
                "sha256={}",
                hex::encode(
                    hmac::sign(
                        &hmac::Key::new(hmac::HMAC_SHA256, b"my-secret"),
                        br#"{"hello":"world"}"#
                    )
                    .as_ref()
                )
            )
        );
    }

    #[test]
    fn markdown_message_places_verdict_and_link_before_body() {
        let result = DockerAnalyzeResult::default();
        let summary = result.summary();
        let failures = vec![];
        let ctx = NotifyContext {
            image: "redis:alpine",
            result: &result,
            summary: &summary,
            gate_passed: false,
            gate_verdict_md: Some("**CI gate: failed**\n- FAIL: check"),
            gate_failures: &failures,
            ai_report: None,
            summary_md: "**Efficiency:** 98 %",
            report_url: Some("https://diving.example.com/?image=redis%3Aalpine&arch=amd64"),
            lang: Lang::En,
        };
        let msg = build_markdown_message(&ctx);
        let verdict_pos = msg.find("**CI gate: failed**").unwrap();
        let link_pos = msg.find("[Open full report](").unwrap();
        let body_pos = msg.find("**Efficiency:** 98 %").unwrap();
        assert!(verdict_pos < link_pos && link_pos < body_pos, "{msg}");
    }
}
