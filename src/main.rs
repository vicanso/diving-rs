use axum::{error_handling::HandleErrorLayer, middleware::from_fn, Router};
use clap::Parser;
use human_panic::setup_panic;
use std::net::SocketAddr;
use std::time::Duration;
use std::{env, str::FromStr};
use tokio::signal;
use tokio_cron_scheduler::{Job, JobScheduler};
use tower::ServiceBuilder;
use tracing::Level;
use tracing::{error, info};
use tracing_subscriber::FmtSubscriber;

mod config;
mod controller;
mod dist;
mod error;
mod image;
mod middleware;
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
}

impl Args {
    fn is_terminal_type(&self) -> bool {
        self.mode == "terminal"
    }
}

fn init_logger() {
    let mut level = Level::INFO;
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

async fn start_scheduler() {
    let scheduler = JobScheduler::new().await.unwrap();
    scheduler
        .add(
            // TODO 后续调整为可配置
            Job::new_async("@hourly", |_, _| {
                Box::pin(async {
                    let result = clear_blob_files().await;
                    if let Err(err) = result {
                        error!(err = err.to_string(), "clear blob files fail")
                    } else {
                        info!("clear blob files success")
                    }
                })
            })
            .unwrap(),
        )
        .await
        .unwrap();
    scheduler.start().await.unwrap();
}

// 分析镜像（错误直接以字符串返回）
async fn analyze(image: String) -> Result<(), String> {
    // 命令行模式下清除过期数据
    clear_blob_files().await.map_err(|item| item.to_string())?;
    let image_info = parse_image_info(&image);
    let result = analyze_docker_image(image_info)
        .await
        .map_err(|item| item.to_string())?;
    ui::run_app(result).map_err(|item| item.to_string())?;
    Ok(())
}

#[tokio::main]
async fn run() {
    // 启动时确保可以读取配置
    config::must_load_config();
    let args = Args::parse();
    if args.is_terminal_type() {
        if let Some(value) = args.image {
            TRACE_ID
                .scope(generate_trace_id(), async {
                    if let Err(err) = analyze(value).await {
                        error!(err, "analyze image fail")
                    }
                })
                .await;
        } else {
            error!("image can not be nil")
        }
    } else {
        start_scheduler().await;
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
        let addr = args.listen.parse().unwrap();
        info!("listening on http://{addr}");
        axum::Server::bind(&addr)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(shutdown_signal())
            .await
            .unwrap();
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
    setup_panic!();
    init_logger();
    run();
}
