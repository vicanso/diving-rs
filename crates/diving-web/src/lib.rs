//! Web server, REST API, React SPA, MCP server, and webhook runner for diving.

pub mod controller;
pub mod dist;
pub mod error;
pub mod http_util;
pub mod mcp;
pub mod middleware;
pub mod state;
pub mod webhook;

use std::env;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::{error_handling::HandleErrorLayer, middleware::from_fn, Router};
use clap::Parser;
use diving_app::ai::AiConfig;
use diving_app::config::LoadedConfig;
use diving_app::logging::EXIT_ERROR;
use diving_app::notify::{NotifyConfig, NotifyInputs};
use diving_core::{Analyzer, Lang};
use reqwest::Client;
use tokio::signal;
use tower::ServiceBuilder;
use tracing::{error, info, warn};

use crate::error::handle_error;
use crate::middleware::{access_log, entry};
use crate::state::{McpStatus, WebState};
use crate::webhook::WebhookConfig;

/// Web server mode for diving (REST API, React SPA, MCP endpoint, and webhook runner).
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct WebArgs {
    /// Legacy compatibility flag: accepted for `"web"`, rejected otherwise.
    #[arg(short, long, default_value = "web", hide = true)]
    pub mode: String,
    /// Listen address for the web server.
    #[arg(short, long, default_value = "127.0.0.1:7001")]
    pub listen: String,
    /// Config file path (or $DIVING_CONFIG). Defaults to ~/.diving/config.yml
    #[arg(short, long)]
    pub config: Option<String>,
    /// Recommendation output language: en or zh (overrides $DIVING_LANG/$LANG)
    #[arg(long)]
    pub lang: Option<String>,
    /// Skip reading previous AI-history snapshots during webhook AI analysis.
    #[arg(long)]
    pub no_ai_history: bool,
    /// OpenAI-compatible API key; enables AI analysis (or $OPENAI_API_KEY)
    #[arg(long)]
    pub ai_api_key: Option<String>,
    /// OpenAI-compatible API base URL (or $OPENAI_BASE_URL, default OpenAI)
    #[arg(long)]
    pub ai_base_url: Option<String>,
    /// AI model name (or $OPENAI_MODEL, default gpt-4o)
    #[arg(long)]
    pub ai_model: Option<String>,
    /// AI system prompt override (or $OPENAI_SYSTEM_PROMPT)
    #[arg(long)]
    pub ai_system_prompt: Option<String>,
    /// WeCom (企业微信) group-bot webhook URL or key (or $WECOM_WEBHOOK)
    #[arg(long)]
    pub wecom_webhook: Option<String>,
    /// Feishu/Lark (飞书) custom bot webhook URL or hook ID (or $FEISHU_WEBHOOK)
    #[arg(long)]
    pub feishu_webhook: Option<String>,
    /// Feishu/Lark bot signing secret (or $FEISHU_SECRET)
    #[arg(long)]
    pub feishu_secret: Option<String>,
    /// DingTalk (钉钉) custom bot webhook URL or access_token (or $DINGTALK_WEBHOOK)
    #[arg(long)]
    pub dingtalk_webhook: Option<String>,
    /// DingTalk bot signing secret (`SEC...`, or $DINGTALK_SECRET)
    #[arg(long)]
    pub dingtalk_secret: Option<String>,
    /// Slack incoming webhook URL (or $SLACK_WEBHOOK)
    #[arg(long)]
    pub slack_webhook: Option<String>,
    /// Generic outbound HTTP JSON webhook URL (or $NOTIFY_WEBHOOK)
    #[arg(long)]
    pub notify_webhook: Option<String>,
    /// HMAC-SHA256 secret signing the outbound `X-Diving-Signature` header (or $NOTIFY_WEBHOOK_SECRET)
    #[arg(long)]
    pub notify_webhook_secret: Option<String>,
    /// Public base URL of the web UI (e.g. `https://diving.example.com`) for notification links (or $DIVING_PUBLIC_URL)
    #[arg(long)]
    pub public_url: Option<String>,
    /// Bearer/signature token required on `POST /api/webhook` (or $DIVING_WEBHOOK_TOKEN).
    #[arg(long)]
    pub webhook_token: Option<String>,
    /// Disable the inbound `POST /api/webhook` endpoint
    #[arg(long)]
    pub no_webhook: bool,
    /// Bearer token required on the `/mcp` endpoint (or $DIVING_MCP_TOKEN).
    #[arg(long)]
    pub mcp_token: Option<String>,
    /// Disable serving the MCP endpoint at `/mcp`
    #[arg(long)]
    pub no_mcp: bool,
}

impl WebArgs {
    /// The `--config` path, if given.
    pub fn config_file(&self) -> Option<&str> {
        self.config.as_deref()
    }
}

fn start_cleanup_task(analyzer: Analyzer, interval: Duration) {
    tokio::spawn(async move {
        loop {
            if let Err(err) = analyzer.cleanup_caches().await {
                warn!(err = err.to_string(), "cleanup cache fail");
            }
            tokio::time::sleep(interval).await;
        }
    });
}

fn warn_if_mcp_unreachable(listen: &str, has_token: bool, allowed_hosts: &[String]) {
    let exposed = !listen.starts_with("127.0.0.1:")
        && !listen.starts_with("[::1]:")
        && !listen.starts_with("localhost:");
    if exposed && !has_token && allowed_hosts.is_empty() {
        warn!(
            listen,
            "MCP endpoint /mcp will reject non-loopback Host headers unless --mcp-token / DIVING_MCP_TOKEN or mcp_allowed_hosts is configured"
        );
    }
}

fn warn_if_webhook_unauthenticated(listen: &str, has_token: bool, allow_public: bool) {
    if !has_token && allow_public {
        warn!(
            listen,
            "webhook endpoint /api/webhook accepts unauthenticated requests from any IP (webhook_allow_public: true)"
        );
    } else if !has_token
        && !listen.starts_with("127.0.0.1:")
        && !listen.starts_with("[::1]:")
        && !listen.starts_with("localhost:")
    {
        info!(
            listen,
            "webhook endpoint /api/webhook has no token configured and accepts private/loopback sources only"
        );
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

pub fn build_app_router(
    state: Arc<WebState>,
    webhook_cfg: Option<WebhookConfig>,
    mcp_token: Option<String>,
) -> Router {
    let mut routes = controller::new_router(state.clone());
    if state.mcp_status.enabled {
        routes = routes.merge(mcp::new_router(state.clone(), mcp_token));
    }
    if let Some(cfg) = webhook_cfg {
        routes = routes.merge(webhook::new_router(state, cfg));
    }
    Router::new()
        .merge(routes)
        .layer(from_fn(entry))
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(handle_error))
                .timeout(Duration::from_secs(10 * 60)),
        )
        .layer(from_fn(access_log))
}

pub async fn run(args: WebArgs, loaded: LoadedConfig, analyzer: Analyzer, http_client: Client) {
    let default_lang = Lang::resolve(args.lang.as_deref());
    let ai_cfg = AiConfig::resolve(
        args.ai_api_key.as_deref(),
        args.ai_base_url.as_deref(),
        args.ai_model.as_deref(),
        args.ai_system_prompt.as_deref(),
    );
    let notify_cfg = NotifyConfig::resolve(
        NotifyInputs {
            wecom_webhook: args.wecom_webhook.as_deref(),
            feishu_webhook: args.feishu_webhook.as_deref(),
            feishu_secret: args.feishu_secret.as_deref(),
            dingtalk_webhook: args.dingtalk_webhook.as_deref(),
            dingtalk_secret: args.dingtalk_secret.as_deref(),
            slack_webhook: args.slack_webhook.as_deref(),
            notify_webhook: args.notify_webhook.as_deref(),
            notify_webhook_secret: args.notify_webhook_secret.as_deref(),
            public_url: args.public_url.as_deref(),
        },
        &loaded.config,
    );

    start_cleanup_task(analyzer.clone(), loaded.config.cleanup_interval());

    let mcp_token = args
        .mcp_token
        .or_else(|| env::var("DIVING_MCP_TOKEN").ok())
        .filter(|t| !t.trim().is_empty());
    let mcp_status = McpStatus {
        enabled: !args.no_mcp,
        token_required: mcp_token.is_some(),
    };
    let mcp_allowed_hosts = loaded.config.mcp_allowed_hosts();
    if mcp_status.enabled {
        warn_if_mcp_unreachable(&args.listen, mcp_status.token_required, &mcp_allowed_hosts);
    }

    let webhook_cfg = if args.no_webhook {
        None
    } else {
        let webhook_token = args
            .webhook_token
            .or_else(|| env::var("DIVING_WEBHOOK_TOKEN").ok())
            .or_else(|| loaded.config.webhook_token.clone())
            .filter(|t| !t.trim().is_empty());
        let allow_public = loaded.config.webhook_allow_public();
        warn_if_webhook_unauthenticated(&args.listen, webhook_token.is_some(), allow_public);
        Some(WebhookConfig {
            token: webhook_token,
            allow_public,
            ai_cfg,
            no_ai_history: args.no_ai_history,
            notify_cfg,
            default_lang,
        })
    };

    let state = Arc::new(WebState::new(
        analyzer,
        http_client,
        loaded.config_dir.clone(),
        loaded.config.gate_config(),
        loaded.config.max_download_file_size(),
        mcp_status,
        mcp_allowed_hosts,
    ));
    let app = build_app_router(state, webhook_cfg, mcp_token);

    let addr: SocketAddr = match args.listen.parse() {
        Ok(addr) => addr,
        Err(err) => {
            error!(
                listen = args.listen,
                err = err.to_string(),
                "invalid listen address"
            );
            std::process::exit(EXIT_ERROR);
        }
    };
    info!("listening on http://{}", addr);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {addr}: {e}"));
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .unwrap_or_else(|e| panic!("server error: {e}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_args_accepts_legacy_mode_web_flag() {
        let args = WebArgs::try_parse_from([
            "diving-web",
            "--mode",
            "web",
            "--listen",
            "0.0.0.0:7001",
            "--no-webhook",
        ])
        .unwrap();
        assert_eq!(args.mode, "web");
        assert_eq!(args.listen, "0.0.0.0:7001");
        assert!(args.no_webhook);
    }
}
