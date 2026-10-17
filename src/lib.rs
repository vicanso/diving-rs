//! diving-rs library surface.
//!
//! The binary entry point lives in `main.rs`; all analysis, web, and UI
//! modules live here so unit tests can run via `cargo test --lib`.

pub mod ai;
pub mod config;
pub mod controller;
pub mod diff;
pub mod dist;
pub mod error;
pub mod i18n;
pub mod image;
pub mod markdown;
pub mod mcp;
pub mod middleware;
pub mod recommend;
pub mod store;
pub mod task_local;
pub mod ui;
pub mod util;
pub mod wecom;

use axum::{error_handling::HandleErrorLayer, middleware::from_fn, Router};
use bytesize::ByteSize;
use clap::{Parser, Subcommand};
use colored::*;
use std::fs;
use std::net::SocketAddr;
use std::time::Duration;
use std::{env, str::FromStr};
use tokio::signal;
use tower::ServiceBuilder;
use tracing::Level;
use tracing::{error, info, warn};
use tracing_subscriber::FmtSubscriber;

use controller::new_router;
use diff::{diff_images, to_diff_markdown, ImageDiff};
use image::{
    analyze_docker_image, analyze_docker_image_or_last_known, parse_image_info,
    DockerAnalyzeResult, DockerAnalyzeSummary,
};
use mcp::McpStatus;
use middleware::{access_log, entry};
use recommend::gate_violations;
use store::{clear_analysis_files, clear_blob_files, enforce_layer_cache_limit};
use task_local::{generate_trace_id, TRACE_ID};

/// A tool for exploring each layer in a docker image.
/// It can run in terminal or as a web service.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    #[command(subcommand)]
    command: Option<Command>,
    /// Running mode of diving, terminal or web
    #[arg(short, long, default_value = "terminal")]
    mode: String,
    image: Option<String>,
    /// The listen addr of web mode
    #[arg(short, long, default_value = "127.0.0.1:7001")]
    listen: String,
    /// Config file path (or $DIVING_CONFIG). Defaults to ~/.diving/config.yml
    #[arg(short, long, global = true)]
    config: Option<String>,
    /// The result output file
    #[arg(short, long, global = true)]
    output_file: Option<String>,
    /// Also write a short Markdown summary (headline numbers, largest files,
    /// recommendations, and the CI verdict under CI=true), e.g. for a
    /// pull-request comment. `-` prints it to stdout
    #[arg(long, global = true)]
    summary: Option<String>,
    /// Include base image layers in the analysis (auto-detected and hidden by default)
    #[arg(long)]
    no_skip_base: bool,
    /// Skip cross-layer duplicate file detection (and its analysis cache),
    /// trading the dup card for a faster cold analysis. Useful in CI.
    #[arg(long, global = true)]
    no_verify_dup: bool,
    /// Skip reading the previous AI-history snapshot — the AI report does no
    /// regression comparison this run (the current analysis is still recorded
    /// for future runs). Only affects AI mode (`--ai-api-key`).
    #[arg(long)]
    no_ai_history: bool,
    /// Recommendation output language: en or zh (overrides $DIVING_LANG/$LANG)
    #[arg(long, global = true)]
    lang: Option<String>,
    /// OpenAI-compatible API key; enables AI analysis (or $OPENAI_API_KEY)
    #[arg(long)]
    ai_api_key: Option<String>,
    /// OpenAI-compatible API base URL (or $OPENAI_BASE_URL, default OpenAI)
    #[arg(long)]
    ai_base_url: Option<String>,
    /// AI model name (or $OPENAI_MODEL, default gpt-4o)
    #[arg(long)]
    ai_model: Option<String>,
    /// AI system prompt override; replaces the built-in DevSecOps template (or $OPENAI_SYSTEM_PROMPT)
    #[arg(long)]
    ai_system_prompt: Option<String>,
    /// WeCom (企业微信) group-bot webhook URL or key; pushes the result there (or $WECOM_WEBHOOK)
    #[arg(long)]
    wecom_webhook: Option<String>,
    /// Registry username for private pulls (or $REGISTRY_USERNAME). Also
    /// falls back to `~/.docker/config.json` / credential helpers when unset.
    #[arg(long, short = 'u', global = true)]
    username: Option<String>,
    /// Registry password / PAT (or $REGISTRY_PASSWORD). Prefer
    /// `--password-stdin` in scripts so the secret is not in argv.
    #[arg(long, global = true)]
    password: Option<String>,
    /// Read the registry password from stdin (one line). Mutually useful with
    /// `--username` / `$REGISTRY_USERNAME`.
    #[arg(long, global = true)]
    password_stdin: bool,
    /// Web mode: bearer token required on the `/mcp` endpoint (or
    /// $DIVING_MCP_TOKEN). Without it `/mcp` only accepts loopback `Host`
    /// headers plus `mcp_allowed_hosts` from config.yml.
    #[arg(long)]
    mcp_token: Option<String>,
    /// Web mode: don't serve the MCP endpoint at `/mcp`
    #[arg(long)]
    no_mcp: bool,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Compare two images: size change, shared and new layers, added /
    /// removed / resized files and recommendation changes. Prints Markdown;
    /// `-o` writes JSON (or Markdown for `.md`). Under CI=true it fails when
    /// the target grew more than `highest_size_increase` in config.yml
    Diff {
        /// The image to compare against, e.g. `app:1.0` or `app:main`
        base: String,
        /// The image to check, e.g. `app:1.1` or `app:pr-42`
        target: String,
    },
}

impl Args {
    pub fn is_terminal_type(&self) -> bool {
        self.mode == "terminal"
    }
    /// The `--config` path, if given.
    pub fn config_file(&self) -> Option<&str> {
        self.config.as_deref()
    }
}

pub fn init_logger(terminal_mode: bool) {
    // Terminal mode prints user-friendly progress to stderr and doesn't need
    // structured request-level logs by default. Web mode keeps INFO for traceable
    // server logs. LOG_LEVEL env var still overrides either default.
    let mut level = if terminal_mode {
        Level::WARN
    } else {
        Level::INFO
    };
    if let Ok(log_level) = env::var("LOG_LEVEL") {
        if let Ok(value) = Level::from_str(log_level.as_str()) {
            level = value;
        }
    }
    let timer = tracing_subscriber::fmt::time::OffsetTime::local_rfc_3339().unwrap_or_else(|_| {
        tracing_subscriber::fmt::time::OffsetTime::new(
            time::UtcOffset::from_hms(0, 0, 0).unwrap(),
            time::format_description::well_known::Rfc3339,
        )
    });
    let env = std::env::var("RUST_ENV").unwrap_or_default();
    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .with_timer(timer)
        .with_ansi(env != "production")
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
}

fn start_cleanup_task() {
    let interval_hours = config::must_load_config()
        .cleanup_interval_hours
        .unwrap_or(1);
    let duration = Duration::from_secs(interval_hours * 3600);
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(duration);
        ticker.tick().await; // skip immediate first tick
        loop {
            ticker.tick().await;
            let result = clear_blob_files().await;
            if let Err(err) = result {
                error!(err = err.to_string(), "clear blob files fail");
            } else {
                info!("clear blob files success");
            }
            // Sweep stale analysis-result cache entries using the same TTL.
            if let Err(err) = clear_analysis_files().await {
                error!(err = err.to_string(), "clear analysis cache fail");
            } else {
                info!("clear analysis cache success");
            }
            // TTL 清理之后按容量上限淘汰（配置 max_layer_cache_size 才生效）。
            if let Err(err) = enforce_layer_cache_limit().await {
                error!(err = err.to_string(), "enforce cache size limit fail");
            }
        }
    });
}

fn is_ci() -> bool {
    env::var_os("CI").unwrap_or_default() == "true"
}

/// Exit code when the image fails the CI gate. Kept apart from
/// [`EXIT_ERROR`] so a pipeline can tell "this image is not good enough"
/// from "diving could not do its job".
const EXIT_GATE_FAILED: i32 = 1;
/// Exit code when diving itself fails: the image can't be pulled or
/// analyzed, the config is invalid, a push fails, …
pub const EXIT_ERROR: i32 = 2;

// 镜像分析的入参集合（由 CLI args 解析而来）。
struct AnalyzeOptions {
    image: String,
    output_file: String,
    summary_file: Option<String>,
    skip_base: bool,
    verify_dup: bool,
    no_ai_history: bool,
    lang: i18n::Lang,
    ai_cfg: Option<ai::AiConfig>,
    wecom_cfg: Option<wecom::WecomConfig>,
    credentials: Option<image::RegistryCredentials>,
}

/// One failed CI check: the i18n key of its message plus the arguments that
/// follow the `{0}` "FAIL" label.
struct GateFailure {
    key: &'static str,
    args: Vec<String>,
}

impl GateFailure {
    fn render(&self, lang: i18n::Lang, label: &str) -> String {
        let mut args = vec![label];
        args.extend(self.args.iter().map(String::as_str));
        i18n::fill(i18n::tr(lang, self.key), &args)
    }
}

/// The CI gate's thresholds, as configured.
struct GateLimits<'a> {
    /// Minimum efficiency score, 0–100.
    lowest_efficiency: u64,
    highest_wasted_bytes: u64,
    highest_user_wasted_percent: f64,
    /// Uncompressed image size cap, the SIZE `docker images` shows.
    highest_image_size: Option<u64>,
    fail_on_severity: Option<&'a str>,
    ignored: &'a [String],
}

impl GateLimits<'static> {
    fn from_config() -> Self {
        GateLimits {
            lowest_efficiency: (config::get_lowest_efficiency() * 100.0) as u64,
            highest_wasted_bytes: config::get_highest_wasted_bytes(),
            highest_user_wasted_percent: config::get_highest_user_wasted_percent(),
            highest_image_size: config::get_highest_image_size(),
            fail_on_severity: config::get_fail_on_severity(),
            ignored: config::get_ignore_recommendations(),
        }
    }
}

/// Run the CI checks as configured. Empty means the image passes.
fn evaluate_gate(
    result: &DockerAnalyzeResult,
    summary: &DockerAnalyzeSummary,
    lang: i18n::Lang,
) -> Vec<GateFailure> {
    evaluate_gate_with(result, summary, lang, &GateLimits::from_config())
}

/// The efficiency thresholds, the optional image-size cap and the optional
/// `fail_on_severity` gate, against explicit `limits`.
fn evaluate_gate_with(
    result: &DockerAnalyzeResult,
    summary: &DockerAnalyzeSummary,
    lang: i18n::Lang,
    limits: &GateLimits,
) -> Vec<GateFailure> {
    let lowest_efficiency = limits.lowest_efficiency;
    let highest_wasted_bytes = limits.highest_wasted_bytes;
    let highest_user_wasted_percent = limits.highest_user_wasted_percent;
    let mut failures = vec![];
    if summary.score < lowest_efficiency {
        failures.push(GateFailure {
            key: "cli.check.eff",
            args: vec![lowest_efficiency.to_string()],
        });
    }
    if summary.wasted_size > highest_wasted_bytes {
        failures.push(GateFailure {
            key: "cli.check.bytes",
            args: vec![ByteSize(highest_wasted_bytes).to_string()],
        });
    }
    if summary.wasted_percent > highest_user_wasted_percent {
        failures.push(GateFailure {
            key: "cli.check.pct",
            args: vec![format!("{highest_user_wasted_percent:.2}")],
        });
    }
    if let Some(limit) = limits.highest_image_size {
        if result.total_size > limit {
            failures.push(GateFailure {
                key: "cli.check.size",
                args: vec![
                    ByteSize(result.total_size).to_string(),
                    ByteSize(limit).to_string(),
                ],
            });
        }
    }
    if let Some(threshold) = limits.fail_on_severity {
        let count = gate_violations(&result.recommendations, threshold, limits.ignored).len();
        if count > 0 {
            failures.push(GateFailure {
                key: "cli.check.severity",
                args: vec![
                    count.to_string(),
                    i18n::tr(lang, &format!("sev.{threshold}")).to_string(),
                ],
            });
        }
    }
    failures
}

/// The gate outcome as Markdown, for the WeCom push: a bold verdict line and,
/// when it failed, one bullet per failed check.
fn gate_verdict_md(failures: &[GateFailure], lang: i18n::Lang) -> String {
    if failures.is_empty() {
        return format!("**{}**", i18n::tr(lang, "cli.gate.pass"));
    }
    let label = i18n::tr(lang, "cli.fail");
    let mut md = format!("**{}**", i18n::tr(lang, "cli.gate.fail"));
    for failure in failures {
        md.push_str(&format!("\n- {}", failure.render(lang, label)));
    }
    md
}

// 分析镜像。`Ok(false)` 表示分析成功但没通过 CI 卡口；`Err` 是分析或
// 输出本身失败（错误直接以字符串返回）。
async fn analyze(opts: AnalyzeOptions) -> Result<bool, String> {
    let AnalyzeOptions {
        image,
        output_file,
        summary_file,
        skip_base,
        verify_dup,
        no_ai_history,
        lang,
        ai_cfg,
        wecom_cfg,
        credentials,
    } = opts;
    sweep_caches().await?;
    let result = analyze_for_cli(&image, lang, verify_dup, credentials).await?;
    // The gate is decided before anything is sent, so in CI the WeCom push
    // can carry the verdict instead of leaving readers to guess.
    let summary = result.summary();
    let failures = evaluate_gate(&result, &summary, lang);
    let verdict = is_ci().then(|| gate_verdict_md(&failures, lang));
    // AI analysis takes precedence: print the model's report and skip the TUI.
    let reported = ai_cfg.is_some() || wecom_cfg.is_some();
    if let Some(ai_cfg) = ai_cfg {
        let mut md = markdown::to_markdown(&result, skip_base, lang);
        // Inline the actual ENTRYPOINT/CMD startup script(s) so the model can
        // reason about what the container really runs. Decompresses layers, so
        // keep it off the async executor.
        let scripts = tokio::task::block_in_place(|| ai::entrypoint_scripts_md(&result, lang));
        if !scripts.is_empty() {
            md.push_str("\n\n");
            md.push_str(&scripts);
        }
        let report = ai::analyze_with_ai(&image, &md, &ai_cfg, lang, no_ai_history)
            .await
            .map_err(|e| i18n::fill(i18n::tr(lang, "ai.fail"), &[&e]))?;
        println!("{}", i18n::tr(lang, "ai.report").bold().green());
        println!("{report}");
        // Smart selection: with AI enabled, push the concise AI report.
        if let Some(wecom_cfg) = wecom_cfg.as_ref() {
            push_to_wecom(wecom_cfg, &image, verdict.as_deref(), &report, lang).await?;
        }
    } else if let Some(wecom_cfg) = wecom_cfg.as_ref() {
        // WeCom push without AI: send a concise summary and skip the TUI.
        let content = wecom_summary(&result, lang);
        push_to_wecom(wecom_cfg, &image, verdict.as_deref(), &content, lang).await?;
    }
    // Outside CI the AI report / WeCom push is the whole run. In CI the gate
    // below still applies: sending a report must not switch the checks off.
    let wants_output = !output_file.is_empty() || summary_file.is_some();
    if reported && !is_ci() && !wants_output {
        return Ok(true);
    }
    if is_ci() || wants_output {
        println!("{}", i18n::tr(lang, "cli.result").bold().green());
        println!(
            "{}",
            i18n::fill(
                i18n::tr(lang, "cli.efficiency"),
                &[&summary.score.to_string()]
            )
        );
        println!(
            "{}",
            i18n::fill(
                i18n::tr(lang, "cli.wasted"),
                &[
                    &summary.wasted_size.to_string(),
                    &ByteSize(summary.wasted_size).to_string(),
                ]
            )
        );
        let ignored = config::get_ignore_recommendations();
        if !result.recommendations.is_empty() {
            println!("{}", i18n::tr(lang, "cli.recs").bold().green());
            for r in &result.recommendations {
                let tag = format!(
                    "[{}/{}]",
                    i18n::tr(lang, &format!("sev.{}", r.severity)),
                    i18n::tr(lang, &format!("cat.{}", r.category)),
                );
                let saved = if r.est_saved_bytes > 0 {
                    i18n::fill(
                        i18n::tr(lang, "cli.saved"),
                        &[&ByteSize(r.est_saved_bytes).to_string()],
                    )
                } else {
                    String::new()
                };
                let colored = match r.severity.as_str() {
                    "high" => tag.red(),
                    "medium" => tag.yellow(),
                    "low" => tag.green(),
                    _ => tag.cyan(),
                };
                // The id is what `ignore_recommendations` takes.
                let note = if ignored.contains(&r.id) {
                    format!(" ({}, {})", r.id, i18n::tr(lang, "cli.ignored"))
                } else {
                    format!(" ({})", r.id)
                };
                println!("  {colored} {}{saved}{note}", r.title);
            }
        }

        let fail = i18n::tr(lang, "cli.fail").red().to_string();
        for failure in &failures {
            println!("{}", failure.render(lang, &fail));
        }
        if !output_file.is_empty() {
            let is_markdown = output_file == "-"
                || output_file.ends_with(".md")
                || output_file.ends_with(".markdown");
            let content = if is_markdown {
                markdown::to_markdown(&result, skip_base, lang)
            } else {
                serde_json::to_string(&result.report(&summary)).map_err(|err| err.to_string())?
            };
            if output_file == "-" {
                print!("{}", content);
            } else {
                fs::write(output_file, content).map_err(|err| err.to_string())?;
            }
        }
        if let Some(target) = &summary_file {
            let md = markdown::to_summary_markdown(&result, lang, verdict.as_deref());
            if target == "-" {
                print!("{md}");
            } else {
                fs::write(target, md).map_err(|err| err.to_string())?;
            }
        }
        // CI gate always applies when CI=true — including runs that also
        // write `-o/--output-file`. Previously a non-empty output path
        // swallowed threshold failures (exit 0), which broke pipelines that
        // both archive the report and enforce efficiency checks.
        if is_ci() && !failures.is_empty() {
            return Ok(false);
        }
    } else {
        ui::run_app(result, lang).map_err(|item| item.to_string())?;
    }
    Ok(true)
}

// 命令行模式下清除过期数据
async fn sweep_caches() -> Result<(), String> {
    clear_blob_files().await.map_err(|item| item.to_string())?;
    // Analysis-cache sweep is best-effort — never fail the CLI on cleanup.
    if let Err(err) = clear_analysis_files().await {
        tracing::warn!(err = err.to_string(), "clear analysis cache fail");
    }
    // 容量上限淘汰同样 best-effort（未配置 max_layer_cache_size 时为 no-op）。
    if let Err(err) = enforce_layer_cache_limit().await {
        tracing::warn!(err = err.to_string(), "enforce cache size limit fail");
    }
    Ok(())
}

async fn analyze_for_cli(
    image: &str,
    lang: i18n::Lang,
    verify_dup: bool,
    credentials: Option<image::RegistryCredentials>,
) -> Result<DockerAnalyzeResult, String> {
    let image_info = parse_image_info(image);
    eprintln!("{}", i18n::fill(i18n::tr(lang, "cli.analyzing"), &[image]));
    // A CI gate has to judge the image as it is now, so there an
    // unavailable registry stays an error. Interactive runs fall back to the
    // last cached analysis of the reference, flagged as possibly stale.
    if is_ci() {
        analyze_docker_image(image_info, lang, false, verify_dup, credentials).await
    } else {
        analyze_docker_image_or_last_known(image_info, lang, false, verify_dup, credentials).await
    }
    .map_err(|item| item.to_string())
}

// `diving diff` 的入参集合。
struct DiffOptions {
    base: String,
    target: String,
    output_file: String,
    summary_file: Option<String>,
    verify_dup: bool,
    lang: i18n::Lang,
    credentials: Option<image::RegistryCredentials>,
}

/// The diff's CI check: how much larger the target may be than the base.
fn evaluate_diff_gate(diff: &ImageDiff, highest_size_increase: Option<u64>) -> Vec<GateFailure> {
    match highest_size_increase {
        Some(limit) if diff.size_delta > limit as i64 => vec![GateFailure {
            key: "cli.check.growth",
            args: vec![
                ByteSize(diff.size_delta.unsigned_abs()).to_string(),
                ByteSize(limit).to_string(),
            ],
        }],
        _ => vec![],
    }
}

// 对比两个镜像。返回值含义同 `analyze`。
async fn diff(opts: DiffOptions) -> Result<bool, String> {
    let DiffOptions {
        base,
        target,
        output_file,
        summary_file,
        verify_dup,
        lang,
        credentials,
    } = opts;
    sweep_caches().await?;
    // One after the other: the layers the two images share are downloaded
    // once, by the first analysis, and are cache hits for the second.
    let base = analyze_for_cli(&base, lang, verify_dup, credentials.clone()).await?;
    let target = analyze_for_cli(&target, lang, verify_dup, credentials).await?;
    let diff = diff_images(&base, &target);
    let failures = evaluate_diff_gate(&diff, config::get_highest_size_increase());
    let verdict = is_ci().then(|| gate_verdict_md(&failures, lang));
    let md = to_diff_markdown(&diff, lang, verdict.as_deref());

    if output_file.is_empty() || output_file == "-" {
        print!("{md}");
    } else {
        println!("{}", i18n::tr(lang, "cli.result").bold().green());
        println!(
            "{}",
            i18n::fill(
                i18n::tr(lang, "cli.diff.size"),
                &[
                    &ByteSize(diff.base.total_size).to_string(),
                    &ByteSize(diff.target.total_size).to_string(),
                    &format!(
                        "{}{}",
                        if diff.size_delta < 0 { "-" } else { "+" },
                        ByteSize(diff.size_delta.unsigned_abs())
                    ),
                ]
            )
        );
        let content = if output_file.ends_with(".md") || output_file.ends_with(".markdown") {
            md.clone()
        } else {
            serde_json::to_string(&diff).map_err(|err| err.to_string())?
        };
        fs::write(output_file, content).map_err(|err| err.to_string())?;
    }
    if let Some(target) = &summary_file {
        if target == "-" {
            print!("{md}");
        } else {
            fs::write(target, &md).map_err(|err| err.to_string())?;
        }
    }
    let fail = i18n::tr(lang, "cli.fail").red().to_string();
    for failure in &failures {
        println!("{}", failure.render(lang, &fail));
    }
    Ok(!(is_ci() && !failures.is_empty()))
}

/// Exit for a finished CLI run: `Ok(false)` means the gate failed.
fn exit_on(outcome: Result<bool, String>, lang: i18n::Lang) {
    match outcome {
        Ok(true) => {}
        Ok(false) => {
            error!("{}", i18n::tr(lang, "cli.cifail"));
            std::process::exit(EXIT_GATE_FAILED)
        }
        Err(err) => {
            error!(err, "analyze image fail");
            std::process::exit(EXIT_ERROR)
        }
    }
}

// 将分析结果（AI 报告或精简摘要）推送到企微机器人
async fn push_to_wecom(
    cfg: &wecom::WecomConfig,
    image: &str,
    verdict: Option<&str>,
    content: &str,
    lang: i18n::Lang,
) -> Result<(), String> {
    eprintln!("{}", i18n::tr(lang, "wecom.sending"));
    // The CI verdict sits right under the title: WeCom truncates long
    // messages from the end, and the verdict is the part that must survive.
    let verdict = verdict.map(|v| format!("{v}\n\n")).unwrap_or_default();
    let msg = format!(
        "## {}: {}\n\n{}{}",
        i18n::tr(lang, "wecom.title"),
        image,
        verdict,
        content
    );
    wecom::send_markdown(cfg, &msg)
        .await
        .map_err(|e| i18n::fill(i18n::tr(lang, "wecom.fail"), &[&e]))?;
    println!("{}", i18n::tr(lang, "wecom.sent").bold().green());
    Ok(())
}

// 无 AI 时推送的精简摘要：效率分 / 浪费空间 / 优化建议
fn wecom_summary(result: &image::DockerAnalyzeResult, lang: i18n::Lang) -> String {
    let summary = result.summary();
    let mut s = String::new();
    // A cached result shown because the registry was unavailable says so
    // up front, like the Markdown report does.
    if let Some(as_of) = &result.stale_as_of {
        s.push_str(&format!(
            "> {}\n\n",
            i18n::fill(i18n::tr(lang, "md.stale"), &[as_of])
        ));
    }
    s.push_str(&format!(
        "**{}:** {} %\n**{}:** {} bytes ({})\n",
        i18n::tr(lang, "md.f.eff"),
        summary.score,
        i18n::tr(lang, "md.f.wasted"),
        summary.wasted_size,
        ByteSize(summary.wasted_size),
    ));
    if !result.recommendations.is_empty() {
        s.push_str(&format!("\n### {}\n", i18n::tr(lang, "md.recs")));
        for r in &result.recommendations {
            let tag = format!(
                "[{}/{}]",
                i18n::tr(lang, &format!("sev.{}", r.severity)),
                i18n::tr(lang, &format!("cat.{}", r.category)),
            );
            let saved = if r.est_saved_bytes > 0 {
                i18n::fill(
                    i18n::tr(lang, "cli.saved"),
                    &[&ByteSize(r.est_saved_bytes).to_string()],
                )
            } else {
                String::new()
            };
            s.push_str(&format!("- {} {}{}\n", tag, r.title, saved));
        }
    }
    s
}

pub async fn run(args: Args) {
    // 启动时确保可以读取配置
    config::must_load_config();
    if args.is_terminal_type() {
        let lang = i18n::Lang::resolve(args.lang.as_deref());
        let ai_cfg = ai::AiConfig::resolve(
            args.ai_api_key.as_deref(),
            args.ai_base_url.as_deref(),
            args.ai_model.as_deref(),
            args.ai_system_prompt.as_deref(),
        );
        let wecom_cfg = wecom::WecomConfig::resolve(args.wecom_webhook.as_deref());
        // Base layers are auto-detected and hidden by default; --no-skip-base
        // opts back in.
        let skip_base = !args.no_skip_base;
        // Cross-layer duplicate detection is on by default; --no-verify-dup
        // skips it (and bypasses the analysis cache).
        let verify_dup = !args.no_verify_dup;
        let credentials = image::resolve_explicit(
            args.username.as_deref(),
            args.password.as_deref(),
            args.password_stdin,
        );
        if let Some(Command::Diff { base, target }) = args.command {
            TRACE_ID
                .scope(generate_trace_id(), async {
                    let outcome = diff(DiffOptions {
                        base,
                        target,
                        output_file: args.output_file.unwrap_or_default(),
                        summary_file: args.summary,
                        verify_dup,
                        lang,
                        credentials,
                    })
                    .await;
                    exit_on(outcome, lang);
                })
                .await;
        } else if let Some(value) = args.image {
            TRACE_ID
                .scope(generate_trace_id(), async {
                    let outcome = analyze(AnalyzeOptions {
                        image: value,
                        output_file: args.output_file.unwrap_or_default(),
                        summary_file: args.summary,
                        skip_base,
                        verify_dup,
                        no_ai_history: args.no_ai_history,
                        lang,
                        ai_cfg,
                        wecom_cfg,
                        credentials,
                    })
                    .await;
                    exit_on(outcome, lang);
                })
                .await;
        } else {
            error!("image can not be nil");
            std::process::exit(EXIT_ERROR)
        }
    } else {
        start_cleanup_task();
        let mcp_token = args
            .mcp_token
            .or_else(|| env::var("DIVING_MCP_TOKEN").ok())
            .filter(|t| !t.trim().is_empty());
        let mcp_status = McpStatus {
            enabled: !args.no_mcp,
            token_required: !args.no_mcp && mcp_token.is_some(),
        };
        let mut routes = new_router(mcp_status);
        if mcp_status.enabled {
            warn_if_mcp_unreachable(&args.listen, mcp_status.token_required);
            routes = routes.merge(mcp::new_router(mcp_token));
        }
        // build our application with a route
        let app = Router::new()
            .merge(routes)
            .layer(
                ServiceBuilder::new()
                    .layer(HandleErrorLayer::new(error::handle_error))
                    .timeout(Duration::from_secs(10 * 60)),
            )
            // 后面的layer先执行
            .layer(from_fn(access_log))
            .layer(from_fn(entry));

        info!(
            version = env!("CARGO_PKG_VERSION"),
            listen = args.listen,
            "diving-rs listening"
        );
        let listener = tokio::net::TcpListener::bind(&args.listen)
            .await
            .unwrap_or_else(|e| {
                error!(
                    err = e.to_string(),
                    addr = args.listen,
                    "failed to bind TCP listener"
                );
                std::process::exit(EXIT_ERROR);
            });

        if let Err(err) = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal())
        .await
        {
            error!(err = err.to_string(), "server error");
        }
    }
}

/// `/mcp` rejects non-loopback `Host` headers unless a token or
/// `mcp_allowed_hosts` is configured; say so when the server is exposed
/// beyond loopback, instead of leaving remote clients with a bare 403.
fn warn_if_mcp_unreachable(listen: &str, has_token: bool) {
    let exposed = listen
        .parse::<SocketAddr>()
        .is_ok_and(|addr| !addr.ip().is_loopback());
    if exposed && !has_token && config::get_mcp_allowed_hosts().is_empty() {
        warn!(
            listen,
            "/mcp only accepts loopback Host headers; set --mcp-token or \
             mcp_allowed_hosts in ~/.diving/config.yml for remote MCP clients"
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
        // TODO 后续有需要可在此设置ping的状态
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

    info!("signal received, starting graceful shutdown");
}

#[cfg(test)]
mod tests {
    use super::*;
    use i18n::Lang;

    fn limits() -> GateLimits<'static> {
        GateLimits {
            lowest_efficiency: 95,
            highest_wasted_bytes: 20 * 1024 * 1024,
            highest_user_wasted_percent: 0.1,
            highest_image_size: None,
            fail_on_severity: None,
            ignored: &[],
        }
    }

    fn failed_keys(result: &DockerAnalyzeResult, limits: &GateLimits) -> Vec<&'static str> {
        let summary = result.summary();
        evaluate_gate_with(result, &summary, Lang::En, limits)
            .iter()
            .map(|f| f.key)
            .collect()
    }

    #[test]
    fn diff_gate_fails_only_on_growth_past_the_limit() {
        let grew = |size_delta| ImageDiff {
            size_delta,
            ..Default::default()
        };
        let mib = 1024 * 1024;
        assert!(evaluate_diff_gate(&grew(30 * mib), None).is_empty());
        assert!(evaluate_diff_gate(&grew(-30 * mib), Some(0)).is_empty());
        assert!(evaluate_diff_gate(&grew(20 * mib), Some(20 * mib as u64)).is_empty());
        let failures = evaluate_diff_gate(&grew(30 * mib), Some(20 * mib as u64));
        assert_eq!(failures.len(), 1);
        assert_eq!(
            failures[0].render(Lang::En, "FAIL"),
            "FAIL: image growth check, 30.0 MiB larger uncompressed, highest: 20.0 MiB"
        );
    }

    #[test]
    fn diff_subcommand_takes_the_shared_flags_anywhere() {
        let args =
            Args::try_parse_from(["diving", "diff", "app:1", "app:2", "-o", "d.json"]).unwrap();
        assert!(matches!(
            &args.command,
            Some(Command::Diff { base, target }) if base == "app:1" && target == "app:2"
        ));
        assert_eq!(args.output_file.as_deref(), Some("d.json"));
        let args =
            Args::try_parse_from(["diving", "--lang", "zh", "diff", "a", "b", "-c", "x.yml"])
                .unwrap();
        assert_eq!(args.lang.as_deref(), Some("zh"));
        assert_eq!(args.config_file(), Some("x.yml"));
        // A plain image reference still means "analyze this image".
        let args = Args::try_parse_from(["diving", "redis:alpine", "-o", "-"]).unwrap();
        assert!(args.command.is_none());
        assert_eq!(args.image.as_deref(), Some("redis:alpine"));
    }

    #[test]
    fn image_size_cap_compares_the_uncompressed_size() {
        let result = DockerAnalyzeResult {
            size: 200 * 1024 * 1024,
            total_size: 600 * 1024 * 1024,
            ..Default::default()
        };
        // Off unless configured.
        assert!(failed_keys(&result, &limits()).is_empty());
        let cap = |mib: u64| GateLimits {
            highest_image_size: Some(mib * 1024 * 1024),
            ..limits()
        };
        // 600 MiB uncompressed fails a 500 MiB cap even though only 200 MiB
        // is pulled over the wire.
        assert_eq!(failed_keys(&result, &cap(500)), ["cli.check.size"]);
        assert!(failed_keys(&result, &cap(600)).is_empty());

        let summary = result.summary();
        let failure = &evaluate_gate_with(&result, &summary, Lang::En, &cap(500))[0];
        assert_eq!(
            failure.render(Lang::En, "FAIL"),
            "FAIL: image size check, 600.0 MiB uncompressed, highest: 500.0 MiB"
        );
    }

    #[test]
    fn wecom_summary_flags_a_stale_result_first() {
        let fresh = DockerAnalyzeResult::default();
        assert!(wecom_summary(&fresh, Lang::En).starts_with("**Efficiency"));

        let stale = DockerAnalyzeResult {
            stale_as_of: Some("2026-10-06T04:22:25+00:00".to_string()),
            ..Default::default()
        };
        let summary = wecom_summary(&stale, Lang::En);
        assert!(
            summary.starts_with(
                "> Registry unavailable — this is the analysis cached at 2026-10-06T04:22:25+00:00"
            ),
            "{summary}"
        );
        assert!(summary.contains("**Efficiency"));
    }

    #[test]
    fn gate_verdict_lists_each_failed_check_under_the_verdict() {
        assert_eq!(gate_verdict_md(&[], Lang::En), "**CI gate: passed**");

        let failures = [
            GateFailure {
                key: "cli.check.eff",
                args: vec!["95".to_string()],
            },
            GateFailure {
                key: "cli.check.severity",
                args: vec!["2".to_string(), "high".to_string()],
            },
        ];
        assert_eq!(
            gate_verdict_md(&failures, Lang::En),
            "**CI gate: failed**\n\
             - FAIL: lowest efficiency check, lowest: 95\n\
             - FAIL: recommendation severity check, 2 at high or above"
        );
        assert!(gate_verdict_md(&failures, Lang::Zh).starts_with("**CI 卡口：未通过**"));
    }
}
