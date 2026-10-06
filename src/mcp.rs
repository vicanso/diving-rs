//! MCP (Model Context Protocol) endpoint for web mode.
//!
//! Mounted at `/mcp` on the same axum server as `/api/*`, over the
//! Streamable HTTP transport in stateless mode (no server-side sessions, so
//! nothing to leak or drain on shutdown). The tools are thin wrappers over
//! the web handlers: they share the singleflight + on-disk analysis cache
//! and the `registry_allowlist`, so an MCP call and a browser request for
//! the same image cost one analysis.
//!
//! Layer numbers are 1-based everywhere in this module — the same numbering
//! the Markdown report (`Layer 3`) and recommendation samples (`L3 path`)
//! use — so a model can feed what it read straight back into a tool.

use crate::config::{get_max_download_file_size, get_mcp_allowed_hosts};
use crate::controller::{
    add_to_latest_image_cache, analyze_singleflight, ensure_registry_allowed, latest_images,
};
use crate::i18n::Lang;
use crate::image::{
    get_file_content_from_layer, parse_image_info, DockerAnalyzeResult, FileTreeItem, Op,
    REGISTRY_LOCAL_DOCKER, REGISTRY_LOCAL_FILE,
};
use crate::markdown::to_markdown;
use crate::store::{get_blob_path, is_safe_blob_id};
use axum::extract::{Request, State};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use bytesize::ByteSize;
use http::{header, StatusCode};
use ring::digest::{digest, SHA256};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, ProgressNotificationParam, ServerCapabilities, ServerConfig};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{schemars, tool, tool_handler, tool_router, RoleServer, ServerHandler};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::File;
use std::future::Future;
use std::io::BufReader;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How often a long-running analysis emits a progress notification (only
/// when the client sent a `progressToken`). Keeps clients that reset their
/// request timeout on progress from giving up on a cold, multi-minute pull.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
/// `list_files` page size default / hard cap.
const DEFAULT_LIST_LIMIT: usize = 100;
const MAX_LIST_LIMIT: usize = 1000;
/// `read_file` returns at most this much text; the rest is cut with a marker.
/// Model context is the scarce resource here, not server memory.
const MAX_READ_TEXT_BYTES: usize = 256 * 1024;
/// A NUL byte in this prefix marks the file as binary.
const BINARY_SNIFF_BYTES: usize = 8 * 1024;

const INSTRUCTIONS: &str = "diving analyzes Docker/OCI image layers: wasted space, \
leaked secrets, bloat and runtime compatibility. Start with `analyze_image` for a \
Markdown report, or `get_findings` for the same conclusions as JSON. Drill down with \
`list_files` and `read_file`. Layer numbers are 1-based and match the report. The first \
analysis of an image downloads every layer and can take minutes; repeat calls hit the \
cache and return quickly.";

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ImageParams {
    /// Image reference, e.g. `redis:alpine`, `ghcr.io/org/app:1.2`,
    /// `registry.example.com:5000/team/app@sha256:…`.
    pub image: String,
    /// Platform architecture for multi-arch images: `amd64`, `arm64`, …
    /// Defaults to the server's architecture.
    pub arch: Option<String>,
    /// Language of the recommendations text: `en` or `zh`. Defaults to the
    /// server's locale.
    pub lang: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AnalyzeImageParams {
    #[serde(flatten)]
    pub target: ImageParams,
    /// Hide auto-detected base-image layers from the report (default
    /// `true`). Set `false` to include them.
    pub skip_base: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListFilesParams {
    #[serde(flatten)]
    pub target: ImageParams,
    /// Only list files from this layer (1-based, as numbered in the
    /// report). Omit to search every layer.
    pub layer: Option<usize>,
    /// Only files under this directory, e.g. `usr/lib` or `/app`.
    pub path_prefix: Option<String>,
    /// Case-insensitive substring the path must contain.
    pub keyword: Option<String>,
    /// Only files at least this many bytes.
    pub min_size: Option<u64>,
    /// Sort largest first instead of by layer and path.
    pub sort_by_size: Option<bool>,
    /// Number of matching files to skip (pagination).
    pub offset: Option<usize>,
    /// Page size (default 100, max 1000).
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadFileParams {
    #[serde(flatten)]
    pub target: ImageParams,
    /// Layer holding the file (1-based, as numbered in the report).
    pub layer: usize,
    /// Path of the file inside the layer, e.g. `etc/nginx/nginx.conf`.
    pub path: String,
}

/// One row of `list_files` output.
#[derive(Debug, PartialEq, Serialize)]
struct FileRow {
    layer: usize,
    path: String,
    size: u64,
    mode: String,
    /// `added` | `modified` | `removed`
    op: &'static str,
    #[serde(skip_serializing_if = "String::is_empty")]
    link: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FilePage {
    total: usize,
    offset: usize,
    files: Vec<FileRow>,
}

/// Filter + pagination knobs for [`list_files_in`], separated from the
/// wire params so it can be unit-tested without an analysis.
#[derive(Debug, Default)]
struct FileQuery {
    layer: Option<usize>,
    path_prefix: Option<String>,
    keyword: Option<String>,
    min_size: Option<u64>,
    sort_by_size: bool,
    offset: usize,
    limit: usize,
}

#[derive(Debug, Clone, Default)]
pub struct DivingMcp;

/// What the web UI needs to describe the endpoint to a visitor, reported by
/// `/api/latest-images`. Only whether a token is required — never the token.
#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    pub enabled: bool,
    pub token_required: bool,
}

#[tool_router]
impl DivingMcp {
    #[tool(
        description = "Analyze a container image and return a Markdown report: efficiency \
score, wasted space, per-layer changes, recommendations, sensitive files, cross-layer \
duplicates and runtime compatibility.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn analyze_image(
        &self,
        Parameters(params): Parameters<AnalyzeImageParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let lang = Lang::resolve(params.target.lang.as_deref());
        let result = run_analysis(&params.target, lang, &ctx).await?;
        Ok(to_markdown(&result, params.skip_base.unwrap_or(true), lang))
    }

    #[tool(
        description = "Analyze a container image and return its findings as JSON: \
efficiency, wasted bytes, recommendations, sensitive files, big modified files, \
cross-layer duplicates, runtime compatibility and a per-layer summary.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn get_findings(
        &self,
        Parameters(params): Parameters<ImageParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let lang = Lang::resolve(params.lang.as_deref());
        let result = run_analysis(&params, lang, &ctx).await?;
        Ok(findings_json(&result).to_string())
    }

    #[tool(
        description = "List files in an analyzed image, filtered by layer, directory, \
keyword or size, with pagination. Returns JSON rows of {layer, path, size, mode, op}.",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn list_files(
        &self,
        Parameters(params): Parameters<ListFilesParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let lang = Lang::resolve(params.target.lang.as_deref());
        let result = run_analysis(&params.target, lang, &ctx).await?;
        let query = FileQuery {
            layer: params.layer,
            path_prefix: params.path_prefix,
            keyword: params.keyword,
            min_size: params.min_size,
            sort_by_size: params.sort_by_size.unwrap_or(false),
            offset: params.offset.unwrap_or(0),
            limit: params
                .limit
                .unwrap_or(DEFAULT_LIST_LIMIT)
                .clamp(1, MAX_LIST_LIMIT),
        };
        let page = list_files_in(&result.file_tree_list, &query)?;
        serde_json::to_string(&page).map_err(|e| e.to_string())
    }

    #[tool(
        description = "Read a text file from one layer of an analyzed registry image \
(up to 256 KiB; binary files are reported, not returned).",
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn read_file(
        &self,
        Parameters(params): Parameters<ReadFileParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<String, String> {
        let registry = parse_image_info(&params.target.image).registry;
        if registry == REGISTRY_LOCAL_FILE || registry == REGISTRY_LOCAL_DOCKER {
            return Err("read_file only supports registry images".to_string());
        }
        let lang = Lang::resolve(params.target.lang.as_deref());
        let result = run_analysis(&params.target, lang, &ctx).await?;
        let layer = layer_slot(params.layer, result.layers.len())
            .and_then(|i| result.layers.get(i))
            .ok_or_else(|| {
                format!(
                    "layer {} out of range (image has {} layers)",
                    params.layer,
                    result.layers.len()
                )
            })?;
        if layer.empty || !is_safe_blob_id(&layer.digest) {
            return Err(format!("layer {} has no file content", params.layer));
        }
        let file_path = normalize_path(&params.path).to_string();
        // A link has no content of its own. Say where it points instead of
        // returning an empty string the model cannot tell from an empty file.
        let link = layer_slot(params.layer, result.file_tree_list.len())
            .and_then(|i| find_file(&result.file_tree_list[i], &file_path))
            .map(|item| item.link.as_str())
            .filter(|link| !link.is_empty());
        if let Some(link) = link {
            return Ok(format!(
                "{file_path} is a link to {} — read that path instead (it may be in another layer).",
                resolve_link(&file_path, link)
            ));
        }
        let blob_path = get_blob_path(&layer.digest);
        let media_type = layer.media_type.clone();
        let max_bytes = get_max_download_file_size();
        let wanted = file_path.clone();
        // Decompression is CPU-bound; keep it off the async workers.
        let content = tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
            let file = File::open(&blob_path).map_err(|e| e.to_string())?;
            get_file_content_from_layer(BufReader::new(file), &media_type, &wanted, max_bytes)
                .map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())??;
        Ok(render_file_text(&file_path, &content))
    }

    #[tool(
        description = "List the most recently analyzed image references on this server.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn latest_images(&self) -> String {
        json!({ "images": latest_images() }).to_string()
    }
}

#[tool_handler]
impl ServerHandler for DivingMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("diving", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

/// `/mcp` router. With `token`, every request must carry
/// `Authorization: Bearer <token>` and the `Host` check is skipped (the
/// token already defeats DNS rebinding); without it only loopback hosts
/// plus `mcp_allowed_hosts` from config are accepted.
pub fn new_router(token: Option<String>) -> Router {
    let mut config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true);
    let extra_hosts = get_mcp_allowed_hosts();
    if token.is_some() || extra_hosts.iter().any(|h| h == "*") {
        config = config.disable_allowed_hosts();
    } else {
        let hosts: Vec<String> = config
            .allowed_hosts
            .iter()
            .chain(extra_hosts)
            .cloned()
            .collect();
        config = config.with_allowed_hosts(hosts);
    }
    let service: StreamableHttpService<DivingMcp, LocalSessionManager> =
        StreamableHttpService::new(|| Ok(DivingMcp), Default::default(), config);
    let router = Router::new().nest_service("/mcp", service);
    match token {
        Some(token) => router.layer(from_fn_with_state(
            Arc::new(token_fingerprint(&token)),
            require_token,
        )),
        None => router,
    }
}

async fn require_token(State(expected): State<Arc<Vec<u8>>>, req: Request, next: Next) -> Response {
    let authorized = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|got| digests_equal(&token_fingerprint(got.trim()), &expected));
    if !authorized {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer")],
            "unauthorized",
        )
            .into_response();
    }
    next.run(req).await
}

/// Tokens are compared as SHA-256 digests so the comparison runs over a
/// fixed length and leaks neither the token's length nor a matching prefix.
fn token_fingerprint(token: &str) -> Vec<u8> {
    digest(&SHA256, token.as_bytes()).as_ref().to_vec()
}

fn digests_equal(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Registry check → shared analysis (with progress heartbeat) → recent-list.
async fn run_analysis(
    params: &ImageParams,
    lang: Lang,
    ctx: &RequestContext<RoleServer>,
) -> Result<Arc<DockerAnalyzeResult>, String> {
    let image = with_arch(&params.image, params.arch.as_deref());
    ensure_registry_allowed(&parse_image_info(&image)).map_err(|e| e.message)?;
    let result = with_heartbeat(analyze_singleflight(image.clone(), lang, true), &image, ctx)
        .await
        .map_err(|e| e.message)?;
    add_to_latest_image_cache(&image);
    Ok(result)
}

/// Fold the tool's `arch` argument into the `?arch=` query the image parser
/// understands — same shape the web frontend sends. Local sources carry
/// their own platform, and an explicit `?arch=` in `image` wins.
fn with_arch(image: &str, arch: Option<&str>) -> String {
    let image = image.trim();
    let arch = arch.map(str::trim).unwrap_or_default();
    let is_local = image.starts_with("file://") || image.starts_with("docker://");
    if arch.is_empty() || is_local || image.contains("?arch=") {
        return image.to_string();
    }
    format!("{image}?arch={arch}")
}

/// Await `fut`, emitting a progress notification every
/// [`HEARTBEAT_INTERVAL`] if the client asked for progress.
async fn with_heartbeat<F: Future>(
    fut: F,
    image: &str,
    ctx: &RequestContext<RoleServer>,
) -> F::Output {
    let Some(token) = ctx.meta.get_progress_token() else {
        return fut.await;
    };
    let started = Instant::now();
    let mut ticker = tokio::time::interval(HEARTBEAT_INTERVAL);
    // The first tick completes immediately; skip it.
    ticker.tick().await;
    tokio::pin!(fut);
    loop {
        tokio::select! {
            out = &mut fut => return out,
            _ = ticker.tick() => {
                let secs = started.elapsed().as_secs();
                let param = ProgressNotificationParam::new(token.clone(), secs as f64)
                    .with_message(format!("analyzing {image} ({secs}s elapsed)"));
                // Best-effort: a client that went away just misses the tick.
                let _ = ctx.peer.notify_progress(param).await;
            }
        }
    }
}

/// 1-based layer number → index, `None` when out of range.
fn layer_slot(layer: usize, len: usize) -> Option<usize> {
    (1..=len).contains(&layer).then(|| layer - 1)
}

/// Strip the leading `/` a model tends to add and the `./` some tars store.
fn normalize_path(path: &str) -> &str {
    let path = path.trim();
    let path = path.strip_prefix("./").unwrap_or(path);
    path.trim_start_matches('/')
}

/// The entry at `path` in one layer's tree, if that layer has it.
fn find_file<'a>(items: &'a [FileTreeItem], path: &str) -> Option<&'a FileTreeItem> {
    // Tars that store `./usr/...` put everything under a `.` root.
    let items = match items {
        [root] if root.name == "." => &root.children,
        _ => items,
    };
    let (name, rest) = match path.split_once('/') {
        Some((name, rest)) => (name, Some(rest)),
        None => (path, None),
    };
    match rest {
        None => items.iter().find(|item| item.name == name),
        Some(rest) => items
            .iter()
            .filter(|item| item.name == name)
            .find_map(|dir| find_file(&dir.children, rest)),
    }
}

/// The image path a link at `path` points to: absolute targets are taken
/// from the image root, relative ones from the link's own directory.
fn resolve_link(path: &str, target: &str) -> String {
    let mut parts: Vec<&str> = if target.starts_with('/') {
        vec![]
    } else {
        let mut dir: Vec<&str> = path.split('/').collect();
        dir.pop();
        dir
    };
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

fn op_name(op: &Op) -> &'static str {
    match op {
        Op::Removed => "removed",
        Op::Modified => "modified",
        Op::None | Op::Added => "added",
    }
}

fn collect_files(
    items: &[FileTreeItem],
    layer: usize,
    parent: &str,
    query: &FileQuery,
    out: &mut Vec<FileRow>,
) {
    for item in items {
        let path = if parent.is_empty() {
            item.name.clone()
        } else {
            format!("{parent}/{}", item.name)
        };
        if !item.children.is_empty() {
            collect_files(&item.children, layer, &path, query, out);
            continue;
        }
        // Directory entries never reach the tree (`collect_tar_entries`
        // drops them); the one nameless leaf is an opaque whiteout on the
        // image root, which has no path to show.
        if item.name.is_empty() {
            continue;
        }
        let path = normalize_path(&path).to_string();
        if query.min_size.is_some_and(|min| item.size < min) {
            continue;
        }
        if let Some(prefix) = query.path_prefix.as_deref().map(normalize_path) {
            let prefix = prefix.trim_end_matches('/');
            let under = path == prefix
                || path
                    .strip_prefix(prefix)
                    .is_some_and(|rest| prefix.is_empty() || rest.starts_with('/'));
            if !under {
                continue;
            }
        }
        if let Some(keyword) = query.keyword.as_deref() {
            if !path.to_lowercase().contains(&keyword.to_lowercase()) {
                continue;
            }
        }
        out.push(FileRow {
            layer: layer + 1,
            path,
            size: item.size,
            mode: item.mode.clone(),
            op: op_name(&item.op),
            link: item.link.clone(),
        });
    }
}

fn list_files_in(trees: &[Vec<FileTreeItem>], query: &FileQuery) -> Result<FilePage, String> {
    let mut rows = vec![];
    match query.layer {
        Some(layer) => {
            let index = layer_slot(layer, trees.len()).ok_or_else(|| {
                format!(
                    "layer {layer} out of range (image has {} layers)",
                    trees.len()
                )
            })?;
            collect_files(&trees[index], index, "", query, &mut rows);
        }
        None => {
            for (index, tree) in trees.iter().enumerate() {
                collect_files(tree, index, "", query, &mut rows);
            }
        }
    }
    if query.sort_by_size {
        rows.sort_by_key(|row| std::cmp::Reverse(row.size));
    }
    let total = rows.len();
    let files = rows
        .into_iter()
        .skip(query.offset)
        .take(query.limit)
        .collect();
    Ok(FilePage {
        total,
        offset: query.offset,
        files,
    })
}

fn render_file_text(path: &str, content: &[u8]) -> String {
    let sniff = &content[..content.len().min(BINARY_SNIFF_BYTES)];
    if sniff.contains(&0) {
        return format!(
            "{path} is a binary file ({}); content not returned.",
            ByteSize(content.len() as u64)
        );
    }
    let shown = &content[..content.len().min(MAX_READ_TEXT_BYTES)];
    let mut text = String::from_utf8_lossy(shown).into_owned();
    if shown.len() < content.len() {
        text.push_str(&format!(
            "\n\n[truncated: showing the first {} of {}]",
            ByteSize(shown.len() as u64),
            ByteSize(content.len() as u64)
        ));
    }
    text
}

/// The analysis minus the raw file trees / per-file summary, which are
/// what `list_files` is for. Nested `layerIndex` (0-based) fields become
/// 1-based `layer` so every number the model sees matches the report.
fn findings_json(result: &DockerAnalyzeResult) -> Value {
    let summary = result.summary();
    let layers: Vec<Value> = result
        .layers
        .iter()
        .enumerate()
        .map(|(i, layer)| {
            json!({
                "layer": i + 1,
                "cmd": layer.cmd,
                "created": layer.created,
                "size": layer.size,
                "unpackSize": layer.unpack_size,
                "empty": layer.empty,
            })
        })
        .collect();
    let mut value = json!({
        "name": result.name,
        "arch": result.arch,
        "os": result.os,
        "user": result.user,
        "baseOs": result.base_os,
        "size": result.size,
        "totalSize": result.total_size,
        "efficiencyScore": summary.score,
        "wastedSize": summary.wasted_size,
        "wastedPercent": summary.wasted_percent,
        "tags": result.tags,
        "recommendations": result.recommendations,
        "sensitiveFiles": result.sensitive_files,
        "bigModifiedFiles": result.big_modified_file_list,
        "duplicateGroups": result.duplicate_groups,
        "runtimeCompat": result.runtime_compat,
        "layers": layers,
    });
    renumber_layers(&mut value);
    value
}

fn renumber_layers(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(index) = map.get("layerIndex").and_then(Value::as_u64) {
                map.remove("layerIndex");
                map.insert("layer".to_string(), json!(index + 1));
            }
            map.values_mut().for_each(renumber_layers);
        }
        Value::Array(items) => items.iter_mut().for_each(renumber_layers),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64, op: Op) -> FileTreeItem {
        FileTreeItem {
            name: name.to_string(),
            size,
            mode: "-rw-r--r--".to_string(),
            op,
            ..Default::default()
        }
    }

    fn dir(name: &str, children: Vec<FileTreeItem>) -> FileTreeItem {
        FileTreeItem {
            name: name.to_string(),
            size: children.iter().map(|c| c.size).sum(),
            children,
            ..Default::default()
        }
    }

    fn fixture() -> Vec<Vec<FileTreeItem>> {
        vec![
            vec![
                dir(
                    "usr",
                    vec![dir(
                        "lib",
                        vec![
                            file("libc.so", 900, Op::None),
                            file("libz.so", 100, Op::None),
                        ],
                    )],
                ),
                dir("etc", vec![file("app.conf", 10, Op::None)]),
            ],
            // Empty (ENV/LABEL) layer.
            vec![],
            vec![
                dir("etc", vec![file("app.conf", 12, Op::Modified)]),
                dir(
                    "usr",
                    vec![dir("lib", vec![file("libz.so", 0, Op::Removed)])],
                ),
                dir("usrlocal", vec![file("x", 5, Op::None)]),
            ],
        ]
    }

    fn query() -> FileQuery {
        FileQuery {
            limit: DEFAULT_LIST_LIMIT,
            ..Default::default()
        }
    }

    fn paths(page: &FilePage) -> Vec<(usize, &str)> {
        page.files
            .iter()
            .map(|f| (f.layer, f.path.as_str()))
            .collect()
    }

    #[test]
    fn with_arch_appends_query_for_registry_images_only() {
        assert_eq!(
            with_arch("redis:alpine", Some("arm64")),
            "redis:alpine?arch=arm64"
        );
        assert_eq!(with_arch(" redis:alpine ", None), "redis:alpine");
        assert_eq!(with_arch("redis:alpine", Some(" ")), "redis:alpine");
        assert_eq!(
            with_arch("redis:alpine?arch=amd64", Some("arm64")),
            "redis:alpine?arch=amd64"
        );
        assert_eq!(
            with_arch("file:///tmp/a.tar", Some("arm64")),
            "file:///tmp/a.tar"
        );
        assert_eq!(
            with_arch("docker://app:dev", Some("arm64")),
            "docker://app:dev"
        );
    }

    #[test]
    fn list_files_numbers_layers_from_one_and_maps_ops() {
        let page = list_files_in(&fixture(), &query()).unwrap();
        assert_eq!(page.total, 6);
        assert_eq!(
            paths(&page),
            vec![
                (1, "usr/lib/libc.so"),
                (1, "usr/lib/libz.so"),
                (1, "etc/app.conf"),
                (3, "etc/app.conf"),
                (3, "usr/lib/libz.so"),
                (3, "usrlocal/x"),
            ]
        );
        let ops: Vec<&str> = page.files.iter().map(|f| f.op).collect();
        assert_eq!(
            ops,
            ["added", "added", "added", "modified", "removed", "added"]
        );
    }

    #[test]
    fn list_files_filters_by_layer_prefix_keyword_and_size() {
        let trees = fixture();
        let page = list_files_in(
            &trees,
            &FileQuery {
                layer: Some(3),
                ..query()
            },
        )
        .unwrap();
        assert_eq!(page.total, 3);

        // A prefix matches whole path segments: `/usr` must not match `usrlocal`.
        let page = list_files_in(
            &trees,
            &FileQuery {
                path_prefix: Some("/usr/".to_string()),
                ..query()
            },
        )
        .unwrap();
        assert_eq!(
            paths(&page),
            vec![
                (1, "usr/lib/libc.so"),
                (1, "usr/lib/libz.so"),
                (3, "usr/lib/libz.so")
            ]
        );

        let page = list_files_in(
            &trees,
            &FileQuery {
                keyword: Some("LIBZ".to_string()),
                min_size: Some(1),
                ..query()
            },
        )
        .unwrap();
        assert_eq!(paths(&page), vec![(1, "usr/lib/libz.so")]);
    }

    #[test]
    fn list_files_sorts_and_paginates() {
        let page = list_files_in(
            &fixture(),
            &FileQuery {
                sort_by_size: true,
                offset: 1,
                limit: 2,
                ..query()
            },
        )
        .unwrap();
        assert_eq!(page.total, 6);
        assert_eq!(page.offset, 1);
        assert_eq!(
            paths(&page),
            vec![(1, "usr/lib/libz.so"), (3, "etc/app.conf")]
        );
    }

    #[test]
    fn list_files_rejects_out_of_range_layer() {
        for layer in [0, 4] {
            let err = list_files_in(
                &fixture(),
                &FileQuery {
                    layer: Some(layer),
                    ..query()
                },
            )
            .unwrap_err();
            assert!(err.contains("out of range"), "{err}");
        }
    }

    #[test]
    fn list_files_normalizes_dot_root_and_skips_nameless_leaf() {
        let trees = vec![vec![
            file("", 0, Op::Removed),
            dir(".", vec![dir("var", vec![file("log", 1, Op::None)])]),
        ]];
        let page = list_files_in(&trees, &query()).unwrap();
        assert_eq!(paths(&page), vec![(1, "var/log")]);
    }

    #[test]
    fn resolve_link_handles_relative_absolute_and_sibling_targets() {
        // Debian's /etc/os-release
        assert_eq!(
            resolve_link("etc/os-release", "../usr/lib/os-release"),
            "usr/lib/os-release"
        );
        // Alpine's /bin/sh -> /bin/busybox
        assert_eq!(resolve_link("bin/sh", "/bin/busybox"), "bin/busybox");
        assert_eq!(
            resolve_link("usr/bin/python", "python3.12"),
            "usr/bin/python3.12"
        );
        assert_eq!(resolve_link("a/b/c", "./d/../e"), "a/b/e");
        // More `..` than there are directories stops at the image root.
        assert_eq!(resolve_link("bin/x", "../../../etc/passwd"), "etc/passwd");
    }

    #[test]
    fn find_file_walks_one_layer_tree() {
        let mut link = file("os-release", 0, Op::None);
        link.link = "../usr/lib/os-release".to_string();
        let tree = vec![
            dir("etc", vec![link, file("hostname", 5, Op::None)]),
            dir(
                "usr",
                vec![dir("lib", vec![file("os-release", 267, Op::None)])],
            ),
        ];
        assert_eq!(
            find_file(&tree, "etc/os-release").map(|f| f.link.as_str()),
            Some("../usr/lib/os-release")
        );
        assert_eq!(
            find_file(&tree, "usr/lib/os-release").map(|f| f.size),
            Some(267)
        );
        assert!(find_file(&tree, "etc/missing").is_none());
        assert!(find_file(&tree, "etc/hostname/x").is_none());
        // A `./`-rooted tar keeps everything under a `.` directory.
        let dotted = vec![dir(".", tree)];
        assert_eq!(find_file(&dotted, "etc/hostname").map(|f| f.size), Some(5));
    }

    #[test]
    fn normalize_path_strips_root_and_dot_prefixes() {
        assert_eq!(normalize_path("/etc/passwd"), "etc/passwd");
        assert_eq!(normalize_path("./etc/passwd"), "etc/passwd");
        assert_eq!(normalize_path(" etc/passwd "), "etc/passwd");
    }

    #[test]
    fn render_file_text_handles_binary_and_truncation() {
        assert!(render_file_text("bin/app", b"\x7fELF\0\0").contains("binary file"));
        assert_eq!(render_file_text("a.txt", b"hello"), "hello");
        let big = vec![b'a'; MAX_READ_TEXT_BYTES + 10];
        let text = render_file_text("big.txt", &big);
        assert!(text.starts_with("aaaa"));
        assert!(text.contains("[truncated:"), "{}", &text[text.len() - 80..]);
    }

    #[test]
    fn renumber_layers_rewrites_nested_indices() {
        let mut value = json!({
            "sensitiveFiles": [{"path": "a", "layerIndex": 0}],
            "duplicateGroups": [{"paths": [{"layerIndex": 2, "path": "b"}]}],
        });
        renumber_layers(&mut value);
        assert_eq!(
            value,
            json!({
                "sensitiveFiles": [{"path": "a", "layer": 1}],
                "duplicateGroups": [{"paths": [{"layer": 3, "path": "b"}]}],
            })
        );
    }

    #[test]
    fn token_comparison_matches_exact_token_only() {
        let expected = token_fingerprint("s3cret");
        assert!(digests_equal(&token_fingerprint("s3cret"), &expected));
        assert!(!digests_equal(&token_fingerprint("s3cre"), &expected));
        assert!(!digests_equal(&token_fingerprint(""), &expected));
    }
}
