use axum::{error_handling::HandleErrorLayer, middleware::from_fn, Router};
use axum_client_ip::SecureClientIpSource;
use clap::Parser;
use std::net::SocketAddr;
use std::time::Duration;
use std::{env, str::FromStr};
use tokio::signal;
use tower::ServiceBuilder;
use tracing::info;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

mod config;
mod controller;
mod error;
mod image;
mod middleware;
mod store;
mod ui;

use crate::{
    controller::new_router,
    image::{analyze_docker_image, parse_image_info},
    middleware::access_log,
    store::clear_blob_files,
};

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Running mode of diving
    #[arg(short, long, default_value = "terminal")]
    mode: String,
    image: Option<String>,
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
    let subscriber = FmtSubscriber::builder().with_max_level(level).finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
}

#[tokio::main]
async fn main() {
    init_logger();

    let args = Args::parse();
    if args.is_terminal_type() {
        // 命令行模式下清除过期数据
        clear_blob_files().await.unwrap();
        if args.image.is_none() {
            panic!("image cat not be nil")
        }
        if let Some(value) = args.image {
            let image_info = parse_image_info(&value);
            let result = analyze_docker_image(image_info).await.unwrap();
            ui::run_app(result).unwrap();
        }
    } else {
        // build our application with a route
        let app = Router::new()
            .merge(new_router())
            .layer(
                ServiceBuilder::new()
                    .layer(HandleErrorLayer::new(error::handle_error))
                    .timeout(Duration::from_secs(10 * 60)),
            )
            // TODO 添加compression
            // 后面的layer先执行
            .layer(from_fn(access_log))
            .layer(SecureClientIpSource::ConnectInfo.into_extension());
        let addr = "127.0.0.1:7000".parse().unwrap();
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
