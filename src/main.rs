use axum::{error_handling::HandleErrorLayer, middleware::from_fn, Router};
use bytesize::ByteSize;
use clap::Parser;
use colored::*;
use std::fs;
use std::net::SocketAddr;
use std::time::Duration;
use std::{env, str::FromStr};
use tokio::signal;
use tower::ServiceBuilder;
use tracing::Level;
use tracing::{error, info};
use tracing_subscriber::FmtSubscriber;

mod config;
mod controller;
mod dist;
mod error;
mod image;
mod markdown;
mod middleware;
mod recommend;
mod store;
mod task_local;
mod ui;
mod util;

use controller::new_router;
use image::{analyze_docker_image, parse_image_info};
use middleware::{access_log, entry};
use store::clear_blob_files;
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
    /// Auto-detect and hide base image layers in Markdown output
    #[arg(long, default_value_t = false)]
    skip_base: bool,
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
        }
    });
}

fn is_ci() -> bool {
    env::var_os("CI").unwrap_or_default() == "true"
}

// 分析镜像（错误直接以字符串返回）
async fn analyze(image: String, output_file: String, skip_base: bool) -> Result<(), String> {
    // 命令行模式下清除过期数据
    clear_blob_files().await.map_err(|item| item.to_string())?;
    let image_info = parse_image_info(&image);
    eprintln!("Analyzing {}...", image);
    let result = analyze_docker_image(image_info)
        .await
        .map_err(|item| item.to_string())?;
    if is_ci() || !output_file.is_empty() {
        let summary = result.summary();
        let lowest_efficiency = (config::get_lowest_efficiency() * 100.0) as u64;
        let highest_wasted_bytes = config::get_highest_wasted_bytes();
        let highest_user_wasted_percent = config::get_highest_user_wasted_percent();
        println!("{}", "Analyze result:".bold().green());
        println!("  efficiency: {} %", summary.score);
        println!(
            "  wasted bytes: {} bytes ({})",
            summary.wasted_size,
            ByteSize(summary.wasted_size)
        );
        if !result.recommendations.is_empty() {
            println!("{}", "Optimization recommendations:".bold().green());
            for r in &result.recommendations {
                let tag = format!("[{}/{}]", r.severity, r.category);
                let saved = if r.est_saved_bytes > 0 {
                    format!(" (~{} saved)", ByteSize(r.est_saved_bytes))
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
        if summary.score < lowest_efficiency {
            println!(
                "{}: lowest efficiency check, lowest: {}",
                "FAIL".red(),
                lowest_efficiency
            );
            passed = false;
        }
        if summary.wasted_size > highest_wasted_bytes {
            println!(
                "{}: highest wasted bytes check, highest: {}",
                "FAIL".red(),
                ByteSize(highest_wasted_bytes)
            );
            passed = false;
        }
        if summary.wasted_percent > highest_user_wasted_percent {
            println!(
                "{}: highest user wasted percent check, highest: {:.2}",
                "FAIL".red(),
                highest_user_wasted_percent
            );
            passed = false;
        }
        if !output_file.is_empty() {
            let is_markdown = output_file == "-"
                || output_file.ends_with(".md")
                || output_file.ends_with(".markdown");
            let content = if is_markdown {
                markdown::to_markdown(&result, skip_base)
            } else {
                serde_json::to_string(&result).map_err(|err| err.to_string())?
            };
            if output_file == "-" {
                print!("{}", content);
            } else {
                fs::write(output_file, content).map_err(|err| err.to_string())?;
            }
        } else if !passed {
            return Err("CI check fail".to_string());
        }
    } else {
        ui::run_app(result).map_err(|item| item.to_string())?;
    }
    Ok(())
}

#[tokio::main]
async fn run(args: Args) {
    // 启动时确保可以读取配置
    config::must_load_config();
    if args.is_terminal_type() {
        if let Some(value) = args.image {
            TRACE_ID
                .scope(generate_trace_id(), async {
                    if let Err(err) =
                        analyze(value, args.output_file.unwrap_or_default(), args.skip_base).await
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
    // Because we need to get the local offset before Tokio spawns any threads, our `main`
    // function cannot use `tokio::main`.
    std::panic::set_hook(Box::new(|e| {
        error!(category = "panic", message = e.to_string(),);
        std::process::exit(1);
    }));
    let args = Args::parse();
    init_logger(args.is_terminal_type());
    run(args);
}
