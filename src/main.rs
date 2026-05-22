use axum::{error_handling::HandleErrorLayer, middleware::from_fn, Router};
use bytesize::ByteSize;
use clap::Parser;
use colored::*;
use mimalloc::MiMalloc;
use std::fs;
use std::net::SocketAddr;
use std::time::Duration;
use std::{env, str::FromStr};
use tokio::signal;
use tower::ServiceBuilder;
use tracing::Level;
use tracing::{error, info};
use tracing_subscriber::FmtSubscriber;

// mimalloc replaces the system allocator process-wide. On the multi-threaded
// reqwest + tar/zstd decompression workload it typically delivers ~10–20%
// throughput / lower latency vs. glibc malloc, especially under concurrent
// web requests.
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

mod ai;
mod config;
mod controller;
mod dist;
mod error;
mod i18n;
mod image;
mod markdown;
mod middleware;
mod recommend;
mod store;
mod task_local;
mod ui;
mod util;
mod wecom;

use controller::new_router;
use image::{analyze_docker_image, parse_image_info};
use middleware::{access_log, entry};
use store::{clear_analysis_files, clear_blob_files};
use task_local::{generate_trace_id, TRACE_ID};

/// A tool for exploring each layer in a docker image.
/// It can run in terminal or as a web service.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Running mode of diving, terminal or web
    #[arg(short, long, default_value = "terminal")]
    mode: String,
    image: Option<String>,
    /// The listen addr of web mode
    #[arg(short, long, default_value = "127.0.0.1:7001")]
    listen: String,
    /// The result output file
    #[arg(short, long)]
    output_file: Option<String>,
    /// Include base image layers in the analysis (auto-detected and hidden by default)
    #[arg(long)]
    no_skip_base: bool,
    /// Skip cross-layer duplicate file detection (and its analysis cache),
    /// trading the dup card for a faster cold analysis. Useful in CI.
    #[arg(long)]
    no_verify_dup: bool,
    /// Skip reading the previous AI-history snapshot — the AI report does no
    /// regression comparison this run (the current analysis is still recorded
    /// for future runs). Only affects AI mode (`--ai-api-key`).
    #[arg(long)]
    no_ai_history: bool,
    /// Recommendation output language: en or zh (overrides $DIVING_LANG/$LANG)
    #[arg(long)]
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
    /// WeCom (企业微信) group-bot webhook URL or key; pushes the result there (or $WECOM_WEBHOOK)
    #[arg(long)]
    wecom_webhook: Option<String>,
}

impl Args {
    fn is_terminal_type(&self) -> bool {
        self.mode == "terminal"
    }
}

fn init_logger(terminal_mode: bool) {
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
        }
    });
}

fn is_ci() -> bool {
    env::var_os("CI").unwrap_or_default() == "true"
}

// 镜像分析的入参集合（由 CLI args 解析而来）。
struct AnalyzeOptions {
    image: String,
    output_file: String,
    skip_base: bool,
    verify_dup: bool,
    no_ai_history: bool,
    lang: i18n::Lang,
    ai_cfg: Option<ai::AiConfig>,
    wecom_cfg: Option<wecom::WecomConfig>,
}

// 分析镜像（错误直接以字符串返回）
async fn analyze(opts: AnalyzeOptions) -> Result<(), String> {
    let AnalyzeOptions {
        image,
        output_file,
        skip_base,
        verify_dup,
        no_ai_history,
        lang,
        ai_cfg,
        wecom_cfg,
    } = opts;
    // 命令行模式下清除过期数据
    clear_blob_files().await.map_err(|item| item.to_string())?;
    // Analysis-cache sweep is best-effort — never fail the CLI on cleanup.
    if let Err(err) = clear_analysis_files().await {
        tracing::warn!(err = err.to_string(), "clear analysis cache fail");
    }
    let image_info = parse_image_info(&image);
    eprintln!("{}", i18n::fill(i18n::tr(lang, "cli.analyzing"), &[&image]));
    let result = analyze_docker_image(image_info, lang, false, verify_dup)
        .await
        .map_err(|item| item.to_string())?;
    // AI analysis takes precedence: print the model's report and skip the TUI.
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
            push_to_wecom(wecom_cfg, &image, &report, lang).await?;
        }
        return Ok(());
    }
    // WeCom push without AI: send a concise summary and skip the TUI.
    if let Some(wecom_cfg) = wecom_cfg.as_ref() {
        let summary = wecom_summary(&result, lang);
        push_to_wecom(wecom_cfg, &image, &summary, lang).await?;
        return Ok(());
    }
    if is_ci() || !output_file.is_empty() {
        let summary = result.summary();
        let lowest_efficiency = (config::get_lowest_efficiency() * 100.0) as u64;
        let highest_wasted_bytes = config::get_highest_wasted_bytes();
        let highest_user_wasted_percent = config::get_highest_user_wasted_percent();
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
                println!("  {colored} {}{saved}", r.title);
            }
        }

        let mut passed = true;
        let fail = i18n::tr(lang, "cli.fail").red().to_string();
        if summary.score < lowest_efficiency {
            println!(
                "{}",
                i18n::fill(
                    i18n::tr(lang, "cli.check.eff"),
                    &[&fail, &lowest_efficiency.to_string()]
                )
            );
            passed = false;
        }
        if summary.wasted_size > highest_wasted_bytes {
            println!(
                "{}",
                i18n::fill(
                    i18n::tr(lang, "cli.check.bytes"),
                    &[&fail, &ByteSize(highest_wasted_bytes).to_string()]
                )
            );
            passed = false;
        }
        if summary.wasted_percent > highest_user_wasted_percent {
            println!(
                "{}",
                i18n::fill(
                    i18n::tr(lang, "cli.check.pct"),
                    &[&fail, &format!("{highest_user_wasted_percent:.2}")]
                )
            );
            passed = false;
        }
        if !output_file.is_empty() {
            let is_markdown = output_file == "-"
                || output_file.ends_with(".md")
                || output_file.ends_with(".markdown");
            let content = if is_markdown {
                markdown::to_markdown(&result, skip_base, lang)
            } else {
                serde_json::to_string(&result).map_err(|err| err.to_string())?
            };
            if output_file == "-" {
                print!("{}", content);
            } else {
                fs::write(output_file, content).map_err(|err| err.to_string())?;
            }
        } else if !passed {
            return Err(i18n::tr(lang, "cli.cifail").to_string());
        }
    } else {
        ui::run_app(result, lang).map_err(|item| item.to_string())?;
    }
    Ok(())
}

// 将分析结果（AI 报告或精简摘要）推送到企微机器人
async fn push_to_wecom(
    cfg: &wecom::WecomConfig,
    image: &str,
    content: &str,
    lang: i18n::Lang,
) -> Result<(), String> {
    eprintln!("{}", i18n::tr(lang, "wecom.sending"));
    let msg = format!(
        "## {}: {}\n\n{}",
        i18n::tr(lang, "wecom.title"),
        image,
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
    let mut s = format!(
        "**{}:** {} %\n**{}:** {} bytes ({})\n",
        i18n::tr(lang, "md.f.eff"),
        summary.score,
        i18n::tr(lang, "md.f.wasted"),
        summary.wasted_size,
        ByteSize(summary.wasted_size),
    );
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

async fn run(args: Args) {
    // 启动时确保可以读取配置
    config::must_load_config();
    if args.is_terminal_type() {
        let lang = i18n::Lang::resolve(args.lang.as_deref());
        let ai_cfg = ai::AiConfig::resolve(
            args.ai_api_key.as_deref(),
            args.ai_base_url.as_deref(),
            args.ai_model.as_deref(),
        );
        let wecom_cfg = wecom::WecomConfig::resolve(args.wecom_webhook.as_deref());
        // Base layers are auto-detected and hidden by default; --no-skip-base
        // opts back in.
        let skip_base = !args.no_skip_base;
        // Cross-layer duplicate detection is on by default; --no-verify-dup
        // skips it (and bypasses the analysis cache).
        let verify_dup = !args.no_verify_dup;
        if let Some(value) = args.image {
            TRACE_ID
                .scope(generate_trace_id(), async {
                    if let Err(err) = analyze(AnalyzeOptions {
                        image: value,
                        output_file: args.output_file.unwrap_or_default(),
                        skip_base,
                        verify_dup,
                        no_ai_history: args.no_ai_history,
                        lang,
                        ai_cfg,
                        wecom_cfg,
                    })
                    .await
                    {
                        error!(err, "analyze image fail");
                        std::process::exit(1)
                    }
                })
                .await;
        } else {
            error!("image can not be nil")
        }
    } else {
        start_cleanup_task();
        // build our application with a route
        let app = Router::new()
            .merge(new_router())
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
                std::process::exit(1);
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

fn main() {
    // `OffsetTime::local_rfc_3339` (in init_logger) must read the local
    // timezone before Tokio spawns any worker threads — so we build the
    // runtime explicitly here instead of using `#[tokio::main]`. As a
    // bonus, this lets us size the worker pool from `config.threads`,
    // unifying the "how parallel?" knob across tokio and the per-image
    // layer semaphore.
    std::panic::set_hook(Box::new(|e| {
        error!(category = "panic", message = e.to_string(),);
        std::process::exit(1);
    }));
    let args = Args::parse();
    init_logger(args.is_terminal_type());

    let worker_threads = config::must_load_config()
        .threads
        .unwrap_or_else(num_cpus::get)
        .max(1);

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(worker_threads)
        .build()
        .unwrap_or_else(|e| panic!("failed to build tokio runtime: {e}"))
        .block_on(run(args));
}
