use std::{env, str::FromStr};
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

mod config;
mod image;
mod store;
mod ui;

use crate::{
    config::{get_config_path, get_layer_path, load_config},
    image::{get_file_content_from_layer, get_files_from_layer, DockerClient},
    store::clear_blob_files,
};

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
    let c = DockerClient::new();
    let result = c.analyze("vicanso", "static", "latest").await.unwrap();
    // let result = c.analyze("vicanso", "image-optim", "latest").await.unwrap();
    ui::run_app(result).unwrap();
    // println!("{:?}", load_config());
    // load_config();
    // // 初始化layer path
    // get_layer_path();
    // clear_blob_files().await;

    // let c = DockerClient::new();
    // c.analyze("vicanso", "image-optim", "latest").await;
    // println!(
    //     "{:?}",
    //     c.get_manifest("vicanso", "image-optim", "latest",).await
    // );
    // println!("{:?}", c.list_manifest("vicanso", "image-optim", "latest",).await);
    // println!(
    //     "{:?}",
    //     c.get_image_config("vicanso", "image-optim", "latest",)
    //         .await
    // );

    // println!("{}", chrono::Utc::now().to_string());
    // let result = c.analyze("vicanso", "image-optim", "latest").await;
    // println!("{}", chrono::Utc::now().to_string());
    // std::fs::write(
    //     "./test.json",
    //     serde_json::to_string(&result.unwrap()).unwrap(),
    // );

    // println!("{:?}", result);

    // let data = c
    //     .get_blob(
    //         "vicanso",
    //         "image-optim",
    //         "sha256:e12df60d443dea60d04b5e90525a60cd6a18ce08b34335569b399c9d7e9d87b8",
    //     )
    //     .await
    //     .unwrap();
    //     println!("{:?}", std::string::String::from_utf8_lossy(&data));

    // get_files_from_layer(&data, true).await;
    // let data = get_file_content_from_layer(&data, true, "usr/share/zoneinfo/right/WET")
    //     .await
    //     .unwrap();
    // println!("{:?}", std::string::String::from_utf8_lossy(&data));
}
