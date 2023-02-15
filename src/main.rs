use clap::Parser;
use std::{env, str::FromStr};
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

mod config;
mod image;
mod store;
mod ui;

use crate::{
    image::{parse_image_info, DockerClient},
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
            let c = DockerClient::new(&image_info.registry);
            let result = c
                .analyze(&image_info.user, &image_info.name, &image_info.tag)
                .await
                .unwrap();
            ui::run_app(result).unwrap();
        }
    }
}
