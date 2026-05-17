use crate::dist::{get_static_file, StaticFile};
use crate::error::{HTTPError, HTTPResult};
use crate::image::{analyze_docker_image, get_file_content_from_layer, parse_image_info};
use crate::markdown;
use crate::store::get_blob_path;
use axum::response::{IntoResponse, Response};
use axum::{extract::Query, routing::get, Json, Router};
use http::header;
use http::Uri;
use lru::LruCache;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::num::NonZeroUsize;
use std::sync::Mutex;

const VERSION: &str = env!("CARGO_PKG_VERSION");
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
    format: Option<String>,
    skip_base: Option<bool>,
}

fn get_latest_image_cache() -> &'static Mutex<LruCache<String, ()>> {
    static LATEST_IMAGE_CACHE: OnceCell<Mutex<LruCache<String, ()>>> = OnceCell::new();
    LATEST_IMAGE_CACHE.get_or_init(|| {
        let c = LruCache::new(NonZeroUsize::new(5).unwrap());
        Mutex::new(c)
    })
}
fn add_to_latest_image_cache(name: &str) {
    if let Ok(mut cache) = get_latest_image_cache().lock() {
        cache.put(name.to_owned(), ());
    }
}

async fn analyze(Query(params): Query<AnalyzeParams>) -> HTTPResult<Response> {
    let image_info = parse_image_info(&params.image);
    let result = analyze_docker_image(image_info).await?;
    add_to_latest_image_cache(&params.image);
    if params.format.as_deref() == Some("markdown") {
        let md = markdown::to_markdown(&result, params.skip_base.unwrap_or(false));
        return Ok(([(header::CONTENT_TYPE, "text/markdown; charset=utf-8")], md).into_response());
    }
    Ok(Json(result).into_response())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LatestImageResp {
    pub images: Vec<String>,
    pub version: String,
}

async fn get_latest_images() -> JSONResult<LatestImageResp> {
    let image_list = if let Ok(cache) = get_latest_image_cache().lock() {
        cache.iter().map(|(name, _)| name.clone()).collect()
    } else {
        vec![]
    };
    Ok(Json(LatestImageResp {
        images: image_list,
        version: VERSION.to_owned(),
    }))
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
    let path = get_blob_path(&params.digest);
    let (name, content) = tokio::task::block_in_place(|| -> HTTPResult<(String, Vec<u8>)> {
        let file = std::fs::File::open(&path)
            .map_err(|e| HTTPError::new_with_category(&e.to_string(), "blob"))?;
        let content = get_file_content_from_layer(
            std::io::BufReader::new(file),
            &params.media_type,
            &params.file,
        )?;
        let raw_name = params.file.split('/').next_back().unwrap_or_default();
        // Strip characters that would break the Content-Disposition header value
        let name = raw_name
            .chars()
            .filter(|c| *c != '"' && *c != '\\' && *c != '\n' && *c != '\r')
            .collect::<String>();
        Ok((name, content))
    })?;
    Ok(DownloadFile { name, content })
}

async fn serve(uri: Uri) -> StaticFile {
    let mut filename = &uri.path()[1..];
    // html无版本号，因此不设置缓存
    if filename.is_empty() {
        filename = "index.html";
    }
    get_static_file(filename)
}
