use crate::config::{
    get_analysis_memory_ttl, get_max_concurrent_analyses, get_max_download_file_size,
    get_registry_allowlist,
};
use crate::dist::{get_static_file, StaticFile};
use crate::error::{HTTPError, HTTPResult};
use crate::i18n;
use crate::image::{
    analyze_docker_image_or_last_known, get_file_content_from_layer, parse_image_info,
    registry_host, resolve_explicit, DockerAnalyzeResult, ImageInfo, REGISTRY_LOCAL_DOCKER,
    REGISTRY_LOCAL_FILE,
};
use crate::markdown;
use crate::mcp::McpStatus;
use crate::recommend::build_recommendations;
use crate::store::{get_blob_path, is_safe_blob_id, RecentCache};
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use axum::{routing::get, Json, Router};
use futures::future::{FutureExt, Shared};
use futures::Future;
use http::header;
use http::Uri;
use lru::LruCache;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{Mutex, Semaphore};
use tower_http::compression::CompressionLayer;

const VERSION: &str = env!("CARGO_PKG_VERSION");
type JSONResult<T> = HTTPResult<Json<T>>;

/// Shared in-flight analysis future. Output must be `Clone` for
/// `futures::Shared` — wrap the result in `Arc`. The `Lang` records which
/// language the flight's recommendations were built in, so awaiters with a
/// different language know to rebuild them.
type AnalysisFuture = Shared<
    Pin<Box<dyn Future<Output = Result<(Arc<DockerAnalyzeResult>, i18n::Lang), String>> + Send>>,
>;

fn in_flight() -> &'static Mutex<HashMap<String, AnalysisFuture>> {
    static IN_FLIGHT: OnceCell<Mutex<HashMap<String, AnalysisFuture>>> = OnceCell::new();
    IN_FLIGHT.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Slots for analyses running at once (`max_concurrent_analyses`), or
/// `None` when unlimited. Each analysis already fans out across its layers,
/// so without a cap a burst of requests for *different* images multiplies
/// downloads, decompression and memory.
fn analysis_slots() -> Option<&'static Semaphore> {
    static SLOTS: OnceCell<Option<Semaphore>> = OnceCell::new();
    SLOTS
        .get_or_init(|| get_max_concurrent_analyses().map(Semaphore::new))
        .as_ref()
}

/// Run `task` holding one of `slots`, waiting for a free one first.
async fn in_slot<T>(slots: Option<&Semaphore>, task: impl Future<Output = T>) -> T {
    // The semaphore is never closed, so `acquire` cannot fail.
    let _slot = match slots {
        Some(slots) => slots.acquire().await.ok(),
        None => None,
    };
    task.await
}

/// Finished analyses kept in memory at once. A result can run to tens of
/// MB, so this stays small.
const RECENT_ANALYSES_CAPACITY: usize = 4;

type RecentAnalyses = RecentCache<(Arc<DockerAnalyzeResult>, i18n::Lang)>;

/// Recently finished registry analyses, or `None` when
/// `analysis_memory_ttl` is `0s`. Without it every request for an
/// already-analyzed image still costs a registry round trip (the manifest
/// HEAD that validates the on-disk cache) plus a re-parse of that cache —
/// paid on each tool call of an MCP session.
fn recent_analyses() -> Option<&'static std::sync::Mutex<RecentAnalyses>> {
    static RECENT: OnceCell<Option<std::sync::Mutex<RecentAnalyses>>> = OnceCell::new();
    RECENT
        .get_or_init(|| {
            let ttl = get_analysis_memory_ttl();
            let capacity = NonZeroUsize::new(RECENT_ANALYSES_CAPACITY)?;
            (!ttl.is_zero()).then(|| std::sync::Mutex::new(RecentCache::new(capacity, ttl)))
        })
        .as_ref()
}

/// Remember a finished analysis, and schedule the sweep that frees it once
/// the TTL is up even if no further request arrives.
fn remember_analysis(key: String, result: Arc<DockerAnalyzeResult>, lang: i18n::Lang) {
    let Some(recent) = recent_analyses() else {
        return;
    };
    if let Ok(mut cache) = recent.lock() {
        cache.put(key, (result, lang), Instant::now());
    }
    tokio::spawn(async move {
        tokio::time::sleep(get_analysis_memory_ttl()).await;
        if let Ok(mut cache) = recent.lock() {
            cache.purge_expired(Instant::now());
        }
    });
}

/// `result` with recommendations in `lang`. They are the only
/// language-dependent part, so a mismatch costs one clone + rule pass
/// instead of a second analysis.
fn in_lang(
    result: Arc<DockerAnalyzeResult>,
    result_lang: i18n::Lang,
    lang: i18n::Lang,
) -> Arc<DockerAnalyzeResult> {
    if result_lang == lang {
        return result;
    }
    let mut owned = result.as_ref().clone();
    owned.recommendations = build_recommendations(&owned, lang);
    Arc::new(owned)
}

pub fn new_router(mcp: McpStatus) -> Router {
    Router::new()
        .route("/ping", get(ping))
        .route("/api/analyze", get(analyze))
        .route("/api/file", get(get_file))
        .route("/api/latest-images", get(get_latest_images))
        .fallback(get(serve))
        // gzip when the client accepts it. Only these routes: `/mcp` is
        // mounted separately and streams SSE, which must not be buffered.
        .layer(CompressionLayer::new())
        .with_state(mcp)
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

fn get_latest_image_cache() -> &'static std::sync::Mutex<LruCache<String, ()>> {
    static LATEST_IMAGE_CACHE: OnceCell<std::sync::Mutex<LruCache<String, ()>>> = OnceCell::new();
    LATEST_IMAGE_CACHE.get_or_init(|| {
        let c = LruCache::new(NonZeroUsize::new(5).unwrap());
        std::sync::Mutex::new(c)
    })
}
pub(crate) fn add_to_latest_image_cache(name: &str) {
    if let Ok(mut cache) = get_latest_image_cache().lock() {
        cache.put(name.to_owned(), ());
    }
}

/// Deduplicate concurrent analyses of the same image/arch/dup settings.
/// Waiters share one download + decompress pipeline via `futures::Shared`.
///
/// Language is deliberately NOT part of the key: it only affects the derived
/// recommendations. A waiter whose language differs from the flight's rebuilds
/// them below — one clone + rule pass instead of a duplicate full analysis.
pub(crate) async fn analyze_singleflight(
    image: String,
    lang: i18n::Lang,
    verify_dup: bool,
) -> Result<Arc<DockerAnalyzeResult>, HTTPError> {
    let image_info = parse_image_info(&image);
    // Key on identity that affects the analysis itself. Credentials come
    // from env/docker-config of the process and are not part of the query.
    let key = format!(
        "{}|{}|{}|{}|{}|{}",
        image_info.registry,
        image_info.user,
        image_info.name,
        image_info.tag,
        image_info.arch,
        verify_dup
    );

    // Local sources are never remembered: a tar or a local docker image can
    // be rebuilt under the same name, and nothing cheap tells us it was.
    let is_local = matches!(
        image_info.registry.as_str(),
        REGISTRY_LOCAL_FILE | REGISTRY_LOCAL_DOCKER
    );
    let recent = recent_analyses().filter(|_| !is_local);
    if let Some(recent) = recent {
        let hit = recent
            .lock()
            .ok()
            .and_then(|mut cache| cache.get(&key, Instant::now()));
        if let Some((result, result_lang)) = hit {
            return Ok(in_lang(result, result_lang, lang));
        }
    }

    // Web mode: credentials from env only (never query params — avoid logs).
    let credentials = resolve_explicit(None, None, false);

    let fut = {
        let mut map = in_flight().lock().await;
        if let Some(existing) = map.get(&key) {
            existing.clone()
        } else {
            let image_info = parse_image_info(&image);
            let key_for_cleanup = key.clone();
            let shared = async move {
                // The slot is taken inside the flight, so every request
                // waiting on this image shares the one slot.
                let analysis = analyze_docker_image_or_last_known(
                    image_info,
                    lang,
                    true,
                    verify_dup,
                    credentials,
                );
                let result = in_slot(analysis_slots(), analysis)
                    .await
                    .map(|r| (Arc::new(r), lang))
                    .map_err(|e| e.to_string());
                if let (Ok((analysis, _)), false) = (&result, is_local) {
                    remember_analysis(key_for_cleanup.clone(), analysis.clone(), lang);
                }
                // Drop from the map once done so the next request can refresh.
                in_flight().lock().await.remove(&key_for_cleanup);
                result
            }
            .boxed()
            .shared();
            map.insert(key, shared.clone());
            shared
        }
    };

    let (result, flight_lang) = fut
        .await
        .map_err(|e| HTTPError::new_with_category(&e, "docker"))?;
    // 语言不同的并发请求：共享同一次分析，仅重建语言相关的 recommendations。
    Ok(in_lang(result, flight_lang, lang))
}

/// registry 白名单校验：允许列表为空时不限制；否则仅放行列表内的
/// registry host，`file://` / `docker://` 需显式加入 `local-file` /
/// `local-docker`（对公网部署它们等价于任意本地文件读取 / 命令执行面）。
pub(crate) fn ensure_registry_allowed(image_info: &ImageInfo) -> HTTPResult<()> {
    let allowlist = get_registry_allowlist();
    if allowlist.is_empty() {
        return Ok(());
    }
    let id = match image_info.registry.as_str() {
        REGISTRY_LOCAL_FILE | REGISTRY_LOCAL_DOCKER => image_info.registry.clone(),
        registry => registry_host(registry).unwrap_or_default(),
    }
    .to_lowercase();
    if allowlist.iter().any(|allowed| allowed == &id) {
        return Ok(());
    }
    Err(HTTPError::new_with_category_status(
        &format!("registry {id} is not allowed"),
        "forbidden",
        403,
    ))
}

async fn analyze(Query(params): Query<AnalyzeParams>) -> HTTPResult<Response> {
    let lang = i18n::Lang::resolve(params.lang.as_deref());
    let verify_dup = !params.no_verify_dup.unwrap_or(false);
    let include_tree = params.include_tree.unwrap_or(true);

    ensure_registry_allowed(&parse_image_info(&params.image))?;
    let result = analyze_singleflight(params.image.clone(), lang, verify_dup).await?;
    add_to_latest_image_cache(&params.image);

    if params.format.as_deref() == Some("markdown") {
        // Base layers are hidden by default (matches the CLI); pass
        // `skipBase=false` to include them.
        let md = markdown::to_markdown(result.as_ref(), params.skip_base.unwrap_or(true), lang);
        return Ok(([(header::CONTENT_TYPE, "text/markdown; charset=utf-8")], md).into_response());
    }

    let summary = result.summary();
    if include_tree {
        // Serialize straight from the shared Arc — the file trees can run to
        // tens of MB and a `.clone()` here doubled peak memory per request.
        return Ok(Json(result.report(&summary)).into_response());
    }
    // Slim response: drop the hierarchical file trees (often the bulk of
    // the payload) while keeping layers, recommendations, sensitive files,
    // duplicates, etc. Built field-by-field so the trees are never cloned;
    // adding a field to `DockerAnalyzeResult` fails compilation here, which
    // is the reminder to decide whether the slim payload should carry it.
    let src = result.as_ref();
    let slim = DockerAnalyzeResult {
        name: src.name.clone(),
        arch: src.arch.clone(),
        os: src.os.clone(),
        user: src.user.clone(),
        envs: src.envs.clone(),
        labels: src.labels.clone(),
        dockerfile: src.dockerfile.clone(),
        base_os: src.base_os.clone(),
        supported_archs: src.supported_archs.clone(),
        layers: src.layers.clone(),
        size: src.size,
        total_size: src.total_size,
        file_tree_list: vec![],
        file_summary_list: src.file_summary_list.clone(),
        big_modified_file_list: src.big_modified_file_list.clone(),
        sensitive_files: src.sensitive_files.clone(),
        tags: src.tags.clone(),
        recommendations: src.recommendations.clone(),
        duplicate_groups: src.duplicate_groups.clone(),
        runtime_compat: src.runtime_compat.clone(),
        stale_as_of: src.stale_as_of.clone(),
    };
    Ok(Json(slim.report(&summary)).into_response())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LatestImageResp {
    pub images: Vec<String>,
    pub version: String,
    /// Drives the web UI's MCP button / setup dialog.
    pub mcp: McpStatus,
}

/// Most-recently analyzed image names, newest first.
pub(crate) fn latest_images() -> Vec<String> {
    if let Ok(cache) = get_latest_image_cache().lock() {
        cache.iter().map(|(name, _)| name.clone()).collect()
    } else {
        vec![]
    }
}

async fn get_latest_images(State(mcp): State<McpStatus>) -> JSONResult<LatestImageResp> {
    Ok(Json(LatestImageResp {
        images: latest_images(),
        version: VERSION.to_owned(),
        mcp,
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
    // Reject path-traversal digests before touching the filesystem.
    if !is_safe_blob_id(&params.digest) {
        return Err(HTTPError::new_with_category("invalid layer digest", "blob"));
    }
    let path = get_blob_path(&params.digest);
    let media_type = params.media_type;
    let file_path = params.file;
    // 大小上限（默认 100MB，可配置）：整个文件会读进内存再响应，
    // 超限直接 TooLarge 拒绝，避免单个请求打爆服务内存。
    let max_bytes = get_max_download_file_size();
    // Decompression runs on the blocking pool so request-serving async
    // workers stay free even while extracting from a large layer.
    let (name, content) = tokio::task::spawn_blocking(move || -> HTTPResult<(String, Vec<u8>)> {
        let file =
            File::open(&path).map_err(|e| HTTPError::new_with_category(&e.to_string(), "blob"))?;
        let content =
            get_file_content_from_layer(BufReader::new(file), &media_type, &file_path, max_bytes)?;
        let raw_name = file_path.split('/').next_back().unwrap_or_default();
        // Strip characters that would break the Content-Disposition header value
        let name = raw_name
            .chars()
            .filter(|c| *c != '"' && *c != '\\' && *c != '\n' && *c != '\r')
            .collect::<String>();
        Ok((name, content))
    })
    .await
    .map_err(|e| HTTPError::new_with_category(&e.to_string(), "blob"))??;
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// Run eight overlapping tasks through `in_slot` and report the most
    /// that were ever inside at the same moment.
    async fn peak_concurrency(slots: Option<Semaphore>) -> usize {
        let slots = Arc::new(slots);
        let running = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let (slots, running, peak) = (slots.clone(), running.clone(), peak.clone());
                tokio::spawn(async move {
                    in_slot(slots.as_ref().as_ref(), async {
                        let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(now, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(20)).await;
                        running.fetch_sub(1, Ordering::SeqCst);
                    })
                    .await
                })
            })
            .collect();
        for task in tasks {
            task.await.unwrap();
        }
        peak.load(Ordering::SeqCst)
    }

    #[tokio::test]
    async fn in_slot_caps_how_many_tasks_run_at_once() {
        assert_eq!(peak_concurrency(Some(Semaphore::new(1))).await, 1);
        assert_eq!(peak_concurrency(Some(Semaphore::new(3))).await, 3);
        // No semaphore, no limit: all eight overlap.
        assert_eq!(peak_concurrency(None).await, 8);
    }
}
