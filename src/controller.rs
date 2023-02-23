use crate::dist::{get_asset, ServeFile};
use crate::error::HTTPResult;
use crate::image::{analyze_docker_image, parse_image_info, DockerAnalyzeResult};
use axum::{extract::Query, routing::get, Json, Router};
use http::Uri;
use serde::Deserialize;

type JSONResult<T> = HTTPResult<Json<T>>;

pub fn new_router() -> Router {
    Router::new()
        .route("/ping", get(ping))
        .route("/api/analyze", get(analyze))
        .fallback(get(serve))
}

async fn ping() -> &'static str {
    "pong"
}

#[derive(Debug, Deserialize)]
struct AnalyzeParams {
    image: String,
}

async fn analyze(Query(params): Query<AnalyzeParams>) -> JSONResult<DockerAnalyzeResult> {
    let image_info = parse_image_info(&params.image);
    let result = analyze_docker_image(image_info).await?;
    Ok(Json(result))
}

async fn serve(uri: Uri) -> ServeFile {
    let mut filename = &uri.path()[1..];
    let mut max_age = Some(7 * 24 * 3600);
    // html无版本号，因此不设置缓存
    if filename.is_empty() {
        filename = "index.html";
        max_age = None;
    }
    let mut file = get_asset(filename);
    file.max_age = max_age;

    file
}
