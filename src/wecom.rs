//! Push the analysis result to a WeCom (企业微信) group-bot webhook.
//!
//! Enabled via `--wecom-webhook <url|key>` (or `$WECOM_WEBHOOK`). The bot's
//! `markdown` message type has a hard ~4096-byte limit, so callers send a
//! concise payload (the AI report, or a short summary); we still clamp here
//! as a safety net so an oversized message never silently fails to deliver.

use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::util::get_http_client;

// WeCom markdown content hard limit is 4096 bytes; keep margin for the
// truncation marker and any multibyte boundary backoff.
const MAX_CONTENT_BYTES: usize = 4000;
const WEBHOOK_BASE: &str = "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=";

/// Resolved WeCom webhook target. The feature is enabled only when a webhook
/// (CLI arg or `$WECOM_WEBHOOK`) is present.
#[derive(Debug, Clone)]
pub struct WecomConfig {
    pub webhook: String,
}

impl WecomConfig {
    /// Resolve from the explicit CLI value, falling back to `$WECOM_WEBHOOK`.
    /// A full `http(s)://` URL is used verbatim; anything else is treated as a
    /// bare bot key and expanded to the standard webhook URL. Returns `None`
    /// when nothing is configured (feature off).
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

fn first_non_empty(explicit: Option<&str>, env_key: &str) -> Option<String> {
    if let Some(v) = explicit {
        let v = v.trim();
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    if let Ok(v) = std::env::var(env_key) {
        let v = v.trim().to_string();
        if !v.is_empty() {
            return Some(v);
        }
    }
    None
}

/// Clamp to at most `MAX_CONTENT_BYTES`, never splitting a UTF-8 char, and
/// append a marker when content was cut so the recipient knows it's partial.
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

/// Send `content` as a WeCom group-bot `markdown` message. WeCom answers HTTP
/// 200 even on logical errors, so the JSON `errcode` is the real success
/// signal and is checked explicitly.
pub async fn send_markdown(cfg: &WecomConfig, content: &str) -> Result<(), String> {
    let content = clamp(content);

    let body = WecomRequest {
        msgtype: "markdown",
        markdown: MarkdownBody { content: &content },
    };

    let resp = get_http_client()
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
        // No explicit value and (in test) no env var set.
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
        // 3-byte chars: the 4000-byte budget is not a multiple of 3, forcing
        // the boundary backoff. Result must stay valid UTF-8 with the marker.
        let big = "中".repeat(3000);
        let out = clamp(&big);
        assert!(out.len() <= MAX_CONTENT_BYTES);
        let marker = "\n\n… (truncated)";
        assert!(out.ends_with(marker));
        assert!(out.is_char_boundary(out.len()));
        // Every retained char is intact (no replacement chars from a bad cut).
        let body = &out[..out.len() - marker.len()];
        assert!(!body.is_empty());
        assert!(body.chars().all(|c| c == '中'));
    }
}
