use clap::Parser;
use std::{env, str::FromStr};
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

mod config;
mod image;
mod store;
mod ui;

use crate::{image::DockerClient, store::clear_blob_files};

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
            let mut image = value.clone();
            if !image.contains(':') {
                image += ":latest";
            }
            let mut values: Vec<&str> = image.split(&['/', ':']).collect();
            let docker_service = "docker";
            if values.len() == 2 {
                values.reverse();
                values.push("library");
                values.push(docker_service);
                values.reverse();
            } else if values.len() == 3 {
                values.reverse();
                values.push(docker_service);
                values.reverse();
            }
            let c = if values[0] == docker_service {
                DockerClient::new()
            } else {
                DockerClient::new_custom(values[0], values[0], values[0])
            };
            let result = c.analyze(values[1], values[2], values[3]).await.unwrap();
            ui::run_app(result).unwrap();
        }
    }
}
