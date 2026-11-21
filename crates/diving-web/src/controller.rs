use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::{Query, Request, State};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use axum::{routing::get, Json, Router};
use diving_core::{
    resolve_explicit_credentials, to_diff_markdown, to_markdown, to_summary_markdown,
    AnalyzeOptions, Error as CoreError, ImagePreview, Lang, PreviewOptions,
};
use futures::stream;
use http::{header, StatusCode, Uri};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tower_http::compression::CompressionLayer;

use crate::dist::{get_static_file, StaticFile};
use crate::error::{HTTPError, HTTPResult};
use crate::http_util::{digests_equal, token_fingerprint};
use crate::state::{McpStatus, WebState};

const VERSION: &str = env!("CARGO_PKG_VERSION");
type JSONResult<T> = HTTPResult<Json<T>>;

pub fn new_router(state: Arc<WebState>) -> Router {
    let api_routes = Router::new()
        .route("/api/analyze", get(analyze))
        .route("/api/diff", get(diff))
        .route("/api/manifest", get(manifest))
        .route("/api/file", get(get_file))
        .route("/api/file-preview", get(preview_file))
        .route("/api/latest-images", get(get_latest_images))
        .layer(from_fn_with_state(state.clone(), require_api_token));

    Router::new()
        .route("/ping", get(ping))
        .merge(api_routes)
        .fallback(get(serve))
        // gzip when the client accepts it. Only these routes: `/mcp` is
        // mounted separately and streams SSE, which must not be buffered.
        .layer(CompressionLayer::new())
        .with_state(state)
}

async fn require_api_token(
    State(state): State<Arc<WebState>>,
    req: Request,
    next: Next,
) -> Response {
    let Some(expected) = state.api_token_fingerprint.as_deref() else {
        return next.run(req).await;
    };
    let supplied = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
        .or_else(|| req.headers().get("x-api-key").and_then(|v| v.to_str().ok()))
        .map(str::trim)
        .filter(|s| !s.is_empty());

    match supplied {
        Some(token) if digests_equal(&token_fingerprint(token), expected) => next.run(req).await,
        _ => {
            let mut err = HTTPError::new_with_category("missing or invalid API token", "auth");
            err.status = StatusCode::UNAUTHORIZED.as_u16();
            err.into_response()
        }
    }
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
    /// Recommendation language: `en` or `zh`. Falls back to server env.
    lang: Option<String>,
    /// When `false`, omit `fileTreeList` from the JSON response to shrink
    /// the payload (dashboards / CI). Defaults to `true`.
    include_tree: Option<bool>,
    /// When `true`, skip cross-layer duplicate detection (faster cold path).
    no_verify_dup: Option<bool>,
}

#[derive(Deserialize, Debug)]
struct ManifestParams {
    /// Registry image (with `?arch=` like `/api/analyze`).
    image: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct DiffParams {
    /// The image compared against (with `?arch=` like `image` above).
    base: String,
    /// The image checked.
    target: String,
    /// `markdown` for the Markdown diff instead of JSON.
    format: Option<String>,
    lang: Option<String>,
}

async fn analyze(
    State(state): State<Arc<WebState>>,
    Query(params): Query<AnalyzeParams>,
) -> HTTPResult<Response> {
    let lang = Lang::resolve(params.lang.as_deref());
    let verify_dup = !params.no_verify_dup.unwrap_or(false);
    let include_tree = params.include_tree.unwrap_or(true);

    let opts = AnalyzeOptions::default()
        .lang(lang)
        .verify_dup(verify_dup)
        .fallback_to_last_known(true)
        .credentials(resolve_explicit_credentials(None, None, false));
    let result = state.analyzer.analyze(&params.image, opts).await?;
    state.add_latest_image(&params.image);

    if params.format.as_deref() == Some("summary") {
        let md = to_summary_markdown(result.as_ref(), lang, None);
        return Ok(([(header::CONTENT_TYPE, "text/markdown; charset=utf-8")], md).into_response());
    }
    if params.format.as_deref() == Some("markdown") {
        // Base layers are hidden by default (matches the CLI); pass
        // `skipBase=false` to include them.
        let md = to_markdown(result.as_ref(), params.skip_base.unwrap_or(true), lang);
        return Ok(([(header::CONTENT_TYPE, "text/markdown; charset=utf-8")], md).into_response());
    }

    let summary = result.summary();
    if include_tree {
        return Ok(Json(result.report(&summary)).into_response());
    }
    Ok(Json(result.without_file_trees().report(&summary)).into_response())
}

/// The image's layers from its manifest and config alone, in about a second
/// — what the web shows while a first analysis downloads the layers.
async fn manifest(
    State(state): State<Arc<WebState>>,
    Query(params): Query<ManifestParams>,
) -> JSONResult<ImagePreview> {
    let opts =
        PreviewOptions::default().credentials(resolve_explicit_credentials(None, None, false));
    let preview = state.analyzer.preview(&params.image, opts).await?;
    Ok(Json(preview))
}

/// Compare two images sequentially so shared layers download once.
async fn diff(
    State(state): State<Arc<WebState>>,
    Query(params): Query<DiffParams>,
) -> HTTPResult<Response> {
    let lang = Lang::resolve(params.lang.as_deref());
    let opts = AnalyzeOptions::default()
        .lang(lang)
        .verify_dup(true)
        .fallback_to_last_known(true)
        .credentials(resolve_explicit_credentials(None, None, false));
    let diff = state
        .analyzer
        .diff(&params.base, &params.target, opts)
        .await?;
    state.add_latest_image(&params.base);
    state.add_latest_image(&params.target);

    if params.format.as_deref() == Some("markdown") {
        let md = to_diff_markdown(&diff, lang, None);
        return Ok(([(header::CONTENT_TYPE, "text/markdown; charset=utf-8")], md).into_response());
    }
    Ok(Json(diff).into_response())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LatestImageResp {
    pub images: Vec<String>,
    pub version: String,
    /// Drives the web UI's MCP button / setup dialog.
    pub mcp: McpStatus,
}

async fn get_latest_images(State(state): State<Arc<WebState>>) -> JSONResult<LatestImageResp> {
    Ok(Json(LatestImageResp {
        images: state.latest_images(),
        version: VERSION.to_owned(),
        mcp: state.mcp_status,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GetFileParams {
    digest: String,
    media_type: String,
    file: String,
}

/// Text shown by the web preview; longer files are cut, not refused.
const PREVIEW_TEXT_BYTES: u64 = 256 * 1024;
/// A NUL byte in this prefix marks the file as binary.
const PREVIEW_SNIFF_BYTES: usize = 8 * 1024;
/// How much of a binary file the hex view shows.
const PREVIEW_HEX_BYTES: usize = 512;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FilePreview {
    size: u64,
    binary: bool,
    /// Whether `content` covers less than the whole file.
    truncated: bool,
    /// The text, or a hex dump of the first bytes of a binary file.
    content: String,
}

/// Classic `hexdump -C` layout: offset, 16 bytes in two groups, ASCII.
fn hex_dump(bytes: &[u8]) -> String {
    let mut out = String::new();
    for (row, chunk) in bytes.chunks(16).enumerate() {
        out.push_str(&format!("{:08x} ", row * 16));
        for i in 0..16 {
            if i == 8 {
                out.push(' ');
            }
            match chunk.get(i) {
                Some(b) => out.push_str(&format!(" {b:02x}")),
                None => out.push_str("   "),
            }
        }
        let ascii: String = chunk
            .iter()
            .map(|&b| {
                if b.is_ascii_graphic() || b == b' ' {
                    b as char
                } else {
                    '.'
                }
            })
            .collect();
        out.push_str(&format!("  |{ascii}|\n"));
    }
    out
}

fn build_preview(head: &[u8], size: u64) -> FilePreview {
    let sniff = &head[..head.len().min(PREVIEW_SNIFF_BYTES)];
    if sniff.contains(&0) {
        let shown = &head[..head.len().min(PREVIEW_HEX_BYTES)];
        return FilePreview {
            size,
            binary: true,
            truncated: (shown.len() as u64) < size,
            content: hex_dump(shown),
        };
    }
    FilePreview {
        size,
        binary: false,
        truncated: (head.len() as u64) < size,
        content: String::from_utf8_lossy(head).into_owned(),
    }
}

/// The first part of a file in a cached layer, for the web UI to show
/// inline: text up to [`PREVIEW_TEXT_BYTES`], or a hex dump for a binary.
/// Reads only that much, whatever the file's size.
async fn preview_file(
    State(state): State<Arc<WebState>>,
    Query(params): Query<GetFileParams>,
) -> JSONResult<FilePreview> {
    let analyzer = state.analyzer.clone();
    let digest = params.digest;
    let media_type = params.media_type;
    let file_path = params.file;
    let preview = tokio::task::spawn_blocking(move || -> HTTPResult<FilePreview> {
        let (head, size) =
            analyzer.read_file_head(&digest, &media_type, &file_path, PREVIEW_TEXT_BYTES)?;
        Ok(build_preview(&head, size))
    })
    .await
    .map_err(|e| HTTPError::new_with_category(&e.to_string(), "blob"))??;
    Ok(Json(preview))
}

async fn get_file(
    State(state): State<Arc<WebState>>,
    Query(params): Query<GetFileParams>,
) -> HTTPResult<Response> {
    let analyzer = state.analyzer.clone();
    let max_bytes = state.max_download_file_size;
    let digest = params.digest;
    let media_type = params.media_type;
    let file_path = params.file;
    let raw_name = file_path.split('/').next_back().unwrap_or_default();
    // Strip characters that would break the Content-Disposition header value
    let name: String = raw_name
        .chars()
        .filter(|c| *c != '"' && *c != '\\' && *c != '\n' && *c != '\r')
        .collect();

    // Stream file chunks from a blocking task over a bounded channel so large
    // downloads (up to 100 MiB) never buffer the whole payload in RAM.
    let (tx, mut rx) = mpsc::channel::<Result<Bytes, CoreError>>(8);
    tokio::task::spawn_blocking(move || {
        let res = analyzer.stream_file(&digest, &media_type, &file_path, max_bytes, |chunk| {
            tx.blocking_send(Ok(Bytes::copy_from_slice(chunk)))
                .map_err(|_| CoreError::Other {
                    message: "download client disconnected".to_string(),
                })
        });
        if let Err(err) = res {
            let _ = tx.blocking_send(Err(err));
        }
    });

    // Await the first chunk (or immediate error) before committing 200 OK so
    // invalid digest / not-found / FileTooLarge still return proper 4xx codes.
    let first = match rx.recv().await {
        Some(Ok(chunk)) => Some(chunk),
        Some(Err(err)) => return Err(err.into()),
        None => None,
    };

    let body_stream = stream::unfold((first, rx), |(pending, mut rx)| async move {
        if let Some(chunk) = pending {
            return Some((Ok::<Bytes, std::io::Error>(chunk), (None, rx)));
        }
        match rx.recv().await {
            Some(Ok(chunk)) => Some((Ok(chunk), (None, rx))),
            Some(Err(err)) => Some((Err(std::io::Error::other(err.to_string())), (None, rx))),
            None => None,
        }
    });

    let disposition = format!(r#"attachment; filename="{name}""#);
    Ok((
        [
            (
                header::CONTENT_TYPE,
                mime::APPLICATION_OCTET_STREAM.as_ref(),
            ),
            (header::CONTENT_DISPOSITION, disposition.as_str()),
        ],
        Body::from_stream(body_stream),
    )
        .into_response())
}

async fn serve(uri: Uri) -> StaticFile {
    let mut filename = &uri.path()[1..];
    if filename.is_empty() {
        filename = "index.html";
    }
    get_static_file(filename)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_shows_text_and_hex_dumps_binaries() {
        let text = build_preview(b"user nginx;\nworker_processes auto;\n", 34);
        assert!(!text.binary && !text.truncated);
        assert!(text.content.starts_with("user nginx;"));

        assert!(build_preview(b"abc", 10).truncated);

        let elf = build_preview(b"\x7fELF\x02\x01\x01\0\0\0\0\0\0\0\0\0\x03\0>\0", 20);
        assert!(elf.binary);
        assert_eq!(
            elf.content,
            "00000000  7f 45 4c 46 02 01 01 00  00 00 00 00 00 00 00 00  |.ELF............|\n\
             00000010  03 00 3e 00                                       |..>.|\n"
        );
    }

    #[tokio::test]
    async fn api_token_protects_api_routes_while_keeping_ping_open() {
        use diving_core::{Analyzer, GateConfig};
        use tower::ServiceExt;

        let analyzer = Analyzer::builder().build().unwrap();
        let state = Arc::new(
            WebState::new(
                analyzer,
                reqwest::Client::new(),
                std::path::PathBuf::new(),
                GateConfig::default(),
                1024 * 1024,
                McpStatus::default(),
                vec![],
            )
            .with_api_token(Some("super-secret-token")),
        );
        let app = new_router(state);

        // /ping stays open
        let resp = app
            .clone()
            .oneshot(Request::builder().uri("/ping").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // /api/* without token -> 401
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/latest-images")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        // /api/* with Bearer token -> 200
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/latest-images")
                    .header(header::AUTHORIZATION, "Bearer super-secret-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // /api/* with X-API-Key -> 200
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/latest-images")
                    .header("x-api-key", "super-secret-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
