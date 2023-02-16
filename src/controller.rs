use crate::error::HTTPResult;
use crate::image::{analyze_docker_image, parse_image_info, DockerAnalyzeResult};
use axum::{extract::Query, routing::get, Json, Router};
use serde::Deserialize;

type JSONResult<T> = HTTPResult<Json<T>>;

pub fn new_router() -> Router {
    Router::new()
        .route("/ping", get(ping))
        .route("/analyze", get(analyze))
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
