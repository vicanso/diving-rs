use crate::dist::{get_static_file, StaticFile};
use crate::error::HTTPResult;
use crate::image::{
    analyze_docker_image, get_file_content_from_layer, parse_image_info, DockerAnalyzeResult,
};
use crate::store::get_blob_from_file;
use axum::response::{IntoResponse, Response};
use axum::{extract::Query, routing::get, Json, Router};
use http::header;
use http::Uri;
use lru::LruCache;
use once_cell::sync::OnceCell;
use serde::Deserialize;
use std::num::NonZeroUsize;
use std::sync::Mutex;

type JSONResult<T> = HTTPResult<Json<T>>;

pub fn new_router() -> Router {
    Router::new()
        .route("/ping", get(ping))
        .route("/api/analyze", get(analyze))
        .route("/api/file", get(get_file))
        .route("/api/latest-images", get(get_latest_images))
        .fallback(get(serve))
}

async fn ping() -> &'static str {
    "pong"
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnalyzeParams {
    image: String,
}

fn get_latest_image_cache() -> &'static Mutex<LruCache<String, String>> {
    static LATEST_IMAGE_CACHE: OnceCell<Mutex<LruCache<String, String>>> = OnceCell::new();
    LATEST_IMAGE_CACHE.get_or_init(|| {
        let c = LruCache::new(NonZeroUsize::new(10).unwrap());
        Mutex::new(c)
    })
}
fn add_to_latest_image_cache(name: &String) {
    if let Ok(mut cache) = get_latest_image_cache().lock() {
        cache.put(name.to_string(), "".to_string());
    }
}

async fn analyze(Query(params): Query<AnalyzeParams>) -> JSONResult<DockerAnalyzeResult> {
    let image_info = parse_image_info(&params.image);
    let name = image_info.name.clone();
    let result = analyze_docker_image(image_info).await?;
    add_to_latest_image_cache(&name);
    Ok(Json(result))
}

async fn get_latest_images() -> JSONResult<Vec<String>> {
    let mut image_list = vec![];
    if let Ok(cache) = get_latest_image_cache().lock() {
        for (name, _) in cache.iter() {
            image_list.push(name.clone());
        }
    }
    Ok(Json(image_list))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetFileParams {
    digest: String,
    media_type: String,
    file: String,
}

struct DownloadFile {
    name: String,
    content: Vec<u8>,
}
impl IntoResponse for DownloadFile {
    fn into_response(self) -> Response {
        let disposition = format!("attachment; filename=\"{}\"", self.name);
        (
            [
                (
                    header::CONTENT_TYPE,
                    mime::APPLICATION_OCTET_STREAM.as_ref(),
                ),
                (header::CONTENT_DISPOSITION, disposition.as_str()),
            ],
            self.content,
        )
            .into_response()
    }
}

async fn get_file(Query(params): Query<GetFileParams>) -> HTTPResult<DownloadFile> {
    let buf = get_blob_from_file(&params.digest).await?;
    let content = get_file_content_from_layer(&buf, &params.media_type, &params.file).await?;
    let name = params.file.split('/').last().unwrap_or_default();
    Ok(DownloadFile {
        name: name.to_string(),
        content,
    })
}

async fn serve(uri: Uri) -> StaticFile {
    let mut filename = &uri.path()[1..];
    // html无版本号，因此不设置缓存
    if filename.is_empty() {
        filename = "index.html";
    }
    get_static_file(filename)
}
