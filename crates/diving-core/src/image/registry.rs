use super::image_ref::{platform_name, repository_path, ImageInfo, REGISTRY_LOCAL_FILE};
use super::layer::{get_files_from_layer, ImageLayerInfo, TarIndex};
use super::local::{read_oci_layout, LocalManifest, MANIFEST_ACCEPTS};
use super::oci_image::{ImageConfig, ImageIndex, ImageManifest, ImageManifestLayer};
use super::registry_auth::{
    registry_host, resolve_for_registry, DockerConfig, RegistryCredentials, RegistryRule,
    RegistryRuleSet,
};
use super::scan::compression_label;
use crate::error::{Error, IoSnafu, JsonSnafu, RequestSnafu, ResponseJsonSnafu, Result};
use crate::progress::{emit_progress, ProgressCallback, ProgressEvent};
use crate::store::{
    save_blob_to_file, sha256_hex, sha256_hex_of_file, tmp_sibling_path, touch_blob_times,
    CacheStore,
};
use crate::tl_info;
use crate::trace::{clone_value_from_task_local, TRACE_ID};
use chrono::{DateTime, Utc};
use futures::StreamExt;
use http::{Method, StatusCode};
use lru::LruCache;
use reqwest::Url;
use ring::digest::{Context, SHA256};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use snafu::ResultExt;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, Read, SeekFrom, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::num::NonZeroUsize;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::{OnceCell as TokioOnceCell, Semaphore};
use tracing::debug;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct DockerTokenInfo {
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
    expires_in: Option<i32>,
    issued_at: Option<String>,
}

impl DockerTokenInfo {
    fn bearer(&self) -> String {
        self.token
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| self.access_token.as_deref().filter(|s| !s.is_empty()))
            .unwrap_or("")
            .to_string()
    }

    fn expired(&self) -> bool {
        let issued_at = self.issued_at.as_deref().unwrap_or("");
        if let Ok(value) = DateTime::<Utc>::from_str(issued_at) {
            let offset = (self.expires_in.unwrap_or(600) - 10) as i64;
            let now = Utc::now().timestamp();
            return value.timestamp() + offset <= now;
        }
        false
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ImageManifestCacheInfo {
    expired_at: i64,
    manifest: ImageManifest,
    supported_archs: Vec<String>,
}

const REGISTRY_CACHE_CAPACITY: NonZeroUsize = NonZeroUsize::new(100).unwrap();
const MAX_JSON_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_LAYERS_PER_IMAGE: usize = 512;

/// Returns `true` if `host` (with optional `:port` or IPv6 `[...]` brackets)
/// refers to localhost, a private/link-local/loopback IP, or an internal domain.
pub fn is_private_or_loopback_host(host: &str) -> bool {
    let trimmed = host.trim().trim_end_matches('.');
    if trimmed.is_empty() {
        return true;
    }
    let bare = if let Some(rest) = trimmed.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else if trimmed.matches(':').count() == 1 {
        trimmed.split(':').next().unwrap_or(trimmed)
    } else {
        trimmed
    };
    let lower = bare.to_ascii_lowercase();
    if lower == "localhost"
        || lower.ends_with(".localhost")
        || lower.ends_with(".local")
        || lower.ends_with(".internal")
    {
        return true;
    }
    if let Ok(ip) = lower.parse::<IpAddr>() {
        return is_non_public_ip(ip);
    }
    false
}

fn is_non_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_non_public_v4(v4),
        IpAddr::V6(v6) => is_non_public_v6(v6),
    }
}

fn is_non_public_v4(v4: Ipv4Addr) -> bool {
    let [a, b, ..] = v4.octets();
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.is_multicast()
        || (a == 100 && (64..=127).contains(&b)) // RFC 6598 CGNAT
        || a == 0
}

fn is_non_public_v6(v6: Ipv6Addr) -> bool {
    if let Some(mapped) = v6.to_ipv4_mapped() {
        return is_non_public_v4(mapped);
    }
    let seg0 = v6.segments()[0];
    v6.is_loopback()
        || v6.is_unspecified()
        || v6.is_multicast()
        || (seg0 & 0xfe00) == 0xfc00 // unique local fc00::/7
        || (seg0 & 0xffc0) == 0xfe80 // link-local fe80::/10
}

fn extract_url_host(url: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    Some((scheme, host_port))
}

/// Per-`Analyzer` in-memory LRU caches for registry bearer tokens and manifests.
#[derive(Debug)]
pub(crate) struct RegistryCaches {
    tokens: Mutex<LruCache<String, DockerTokenInfo>>,
    manifests: Mutex<LruCache<String, ImageManifestCacheInfo>>,
}

impl Default for RegistryCaches {
    fn default() -> Self {
        Self {
            tokens: Mutex::new(LruCache::new(REGISTRY_CACHE_CAPACITY)),
            manifests: Mutex::new(LruCache::new(REGISTRY_CACHE_CAPACITY)),
        }
    }
}

impl RegistryCaches {
    pub(crate) fn get_token(&self, key: &str) -> Option<DockerTokenInfo> {
        let mut cache = self.tokens.lock().ok()?;
        let expired = cache.get(key).map(|info| info.expired())?;
        if expired {
            cache.pop(key);
            return None;
        }
        cache.get(key).cloned()
    }

    pub(crate) fn set_token(&self, key: &str, info: DockerTokenInfo) {
        if let Ok(mut cache) = self.tokens.lock() {
            cache.put(key.to_owned(), info);
        }
    }

    pub(crate) fn get_manifest(&self, key: &str) -> Option<(ImageManifest, Vec<String>)> {
        let mut cache = self.manifests.lock().ok()?;
        let valid = cache
            .get(key)
            .map(|info| info.expired_at > Utc::now().timestamp())?;
        if !valid {
            cache.pop(key);
            return None;
        }
        let info = cache.get(key)?;
        debug!(key, "manifest cache hit");
        Some((info.manifest.clone(), info.supported_archs.clone()))
    }

    pub(crate) fn set_manifest(
        &self,
        key: &str,
        manifest: ImageManifest,
        supported_archs: Vec<String>,
        ttl_seconds: i64,
    ) {
        if let Ok(mut cache) = self.manifests.lock() {
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

    pub(crate) fn invalidate_manifest(&self, image_info: &ImageInfo) {
        let prefix = format!(
            "{}/{}/manifests/{}:",
            image_info.registry,
            repository_path(&image_info.user, &image_info.name),
            image_info.tag
        );
        if let Ok(mut cache) = self.manifests.lock() {
            let matched: Vec<String> = cache
                .iter()
                .filter(|(key, _)| key.starts_with(&prefix))
                .map(|(key, _)| key.clone())
                .collect();
            for key in matched {
                cache.pop(&key);
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct AuthInfo {
    pub auth: String,
    pub service: String,
    pub scope: String,
}

pub(crate) fn parse_auth_info(auth: &str) -> AuthInfo {
    let mut auth_info = AuthInfo::default();
    let mut rest = auth;
    while let Some(eq) = rest.find("=\"") {
        let key = rest[..eq]
            .rsplit(|c: char| c.is_whitespace() || c == ',')
            .next()
            .unwrap_or_default();
        let after = &rest[eq + 2..];
        let Some(end) = after.find('"') else {
            break;
        };
        let value = &after[..end];
        if value.is_empty() || value.contains(char::is_whitespace) {
            rest = after;
            continue;
        }
        match key {
            "realm" => auth_info.auth = value.to_string(),
            "service" => auth_info.service = value.to_string(),
            "scope" => auth_info.scope = value.to_string(),
            _ => {}
        }
        rest = &after[end + 1..];
    }
    auth_info
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

pub(crate) fn bytes_match_digest(data: &[u8], digest: &str) -> bool {
    match digest.strip_prefix("sha256:") {
        Some(expected) => sha256_hex(data).eq_ignore_ascii_case(expected),
        None => true,
    }
}

fn auth_scope_key(creds: Option<&RegistryCredentials>, rule_fp: Option<&str>) -> String {
    match (creds.filter(|c| !c.is_empty()), rule_fp) {
        (Some(c), Some(fp)) => sha256_hex(format!("{}:{}|{fp}", c.username, c.password).as_bytes()),
        (Some(c), None) => sha256_hex(format!("{}:{}", c.username, c.password).as_bytes()),
        (None, Some(fp)) => format!("rule:{fp}"),
        (None, None) => "anon".to_string(),
    }
}

/// Resolved credential mode attached to outbound registry API requests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum ResolvedAuth {
    #[default]
    None,
    Bearer(String),
    Basic(String),
}

async fn verify_blob_digest(path: &Path, expected_digest: &str) -> Result<()> {
    let Some(expected) = expected_digest.strip_prefix("sha256:") else {
        return Ok(());
    };
    let path = path.to_path_buf();
    let actual = tokio::task::spawn_blocking(move || sha256_hex_of_file(&path))
        .await
        .map_err(|e| Error::Other {
            message: format!("blob hash task failed: {e}"),
        })?
        .context(IoSnafu {})?;
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(Error::Other {
            message: format!(
                "blob digest mismatch: expected sha256:{expected}, got sha256:{actual}"
            ),
        });
    }
    Ok(())
}

pub(crate) fn resolve_layer_concurrency(configured: Option<usize>, layer_count: usize) -> usize {
    configured
        .unwrap_or_else(|| layer_count.min(num_cpus::get() * 2))
        .max(1)
}

#[derive(Default, Clone)]
pub(crate) struct DockerImageParams {
    pub user: String,
    pub img: String,
    pub tag: String,
    pub auth: ResolvedAuth,
    pub arch: String,
    pub verify_dup: bool,
    pub credentials: Option<RegistryCredentials>,
    pub on_progress: Option<ProgressCallback>,
}

impl DockerImageParams {
    pub(crate) fn repo(&self) -> String {
        repository_path(&self.user, &self.img)
    }
}

#[derive(Clone)]
pub(crate) struct RegistryClient {
    pub(crate) registry: String,
    pub(crate) http_client: reqwest::Client,
    pub(crate) store: Arc<CacheStore>,
    pub(crate) caches: Arc<RegistryCaches>,
    pub(crate) docker_config: Option<Arc<DockerConfig>>,
    pub(crate) layer_concurrency: Option<usize>,
    pub(crate) block_private_hosts: bool,
    pub(crate) registries: Arc<RegistryRuleSet>,
    pub(crate) rule_clients: Arc<HashMap<usize, reqwest::Client>>,
    pub(crate) local_index: Arc<TokioOnceCell<Arc<TarIndex>>>,
    pub(crate) cache_local_blobs: bool,
}

fn parse_oci_sha256_digest(entry: &str) -> Option<String> {
    let hex = entry.strip_prefix("blobs/sha256/")?;
    if hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(format!("sha256:{}", hex.to_ascii_lowercase()))
    } else {
        None
    }
}

fn cache_local_layer_blob(
    index: &TarIndex,
    store: &CacheStore,
    entry: &str,
    size: u64,
) -> Result<String> {
    if let Some(known) = parse_oci_sha256_digest(entry) {
        let dest = store.blob_path(&known);
        if fs::metadata(&dest)
            .map(|m| m.len() == size)
            .unwrap_or(false)
        {
            touch_blob_times(&dest);
            return Ok(known);
        }
    }

    let (mut reader, _) = index.open_reader(entry)?;
    let tmp = tmp_sibling_path(&store.blob_path("local-layer"));
    let write_res = (|| -> Result<String> {
        let mut out = File::create(&tmp).context(IoSnafu {})?;
        let mut hasher = Context::new(&SHA256);
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = reader.read(&mut buf).context(IoSnafu {})?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            out.write_all(&buf[..n]).context(IoSnafu {})?;
        }
        out.flush().context(IoSnafu {})?;
        drop(out);
        let digest = format!("sha256:{}", hex::encode(hasher.finish().as_ref()));
        let dest = store.blob_path(&digest);
        if fs::metadata(&dest)
            .map(|m| m.len() == size)
            .unwrap_or(false)
        {
            let _ = fs::remove_file(&tmp);
            touch_blob_times(&dest);
        } else {
            fs::rename(&tmp, &dest).context(IoSnafu {})?;
        }
        Ok(digest)
    })();
    if write_res.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    write_res
}

impl RegistryClient {
    pub(crate) fn is_local(&self) -> bool {
        self.registry == REGISTRY_LOCAL_FILE
    }

    async fn local_tar_index(&self, tar: &str) -> Result<Arc<TarIndex>> {
        let index = self
            .local_index
            .get_or_try_init(|| async {
                let tar = tar.to_string();
                tokio::task::spawn_blocking(move || TarIndex::build(&tar))
                    .await
                    .map_err(|e| Error::Other {
                        message: format!("tar index task failed: {e}"),
                    })?
                    .map(Arc::new)
            })
            .await?;
        Ok(index.clone())
    }

    async fn get_local_manifest(
        &self,
        image: &str,
        arch: &str,
    ) -> Result<super::local::LocalImage> {
        let index = self.local_tar_index(image).await?;
        if index.size_of("manifest.json").is_none() && index.size_of("index.json").is_some() {
            return read_oci_layout(&index, arch);
        }
        let data = index.read("manifest.json")?;
        let manifest_list =
            serde_json::from_slice::<Vec<LocalManifest>>(&data).context(JsonSnafu {
                context: "get_local_manifest",
            })?;
        let Some(manifest) = manifest_list.into_iter().next() else {
            return Err(Error::Other {
                message: "Local Manifest Not Found".to_string(),
            });
        };
        Ok(super::local::LocalImage {
            config: manifest.config,
            layers: manifest
                .layers
                .into_iter()
                .map(|layer| ImageManifestLayer {
                    media_type: "application/vnd.docker.image.rootfs.diff.tar".to_string(),
                    digest: layer,
                    ..Default::default()
                })
                .collect(),
        })
    }

    fn matched_rule(&self) -> Option<(usize, &RegistryRule)> {
        let host = registry_host(&self.registry)?;
        self.registries.match_host(&host)
    }

    fn client_for_host(&self, host: &str, fallback_rule_idx: Option<usize>) -> &reqwest::Client {
        if let Some(idx) = self.registries.match_index(host) {
            if let Some(client) = self.rule_clients.get(&idx) {
                return client;
            }
        }
        if let Some(idx) = fallback_rule_idx {
            if let Some(client) = self.rule_clients.get(&idx) {
                return client;
            }
        }
        &self.http_client
    }

    fn apply_rule_headers_only(
        rule: Option<&RegistryRule>,
        headers: &mut HashMap<String, String>,
    ) -> Vec<String> {
        let mut origin_only_keys = Vec::new();
        if let Some(rule) = rule {
            for (key, value) in &rule.headers {
                let name = key.trim();
                if name.is_empty() {
                    continue;
                }
                headers.insert(name.to_owned(), value.clone());
                origin_only_keys.push(name.to_ascii_lowercase());
            }
        }
        origin_only_keys
    }

    fn apply_registry_headers(
        &self,
        headers: &mut HashMap<String, String>,
        auth: &ResolvedAuth,
    ) -> Vec<String> {
        let rule = self.matched_rule().map(|(_, r)| r);
        let mut origin_only_keys = Self::apply_rule_headers_only(rule, headers);
        let has_custom_auth = headers
            .keys()
            .any(|k| k.eq_ignore_ascii_case("authorization"));
        if !has_custom_auth {
            match auth {
                ResolvedAuth::Bearer(token) if !token.is_empty() => {
                    headers.insert("Authorization".to_string(), format!("Bearer {token}"));
                    origin_only_keys.push("authorization".to_string());
                }
                ResolvedAuth::Basic(token) if !token.is_empty() => {
                    headers.insert("Authorization".to_string(), format!("Basic {token}"));
                    origin_only_keys.push("authorization".to_string());
                }
                ResolvedAuth::Bearer(_) | ResolvedAuth::Basic(_) | ResolvedAuth::None => {}
            }
        }
        origin_only_keys
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_with_redirects(
        &self,
        method: Method,
        initial_url: String,
        mut headers: HashMap<String, String>,
        origin_only_keys: &[String],
        timeout: Duration,
        fallback_rule_idx: Option<usize>,
        allow_same_registry_host: bool,
    ) -> Result<reqwest::Response> {
        const MAX_REDIRECTS: usize = 10;

        let init_parsed = Url::parse(&initial_url).map_err(|e| Error::Other {
            message: format!("invalid registry URL {initial_url}: {e}"),
        })?;
        let init_origin = init_parsed.origin();
        let registry_origin_host = registry_host(&self.registry);

        let mut current_method = method;
        let mut current_url = initial_url.clone();

        for _hop in 0..=MAX_REDIRECTS {
            let Some((scheme, host_port)) = extract_url_host(&current_url) else {
                return Err(Error::DisallowedRegistry {
                    registry: current_url,
                });
            };
            if self.block_private_hosts {
                let same_registry_host = allow_same_registry_host
                    && registry_origin_host
                        .as_deref()
                        .is_some_and(|reg_host| reg_host.eq_ignore_ascii_case(host_port));
                let allowed = (scheme.eq_ignore_ascii_case("https")
                    && !is_private_or_loopback_host(host_port))
                    || self.registries.allows_private_target(scheme, host_port)
                    || same_registry_host;
                if !allowed {
                    return Err(Error::DisallowedRegistry {
                        registry: host_port.to_string(),
                    });
                }
            }

            let current_parsed = Url::parse(&current_url).map_err(|e| Error::Other {
                message: format!("invalid request URL {current_url}: {e}"),
            })?;
            let same_origin = current_parsed.origin() == init_origin;
            let hop_fallback = if same_origin { fallback_rule_idx } else { None };
            let client = self.client_for_host(host_port, hop_fallback);

            let mut builder = client
                .request(current_method.clone(), current_url.clone())
                .timeout(timeout);
            for (key, value) in &headers {
                builder = builder.header(key.as_str(), value.as_str());
            }
            let resp = builder.send().await.context(RequestSnafu {
                url: current_url.clone(),
            })?;

            if matches!(resp.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                let Some(loc_raw) = resp.headers().get("location").and_then(|v| v.to_str().ok())
                else {
                    return Ok(resp);
                };
                let next_url = current_parsed
                    .join(loc_raw.trim())
                    .map_err(|e| Error::Other {
                        message: format!(
                            "invalid redirect target {loc_raw} from {current_url}: {e}"
                        ),
                    })?;
                if resp.status().as_u16() == 303 && current_method != Method::HEAD {
                    current_method = Method::GET;
                }
                if next_url.origin() != init_origin {
                    headers.retain(|k, _| {
                        !k.eq_ignore_ascii_case("authorization")
                            && !origin_only_keys.iter().any(|ok| k.eq_ignore_ascii_case(ok))
                    });
                }
                current_url = next_url.to_string();
                continue;
            }

            return Ok(resp);
        }

        Err(Error::Other {
            message: format!("too many redirects from {initial_url}"),
        })
    }

    async fn send_request(
        &self,
        url: String,
        headers: HashMap<String, String>,
        origin_only_keys: &[String],
        fallback_rule_idx: Option<usize>,
        allow_same_registry_host: bool,
    ) -> Result<reqwest::Response> {
        const MAX_ATTEMPTS: u32 = 2;
        const MAX_BACKOFF_SECS: u64 = 60;
        const DEFAULT_BACKOFF_SECS: u64 = 5;

        for attempt in 1..=MAX_ATTEMPTS {
            let resp = self
                .execute_with_redirects(
                    Method::GET,
                    url.clone(),
                    headers.clone(),
                    origin_only_keys,
                    Duration::from_secs(30 * 60),
                    fallback_rule_idx,
                    allow_same_registry_host,
                )
                .await?;
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
                return Err(Error::Registry {
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
        origin_only_keys: &[String],
        fallback_rule_idx: Option<usize>,
        allow_same_registry_host: bool,
    ) -> Result<bytes::Bytes> {
        let mut resp = self
            .send_request(
                url.clone(),
                headers,
                origin_only_keys,
                fallback_rule_idx,
                allow_same_registry_host,
            )
            .await?;
        if let Some(len) = resp.content_length() {
            if len > MAX_JSON_RESPONSE_BYTES as u64 {
                return Err(Error::Other {
                    message: format!(
                        "response body from {url} exceeds safety limit ({len} > {MAX_JSON_RESPONSE_BYTES} bytes)"
                    ),
                });
            }
        }
        let mut buf = Vec::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .context(ResponseJsonSnafu { url: url.clone() })?
        {
            if buf.len().saturating_add(chunk.len()) > MAX_JSON_RESPONSE_BYTES {
                return Err(Error::Other {
                    message: format!(
                        "response body from {url} exceeds safety limit ({MAX_JSON_RESPONSE_BYTES} bytes)"
                    ),
                });
            }
            buf.extend_from_slice(&chunk);
        }
        Ok(bytes::Bytes::from(buf))
    }

    async fn get<T: DeserializeOwned>(
        &self,
        url: String,
        headers: HashMap<String, String>,
        origin_only_keys: &[String],
        fallback_rule_idx: Option<usize>,
        allow_same_registry_host: bool,
    ) -> Result<T> {
        let data = self
            .get_bytes(
                url.clone(),
                headers,
                origin_only_keys,
                fallback_rule_idx,
                allow_same_registry_host,
            )
            .await?;
        serde_json::from_slice(&data).context(JsonSnafu { context: "request" })
    }

    async fn download_blob_to_path(
        &self,
        url: String,
        headers: HashMap<String, String>,
        origin_only_keys: &[String],
        fallback_rule_idx: Option<usize>,
        path: &Path,
        expected_digest: &str,
    ) -> Result<()> {
        let tmp = tmp_sibling_path(path);
        let result = async {
            self.stream_blob_with_retry(url, headers, origin_only_keys, fallback_rule_idx, &tmp)
                .await?;
            verify_blob_digest(&tmp, expected_digest).await?;
            tokio::fs::rename(&tmp, path).await.context(IoSnafu {})
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&tmp).await;
        }
        result
    }

    async fn stream_blob_with_retry(
        &self,
        url: String,
        headers: HashMap<String, String>,
        origin_only_keys: &[String],
        fallback_rule_idx: Option<usize>,
        path: &Path,
    ) -> Result<()> {
        const MAX_ATTEMPTS: usize = 4;
        let mut downloaded: u64 = 0;
        for attempt in 1..=MAX_ATTEMPTS {
            let mut req_headers = headers.clone();
            if downloaded > 0 {
                req_headers.insert("Range".to_string(), format!("bytes={downloaded}-"));
            }
            match self
                .stream_blob_once(
                    url.clone(),
                    req_headers,
                    origin_only_keys,
                    fallback_rule_idx,
                    path,
                    &mut downloaded,
                )
                .await
            {
                Ok(()) => return Ok(()),
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

    async fn stream_blob_once(
        &self,
        url: String,
        headers: HashMap<String, String>,
        origin_only_keys: &[String],
        fallback_rule_idx: Option<usize>,
        path: &Path,
        downloaded: &mut u64,
    ) -> Result<()> {
        let resuming = *downloaded > 0;
        let resp = self
            .send_request(
                url.clone(),
                headers,
                origin_only_keys,
                fallback_rule_idx,
                true,
            )
            .await?;
        let resumed = resuming && resp.status().as_u16() == StatusCode::PARTIAL_CONTENT.as_u16();
        let mut file = if resumed {
            let mut f = tokio::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .await
                .context(IoSnafu {})?;
            f.seek(SeekFrom::Start(*downloaded))
                .await
                .context(IoSnafu {})?;
            f
        } else {
            *downloaded = 0;
            tokio::fs::File::create(path).await.context(IoSnafu {})?
        };
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context(RequestSnafu { url: url.clone() })?;
            file.write_all(&chunk).await.context(IoSnafu {})?;
            *downloaded += chunk.len() as u64;
        }
        file.flush().await.context(IoSnafu {})?;
        Ok(())
    }

    pub(crate) async fn get_manifest(
        &self,
        params: &DockerImageParams,
    ) -> Result<(ImageManifest, Vec<String>)> {
        let img = &params.img;
        let tag = &params.tag;
        if self.is_local() {
            let local_image = self.get_local_manifest(img, &params.arch).await?;
            let mut image_manifest: ImageManifest = local_image.into();
            let index = self.local_tar_index(img).await?;
            for layer in image_manifest.layers.iter_mut() {
                layer.size = index.size_of(&layer.digest).unwrap_or(0);
            }
            return Ok((image_manifest, vec![]));
        }

        let repo = params.repo();
        let url = format!("{}/{repo}/manifests/{tag}", self.registry);
        let rule_idx = self.matched_rule().map(|(idx, _)| idx);
        let rule_fp =
            registry_host(&self.registry).and_then(|host| self.registries.auth_fingerprint(&host));
        let key = format!(
            "{url}:{}:{}",
            params.arch,
            auth_scope_key(params.credentials.as_ref(), rule_fp.as_deref())
        );
        if let Some(cached) = self.caches.get_manifest(&key) {
            return Ok(cached);
        }
        tl_info!(url = url, "getting manifest");
        let mut headers = HashMap::new();
        headers.insert("Accept".to_string(), MANIFEST_ACCEPTS.join(", "));
        let origin_only = self.apply_registry_headers(&mut headers, &params.auth);
        let data = self
            .get_bytes(url.clone(), headers, &origin_only, rule_idx, true)
            .await?;
        let is_index = serde_json::from_slice::<Value>(&data)
            .context(JsonSnafu {
                context: "get_manifest",
            })?
            .get("manifests")
            .is_some();
        let (resp, supported_archs): (ImageManifest, Vec<String>) = if !is_index {
            let manifest = serde_json::from_slice(&data).context(JsonSnafu {
                context: "get_manifest_schema2",
            })?;
            (manifest, vec![])
        } else {
            let index = serde_json::from_slice::<ImageIndex>(&data).context(JsonSnafu {
                context: "guess_manifest",
            })?;
            let mut archs: Vec<String> = vec![];
            for m in index.manifests.iter().filter(|m| m.platform.os == "linux") {
                let name = platform_name(&m.platform.architecture, m.platform.variant.as_deref());
                if !archs.contains(&name) {
                    archs.push(name);
                }
            }
            let chosen = index
                .guess_manifest(&params.arch)
                .ok_or_else(|| Error::Other {
                    message: format!("image index of {url} contains no manifests"),
                })?;
            tl_info!(arch = chosen.platform.architecture, "guess manifest");
            let mut headers = HashMap::new();
            headers.insert("Accept".to_string(), chosen.media_type);
            let origin_only = self.apply_registry_headers(&mut headers, &params.auth);
            let url = format!("{}/{repo}/manifests/{}", self.registry, chosen.digest);
            let data = self
                .get_bytes(url.clone(), headers, &origin_only, rule_idx, true)
                .await?;
            let manifest = serde_json::from_slice(&data).context(JsonSnafu {
                context: "get_manifest",
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
        self.caches
            .set_manifest(&key, resp.clone(), supported_archs.clone(), ttl);
        tl_info!(url = url, "got manifest");
        Ok((resp, supported_archs))
    }

    pub(crate) async fn get_image_config(
        &self,
        params: &DockerImageParams,
        config_digest: &str,
    ) -> Result<ImageConfig> {
        let data = if self.is_local() {
            let index = self.local_tar_index(&params.img).await?;
            index.read(config_digest)?
        } else {
            self.get_blob(params, config_digest).await?
        };

        serde_json::from_slice(&data).context(JsonSnafu {
            context: "get_image_config",
        })
    }

    async fn get_blob(&self, params: &DockerImageParams, digest: &str) -> Result<Vec<u8>> {
        let path = self.store.blob_path(digest);
        if let Ok(data) = tokio::fs::read(&path).await {
            if bytes_match_digest(&data, digest) {
                touch_blob_times(&path);
                tl_info!(digest, "blob cache hit");
                return Ok(data);
            }
            tl_info!(digest, "cached blob digest mismatch, refetching");
        }
        let rule_idx = self.matched_rule().map(|(idx, _)| idx);
        let url = format!("{}/{}/blobs/{digest}", self.registry, params.repo());
        tl_info!(url = url, "getting blob");
        let mut headers = HashMap::new();
        let origin_only = self.apply_registry_headers(&mut headers, &params.auth);
        let resp = self
            .get_bytes(url.clone(), headers, &origin_only, rule_idx, true)
            .await?;
        if !bytes_match_digest(&resp, digest) {
            return Err(Error::Other {
                message: format!("blob digest mismatch for {digest}"),
            });
        }

        let _ = save_blob_to_file(&self.store, digest, &resp).await;
        tl_info!(url = url, "got blob");
        Ok(resp.to_vec())
    }

    async fn get_layer_files(
        &self,
        params: &DockerImageParams,
        layer: ImageManifestLayer,
    ) -> Result<(ImageLayerInfo, Option<String>)> {
        let img = &params.img;
        if self.is_local() {
            let index = self.local_tar_index(img).await?;
            let media_type = layer.media_type.clone();
            let digest = layer.digest.clone();
            let cache_blob = self.cache_local_blobs && layer.size > 0;
            let store = self.store.clone();
            let size = layer.size;
            return tokio::task::spawn_blocking(move || {
                if cache_blob {
                    let cached_digest = cache_local_layer_blob(&index, &store, &digest, size)?;
                    let file = File::open(store.blob_path(&cached_digest)).context(IoSnafu {})?;
                    let info = get_files_from_layer(BufReader::new(file), &media_type, size)?;
                    Ok((info, Some(cached_digest)))
                } else {
                    let (reader, size) = index.open_reader(&digest)?;
                    let info = get_files_from_layer(BufReader::new(reader), &media_type, size)?;
                    Ok((info, None))
                }
            })
            .await
            .map_err(|_| Error::Other {
                message: "decompression task join error".to_string(),
            })?;
        }

        let path = self.store.blob_path(&layer.digest);
        let is_cached = tokio::fs::metadata(&path)
            .await
            .map(|m| m.len() == layer.size)
            .unwrap_or(false);

        if !is_cached {
            let rule_idx = self.matched_rule().map(|(idx, _)| idx);
            let url = format!("{}/{}/blobs/{}", self.registry, params.repo(), layer.digest);
            tl_info!(url = url, "getting blob");
            emit_progress(
                params.on_progress.as_ref(),
                ProgressEvent::DownloadingLayer {
                    digest: layer.digest.clone(),
                    size: layer.size,
                    compression: compression_label(&layer.media_type),
                },
            );
            let mut headers = HashMap::new();
            let origin_only = self.apply_registry_headers(&mut headers, &params.auth);
            if let Err(err) = self
                .download_blob_to_path(
                    url.clone(),
                    headers,
                    &origin_only,
                    rule_idx,
                    &path,
                    &layer.digest,
                )
                .await
            {
                if !err.is_unauthorized() {
                    return Err(err);
                }
                tl_info!(digest = layer.digest, "blob unauthorized, refreshing token");
                let refreshed = self.refresh_auth(params).await?;
                let mut headers = HashMap::new();
                let origin_only = self.apply_registry_headers(&mut headers, &refreshed);
                self.download_blob_to_path(
                    url.clone(),
                    headers,
                    &origin_only,
                    rule_idx,
                    &path,
                    &layer.digest,
                )
                .await?;
            }
            tl_info!(url = url, "got blob");
        } else {
            tl_info!(digest = layer.digest, "blob cache hit");
            emit_progress(
                params.on_progress.as_ref(),
                ProgressEvent::LayerCacheHit {
                    digest: layer.digest.clone(),
                    size: layer.size,
                    compression: compression_label(&layer.media_type),
                },
            );
        }

        let compressed_size = if is_cached {
            layer.size
        } else {
            tokio::fs::metadata(&path)
                .await
                .map(|m| m.len())
                .unwrap_or(layer.size)
        };
        let media_type = layer.media_type.clone();
        let digest = layer.digest.clone();
        let info = tokio::task::spawn_blocking(move || {
            if is_cached {
                touch_blob_times(&path);
            }
            let file = File::open(&path).context(IoSnafu {})?;
            get_files_from_layer(BufReader::new(file), &media_type, compressed_size)
        })
        .await
        .map_err(|_| Error::Other {
            message: "decompression task join error".to_string(),
        })??;
        emit_progress(
            params.on_progress.as_ref(),
            ProgressEvent::LayerCompleted {
                digest,
                size: compressed_size,
            },
        );
        Ok((info, None))
    }

    pub(crate) async fn get_all_layer_info(
        &self,
        params: &DockerImageParams,
        layers: &mut [ImageManifestLayer],
    ) -> Result<Vec<ImageLayerInfo>> {
        if layers.len() > MAX_LAYERS_PER_IMAGE {
            return Err(Error::Other {
                message: format!(
                    "image exceeds maximum layer count ({} > {MAX_LAYERS_PER_IMAGE})",
                    layers.len()
                ),
            });
        }
        let trace_id = TRACE_ID
            .try_with(clone_value_from_task_local)
            .unwrap_or_default();
        let threads = resolve_layer_concurrency(self.layer_concurrency, layers.len());
        let sem = Arc::new(Semaphore::new(threads));

        let mut handles = Vec::with_capacity(layers.len());
        for layer in layers.iter().cloned() {
            let client = self.clone();
            let p = params.clone();
            let sem = sem.clone();
            let tid = trace_id.clone();
            handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.map_err(|_| Error::Other {
                    message: "layer semaphore closed".to_string(),
                })?;
                TRACE_ID.scope(tid, client.get_layer_files(&p, layer)).await
            }));
        }

        let mut info_list = Vec::with_capacity(handles.len());
        for (layer, handle) in layers.iter_mut().zip(handles) {
            let (info, cached_digest) = handle.await.map_err(|_| Error::Other {
                message: "task join error".to_string(),
            })??;
            if let Some(digest) = cached_digest {
                layer.digest = digest;
            }
            info_list.push(info);
        }
        Ok(info_list)
    }

    pub(crate) async fn get_auth(&self, params: &DockerImageParams) -> Result<ResolvedAuth> {
        self.fetch_auth(params, false).await
    }

    async fn refresh_auth(&self, params: &DockerImageParams) -> Result<ResolvedAuth> {
        self.fetch_auth(params, true).await
    }

    fn resolve_rule_credentials(&self, params: &DockerImageParams) -> Option<RegistryCredentials> {
        if let Some(c) = params.credentials.as_ref().filter(|c| !c.is_empty()) {
            return Some(c.clone());
        }
        if let Some((_, rule)) = self.matched_rule() {
            if let Some(username) = rule
                .username
                .as_deref()
                .map(str::trim)
                .filter(|u| !u.is_empty())
            {
                return Some(RegistryCredentials {
                    username: username.to_string(),
                    password: rule.password.clone().unwrap_or_default(),
                });
            }
        }
        resolve_for_registry(&self.registry, None, self.docker_config.as_deref())
    }

    async fn fetch_auth(
        &self,
        params: &DockerImageParams,
        skip_cache: bool,
    ) -> Result<ResolvedAuth> {
        if self.is_local() {
            return Ok(ResolvedAuth::None);
        }
        let matched = self.matched_rule();
        let rule_idx = matched.map(|(idx, _)| idx);
        let matched_rule = matched.map(|(_, rule)| rule);
        let creds = self.resolve_rule_credentials(params);

        let has_explicit_or_rule_creds = params.credentials.as_ref().is_some_and(|c| !c.is_empty())
            || matched_rule
                .and_then(|r| r.username.as_deref())
                .is_some_and(|u| !u.trim().is_empty());

        if !has_explicit_or_rule_creds {
            if let Some(static_token) = matched_rule
                .and_then(|r| r.token.as_deref())
                .map(str::trim)
                .filter(|t| !t.is_empty())
            {
                return Ok(ResolvedAuth::Bearer(static_token.to_string()));
            }
            if matched_rule.is_some_and(|r| {
                r.headers
                    .keys()
                    .any(|k| k.eq_ignore_ascii_case("authorization"))
            }) {
                return Ok(ResolvedAuth::None);
            }
        }

        let tag = &params.tag;
        let url = format!("{}/{}/manifests/{tag}", self.registry, params.repo());
        let mut head_headers = HashMap::new();
        head_headers.insert("Accept".to_string(), MANIFEST_ACCEPTS.join(", "));
        let head_origin_only = Self::apply_rule_headers_only(matched_rule, &mut head_headers);

        let resp = self
            .execute_with_redirects(
                Method::HEAD,
                url.clone(),
                head_headers,
                &head_origin_only,
                Duration::from_secs(5 * 60),
                rule_idx,
                true,
            )
            .await?;

        if resp.status().as_u16() != StatusCode::UNAUTHORIZED.as_u16() {
            return Ok(ResolvedAuth::None);
        }

        let mut bearer_auth: Option<AuthInfo> = None;
        for value in resp.headers().get_all("www-authenticate") {
            let Ok(raw) = value.to_str() else {
                continue;
            };
            let trimmed = raw.trim_start();
            if trimmed.len() >= 6 && trimmed[..6].eq_ignore_ascii_case("bearer") {
                let info = parse_auth_info(trimmed);
                if !info.auth.is_empty() {
                    bearer_auth = Some(info);
                    break;
                }
            }
        }

        let Some(auth_info) = bearer_auth else {
            return Ok(match creds {
                Some(c) if !c.is_empty() => ResolvedAuth::Basic(c.basic_token()),
                _ => ResolvedAuth::None,
            });
        };

        let mut token_url = auth_info.auth.clone();
        let mut sep = if token_url.contains('?') { '&' } else { '?' };
        if !auth_info.service.is_empty() {
            token_url.push(sep);
            token_url.push_str("service=");
            token_url.push_str(&auth_info.service);
            sep = '&';
        }
        if !auth_info.scope.is_empty() {
            token_url.push(sep);
            token_url.push_str("scope=");
            token_url.push_str(&auth_info.scope);
        }

        let realm_host = extract_url_host(&auth_info.auth).map(|(_, host)| host);
        let reg_host = registry_host(&self.registry);
        let token_rule = realm_host
            .and_then(|host| self.registries.match_host(host))
            .or_else(|| {
                let same_as_reg = realm_host
                    .zip(reg_host.as_deref())
                    .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b));
                same_as_reg.then_some(matched).flatten()
            });
        let token_rule_idx = token_rule.map(|(idx, _)| idx);
        let mut token_headers = HashMap::new();
        let mut token_origin_only =
            Self::apply_rule_headers_only(token_rule.map(|(_, r)| r), &mut token_headers);
        let rule_fp = realm_host
            .and_then(|h| self.registries.auth_fingerprint(h))
            .or_else(|| {
                reg_host
                    .as_deref()
                    .and_then(|h| self.registries.auth_fingerprint(h))
            });
        let cache_key = format!(
            "{token_url}#auth={}",
            auth_scope_key(creds.as_ref(), rule_fp.as_deref())
        );
        if !skip_cache {
            if let Some(info) = self.caches.get_token(&cache_key) {
                return Ok(ResolvedAuth::Bearer(info.bearer()));
            }
        }
        tl_info!(
            url = token_url,
            authenticated = creds.is_some(),
            "getting token"
        );
        if let Some(c) = creds.as_ref() {
            if !token_headers
                .keys()
                .any(|k| k.eq_ignore_ascii_case("authorization"))
            {
                token_headers.insert(
                    "Authorization".to_string(),
                    format!("Basic {}", c.basic_token()),
                );
                token_origin_only.push("authorization".to_string());
            }
        }
        let mut resp = self
            .get::<DockerTokenInfo>(
                token_url.clone(),
                token_headers,
                &token_origin_only,
                token_rule_idx,
                true,
            )
            .await?;
        let bearer = resp.bearer();
        if bearer.is_empty() {
            return Err(Error::Other {
                message: "registry auth returned empty token".to_string(),
            });
        }
        if resp.issued_at.is_none() {
            resp.issued_at = Some(Utc::now().to_rfc3339());
        }
        self.caches.set_token(&cache_key, resp);
        tl_info!(url = token_url, "got token");
        Ok(ResolvedAuth::Bearer(bearer))
    }

    pub(crate) async fn head_manifest_digest(
        &self,
        params: &DockerImageParams,
    ) -> Option<(String, String)> {
        if self.is_local() {
            return None;
        }
        let tag = &params.tag;
        let url = format!("{}/{}/manifests/{tag}", self.registry, params.repo());
        let rule_idx = self.matched_rule().map(|(idx, _)| idx);
        let mut headers = HashMap::new();
        headers.insert("Accept".to_string(), MANIFEST_ACCEPTS.join(", "));
        let origin_only = self.apply_registry_headers(&mut headers, &params.auth);
        let resp = self
            .execute_with_redirects(
                Method::HEAD,
                url,
                headers,
                &origin_only,
                Duration::from_secs(30),
                rule_idx,
                true,
            )
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let header = |name: &str| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.split(';').next().unwrap_or_default().trim().to_string())
        };
        let digest = header("Docker-Content-Digest")?;
        Some((
            digest,
            super::local::cache_arch(header("Content-Type").as_deref(), &params.arch),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::image_ref::parse_image_info;

    #[test]
    fn docker_token_accepts_both_token_and_access_token() {
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
    fn parse_auth_info_reads_bearer_challenge_params() {
        let info = parse_auth_info(
            r#"Bearer realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:library/redis:pull""#,
        );
        assert_eq!(info.auth, "https://auth.docker.io/token");
        assert_eq!(info.service, "registry.docker.io");
        assert_eq!(info.scope, "repository:library/redis:pull");

        let info = parse_auth_info(
            r#"Bearer service="harbor-registry", realm="https://h.example.com/service/token", error="invalid_token""#,
        );
        assert_eq!(info.auth, "https://h.example.com/service/token");
        assert_eq!(info.service, "harbor-registry");
        assert_eq!(info.scope, "");
    }

    #[test]
    fn parse_auth_info_skips_values_it_cannot_use() {
        let info = parse_auth_info(
            r#"Bearer realm="",scope="repository:a:pull repository:b:pull",service="s""#,
        );
        assert_eq!((info.auth.as_str(), info.scope.as_str()), ("", ""));
        assert_eq!(info.service, "s");

        for header in ["", "Bearer", r#"Basic realm="Registry"#] {
            let info = parse_auth_info(header);
            assert_eq!(info.service, "", "{header}");
        }
    }

    #[test]
    fn bytes_match_digest_verifies_sha256_only() {
        let digest = "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert!(bytes_match_digest(b"abc", digest));
        assert!(!bytes_match_digest(b"abd", digest));
        assert!(bytes_match_digest(b"anything", "layer.tar"));
    }

    #[test]
    fn repository_path_joins_namespace() {
        assert_eq!(repository_path("org", "proj/img"), "org/proj/img");
        assert_eq!(repository_path("", "solo"), "solo");
    }

    #[test]
    fn invalidate_manifest_cache_clears_all_arches_for_tag() {
        let caches = RegistryCaches::default();
        let k_amd64 = "https://index.docker.io/v2/org/demo-inv/manifests/latest:amd64";
        let k_arm64 = "https://index.docker.io/v2/org/demo-inv/manifests/latest:arm64";
        let k_other = "https://index.docker.io/v2/org/demo-inv/manifests/v1:amd64";
        caches.set_manifest(k_amd64, ImageManifest::default(), vec![], 300);
        caches.set_manifest(k_arm64, ImageManifest::default(), vec![], 300);
        caches.set_manifest(k_other, ImageManifest::default(), vec![], 300);

        let info = parse_image_info("org/demo-inv:latest");
        caches.invalidate_manifest(&info);

        assert!(caches.get_manifest(k_amd64).is_none());
        assert!(caches.get_manifest(k_arm64).is_none());
        assert!(caches.get_manifest(k_other).is_some());
    }

    #[test]
    fn auth_scope_and_expired_cache_eviction() {
        let c1 = RegistryCredentials {
            username: "alice".into(),
            password: "secret-1".into(),
        };
        let c2 = RegistryCredentials {
            username: "alice".into(),
            password: "secret-2".into(),
        };
        assert_eq!(auth_scope_key(None, None), "anon");
        assert_ne!(
            auth_scope_key(Some(&c1), None),
            auth_scope_key(Some(&c2), None)
        );
        assert_ne!(auth_scope_key(Some(&c1), None), "anon");
        assert_ne!(auth_scope_key(None, Some("fp1")), "anon");
        assert_ne!(
            auth_scope_key(Some(&c1), Some("fp1")),
            auth_scope_key(Some(&c1), None)
        );

        let caches = RegistryCaches::default();
        caches.set_manifest("expired-manifest", ImageManifest::default(), vec![], -10);
        assert!(caches.get_manifest("expired-manifest").is_none());
        assert!(!caches
            .manifests
            .lock()
            .unwrap()
            .contains("expired-manifest"));

        caches.set_token(
            "expired-token",
            DockerTokenInfo {
                token: Some("tok".into()),
                expires_in: Some(1),
                issued_at: Some("2020-01-01T00:00:00Z".into()),
                ..Default::default()
            },
        );
        assert!(caches.get_token("expired-token").is_none());
        assert!(!caches.tokens.lock().unwrap().contains("expired-token"));
    }
}
