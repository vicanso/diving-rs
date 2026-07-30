use crate::config::{get_layer_concurrency, load_user_sensitive_patterns};
use crate::i18n;
use crate::recommend::{build_recommendations, Recommendation};
use crate::util::get_http_client;
use crate::{task_local::*, tl_info};
use chrono::{DateTime, Utc};
use futures::StreamExt;
use http::StatusCode;
use lru::LruCache;
use once_cell::sync::OnceCell;
use regex::Regex;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use snafu::{ResultExt, Snafu};
use std::io::{BufReader, SeekFrom};
use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
    fs::File,
    num::NonZeroUsize,
    path::Path,
    process::Stdio,
    str::FromStr,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::OnceCell as TokioOnceCell;

use super::image_ref::{repository_path, ImageInfo, REGISTRY_LOCAL_DOCKER, REGISTRY_LOCAL_FILE};
use super::layer::{path_under_dir, TarIndex};
use super::sensitive::{is_dev_artifact, is_pkg_cache, is_sensitive_file};
use super::{
    elf::{analyze_runtime_compat, RuntimeCompat},
    layer::ImageLayerInfo,
    oci_image::{
        detect_cross_layer_duplicates, DuplicateGroup, ImageFileSummary, ImageHistory,
        ImageManifestLayer,
    },
    FileTreeItem, ImageConfig, ImageIndex, ImageLayer, ImageManifest, ImageManifestConfig, Op,
    MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST, MEDIA_TYPE_IMAGE_INDEX, MEDIA_TYPE_MANIFEST_LIST,
};
use super::{get_files_from_layer, get_os_release_from_layer};
use crate::{
    error::HTTPError,
    image::convert_files_to_file_tree,
    store::{
        get_blob_from_file, get_blob_path, read_analysis, save_blob_to_file, sha256_hex,
        sha256_hex_of_file, tmp_sibling_path, write_analysis,
    },
};

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("IO fail: {source}"))]
    IO { source: std::io::Error },
    #[snafu(display("Request {} fail: {}", url, source))]
    Request { source: reqwest::Error, url: String },
    #[snafu(display("Parse {} json fail: {}", url, source))]
    Json { source: reqwest::Error, url: String },
    #[snafu(display("Serde json {category} fail: {source}"))]
    SerdeJson {
        source: serde_json::Error,
        category: String,
    },
    #[snafu(display("Layer handle fail: {}", source))]
    Layer { source: super::layer::Error },
    #[snafu(display("Request {} code: {} fail: {}", url, code, message))]
    Docker {
        message: String,
        code: String,
        url: String,
        // 原始 HTTP 状态码；`code` 是 registry 错误体里的业务码（如
        // "UNAUTHORIZED"），判断 token 过期需要真正的 401。
        status: u16,
    },
    #[snafu(display("{message}"))]
    Whatever { message: String },
}

impl From<Error> for HTTPError {
    fn from(err: Error) -> Self {
        // 对于部分error单独转换
        HTTPError::new_with_category(&err.to_string(), "docker")
    }
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

// token 中途失效（HTTP 401）需要刷新重试；registry 错误体里的业务码
// 不可靠，判断依据是真实状态码。
fn is_unauthorized(err: &Error) -> bool {
    matches!(err, Error::Docker { status, .. } if *status == 401)
}

/// Whether `data` hashes to `digest`. Non-sha256 digests (local tar
/// layouts) are not verifiable and pass through.
fn bytes_match_digest(data: &[u8], digest: &str) -> bool {
    match digest.strip_prefix("sha256:") {
        Some(expected) => sha256_hex(data).eq_ignore_ascii_case(expected),
        None => true,
    }
}

/// Verify an on-disk blob against its OCI digest. Only `sha256:` digests are
/// checked (the only algorithm in practice); others pass through. Hashing a
/// multi-hundred-MB layer is CPU + I/O bound, so it runs on the blocking pool.
async fn verify_blob_digest(path: &Path, expected_digest: &str) -> Result<()> {
    let Some(expected) = expected_digest.strip_prefix("sha256:") else {
        return Ok(());
    };
    let path = path.to_path_buf();
    let actual = tokio::task::spawn_blocking(move || sha256_hex_of_file(&path))
        .await
        .map_err(|e| Error::Whatever {
            message: format!("blob hash task failed: {e}"),
        })?
        .map_err(|err| Error::IO { source: err })?;
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(Error::Whatever {
            message: format!(
                "blob digest mismatch: expected sha256:{expected}, got sha256:{actual}"
            ),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct AuthInfo {
    pub auth: String,
    pub service: String,
    pub scope: String,
}

fn parse_auth_info(auth: &str) -> Result<AuthInfo> {
    static AUTH_RE: once_cell::sync::Lazy<Regex> = once_cell::sync::Lazy::new(|| {
        Regex::new("(?P<key>\\S+?)=\"(?P<value>\\S+?)\",?").expect("auth regex is valid")
    });
    let mut auth_info = AuthInfo::default();
    for caps in AUTH_RE.captures_iter(auth) {
        let value = caps["value"].to_string();
        match &caps["key"] {
            "realm" => auth_info.auth = value,
            "service" => auth_info.service = value,
            "scope" => auth_info.scope = value,
            _ => {}
        }
    }
    Ok(auth_info)
}

#[derive(Debug, Clone, Default)]
pub struct DockerClient {
    registry: String,
    // file:// / docker:// 模式下懒构建的镜像 tar 一趟索引。client 的克隆
    // （get_all_layer_info 按层 spawn 任务时）共享同一份，整个 tar 只扫一次。
    local_index: Arc<TokioOnceCell<Arc<TarIndex>>>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DockerTokenInfo {
    /// Docker Hub returns both `token` and `access_token` (often identical).
    /// They must be separate fields — `#[serde(alias)]` treats them as one
    /// field and fails with "duplicate field `token`" when both are present.
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
    expires_in: Option<i32>,
    issued_at: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageManifestCacheInfo {
    expired_at: i64,
    manifest: ImageManifest,
    supported_archs: Vec<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BigModifiedFileInfo {
    pub path: String,
    pub size: u64,
    pub digest: String,
    pub mode: String,
    pub uid: u64,
    pub gid: u64,
}

/// Parse the content of an OS release file into a human-readable OS name.
fn parse_os_release(content: &[u8], filename: &str) -> Option<String> {
    let text = std::str::from_utf8(content).ok()?.trim();
    match filename {
        "etc/alpine-release" => {
            return Some(format!("Alpine Linux {}", text.lines().next()?.trim()));
        }
        "etc/debian_version" => {
            return Some(format!("Debian {}", text.lines().next()?.trim()));
        }
        f if f.ends_with("redhat-release") => {
            return Some(text.lines().next()?.trim().to_string());
        }
        _ => {}
    }
    // Parse KEY=VALUE or KEY="VALUE" (os-release / lsb-release format)
    let mut pretty_name: Option<String> = None;
    let mut name: Option<String> = None;
    let mut version_id: Option<String> = None;
    let mut version: Option<String> = None;
    for line in text.lines() {
        if let Some((k, v)) = line.trim().split_once('=') {
            let v = v.trim_matches('"').trim_matches('\'').to_string();
            match k {
                "PRETTY_NAME" => pretty_name = Some(v),
                "NAME" => name = Some(v),
                "VERSION_ID" => version_id = Some(v),
                "VERSION" => version = Some(v),
                _ => {}
            }
        }
    }
    if let Some(pn) = pretty_name {
        return Some(pn);
    }
    match (name, version.or(version_id)) {
        (Some(n), Some(v)) => Some(format!("{} {}", n, v)),
        (Some(n), None) => Some(n),
        _ => None,
    }
}

/// Fallback: scan the first few layer commands for known distro names / codenames.
fn detect_os_from_history(layers: &[ImageLayer]) -> Option<String> {
    const DISTROS: &[(&str, &str)] = &[
        // Codenames checked before short names to get the more specific match first
        ("trixie", "Debian 13 (trixie)"),
        ("bookworm", "Debian 12 (bookworm)"),
        ("bullseye", "Debian 11 (bullseye)"),
        ("buster", "Debian 10 (buster)"),
        ("noble", "Ubuntu 24.04 (Noble Numbat)"),
        ("jammy", "Ubuntu 22.04 (Jammy Jellyfish)"),
        ("focal", "Ubuntu 20.04 (Focal Fossa)"),
        ("bionic", "Ubuntu 18.04 (Bionic Beaver)"),
        ("alpine", "Alpine Linux"),
        ("ubuntu", "Ubuntu"),
        ("debian", "Debian"),
        ("centos", "CentOS"),
        ("fedora", "Fedora"),
        ("rhel", "Red Hat Enterprise Linux"),
        ("amazon", "Amazon Linux"),
        ("suse", "openSUSE"),
    ];
    for layer in layers.iter().take(3) {
        let cmd = layer.cmd.to_lowercase();
        for &(pattern, display) in DISTROS {
            if cmd.contains(pattern) {
                return Some(format!("{} (from history)", display));
            }
        }
    }
    None
}

/// Probe the first few manifest layers for OS release files.
/// Uses cached blobs on disk — returns empty string when blobs are not available.
fn probe_base_os(manifest: &ImageManifest) -> String {
    for layer in manifest.layers.iter().take(3) {
        let path = get_blob_path(&layer.digest);
        if !path.exists() {
            continue;
        }
        if let Ok(file) = File::open(&path) {
            if let Some((filename, content)) =
                get_os_release_from_layer(BufReader::new(file), &layer.media_type)
            {
                if let Some(name) = parse_os_release(&content, filename) {
                    return name;
                }
            }
        }
    }
    String::new()
}

/// Reconstruct an approximate Dockerfile from image history.
///
/// Each `created_by` entry follows one of these patterns:
///   `/bin/sh -c #(nop) <INSTRUCTION> <args>`  → metadata instruction (ENV, CMD, …)
///   `/bin/sh -c <command>`                     → RUN <command>
///   `|<n> KEY=val … /bin/sh -c <command>`      → RUN <command> (with build-args)
fn reconstruct_dockerfile(history: &[ImageHistory]) -> String {
    let mut lines: Vec<String> = Vec::with_capacity(history.len());
    for h in history {
        let raw = match h.created_by.as_deref() {
            None | Some("") => continue,
            Some(s) => s,
        };
        // Strip build-arg prefix: |<n> KEY=val ... /bin/sh -c <cmd>
        let raw = if raw.starts_with('|') {
            raw.find("/bin/sh -c ")
                .map(|pos| &raw[pos..])
                .unwrap_or(raw)
        } else {
            raw
        };
        let line = if let Some(rest) = raw.strip_prefix("/bin/sh -c #(nop) ") {
            rest.trim().to_string()
        } else if let Some(rest) = raw.strip_prefix("/bin/sh -c ") {
            format!("RUN {rest}")
        } else {
            raw.to_string()
        };
        lines.push(line);
    }
    lines.join("\n")
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SensitiveFileInfo {
    pub path: String,
    pub size: u64,
    pub layer_index: usize,
    pub reason: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerAnalyzeResult {
    // 镜像名称
    pub name: String,
    // 架构
    pub arch: String,
    // 系统
    pub os: String,
    // 运行用户
    pub user: String,
    // 环境变量
    pub envs: Vec<String>,
    // 镜像label
    pub labels: Vec<String>,
    // 反推的 Dockerfile 内容
    pub dockerfile: String,
    // 基础镜像 OS 指纹
    pub base_os: String,
    // 该镜像支持的架构列表（linux 平台，来自 manifest index）
    pub supported_archs: Vec<String>,
    // 镜像分层数据
    pub layers: Vec<ImageLayer>,
    // 镜像大小
    pub size: u64,
    // 镜像分层解压大小
    pub total_size: u64,
    // 镜像分层对应的文件树
    pub file_tree_list: Vec<Vec<FileTreeItem>>,
    // 镜像删除与更新文件汇总
    pub file_summary_list: Vec<ImageFileSummary>,
    // 本次镜像变化的大文件
    pub big_modified_file_list: Vec<BigModifiedFileInfo>,
    // 疑似敏感文件
    pub sensitive_files: Vec<SensitiveFileInfo>,
    // 启发式风险标签
    pub tags: Vec<String>,
    // 体积/必要性/安全优化建议（由分析数据派生）
    pub recommendations: Vec<Recommendation>,
    // 跨层重复文件组（同内容 hash 出现在不同 layer），由 `--no-verify-dup`
    // 关闭；旧版本/旧 cache 反序列化时为空 vec。
    #[serde(default)]
    pub duplicate_groups: Vec<DuplicateGroup>,
    // 启动二进制 (Entrypoint/Cmd[0]) 的 ELF 兼容性报告：glibc/musl 归类、
    // 最小 glibc 版本、与基础镜像 glibc 版本的对比。旧 cache 无此字段时
    // 退化为默认值（空 `issue`，不产生卡片）。
    #[serde(default)]
    pub runtime_compat: RuntimeCompat,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize)]
pub struct ImageFileWastedSummary {
    pub path: String,
    pub total_size: u64,
    pub count: u32,
}

#[derive(Default, Debug, Clone, Serialize)]
pub struct DockerAnalyzeSummary {
    pub wasted_list: Vec<ImageFileWastedSummary>,
    pub wasted_size: u64,
    pub wasted_percent: f64,
    pub score: u64,
}

impl DockerAnalyzeResult {
    pub fn summary(&self) -> DockerAnalyzeSummary {
        let mut wasted_list: Vec<ImageFileWastedSummary> = vec![];
        let mut path_index: HashMap<&str, usize> = HashMap::new();
        let mut wasted_size = 0;
        for file in self.file_summary_list.iter() {
            let info = &file.info;
            wasted_size += info.size;
            if let Some(&i) = path_index.get(info.path.as_str()) {
                wasted_list[i].count += 1;
                wasted_list[i].total_size += info.size;
            } else {
                path_index.insert(&info.path, wasted_list.len());
                wasted_list.push(ImageFileWastedSummary {
                    path: info.path.clone(),
                    count: 1,
                    total_size: info.size,
                });
            }
        }
        wasted_list.sort_by_key(|b| Reverse(b.total_size));

        // Scratch / empty images have total_size == 0; avoid div-by-zero
        // (release builds use panic=abort so this would kill the process).
        let (score, wasted_percent) = match wasted_size
            .checked_mul(100)
            .and_then(|n| n.checked_div(self.total_size))
        {
            None => (100u64, 0.0f64),
            Some(wasted_pct) => {
                let mut score = 100 - wasted_pct;
                // 有浪费空间，则分数-1
                if wasted_size != 0 {
                    score = score.saturating_sub(1);
                }
                (score, (wasted_size as f64) / (self.total_size as f64))
            }
        };
        DockerAnalyzeSummary {
            wasted_list,
            wasted_size,
            wasted_percent,
            score,
        }
    }
}

impl DockerTokenInfo {
    /// Prefer `token`, fall back to `access_token` (OCI-style responses).
    fn bearer(&self) -> String {
        self.token
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| self.access_token.as_deref().filter(|s| !s.is_empty()))
            .unwrap_or("")
            .to_string()
    }

    // 判断docker token是否已过期
    fn expired(&self) -> bool {
        let issued_at = self.issued_at.as_deref().unwrap_or("");
        if let Ok(value) = DateTime::<Utc>::from_str(issued_at) {
            // 因为后续需要使用token获取数据
            // 因此提交10秒认为过期，避免请求时失效
            let offset = (self.expires_in.unwrap_or(600) - 10) as i64;
            let now = Utc::now().timestamp();
            return value.timestamp() + offset <= now;
        }
        false
    }
}

// 获取docker token的缓存实例
fn get_docker_token_cache() -> &'static Mutex<LruCache<String, DockerTokenInfo>> {
    static DOCKER_TOKEN_CACHE: OnceCell<Mutex<LruCache<String, DockerTokenInfo>>> = OnceCell::new();
    DOCKER_TOKEN_CACHE.get_or_init(|| {
        let c = LruCache::new(NonZeroUsize::new(100).unwrap());
        Mutex::new(c)
    })
}

// 从缓存中获取docker token
fn get_docker_token_from_cache(key: &str) -> Option<DockerTokenInfo> {
    if let Ok(mut cache) = get_docker_token_cache().lock() {
        if let Some(info) = cache.get(key) {
            if info.expired() {
                return None;
            }
            return Some(info.clone());
        }
    }
    None
}

// 将docker token写入缓存
fn set_docker_token_to_cache(key: &str, info: DockerTokenInfo) {
    if let Ok(mut cache) = get_docker_token_cache().lock() {
        cache.put(key.to_owned(), info);
    }
}

// 获取manifest缓存实例
fn get_manifest_cache() -> &'static Mutex<LruCache<String, ImageManifestCacheInfo>> {
    static MANIFEST_CACHE: OnceCell<Mutex<LruCache<String, ImageManifestCacheInfo>>> =
        OnceCell::new();
    MANIFEST_CACHE.get_or_init(|| {
        let c = LruCache::new(NonZeroUsize::new(100).unwrap());
        Mutex::new(c)
    })
}

fn get_manifest_from_cache(key: &str) -> Option<(ImageManifest, Vec<String>)> {
    if let Ok(mut cache) = get_manifest_cache().lock() {
        if let Some(info) = cache.get(key) {
            if info.expired_at > Utc::now().timestamp() {
                tracing::debug!(key, "manifest cache hit");
                return Some((info.manifest.clone(), info.supported_archs.clone()));
            }
        }
    }
    None
}

fn set_manifest_to_cache(
    key: &str,
    manifest: ImageManifest,
    supported_archs: Vec<String>,
    ttl_seconds: i64,
) {
    if let Ok(mut cache) = get_manifest_cache().lock() {
        cache.put(
            key.to_owned(),
            ImageManifestCacheInfo {
                expired_at: Utc::now().timestamp() + ttl_seconds,
                manifest,
                supported_archs,
            },
        );
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DockerRequestErrorResp {
    pub errors: Vec<DockerRequestError>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DockerRequestError {
    pub code: String,
    pub message: String,
}

fn get_value_from_json(v: &[u8], key: &str) -> Result<String> {
    let mut root: Value = serde_json::from_slice(v).context(SerdeJsonSnafu {
        category: "get_from_json",
    })?;
    for k in key.split('.') {
        let value = root.get(k);
        if value.is_none() {
            return Ok("".to_string());
        }
        root = value.unwrap().to_owned();
    }
    Ok(root.as_str().unwrap_or("").to_string())
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalManifest {
    #[serde(rename = "Config")]
    pub config: String,
    #[serde(rename = "RepoTags")]
    pub repo_tags: Vec<String>,
    #[serde(rename = "Layers")]
    pub layers: Vec<String>,
}

impl From<LocalManifest> for ImageManifest {
    fn from(value: LocalManifest) -> Self {
        let layers = value
            .layers
            .iter()
            .map(|layer| ImageManifestLayer {
                media_type: "application/vnd.docker.image.rootfs.diff.tar".to_string(),
                digest: layer.to_string(),
                ..Default::default()
            })
            .collect();
        ImageManifest {
            media_type: MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST.to_string(),
            schema_version: 2,
            config: ImageManifestConfig {
                digest: value.config,
                ..Default::default()
            },
            layers,
        }
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DockerImageParams {
    // 用户
    pub user: String,
    // 镜像
    pub img: String,
    // 镜像tag
    pub tag: String,
    // docker token
    pub token: String,
    // 镜像架构
    pub arch: String,
    // 生成建议时使用的语言（不参与序列化）
    #[serde(skip)]
    pub lang: crate::i18n::Lang,
    // 抑制 stderr 进度日志（web 模式下置 true，避免污染服务端日志）
    #[serde(skip)]
    pub quiet: bool,
    // 是否运行跨层重复文件检测（默认 true）；`--no-verify-dup` 关闭后
    // 还会同时跳过 analysis cache 的读写，避免缓存里出现「未检测」的
    // 不完整结果。
    #[serde(skip)]
    pub verify_dup: bool,
    /// Explicit registry credentials (CLI/env). When `None`,
    /// `get_auth_token` still tries `~/.docker/config.json`.
    #[serde(skip)]
    pub credentials: Option<super::registry_auth::RegistryCredentials>,
}

impl DockerImageParams {
    /// Full repository path for registry URLs (`user/name` or `name`).
    fn repo(&self) -> String {
        repository_path(&self.user, &self.img)
    }
}

// 将 `docker save` 的 stdout 直接重定向到临时文件（由内核完成写入），
// 不再把整个镜像 tar 缓冲进进程内存。stderr 与旧行为一致丢弃。
async fn save_local_docker_to_file(image: &str, path: &Path) -> Result<()> {
    tl_info!(image = image, "saving image");
    let file = File::create(path).map_err(|err| Error::IO { source: err })?;
    let status = tokio::process::Command::new("docker")
        .arg("save")
        .arg(image)
        .stdout(Stdio::from(file))
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(|err| Error::IO { source: err })?;
    if !status.success() {
        return Err(Error::Whatever {
            message: "docker save fail".to_string(),
        });
    }
    tl_info!(image = image, "save image done");
    Ok(())
}

// Short human-readable compression label for a layer's media type, used in
// the CLI progress lines. Mirrors the substring detection that `layer.rs`
// uses to pick the decompressor.
fn compression_label(media_type: &str) -> &'static str {
    if media_type.contains("zstd") {
        "zstd"
    } else if media_type.contains("gzip") {
        "gzip"
    } else {
        "tar"
    }
}

/// 逐层扫描（analyze 的 "diff" 阶段）的聚合产物。
struct LayerScan {
    layers: Vec<ImageLayer>,
    file_tree_list: Vec<Vec<FileTreeItem>>,
    file_summary_list: Vec<ImageFileSummary>,
    big_modified_file_list: Vec<BigModifiedFileInfo>,
    sensitive_files: Vec<SensitiveFileInfo>,
    image_size: u64,
    image_total_size: u64,
    has_pkg_cache: bool,
    has_dev_artifacts: bool,
}

/// 把镜像 history 与每层文件列表合并成分层视图：文件树、修改/删除汇总、
/// 大文件、敏感文件与启发式标记。纯计算、无 I/O，从 `analyze` 拆出以便
/// 独立阅读与测试。
fn scan_layers(
    config: &ImageConfig,
    manifest_layers: &[ImageManifestLayer],
    info_list: &[ImageLayerInfo],
) -> LayerScan {
    let mut image_created = 0;
    if let Some(value) = config.history.last() {
        if let Ok(value) = DateTime::parse_from_rfc3339(&value.created) {
            image_created = value.timestamp();
        }
    }
    let mut layers = vec![];
    let mut file_tree_list: Vec<Vec<FileTreeItem>> = vec![];
    let mut index = 0;
    let mut file_summary_list = vec![];
    let mut image_size = 0;
    let mut image_total_size = 0;
    // path → size for every file seen in previous layers; used for O(1) modification detection
    let mut seen_files: HashMap<String, u64> = HashMap::new();
    // Paths recorded in `file_summary_list` (modified/removed), maintained
    // incrementally. `convert_files_to_file_tree` consumes this set so it
    // no longer rebuilds one from the whole cumulative summary on every
    // layer (which was O(layers²) over the modified-file count).
    let mut modified_paths: HashSet<String> = HashSet::new();
    let mut big_modified_file_list = vec![];
    let mut sensitive_files: Vec<SensitiveFileInfo> = vec![];
    // dedup key: for .git/ files the key is the git-root prefix, otherwise the full path
    let mut sensitive_seen: HashSet<String> = HashSet::new();
    let mut has_pkg_cache = false;
    let mut has_dev_artifacts = false;
    for (layer_index, history) in config.history.iter().enumerate() {
        let is_new = if let Ok(value) = DateTime::parse_from_rfc3339(&history.created) {
            // 如果5分钟内
            image_created - value.timestamp() < 300
        } else {
            false
        };
        let empty = history.empty_layer.unwrap_or_default();
        let mut digest = "".to_string();
        let mut info = &ImageLayerInfo {
            ..Default::default()
        };
        let mut media_type = "".to_string();
        let mut size = 0;
        let mut file_tree = vec![];
        // 只有非空的layer需要获取files
        if !empty {
            // manifest中的layer只对应非空的操作
            if let Some(value) = manifest_layers.get(index) {
                info = info_list.get(index).unwrap();
                size = value.size;
                digest = value.digest.clone();
                media_type = value.media_type.clone();
                // single pass: detect modifications, update seen-files, collect big files
                for file in &info.files {
                    // OCI opaque whiteout: wipe every previously-seen path
                    // under this directory (and the dir itself).
                    if file.is_opaque == Some(true) {
                        let dir = file.path.as_str();
                        let doomed: Vec<(String, u64)> = seen_files
                            .iter()
                            .filter(|(p, _)| path_under_dir(p, dir))
                            .map(|(p, &sz)| (p.clone(), sz))
                            .collect();
                        for (path, prev_size) in doomed {
                            seen_files.remove(&path);
                            let mut file_info = file.clone();
                            file_info.path = path.clone();
                            file_info.size = prev_size;
                            file_info.is_opaque = None;
                            file_summary_list.push(ImageFileSummary {
                                layer_index,
                                op: Op::Removed,
                                info: file_info,
                            });
                            modified_paths.insert(path);
                        }
                        // Opaque marker is not real content — skip identity tracking.
                        continue;
                    }
                    if layer_index != 0 {
                        if let Some(&prev_size) = seen_files.get(&file.path) {
                            let op;
                            let mut file_info = file.clone();
                            if file.is_whiteout.is_some() {
                                op = Op::Removed;
                                file_info.size = prev_size;
                            } else {
                                // Re-adding a path always counts as Modified
                                // for efficiency (the new layer still stores
                                // the bytes), even when size/mode match.
                                op = Op::Modified;
                            }
                            file_summary_list.push(ImageFileSummary {
                                layer_index,
                                op,
                                info: file_info,
                            });
                            modified_paths.insert(file.path.clone());
                        }
                    }
                    if file.is_whiteout.is_some() {
                        seen_files.remove(&file.path);
                    } else {
                        seen_files.insert(file.path.clone(), file.size);
                    }
                    if is_new && file.size >= 1_000_000 && file.link.is_empty() {
                        big_modified_file_list.push(BigModifiedFileInfo {
                            path: file.path.clone(),
                            size: file.size,
                            digest: digest.clone(),
                            mode: file.mode.clone(),
                            uid: file.uid,
                            gid: file.gid,
                        });
                    }
                    // Heuristic tag detection
                    if !has_pkg_cache && is_pkg_cache(&file.path) {
                        has_pkg_cache = true;
                    }
                    if !has_dev_artifacts && is_dev_artifact(&file.path) {
                        has_dev_artifacts = true;
                    }
                    // Sensitive file scan (skip whiteout/deleted entries)
                    if file.is_whiteout.is_none() {
                        let user_cfg = load_user_sensitive_patterns();
                        let hit = if let Some(r) = is_sensitive_file(&file.path) {
                            // Built-in match — suppress if user explicitly ignores it
                            if user_cfg.is_ignored(&file.path) {
                                None
                            } else {
                                Some(r.to_string())
                            }
                        } else {
                            user_cfg.check(&file.path).map(|r| r.to_string())
                        };
                        if let Some(reason) = hit {
                            // For .git/ entries collapse to the git-root to avoid thousands of rows
                            let dedup_key = if let Some(pos) = file
                                .path
                                .find("/.git/")
                                .map(|p| p + 1)
                                .or_else(|| file.path.starts_with(".git/").then_some(0))
                            {
                                format!("{}/.git/", &file.path[..pos])
                            } else {
                                file.path.clone()
                            };
                            if sensitive_seen.insert(dedup_key.clone()) {
                                // For .git/ show the collapsed directory path
                                let display_path = if dedup_key.ends_with("/.git/") {
                                    dedup_key
                                } else {
                                    file.path.clone()
                                };
                                sensitive_files.push(SensitiveFileInfo {
                                    path: display_path,
                                    size: file.size,
                                    layer_index,
                                    reason,
                                });
                            }
                        }
                    }
                }
                image_size += info.size;
                image_total_size += info.unpack_size;
                // Uses the incrementally-maintained `modified_paths` set.
                file_tree = convert_files_to_file_tree(&info.files, &modified_paths);
            }
            index += 1;
        }

        let created_by = history.created_by.clone().unwrap_or_default();

        layers.push(ImageLayer {
            created: history.created.clone(),
            cmd: created_by,
            empty,
            digest,
            media_type,
            unpack_size: info.unpack_size,
            size,
        });
        file_tree_list.push(file_tree);
    }
    LayerScan {
        layers,
        file_tree_list,
        file_summary_list,
        big_modified_file_list,
        sensitive_files,
        image_size,
        image_total_size,
        has_pkg_cache,
        has_dev_artifacts,
    }
}

/// 从 image config 提取运行用户 / 环境变量 / label 列表。
fn extract_image_meta(config: &ImageConfig) -> (String, Vec<String>, Vec<String>) {
    let mut run_user = "".to_string();
    let mut envs = vec![];
    let mut labels = vec![];
    if let Some(ref extra_info) = config.config {
        if let Some(ref value) = extra_info.user {
            run_user = value.to_string();
        }
        if let Some(ref value) = extra_info.env {
            envs = value.clone();
        }
        if let Some(ref value) = extra_info.labels {
            for (k, v) in value.iter() {
                labels.push(format!("{k}={v}"));
            }
        }
    }
    (run_user, envs, labels)
}

/// 启发式风险标签（enrich 阶段的一部分）。
fn build_risk_tags(scan: &LayerScan, run_user: &str) -> Vec<String> {
    let mut tags: Vec<String> = vec![];
    if scan.has_pkg_cache {
        tags.push("[Contains Package Manager Cache]".to_string());
    }
    if scan.has_dev_artifacts {
        tags.push("[Development Artifacts]".to_string());
    }
    if !scan.sensitive_files.is_empty() {
        tags.push("[Potential Secrets]".to_string());
    }
    if run_user.is_empty() || run_user == "root" {
        tags.push("[Runs as Root]".to_string());
    }
    if scan.layers.len() > 30 {
        tags.push(format!("[High Layer Count: {}]", scan.layers.len()));
    }
    tags
}

impl DockerClient {
    pub fn new(register: &str) -> Self {
        DockerClient {
            registry: register.to_string(),
            ..Default::default()
        }
    }
    fn is_local(&self) -> bool {
        self.registry == REGISTRY_LOCAL_FILE
    }
    /// Build (once, on the blocking pool) and share the one-pass index of
    /// the local image tar. 此前 manifest / 每层 size / 每层内容各自全量
    /// 扫一遍 tar，整体是 O(层数 × tar 大小)；索引化后只扫一次。
    async fn local_tar_index(&self, tar: &str) -> Result<Arc<TarIndex>> {
        let index = self
            .local_index
            .get_or_try_init(|| async {
                let tar = tar.to_string();
                tokio::task::spawn_blocking(move || TarIndex::build(&tar))
                    .await
                    .map_err(|e| Error::Whatever {
                        message: format!("tar index task failed: {e}"),
                    })?
                    .context(LayerSnafu {})
                    .map(Arc::new)
            })
            .await?;
        Ok(index.clone())
    }
    async fn get_local_manifest(&self, image: &str) -> Result<LocalManifest> {
        let index = self.local_tar_index(image).await?;
        // manifest.json 只有几 KB：seek + 一次小读，无需再进 blocking pool
        let data = index.read("manifest.json").context(LayerSnafu {})?;

        let manifest_list =
            serde_json::from_slice::<Vec<LocalManifest>>(&data).context(SerdeJsonSnafu {
                category: "get_local_manifest",
            })?;
        if manifest_list.is_empty() {
            return Err(Error::Whatever {
                message: "Local Manifest Not Found".to_string(),
            });
        }
        Ok(manifest_list[0].clone())
    }
    async fn send_request(
        &self,
        url: String,
        headers: HashMap<String, String>,
    ) -> Result<reqwest::Response> {
        // Single retry on HTTP 429. Docker Hub anonymous pulls are
        // throttled tightly (e.g. 200 requests / 6h / IP), and one short
        // wait usually clears the rate-limit window. We honor the
        // `Retry-After` header when present (integer seconds), fall back
        // to 5s otherwise, and cap at 60s so a misconfigured registry
        // can't stall the analysis indefinitely.
        const MAX_ATTEMPTS: u32 = 2;
        const MAX_BACKOFF_SECS: u64 = 60;
        const DEFAULT_BACKOFF_SECS: u64 = 5;

        for attempt in 1..=MAX_ATTEMPTS {
            let mut builder = get_http_client()
                .get(url.clone())
                .timeout(Duration::from_secs(30 * 60));
            for (key, value) in headers.iter() {
                builder = builder.header(key.as_str(), value.as_str());
            }
            let resp = builder
                .send()
                .await
                .context(RequestSnafu { url: url.clone() })?;
            let status = resp.status();
            if status.as_u16() == 429 && attempt < MAX_ATTEMPTS {
                let wait_secs = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.trim().parse::<u64>().ok())
                    .unwrap_or(DEFAULT_BACKOFF_SECS)
                    .min(MAX_BACKOFF_SECS);
                tl_info!(
                    url = url.as_str(),
                    wait_secs = wait_secs,
                    attempt = attempt,
                    "registry returned 429; backing off before retry"
                );
                tokio::time::sleep(Duration::from_secs(wait_secs)).await;
                continue;
            }
            if status.as_u16() >= StatusCode::UNAUTHORIZED.as_u16() {
                let status_code = status.as_u16();
                // Prefer the registry error body when present; never index
                // an empty `errors` array (non-standard registries).
                let (message, code) = match resp.json::<DockerRequestErrorResp>().await {
                    Ok(body) => body
                        .errors
                        .into_iter()
                        .next()
                        .map(|e| (e.message, e.code))
                        .unwrap_or_else(|| {
                            (
                                format!("registry returned HTTP {status_code}"),
                                status_code.to_string(),
                            )
                        }),
                    Err(_) => (
                        format!("registry returned HTTP {status_code}"),
                        status_code.to_string(),
                    ),
                };
                return Err(Error::Docker {
                    message,
                    code,
                    url,
                    status: status_code,
                });
            }
            return Ok(resp);
        }
        unreachable!("send_request retry loop always returns or continues")
    }

    async fn get_bytes(
        &self,
        url: String,
        headers: HashMap<String, String>,
    ) -> Result<bytes::Bytes> {
        let resp = self.send_request(url.clone(), headers).await?;
        resp.bytes().await.context(JsonSnafu { url })
    }

    /// Stream a blob to disk, verify it, and move it into place.
    ///
    /// The download lands in a same-directory temp file; only after the
    /// content sha256 matches `expected_digest` is it atomically renamed to
    /// `path`. A crash mid-download, a truncated body, or a concurrent
    /// analysis sharing this base layer can therefore never leave a
    /// partial/corrupt blob at the final cache path.
    async fn download_blob_to_path(
        &self,
        url: String,
        headers: HashMap<String, String>,
        path: &Path,
        expected_digest: &str,
    ) -> Result<()> {
        let tmp = tmp_sibling_path(path);
        let result = async {
            self.stream_blob_with_retry(url, headers, &tmp).await?;
            verify_blob_digest(&tmp, expected_digest).await?;
            tokio::fs::rename(&tmp, path).await.context(IOSnafu {})
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&tmp).await;
        }
        result
    }

    /// Stream a blob response directly to disk without buffering the whole
    /// body. Large layers from registry CDNs occasionally have the connection
    /// reset mid-transfer (surfaces as reqwest "error decoding response
    /// body"); retry a bounded number of times, resuming from the bytes
    /// already on disk via an HTTP `Range` request so a multi-hundred-MiB
    /// layer is not re-fetched from scratch.
    async fn stream_blob_with_retry(
        &self,
        url: String,
        headers: HashMap<String, String>,
        path: &Path,
    ) -> Result<()> {
        const MAX_ATTEMPTS: usize = 4;
        let mut downloaded: u64 = 0;
        for attempt in 1..=MAX_ATTEMPTS {
            let mut req_headers = headers.clone();
            // Resume from where the previous attempt stopped.
            if downloaded > 0 {
                req_headers.insert("Range".to_string(), format!("bytes={downloaded}-"));
            }
            match self
                .stream_blob_once(url.clone(), req_headers, path, &mut downloaded)
                .await
            {
                Ok(()) => return Ok(()),
                // Only request/stream interruptions are transient; auth and
                // other errors are permanent and must surface immediately.
                Err(err @ Error::Request { .. }) if attempt < MAX_ATTEMPTS => {
                    tl_info!(
                        url = url,
                        attempt,
                        downloaded,
                        err = err.to_string(),
                        "blob download interrupted, retrying"
                    );
                    tokio::time::sleep(Duration::from_secs(attempt as u64)).await;
                }
                Err(err) => return Err(err),
            }
        }
        unreachable!("loop returns on success or on the final attempt's error")
    }

    /// One streaming attempt. `downloaded` tracks bytes persisted so far and
    /// is updated as chunks land; on a fresh start (or when the server ignores
    /// `Range` and replies `200`) the file is truncated and `downloaded` reset.
    async fn stream_blob_once(
        &self,
        url: String,
        headers: HashMap<String, String>,
        path: &Path,
        downloaded: &mut u64,
    ) -> Result<()> {
        let resuming = *downloaded > 0;
        let resp = self.send_request(url.clone(), headers).await?;
        let resumed = resuming && resp.status().as_u16() == StatusCode::PARTIAL_CONTENT.as_u16();
        let mut file = if resumed {
            // Server honored Range: append after the bytes already on disk.
            let mut f = tokio::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .await
                .context(IOSnafu {})?;
            f.seek(SeekFrom::Start(*downloaded))
                .await
                .context(IOSnafu {})?;
            f
        } else {
            // Fresh download, or server ignored Range and sent the full body.
            *downloaded = 0;
            tokio::fs::File::create(path).await.context(IOSnafu {})?
        };
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context(RequestSnafu { url: url.clone() })?;
            file.write_all(&chunk).await.context(IOSnafu {})?;
            *downloaded += chunk.len() as u64;
        }
        file.flush().await.context(IOSnafu {})?;
        Ok(())
    }
    async fn get<T: DeserializeOwned>(
        &self,
        url: String,
        headers: HashMap<String, String>,
    ) -> Result<T> {
        let data = self.get_bytes(url.clone(), headers).await?;
        let result = serde_json::from_slice(&data).context(SerdeJsonSnafu {
            category: "request",
        })?;
        Ok(result)
    }
    // 获取manifest，同时返回该镜像支持的架构列表（linux平台）
    pub async fn get_manifest(
        &self,
        params: &DockerImageParams,
    ) -> Result<(ImageManifest, Vec<String>)> {
        let img = &params.img;
        let tag = &params.tag;
        let token = &params.token;
        if self.is_local() {
            let local_manifest = self.get_local_manifest(img).await?;
            let mut image_manifest: ImageManifest = local_manifest.into();
            let index = self.local_tar_index(img).await?;
            for layer in image_manifest.layers.iter_mut() {
                // 与旧 tar_size 语义一致：条目缺失按 0 处理
                layer.size = index.size_of(&layer.digest).unwrap_or(0);
            }
            return Ok((image_manifest, vec![]));
        }

        let repo = params.repo();
        let url = format!("{}/{repo}/manifests/{tag}", self.registry);
        let key = format!("{url}:{}", params.arch);
        if let Some(cached) = get_manifest_from_cache(&key) {
            return Ok(cached);
        }
        tl_info!(url = url, "getting manifest");
        let mut headers = HashMap::new();
        if !token.is_empty() {
            headers.insert("Authorization".to_string(), format!("Bearer {token}"));
        }
        let accepts = [
            MEDIA_TYPE_IMAGE_INDEX,
            MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST,
            MEDIA_TYPE_MANIFEST_LIST,
        ];
        headers.insert("Accept".to_string(), accepts.join(", "));
        let data = self.get_bytes(url.clone(), headers).await?;
        let media_type = get_value_from_json(&data, "mediaType")?;
        let (resp, supported_archs): (ImageManifest, Vec<String>) = if media_type
            == MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST
        {
            let manifest = serde_json::from_slice(&data).context(SerdeJsonSnafu {
                category: "get_manifest_schema2",
            })?;
            (manifest, vec![])
        } else {
            let index = serde_json::from_slice::<ImageIndex>(&data).context(SerdeJsonSnafu {
                category: "guess_manifest",
            })?;
            // Collect linux platform architectures from the index
            let archs = index
                .manifests
                .iter()
                .filter(|m| m.platform.os == "linux")
                .map(|m| match &m.platform.variant {
                    Some(v) => format!("{}/{}", m.platform.architecture, v),
                    None => m.platform.architecture.clone(),
                })
                .collect();
            let chosen = index
                .guess_manifest(&params.arch)
                .ok_or_else(|| Error::Whatever {
                    message: format!("image index of {url} contains no manifests"),
                })?;
            tl_info!(arch = chosen.platform.architecture, "guess manifest");
            let mut headers = HashMap::new();
            if !token.is_empty() {
                headers.insert("Authorization".to_string(), format!("Bearer {token}"));
            }
            headers.insert("Accept".to_string(), chosen.media_type);
            let url = format!("{}/{repo}/manifests/{}", self.registry, chosen.digest);
            let data = self.get_bytes(url.clone(), headers).await?;
            let manifest = serde_json::from_slice(&data).context(SerdeJsonSnafu {
                category: "get_manifest",
            })?;
            (manifest, archs)
        };
        let mutable_tags = [
            "latest", "edge", "stable", "nightly", "beta", "alpha", "main",
        ];
        let ttl = if mutable_tags.contains(&params.tag.as_str()) {
            5 * 60
        } else {
            60 * 60
        };
        set_manifest_to_cache(&key, resp.clone(), supported_archs.clone(), ttl);
        tl_info!(url = url, "got manifest");
        Ok((resp, supported_archs))
    }
    // 获取镜像的信息
    pub async fn get_image_config(&self, params: &DockerImageParams) -> Result<ImageConfig> {
        let img = &params.img;
        let data = if self.is_local() {
            let local_manifest = self.get_local_manifest(img).await?;
            let index = self.local_tar_index(img).await?;
            index.read(&local_manifest.config).context(LayerSnafu {})?
        } else {
            let (manifest, _) = self.get_manifest(params).await?;
            self.get_blob(params, &manifest.config.digest).await?
        };

        let result = serde_json::from_slice(&data).context(SerdeJsonSnafu {
            category: "get_image_config",
        })?;
        Ok(result)
    }
    // 获取镜像分层的blob
    pub async fn get_blob(&self, params: &DockerImageParams, digest: &str) -> Result<Vec<u8>> {
        // 忽略出错，如果出错直接从网络加载；缓存内容与 digest 不符（如旧
        // 版本非原子写入残留的半截文件）同样按 miss 处理重新下载。
        if let Ok(data) = get_blob_from_file(digest).await {
            if bytes_match_digest(&data, digest) {
                tl_info!(digest, "blob cache hit");
                return Ok(data);
            }
            tl_info!(digest, "cached blob digest mismatch, refetching");
        }
        let token = &params.token;
        let url = format!("{}/{}/blobs/{digest}", self.registry, params.repo());
        tl_info!(url = url, "getting blob");
        let mut headers = HashMap::new();
        if !token.is_empty() {
            headers.insert("Authorization".to_string(), format!("Bearer {token}"));
        }
        let resp = self.get_bytes(url.clone(), headers).await?;
        if !bytes_match_digest(&resp, digest) {
            return Err(Error::Whatever {
                message: format!("blob digest mismatch for {digest}"),
            });
        }

        // 出错忽略
        // 写入数据失败不影响后续
        let _ = save_blob_to_file(digest, &resp).await;
        tl_info!(url = url, "got blob");
        Ok(resp.to_vec())
    }
    async fn get_layer_files(
        &self,
        params: &DockerImageParams,
        layer: ImageManifestLayer,
    ) -> Result<ImageLayerInfo> {
        let img = &params.img;
        if self.is_local() {
            let index = self.local_tar_index(img).await?;
            let media_type = layer.media_type.clone();
            let digest = layer.digest.clone();
            // 通过索引 seek 到层数据后流式解析，不再把整层缓冲进内存。
            // CPU 密集的解压照旧放 blocking pool（而非 block_in_place，
            // 避免占住 runtime worker 饿死其它任务的异步 I/O）。
            return tokio::task::spawn_blocking(move || {
                let (reader, size) = index.open_reader(&digest).context(LayerSnafu {})?;
                get_files_from_layer(BufReader::new(reader), &media_type, size)
                    .context(LayerSnafu {})
            })
            .await
            .map_err(|_| Error::Whatever {
                message: "decompression task join error".to_string(),
            })?;
        }

        let path = get_blob_path(&layer.digest);
        let is_cached = path.exists()
            && std::fs::metadata(&path)
                .map(|m| m.len() == layer.size)
                .unwrap_or(false);

        if !is_cached {
            let token = &params.token;
            let url = format!("{}/{}/blobs/{}", self.registry, params.repo(), layer.digest);
            tl_info!(url = url, "getting blob");
            if !params.quiet {
                eprintln!(
                    "{}",
                    i18n::fill(
                        i18n::tr(params.lang, "prog.download"),
                        &[
                            &layer.digest[..layer.digest.len().min(19)],
                            &bytesize::ByteSize(layer.size).to_string(),
                            compression_label(&layer.media_type),
                        ]
                    )
                );
            }
            let mut headers = HashMap::new();
            if !token.is_empty() {
                headers.insert("Authorization".to_string(), format!("Bearer {token}"));
            }
            if let Err(err) = self
                .download_blob_to_path(url.clone(), headers, &path, &layer.digest)
                .await
            {
                // 大镜像的分析可能超过 bearer token 的有效期（Docker Hub
                // 约 5 分钟）；401 说明 token 在分析中途失效，强制刷新后
                // 重试一次，其余错误直接上抛。
                if !is_unauthorized(&err) {
                    return Err(err);
                }
                tl_info!(digest = layer.digest, "blob unauthorized, refreshing token");
                let token = self.refresh_auth_token(params).await?;
                let mut headers = HashMap::new();
                if !token.is_empty() {
                    headers.insert("Authorization".to_string(), format!("Bearer {token}"));
                }
                self.download_blob_to_path(url.clone(), headers, &path, &layer.digest)
                    .await?;
            }
            tl_info!(url = url, "got blob");
        } else {
            tl_info!(digest = layer.digest, "blob cache hit");
            if !params.quiet {
                eprintln!(
                    "{}",
                    i18n::fill(
                        i18n::tr(params.lang, "prog.cached"),
                        &[
                            &layer.digest[..layer.digest.len().min(19)],
                            &bytesize::ByteSize(layer.size).to_string(),
                            compression_label(&layer.media_type),
                        ]
                    )
                );
            }
        }

        let compressed_size = std::fs::metadata(&path)
            .map(|m| m.len())
            .unwrap_or(layer.size);
        let media_type = layer.media_type.clone();
        // See the local-file branch above: decompression runs on the blocking
        // pool, not via `block_in_place`, to avoid parking runtime workers.
        tokio::task::spawn_blocking(move || {
            let file = File::open(&path).context(IOSnafu {})?;
            get_files_from_layer(BufReader::new(file), &media_type, compressed_size)
                .context(LayerSnafu {})
        })
        .await
        .map_err(|_| Error::Whatever {
            message: "decompression task join error".to_string(),
        })?
    }
    async fn get_all_layer_info(
        &self,
        params: DockerImageParams,
        layers: Vec<ImageManifestLayer>,
    ) -> Result<Vec<ImageLayerInfo>> {
        // 无 scope 时降级为空 traceId（库调用方 / 测试可能不设置），
        // 与 tl_* 宏的 try_with 行为保持一致。
        let trace_id = TRACE_ID
            .try_with(clone_value_from_task_local)
            .unwrap_or_default();
        // Cap concurrent download/decompress work. Defaults to
        // `min(layers, 2×CPUs)`; override with `layer_concurrency` (or legacy
        // `threads`) in config.yml. Oversubscribing past ~2×CPUs just adds
        // scheduler contention on the CPU-bound decompress path.
        let threads = get_layer_concurrency(layers.len());
        let sem = Arc::new(tokio::sync::Semaphore::new(threads));

        let mut handles = Vec::with_capacity(layers.len());
        for layer in layers {
            let s = self.clone();
            let p = params.clone();
            let sem = sem.clone();
            let tid = trace_id.clone();
            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.expect("semaphore closed");
                TRACE_ID.scope(tid, s.get_layer_files(&p, layer)).await
            }));
        }

        let mut info_list = Vec::with_capacity(handles.len());
        for handle in handles {
            let info = handle.await.map_err(|_| Error::Whatever {
                message: "task join error".to_string(),
            })??;
            info_list.push(info);
        }
        Ok(info_list)
    }
    async fn get_auth_token(&self, params: &DockerImageParams) -> Result<String> {
        self.fetch_auth_token(params, false).await
    }
    /// 强制重新获取 token（跳过缓存读取，仍会回写缓存）。用于分析中途
    /// token 提前失效（registry 返回 401 而缓存仍认为未过期）的场景。
    async fn refresh_auth_token(&self, params: &DockerImageParams) -> Result<String> {
        self.fetch_auth_token(params, true).await
    }
    async fn fetch_auth_token(
        &self,
        params: &DockerImageParams,
        skip_cache: bool,
    ) -> Result<String> {
        // 本地文件无需token
        if self.is_local() {
            return Ok("".to_string());
        }
        let tag = &params.tag;
        let url = format!("{}/{}/manifests/{tag}", self.registry, params.repo());
        let builder = get_http_client()
            .head(url.clone())
            .timeout(Duration::from_secs(5 * 60));
        let resp = builder
            .send()
            .await
            .context(RequestSnafu { url: url.clone() })?;
        if resp.status().as_u16() == StatusCode::UNAUTHORIZED.as_u16() {
            if let Some(value) = resp.headers().get("www-authenticate") {
                let auth_info = parse_auth_info(value.to_str().unwrap_or_default())?;
                let url = format!(
                    "{}?service={}&scope={}",
                    auth_info.auth, auth_info.service, auth_info.scope
                );
                // Cache key includes whether we have credentials so an
                // anonymous token is not reused after the user supplies
                // a login (and vice versa).
                let creds = super::registry_auth::resolve_for_registry(
                    &self.registry,
                    params.credentials.as_ref(),
                );
                let cache_key = match &creds {
                    Some(c) => format!("{url}#user={}", c.username),
                    None => url.clone(),
                };
                if !skip_cache {
                    if let Some(info) = get_docker_token_from_cache(&cache_key) {
                        if !info.expired() {
                            return Ok(info.bearer());
                        }
                    }
                }
                tl_info!(url = url, authenticated = creds.is_some(), "getting token");
                let mut headers = HashMap::new();
                if let Some(c) = creds.as_ref() {
                    headers.insert(
                        "Authorization".to_string(),
                        format!("Basic {}", c.basic_token()),
                    );
                }
                let mut resp = self.get::<DockerTokenInfo>(url.clone(), headers).await?;
                let bearer = resp.bearer();
                if bearer.is_empty() {
                    return Err(Error::Whatever {
                        message: "registry auth returned empty token".to_string(),
                    });
                }
                if resp.issued_at.is_none() {
                    resp.issued_at = Some(Utc::now().to_rfc3339());
                }
                set_docker_token_to_cache(&cache_key, resp.clone());
                tl_info!(url = url, "got token");
                return Ok(bearer);
            }
        }
        Ok("".to_string())
    }
    /// Issue HEAD on the manifest endpoint to obtain the
    /// `Docker-Content-Digest` header — used as a content-addressable
    /// cache key for the full analysis result. Any failure (network,
    /// 4xx/5xx, missing header, malformed value) returns `None` so the
    /// caller falls through to the full analysis. Never bubbles an error.
    async fn head_manifest_digest(&self, params: &DockerImageParams) -> Option<String> {
        if self.is_local() {
            return None;
        }
        let tag = &params.tag;
        let token = &params.token;
        let url = format!("{}/{}/manifests/{tag}", self.registry, params.repo());
        let client = get_http_client();
        let accepts = [
            MEDIA_TYPE_IMAGE_INDEX,
            MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST,
            MEDIA_TYPE_MANIFEST_LIST,
        ];
        let mut req = client.head(url).timeout(Duration::from_secs(30));
        req = req.header("Accept", accepts.join(", "));
        if !token.is_empty() {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        let resp = req.send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.headers()
            .get("Docker-Content-Digest")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    }

    pub async fn analyze(&self, params: &mut DockerImageParams) -> Result<DockerAnalyzeResult> {
        if !self.is_local() && !params.quiet {
            eprintln!(
                "{}",
                i18n::fill(i18n::tr(params.lang, "prog.auth"), &[&self.registry])
            );
        }
        let token = self.get_auth_token(params).await?;
        params.token = token;

        // Analysis-result cache. HEAD the manifest to get a
        // `Docker-Content-Digest`; if present and the cache file exists +
        // matches the current schema, return the cached result and skip
        // the entire layer-fetch + file-tree pipeline. Any HEAD failure
        // (network/4xx/5xx/missing header) is swallowed so the normal
        // flow always remains the fallback path.
        // Only consult / populate the analysis cache when duplicate
        // detection is enabled — otherwise we'd risk reading or writing an
        // incomplete result. With `--no-verify-dup`, we go straight to a
        // fresh analyze + skip cache write at the end.
        let cache_digest: Option<String> = if params.verify_dup {
            self.head_manifest_digest(params).await
        } else {
            None
        };
        if let Some(digest) = cache_digest.as_deref() {
            if let Some(mut cached) = read_analysis(digest, &params.arch).await {
                tl_info!(digest = digest, "analysis cache hit");
                if !params.quiet {
                    let short = &digest[..digest.len().min(19)];
                    eprintln!(
                        "{}",
                        i18n::fill(i18n::tr(params.lang, "prog.cache.hit"), &[short])
                    );
                }
                // Recommendations are language-specific and are not stored
                // in the cache — rebuild for the current request's lang.
                cached.recommendations = build_recommendations(&cached, params.lang);
                return Ok(cached);
            }
        }

        if !self.is_local() && !params.quiet {
            eprintln!("{}", i18n::tr(params.lang, "prog.manifest"));
        }
        let (manifest, supported_archs) = self.get_manifest(params).await?;
        let config = self.get_image_config(params).await?;
        let repo = params.repo();
        let tag = &params.tag;

        tl_info!(repo = repo.as_str(), tag = tag, "analyzing image",);

        if !self.is_local() && !params.quiet {
            let layer_count = manifest.layers.len();
            let total_bytes: u64 = manifest.layers.iter().map(|l| l.size).sum();
            eprintln!(
                "{}",
                i18n::fill(
                    i18n::tr(params.lang, "prog.layers"),
                    &[
                        &layer_count.to_string(),
                        &bytesize::ByteSize(total_bytes).to_string(),
                    ]
                )
            );
        }
        let info_list = self
            .get_all_layer_info(params.clone(), manifest.layers.clone())
            .await?;
        // 纯计算的逐层合并（diff 阶段）：文件树、修改/删除汇总、大文件、
        // 敏感文件与启发式标记。见 `scan_layers`。
        let scan = scan_layers(&config, &manifest.layers, &info_list);

        tl_info!(repo = repo.as_str(), tag = tag, "analyze image done",);
        let image_name = format!("{repo}:{tag}");
        let (run_user, envs, labels) = extract_image_meta(&config);

        // OS fingerprinting: probe cached blobs, fall back to history, then "Unknown".
        // Computed up-front so the ELF runtime-compat probe below can compare the
        // entrypoint binary's libc requirements against the host's glibc version.
        let base_os = if !self.is_local() {
            // 与其它重 I/O 一致走 blocking pool（block_in_place 会占住一个
            // runtime worker）。manifest 此后不再使用，直接 move 进闭包。
            let base_os = tokio::task::spawn_blocking(move || probe_base_os(&manifest))
                .await
                .map_err(|e| Error::Whatever {
                    message: format!("base os probe task failed: {e}"),
                })?;
            if base_os.is_empty() {
                detect_os_from_history(&scan.layers)
                    .unwrap_or_else(|| "Scratch / Distroless (no OS identifier found)".to_string())
            } else {
                base_os
            }
        } else {
            detect_os_from_history(&scan.layers).unwrap_or_default()
        };

        let tags = build_risk_tags(&scan, &run_user);
        let LayerScan {
            layers,
            file_tree_list,
            file_summary_list,
            big_modified_file_list,
            sensitive_files,
            image_size,
            image_total_size,
            ..
        } = scan;

        // Cross-layer duplicate detection AND ELF runtime-compat probing
        // both decompress + read layer blobs from disk (CPU + blocking I/O),
        // so they share one `spawn_blocking` hop. The owned `layers` +
        // `file_tree_list` are moved in and handed back out — zero clone.
        // Dup detection alone is skipped under `--no-verify-dup` for
        // performance-sensitive CI; the ELF probe always runs because its
        // cost is bounded (single entrypoint binary, ≤64 MB).
        let verify_dup = params.verify_dup;
        let config_for_probe = config.clone();
        let base_os_for_probe = base_os.clone();
        let (layers, file_tree_list, duplicate_groups, runtime_compat) =
            tokio::task::spawn_blocking(move || {
                let dup = if verify_dup {
                    detect_cross_layer_duplicates(&layers, &file_tree_list)
                } else {
                    vec![]
                };
                let rc = analyze_runtime_compat(
                    &config_for_probe,
                    &layers,
                    &file_tree_list,
                    &base_os_for_probe,
                )
                .unwrap_or_default();
                (layers, file_tree_list, dup, rc)
            })
            .await
            .map_err(|e| Error::Whatever {
                message: format!("post-analysis probe task failed: {e}"),
            })?;

        let mut result = DockerAnalyzeResult {
            name: image_name,
            arch: config.architecture,
            os: config.os,
            user: run_user,
            envs,
            labels,
            dockerfile: reconstruct_dockerfile(&config.history),
            base_os,
            supported_archs,
            layers,
            size: image_size,
            total_size: image_total_size,
            file_tree_list,
            file_summary_list,
            big_modified_file_list,
            sensitive_files,
            tags,
            recommendations: vec![],
            duplicate_groups,
            runtime_compat,
        };
        // Pure derived layer — computed from the result that is already built.
        // Localized at generation time from the resolved environment language.
        result.recommendations = build_recommendations(&result, params.lang);

        // Best-effort cache write. Skips silently on any error (logged
        // inside `write_analysis`) and only runs when HEAD earlier gave
        // us a digest to key on.
        if let Some(digest) = cache_digest.as_deref() {
            write_analysis(digest, &params.arch, &result).await;
        }

        Ok(result)
    }
}

pub async fn analyze_docker_image(
    image_info: ImageInfo,
    lang: crate::i18n::Lang,
    quiet: bool,
    verify_dup: bool,
    credentials: Option<super::registry_auth::RegistryCredentials>,
) -> Result<DockerAnalyzeResult> {
    if image_info.registry == REGISTRY_LOCAL_DOCKER {
        // 临时文件在 analyze 完成前保持存活（NamedTempFile drop 时自动删除）
        let tmpfile = tempfile::Builder::new()
            .tempfile()
            .map_err(|err| Error::IO { source: err })?;
        let filename = tmpfile.path().to_string_lossy().to_string();
        save_local_docker_to_file(&image_info.name, tmpfile.path()).await?;

        let c = DockerClient::new(REGISTRY_LOCAL_FILE);
        c.analyze(&mut DockerImageParams {
            img: filename,
            lang,
            quiet,
            verify_dup,
            credentials,
            ..Default::default()
        })
        .await
    } else {
        let c = DockerClient::new(&image_info.registry);
        c.analyze(&mut DockerImageParams {
            user: image_info.user,
            img: image_info.name,
            tag: image_info.tag,
            arch: image_info.arch,
            lang,
            quiet,
            verify_dup,
            credentials,
            ..Default::default()
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_handles_zero_total_size() {
        let result = DockerAnalyzeResult {
            total_size: 0,
            ..Default::default()
        };
        let summary = result.summary();
        assert_eq!(summary.score, 100);
        assert_eq!(summary.wasted_percent, 0.0);
        assert_eq!(summary.wasted_size, 0);
    }

    #[test]
    fn docker_token_accepts_both_token_and_access_token() {
        // Docker Hub returns both fields; serde(alias) would error with
        // "duplicate field `token`".
        let json = r#"{
            "token": "tok-primary",
            "access_token": "tok-primary",
            "expires_in": 300,
            "issued_at": "2024-01-01T00:00:00Z"
        }"#;
        let info: DockerTokenInfo = serde_json::from_str(json).expect("both fields");
        assert_eq!(info.bearer(), "tok-primary");
    }

    #[test]
    fn docker_token_falls_back_to_access_token() {
        let json = r#"{"access_token": "oci-only", "expires_in": 60}"#;
        let info: DockerTokenInfo = serde_json::from_str(json).expect("access_token only");
        assert_eq!(info.bearer(), "oci-only");
    }

    #[test]
    fn bytes_match_digest_verifies_sha256_only() {
        // sha256("abc")
        let digest = "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(bytes_match_digest(b"abc", digest));
        assert!(!bytes_match_digest(b"abd", digest));
        // Non-sha256 digests (local tar layouts) are not verifiable.
        assert!(bytes_match_digest(b"anything", "layer.tar"));
    }

    #[test]
    fn unauthorized_detection_uses_http_status() {
        assert!(is_unauthorized(&Error::Docker {
            message: "expired".to_string(),
            code: "UNAUTHORIZED".to_string(),
            url: "u".to_string(),
            status: 401,
        }));
        assert!(!is_unauthorized(&Error::Docker {
            message: "denied".to_string(),
            code: "DENIED".to_string(),
            url: "u".to_string(),
            status: 403,
        }));
        assert!(!is_unauthorized(&Error::Whatever {
            message: "x".to_string(),
        }));
    }

    #[test]
    fn repository_path_joins_namespace() {
        assert_eq!(repository_path("org", "proj/img"), "org/proj/img");
        assert_eq!(repository_path("", "solo"), "solo");
    }
}
