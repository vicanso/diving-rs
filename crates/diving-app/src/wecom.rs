//! Push the analysis result to a WeCom (企业微信) group-bot webhook.

use crate::env_util::first_non_empty;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const MAX_CONTENT_BYTES: usize = 4000;
const WEBHOOK_BASE: &str = "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=";

#[derive(Debug, Clone)]
pub struct WecomConfig {
    pub webhook: String,
}

impl WecomConfig {
    pub fn resolve(webhook: Option<&str>) -> Option<WecomConfig> {
        let raw = first_non_empty(webhook, "WECOM_WEBHOOK")?;
        let webhook = if raw.starts_with("http://") || raw.starts_with("https://") {
            raw
        } else {
            format!("{WEBHOOK_BASE}{raw}")
        };
        Some(WecomConfig { webhook })
    }
}

fn clamp(content: &str) -> String {
    if content.len() <= MAX_CONTENT_BYTES {
        return content.to_string();
    }
    const MARK: &str = "\n\n… (truncated)";
    let mut end = MAX_CONTENT_BYTES - MARK.len();
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{MARK}", &content[..end])
}

#[derive(Serialize)]
struct MarkdownBody<'a> {
    content: &'a str,
}

#[derive(Serialize)]
struct WecomRequest<'a> {
    msgtype: &'a str,
    markdown: MarkdownBody<'a>,
}

#[derive(Deserialize)]
struct WecomResponse {
    errcode: i64,
    #[serde(default)]
    errmsg: String,
}

pub async fn send_markdown(
    http_client: &Client,
    cfg: &WecomConfig,
    content: &str,
) -> Result<(), String> {
    let content = clamp(content);

    let body = WecomRequest {
        msgtype: "markdown",
        markdown: MarkdownBody { content: &content },
    };

    let resp = http_client
        .post(&cfg.webhook)
        .timeout(Duration::from_secs(30))
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", text.trim()));
    }

    let parsed: WecomResponse =
        serde_json::from_str(&text).map_err(|e| format!("{e}: {}", text.trim()))?;
    if parsed.errcode != 0 {
        return Err(format!("errcode {}: {}", parsed.errcode, parsed.errmsg));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_expands_bare_key() {
        let cfg = WecomConfig::resolve(Some("abc-123")).expect("bare key resolves");
        assert_eq!(
            cfg.webhook,
            "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=abc-123"
        );
    }

    #[test]
    fn resolve_keeps_full_url_verbatim() {
        let url = "https://example.internal/hook?key=xyz";
        let cfg = WecomConfig::resolve(Some(url)).expect("full url resolves");
        assert_eq!(cfg.webhook, url);
    }

    #[test]
    fn resolve_is_none_without_value() {
        std::env::remove_var("WECOM_WEBHOOK");
        assert!(WecomConfig::resolve(None).is_none());
        assert!(WecomConfig::resolve(Some("   ")).is_none());
    }

    #[test]
    fn clamp_keeps_short_content_unchanged() {
        assert_eq!(clamp("hello"), "hello");
    }

    #[test]
    fn clamp_truncates_on_utf8_boundary() {
        let big = "中".repeat(3000);
        let out = clamp(&big);
        assert!(out.len() <= MAX_CONTENT_BYTES);
        let marker = "\n\n… (truncated)";
        assert!(out.ends_with(marker));
        assert!(out.is_char_boundary(out.len()));
        let body = &out[..out.len() - marker.len()];
        assert!(!body.is_empty());
        assert!(body.chars().all(|c| c == '中'));
    }
}
