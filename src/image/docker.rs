use crate::config::{load_user_sensitive_patterns, must_load_config};
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
use std::io::{BufReader, Write};
use std::{collections::HashMap, num::NonZeroUsize, str::FromStr, sync::Mutex, time::Duration};
use substring::Substring;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use super::{
    get_file_content_from_tar, get_file_size_from_tar, get_files_from_layer,
    get_os_release_from_layer,
};
use super::{
    layer::ImageLayerInfo,
    oci_image::{
        detect_cross_layer_duplicates, DuplicateGroup, ImageFileSummary, ImageHistory,
        ImageManifestLayer,
    },
    FileTreeItem, ImageConfig, ImageIndex, ImageLayer, ImageManifest, ImageManifestConfig, Op,
    MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST, MEDIA_TYPE_IMAGE_INDEX, MEDIA_TYPE_MANIFEST_LIST,
};
use crate::{
    error::HTTPError,
    image::convert_files_to_file_tree,
    store::{get_blob_from_file, get_blob_path, save_blob_to_file},
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

static REGISTRY: &str = "https://index.docker.io/v2";

static REGISTRY_LOCAL_FILE: &str = "local-file";
static REGISTRY_LOCAL_DOCKER: &str = "local-docker";

#[derive(Debug, Clone, Default)]
pub struct ImageInfo {
    // 镜像对应的registry
    pub registry: String,
    // 镜像用户
    pub user: String,
    // 镜像名称
    pub name: String,
    // 镜像版本
    pub tag: String,
    // 镜像架构
    pub arch: String,
}

static FILE_PROTOCOL: &str = "file://";
static LOCAL_DOCKER_PROTOCOL: &str = "docker://";

pub fn parse_image_info(image: &str) -> ImageInfo {
    let mut value = image.to_string();
    if value.starts_with(FILE_PROTOCOL) {
        return ImageInfo {
            registry: REGISTRY_LOCAL_FILE.to_string(),
            name: value.replace(FILE_PROTOCOL, ""),
            ..Default::default()
        };
    }
    if value.starts_with(LOCAL_DOCKER_PROTOCOL) {
        return ImageInfo {
            registry: REGISTRY_LOCAL_DOCKER.to_string(),
            name: value.replace(LOCAL_DOCKER_PROTOCOL, ""),
            ..Default::default()
        };
    }
    let mut arch = "".to_string();
    if let Some(index) = value.find('?') {
        let query = value.substring(index + 1, value.len());
        for item in query.split('&') {
            let arr: Vec<&str> = item.split('=').collect();
            if arr.len() == 2 && arr[0] == "arch" {
                arch = arr[1].to_string();
            }
        }
        value = value.substring(0, index).to_string();
    }
    if !value.contains(':') {
        value += ":latest";
    }

    let mut values: Vec<&str> = value.split(&['/', ':']).collect();
    let tag = values.pop().unwrap_or_default().to_string();
    let mut registry = REGISTRY.to_string();
    let mut user = "library".to_string();
    let mut name = "".to_string();
    match values.len() {
        1 => {
            name = values[0].to_string();
        }
        2 => {
            user = values[0].to_string();
            name = values[1].to_string();
        }
        3 => {
            // 默认仅支持https v2
            registry = format!("https://{}/v2", values[0]);
            user = values[1].to_string();
            name = values[2].to_string();
        }
        _ => {}
    }

    ImageInfo {
        registry,
        user,
        name,
        tag,
        arch,
    }
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
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DockerTokenInfo {
    token: String,
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
        if let Ok(file) = std::fs::File::open(&path) {
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

fn is_pkg_cache(path: &str) -> bool {
    path.starts_with("var/cache/apt/")
        || path.starts_with("var/lib/apt/lists/")
        || path.starts_with("var/cache/apk/")
        || path.starts_with("var/cache/yum/")
        || path.starts_with("var/cache/dnf/")
        || path.starts_with("var/cache/pacman/")
}

fn is_dev_artifact(path: &str) -> bool {
    let p = path;
    // node_modules itself is fine in production; only the build cache is wasteful
    p.starts_with("node_modules/.cache/")
        || p.contains("/node_modules/.cache/")
        || p.starts_with(".git/")
        || p.contains("/.git/")
        || p.starts_with("target/debug/")
        || p.contains("/target/debug/")
        || p.starts_with("__pycache__/")
        || p.contains("/__pycache__/")
        || p.starts_with(".gradle/")
        || p.contains("/.gradle/")
        || p.starts_with(".m2/")
        || p.contains("/.m2/")
}

/// Public CA trust stores hold *public* certificates, not secrets, so they
/// must not trip the cert/key extension heuristics.
///
/// Deliberately scoped: private keys conventionally live in
/// `.../ssl/private/`, which is NOT excluded here and stays flagged.
fn is_public_ca_store(pl: &str, fl: &str) -> bool {
    // Directories whose entire content is public trust certificates.
    const CA_DIRS: &[&str] = &[
        "etc/ssl/certs/",
        "etc/ssl1.1/certs/",
        "etc/pki/tls/certs/",
        "etc/pki/ca-trust/",
        "etc/ca-certificates/",
        "usr/share/ca-certificates/",
        "usr/local/share/ca-certificates/",
        "usr/lib/ssl/certs/",
    ];
    if CA_DIRS
        .iter()
        .any(|d| pl.starts_with(d) || pl.contains(&format!("/{d}")))
    {
        return true;
    }
    // Combined system CA bundle files (the distro/OpenSSL default bundle).
    const CA_BUNDLES: &[&str] = &[
        "ca-certificates.crt",
        "ca-bundle.crt",
        "ca-bundle.pem",
        "tls-ca-bundle.pem",
        "cacert.pem",
    ];
    if CA_BUNDLES.contains(&fl) {
        return true;
    }
    // The default OpenSSL bundle is `.../ssl/cert.pem` (Alpine ships it under
    // etc/ssl and etc/ssl1.1). Scope to an ssl/tls dir so a stray user
    // `cert.pem` elsewhere is still flagged.
    if fl == "cert.pem"
        && (pl.starts_with("etc/ssl")
            || pl.starts_with("etc/pki/tls")
            || pl.contains("/ssl/")
            || pl.contains("/ssl1.1/")
            || pl.contains("/tls/"))
    {
        return true;
    }
    false
}

/// Check whether a file path looks like a sensitive/secret file.
/// Returns a short description of the risk, or None if not sensitive.
fn is_sensitive_file(path: &str) -> Option<&'static str> {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let fl = filename.to_lowercase();
    let pl = path.to_lowercase();

    // .env files
    if fl == ".env" || fl.starts_with(".env.") || fl.ends_with(".env") {
        return Some(".env file");
    }
    // SSH private keys
    if matches!(
        fl.as_str(),
        "id_rsa" | "id_dsa" | "id_ecdsa" | "id_ed25519" | "id_ecdsa_sk" | "id_ed25519_sk"
    ) {
        return Some("SSH private key");
    }
    // AWS credentials
    if pl.contains("/.aws/credentials") {
        return Some("AWS credentials");
    }
    // Public CA trust stores are public certs, not secrets — skip the
    // cert/key extension heuristics for them (private-key dirs stay flagged).
    if is_public_ca_store(&pl, &fl) {
        return None;
    }
    // Private key / certificate file extensions
    if fl.ends_with(".pem")
        || fl.ends_with(".p12")
        || fl.ends_with(".pfx")
        || fl.ends_with(".jks")
        || fl.ends_with(".keystore")
    {
        return Some("Private key / certificate");
    }
    // .key files — flag only if not inside a known-safe subtree (node_modules, etc.)
    if fl.ends_with(".key") && !pl.contains("/node_modules/") {
        return Some("Private key / certificate");
    }
    // Docker registry auth
    if pl.ends_with(".docker/config.json") {
        return Some("Docker registry credentials");
    }
    // Git / network credential stores
    if fl == ".netrc" || fl == ".git-credentials" {
        return Some("Git / network credentials");
    }
    // Kubernetes config
    if fl == "kubeconfig" || fl.ends_with(".kubeconfig") {
        return Some("Kubernetes config");
    }
    // Terraform
    if fl.ends_with(".tfvars") || fl == "terraform.tfstate" {
        return Some("Terraform secrets");
    }
    // GCP / service account JSON keys
    if fl.ends_with("-key.json")
        || ((fl.starts_with("service_account") || fl.starts_with("service-account"))
            && fl.ends_with(".json"))
    {
        return Some("Service account key");
    }
    // Password files
    if fl == ".htpasswd" {
        return Some("Password file");
    }
    // .git directory accidentally copied
    if pl.starts_with(".git/") || pl.contains("/.git/") {
        return Some(".git directory (SCM history)");
    }
    None
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
        wasted_list.sort_by_key(|b| std::cmp::Reverse(b.total_size));

        let mut score = 100 - wasted_size * 100 / self.total_size;
        // 有浪费空间，则分数-1
        if wasted_size != 0 {
            score -= 1;
        }
        DockerAnalyzeSummary {
            wasted_list,
            wasted_size,
            wasted_percent: (wasted_size as f64) / (self.total_size as f64),
            score,
        }
    }
}

impl DockerTokenInfo {
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
}

async fn get_buf_from_local_docker(image: &str) -> Result<Vec<u8>> {
    tl_info!(image = image, "saving image");
    // tokio's Command runs `docker save` without blocking an async worker;
    // `.output()` captures stdout (the image tar) for us.
    let output = tokio::process::Command::new("docker")
        .arg("save")
        .arg(image)
        .output()
        .await
        .map_err(|err| Error::IO { source: err })?;
    if !output.status.success() {
        return Err(Error::Whatever {
            message: "docker save fail".to_string(),
        });
    }
    tl_info!(image = image, "save image done");
    Ok(output.stdout)
}

// Async wrappers that run the synchronous local-tar readers on the blocking
// pool, so the `file://` / `docker://` analysis path never blocks an async
// worker while walking/extracting from an on-disk image tar.
async fn tar_content(tar: &str, filename: &str) -> Result<Vec<u8>> {
    let tar = tar.to_string();
    let filename = filename.to_string();
    tokio::task::spawn_blocking(move || get_file_content_from_tar(&tar, &filename))
        .await
        .map_err(|e| Error::Whatever {
            message: format!("tar read task failed: {e}"),
        })?
        .context(LayerSnafu {})
}

async fn tar_size(tar: &str, filename: &str) -> Result<u64> {
    let tar = tar.to_string();
    let filename = filename.to_string();
    tokio::task::spawn_blocking(move || get_file_size_from_tar(&tar, &filename))
        .await
        .map_err(|e| Error::Whatever {
            message: format!("tar read task failed: {e}"),
        })?
        .context(LayerSnafu {})
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

impl DockerClient {
    pub fn new(register: &str) -> Self {
        DockerClient {
            registry: register.to_string(),
        }
    }
    fn is_local(&self) -> bool {
        self.registry == REGISTRY_LOCAL_FILE
    }
    async fn get_local_manifest(&self, image: &str) -> Result<LocalManifest> {
        let data = tar_content(image, "manifest.json").await?;

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
                let err = resp
                    .json::<DockerRequestErrorResp>()
                    .await
                    .context(JsonSnafu { url: url.clone() })?;
                return Err(Error::Docker {
                    message: err.errors[0].message.clone(),
                    code: err.errors[0].code.clone(),
                    url,
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

    /// Stream a blob response directly to disk without buffering the whole
    /// body. Large layers from registry CDNs occasionally have the connection
    /// reset mid-transfer (surfaces as reqwest "error decoding response
    /// body"); retry a bounded number of times, resuming from the bytes
    /// already on disk via an HTTP `Range` request so a multi-hundred-MiB
    /// layer is not re-fetched from scratch.
    async fn download_blob_to_path(
        &self,
        url: String,
        headers: HashMap<String, String>,
        path: &std::path::Path,
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
        path: &std::path::Path,
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
            f.seek(std::io::SeekFrom::Start(*downloaded))
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
        let user = &params.user;
        let tag = &params.tag;
        let token = &params.token;
        if self.is_local() {
            let local_manifest = self.get_local_manifest(img).await?;
            let mut image_manifest: ImageManifest = local_manifest.into();
            for layer in image_manifest.layers.iter_mut() {
                let size = tar_size(img, &layer.digest).await?;
                layer.size = size;
            }
            return Ok((image_manifest, vec![]));
        }

        let url = format!("{}/{user}/{img}/manifests/{tag}", self.registry);
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
            let chosen = index.guess_manifest(&params.arch);
            tl_info!(arch = chosen.platform.architecture, "guess manifest");
            let mut headers = HashMap::new();
            if !token.is_empty() {
                headers.insert("Authorization".to_string(), format!("Bearer {token}"));
            }
            headers.insert("Accept".to_string(), chosen.media_type);
            let url = format!("{}/{user}/{img}/manifests/{}", self.registry, chosen.digest);
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
            tar_content(img, &local_manifest.config).await?
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
        // 忽略出错，如果出错直接从网络加载
        if let Ok(data) = get_blob_from_file(digest).await {
            tl_info!(digest, "blob cache hit");
            return Ok(data);
        }
        let user = &params.user;
        let img = &params.img;
        let token = &params.token;
        let url = format!("{}/{user}/{img}/blobs/{digest}", self.registry);
        tl_info!(url = url, "getting blob");
        let mut headers = HashMap::new();
        if !token.is_empty() {
            headers.insert("Authorization".to_string(), format!("Bearer {token}"));
        }
        let resp = self.get_bytes(url.clone(), headers).await?;

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
            let buf = tar_content(img, &layer.digest).await?;
            let compressed_size = buf.len() as u64;
            let media_type = layer.media_type.clone();
            return tokio::task::block_in_place(|| {
                get_files_from_layer(std::io::Cursor::new(buf), &media_type, compressed_size)
                    .context(LayerSnafu {})
            });
        }

        let path = get_blob_path(&layer.digest);
        let is_cached = path.exists()
            && std::fs::metadata(&path)
                .map(|m| m.len() == layer.size)
                .unwrap_or(false);

        if !is_cached {
            let user = &params.user;
            let token = &params.token;
            let url = format!("{}/{user}/{img}/blobs/{}", self.registry, layer.digest);
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
            self.download_blob_to_path(url.clone(), headers, &path)
                .await?;
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
        tokio::task::block_in_place(|| {
            let file = std::fs::File::open(&path).context(IOSnafu {})?;
            get_files_from_layer(BufReader::new(file), &media_type, compressed_size)
                .context(LayerSnafu {})
        })
    }
    async fn get_all_layer_info(
        &self,
        params: DockerImageParams,
        layers: Vec<ImageManifestLayer>,
    ) -> Result<Vec<ImageLayerInfo>> {
        let trace_id = TRACE_ID.with(clone_value_from_task_local);
        // Default cap: `min(layers.len(), 2 × logical CPUs)`. Decompression
        // is CPU-bound — oversubscribing beyond ~2× yields no extra
        // throughput, only scheduler contention. Without this cap a 50-layer
        // image would spawn 50 concurrent decompression tasks regardless of
        // host CPU count. An explicit `threads:` in `config.yml` overrides.
        let threads = must_load_config()
            .threads
            .unwrap_or_else(|| layers.len().min(num_cpus::get() * 2))
            .max(1);
        let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(threads));

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
        // 本地文件无需token
        if self.is_local() {
            return Ok("".to_string());
        }
        let user = &params.user;
        let img = &params.img;
        let tag = &params.tag;
        let url = format!("{}/{user}/{img}/manifests/{tag}", self.registry);
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
                if let Some(info) = get_docker_token_from_cache(&url) {
                    if !info.expired() {
                        return Ok(info.token);
                    }
                }
                tl_info!(url = url, "getting token");
                let mut resp = self
                    .get::<DockerTokenInfo>(url.clone(), HashMap::new())
                    .await?;
                if resp.issued_at.is_none() {
                    resp.issued_at = Some(Utc::now().to_rfc3339());
                }
                set_docker_token_to_cache(&url, resp.clone());
                tl_info!(url = url, "got token");
                return Ok(resp.token);
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
        let user = &params.user;
        let img = &params.img;
        let tag = &params.tag;
        let token = &params.token;
        let url = format!("{}/{user}/{img}/manifests/{tag}", self.registry);
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
            if let Some(mut cached) = crate::store::read_analysis(digest, &params.arch).await {
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
        let user = &params.user;
        let img = &params.img;
        let tag = &params.tag;

        let mut layers = vec![];
        // let mut layer_infos = vec![];
        let mut file_tree_list: Vec<Vec<FileTreeItem>> = vec![];
        let mut index = 0;
        let mut file_summary_list = vec![];
        tl_info!(user = user, img = img, tag = tag, "analyzing image",);

        let mut image_size = 0;
        let mut image_total_size = 0;
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
        let mut image_created = 0;
        if let Some(value) = config.history.last() {
            if let Ok(value) = DateTime::parse_from_rfc3339(&value.created) {
                image_created = value.timestamp();
            }
        }
        // path → size for every file seen in previous layers; used for O(1) modification detection
        let mut seen_files: HashMap<String, u64> = HashMap::new();
        let mut big_modified_file_list = vec![];
        let mut sensitive_files: Vec<SensitiveFileInfo> = vec![];
        // dedup key: for .git/ files the key is the git-root prefix, otherwise the full path
        let mut sensitive_seen: std::collections::HashSet<String> =
            std::collections::HashSet::new();
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
                if let Some(value) = manifest.layers.get(index) {
                    info = info_list.get(index).unwrap();
                    size = value.size;
                    digest = value.digest.clone();
                    media_type = value.media_type.clone();
                    // single pass: detect modifications, update seen-files, collect big files
                    for file in &info.files {
                        if layer_index != 0 {
                            if let Some(&prev_size) = seen_files.get(&file.path) {
                                let op;
                                let mut file_info = file.clone();
                                if file.is_whiteout.is_some() {
                                    op = Op::Removed;
                                    file_info.size = prev_size;
                                } else {
                                    op = Op::Modified;
                                }
                                file_summary_list.push(ImageFileSummary {
                                    layer_index,
                                    op,
                                    info: file_info,
                                });
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
                    // convert_files_to_file_tree needs the fully-updated file_summary_list
                    file_tree = convert_files_to_file_tree(&info.files, &file_summary_list);
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

        tl_info!(user = user, img = img, tag = tag, "analyze image done",);
        let image_name = format!("{user}/{img}:{tag}");
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

        // OS fingerprinting: probe cached blobs, fall back to history, then "Unknown"
        let base_os = if !self.is_local() {
            let base_os = tokio::task::block_in_place(|| probe_base_os(&manifest));
            if base_os.is_empty() {
                detect_os_from_history(&layers)
                    .unwrap_or_else(|| "Scratch / Distroless (no OS identifier found)".to_string())
            } else {
                base_os
            }
        } else {
            detect_os_from_history(&layers).unwrap_or_default()
        };

        let mut tags: Vec<String> = vec![];
        if has_pkg_cache {
            tags.push("[Contains Package Manager Cache]".to_string());
        }
        if has_dev_artifacts {
            tags.push("[Development Artifacts]".to_string());
        }
        if !sensitive_files.is_empty() {
            tags.push("[Potential Secrets]".to_string());
        }
        if run_user.is_empty() || run_user == "root" {
            tags.push("[Runs as Root]".to_string());
        }
        if layers.len() > 30 {
            tags.push(format!("[High Layer Count: {}]", layers.len()));
        }

        // Cross-layer duplicate detection decompresses + blake3-hashes layer
        // blobs (CPU + blocking I/O), so run it on the blocking pool via
        // `spawn_blocking`. The owned `layers` + `file_tree_list` are moved
        // in and handed back out — zero clone. Skipped under
        // `--no-verify-dup` for performance-sensitive CI.
        let (layers, file_tree_list, duplicate_groups) = if params.verify_dup {
            tokio::task::spawn_blocking(move || {
                let dup = detect_cross_layer_duplicates(&layers, &file_tree_list);
                (layers, file_tree_list, dup)
            })
            .await
            .map_err(|e| Error::Whatever {
                message: format!("duplicate detection task failed: {e}"),
            })?
        } else {
            (layers, file_tree_list, vec![])
        };

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
        };
        // Pure derived layer — computed from the result that is already built.
        // Localized at generation time from the resolved environment language.
        result.recommendations = build_recommendations(&result, params.lang);

        // Best-effort cache write. Skips silently on any error (logged
        // inside `write_analysis`) and only runs when HEAD earlier gave
        // us a digest to key on.
        if let Some(digest) = cache_digest.as_deref() {
            crate::store::write_analysis(digest, &params.arch, &result).await;
        }

        Ok(result)
    }
}

pub async fn analyze_docker_image(
    image_info: ImageInfo,
    lang: crate::i18n::Lang,
    quiet: bool,
    verify_dup: bool,
) -> Result<DockerAnalyzeResult> {
    if image_info.registry == REGISTRY_LOCAL_DOCKER {
        let buf = get_buf_from_local_docker(&image_info.name).await?;
        let mut tmpfile = tempfile::Builder::new().tempfile().unwrap();
        let filename = tmpfile.path().to_string_lossy().to_string();
        tl_info!("saving tmp file");
        tmpfile.write_all(&buf).context(IOSnafu {})?;
        tmpfile.flush().context(IOSnafu {})?;
        tl_info!("save tmp file done");

        let c = DockerClient::new(REGISTRY_LOCAL_FILE);
        c.analyze(&mut DockerImageParams {
            img: filename,
            lang,
            quiet,
            verify_dup,
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
            ..Default::default()
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_image_info_simple() {
        let info = parse_image_info("redis:alpine");
        assert_eq!(info.name, "redis");
        assert_eq!(info.tag, "alpine");
        assert_eq!(info.user, "library");
        assert_eq!(info.registry, REGISTRY);
    }

    #[test]
    fn test_parse_image_info_no_tag_defaults_to_latest() {
        let info = parse_image_info("redis");
        assert_eq!(info.name, "redis");
        assert_eq!(info.tag, "latest");
    }

    #[test]
    fn test_parse_image_info_with_user() {
        let info = parse_image_info("vicanso/diving:v1.0");
        assert_eq!(info.user, "vicanso");
        assert_eq!(info.name, "diving");
        assert_eq!(info.tag, "v1.0");
    }

    #[test]
    fn test_parse_image_info_with_registry() {
        let info = parse_image_info("registry.example.com/user/image:v2.3");
        assert_eq!(info.registry, "https://registry.example.com/v2");
        assert_eq!(info.user, "user");
        assert_eq!(info.name, "image");
        assert_eq!(info.tag, "v2.3");
    }

    #[test]
    fn test_parse_image_info_file_protocol() {
        let info = parse_image_info("file:///tmp/image.tar");
        assert_eq!(info.registry, REGISTRY_LOCAL_FILE);
        assert_eq!(info.name, "/tmp/image.tar");
    }

    #[test]
    fn test_parse_image_info_docker_protocol() {
        let info = parse_image_info("docker://redis:alpine");
        assert_eq!(info.registry, REGISTRY_LOCAL_DOCKER);
        assert_eq!(info.name, "redis:alpine");
    }

    #[test]
    fn test_parse_image_info_arch_query_param() {
        let info = parse_image_info("redis:alpine?arch=arm64");
        assert_eq!(info.arch, "arm64");
        assert_eq!(info.tag, "alpine");
        assert_eq!(info.name, "redis");
    }

    #[test]
    fn ca_bundles_are_not_flagged_as_secrets() {
        // Public trust stores must not be flagged.
        assert_eq!(is_sensitive_file("etc/ssl/cert.pem"), None);
        assert_eq!(is_sensitive_file("etc/ssl1.1/cert.pem"), None);
        assert_eq!(is_sensitive_file("etc/ssl/certs/ca-certificates.crt"), None);
        assert_eq!(is_sensitive_file("etc/pki/tls/certs/ca-bundle.crt"), None);
        assert_eq!(
            is_sensitive_file("usr/share/ca-certificates/mozilla/GlobalSign.crt"),
            None
        );
    }

    #[test]
    fn real_private_keys_still_flagged() {
        // Private-key locations must stay flagged after the CA exclusion.
        assert!(is_sensitive_file("etc/ssl/private/server.key").is_some());
        assert!(is_sensitive_file("app/config/id_rsa").is_some());
        assert!(is_sensitive_file("home/user/secret.pem").is_some());
        assert!(is_sensitive_file("opt/app/keystore.jks").is_some());
    }
}
