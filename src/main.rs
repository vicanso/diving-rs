mod image;

use crate::image::DockerClient;

#[tokio::main]
async fn main() {
    let c = DockerClient::new();
    println!(
        "{:?}",
        c.get_blob(
            "vicanso",
            "image-optim",
            "sha256:171e88ffc77b7704ab4881f95bcdd6148c2361ee21f8653ee9ee60737b84a206",
        )
        .await
    );
}
