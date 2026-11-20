use super::image_ref::{platform_name, repository_path, ImageInfo, REGISTRY_LOCAL_FILE};
use super::layer::{get_files_from_layer, ImageLayerInfo, TarIndex};
use super::local::{read_oci_layout, LocalManifest, MANIFEST_ACCEPTS};
use super::oci_image::{ImageConfig, ImageIndex, ImageManifest, ImageManifestLayer};
use super::registry_auth::{resolve_for_registry, DockerConfig, RegistryCredentials};
use super::scan::compression_label;
use crate::error::{Error, IoSnafu, JsonSnafu, RequestSnafu, ResponseJsonSnafu, Result};
use crate::progress::{emit_progress, ProgressCallback, ProgressEvent};
use crate::store::{
    save_blob_to_file, sha256_hex, sha256_hex_of_file, tmp_sibling_path, CacheStore,
};
use crate::tl_info;
use crate::trace::{clone_value_from_task_local, TRACE_ID};
use chrono::{DateTime, Utc};
use futures::StreamExt;
use http::StatusCode;
use lru::LruCache;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use snafu::ResultExt;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, SeekFrom};
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

fn auth_scope_key(creds: Option<&RegistryCredentials>) -> String {
    match creds {
        Some(c) if !c.is_empty() => sha256_hex(format!("{}:{}", c.username, c.password).as_bytes()),
        _ => "anon".to_string(),
    }
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
    pub token: String,
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
    pub(crate) local_index: Arc<TokioOnceCell<Arc<TarIndex>>>,
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

    async fn send_request(
        &self,
        url: String,
        headers: HashMap<String, String>,
    ) -> Result<reqwest::Response> {
        const MAX_ATTEMPTS: u32 = 2;
        const MAX_BACKOFF_SECS: u64 = 60;
        const DEFAULT_BACKOFF_SECS: u64 = 5;

        for attempt in 1..=MAX_ATTEMPTS {
            let mut builder = self
                .http_client
                .get(url.clone())
                .timeout(Duration::from_secs(30 * 60));
            for (key, value) in &headers {
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
    ) -> Result<bytes::Bytes> {
        let resp = self.send_request(url.clone(), headers).await?;
        resp.bytes().await.context(ResponseJsonSnafu { url })
    }

    async fn get<T: DeserializeOwned>(
        &self,
        url: String,
        headers: HashMap<String, String>,
    ) -> Result<T> {
        let data = self.get_bytes(url.clone(), headers).await?;
        serde_json::from_slice(&data).context(JsonSnafu { context: "request" })
    }

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
                .stream_blob_once(url.clone(), req_headers, path, &mut downloaded)
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
        path: &Path,
        downloaded: &mut u64,
    ) -> Result<()> {
        let resuming = *downloaded > 0;
        let resp = self.send_request(url.clone(), headers).await?;
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
        let token = &params.token;
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
        let key = format!(
            "{url}:{}:{}",
            params.arch,
            auth_scope_key(params.credentials.as_ref())
        );
        if let Some(cached) = self.caches.get_manifest(&key) {
            return Ok(cached);
        }
        tl_info!(url = url, "getting manifest");
        let mut headers = HashMap::new();
        if !token.is_empty() {
            headers.insert("Authorization".to_string(), format!("Bearer {token}"));
        }
        headers.insert("Accept".to_string(), MANIFEST_ACCEPTS.join(", "));
        let data = self.get_bytes(url.clone(), headers).await?;
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
            if !token.is_empty() {
                headers.insert("Authorization".to_string(), format!("Bearer {token}"));
            }
            headers.insert("Accept".to_string(), chosen.media_type);
            let url = format!("{}/{repo}/manifests/{}", self.registry, chosen.digest);
            let data = self.get_bytes(url.clone(), headers).await?;
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
    ) -> Result<ImageLayerInfo> {
        let img = &params.img;
        if self.is_local() {
            let index = self.local_tar_index(img).await?;
            let media_type = layer.media_type.clone();
            let digest = layer.digest.clone();
            return tokio::task::spawn_blocking(move || {
                let (reader, size) = index.open_reader(&digest)?;
                get_files_from_layer(BufReader::new(reader), &media_type, size)
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
            let token = &params.token;
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
            if !token.is_empty() {
                headers.insert("Authorization".to_string(), format!("Bearer {token}"));
            }
            if let Err(err) = self
                .download_blob_to_path(url.clone(), headers, &path, &layer.digest)
                .await
            {
                if !err.is_unauthorized() {
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
        tokio::task::spawn_blocking(move || {
            let file = File::open(&path).context(IoSnafu {})?;
            get_files_from_layer(BufReader::new(file), &media_type, compressed_size)
        })
        .await
        .map_err(|_| Error::Other {
            message: "decompression task join error".to_string(),
        })?
    }

    pub(crate) async fn get_all_layer_info(
        &self,
        params: &DockerImageParams,
        layers: Vec<ImageManifestLayer>,
    ) -> Result<Vec<ImageLayerInfo>> {
        let trace_id = TRACE_ID
            .try_with(clone_value_from_task_local)
            .unwrap_or_default();
        let threads = resolve_layer_concurrency(self.layer_concurrency, layers.len());
        let sem = Arc::new(Semaphore::new(threads));

        let mut handles = Vec::with_capacity(layers.len());
        for layer in layers {
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
        for handle in handles {
            let info = handle.await.map_err(|_| Error::Other {
                message: "task join error".to_string(),
            })??;
            info_list.push(info);
        }
        Ok(info_list)
    }

    pub(crate) async fn get_auth_token(&self, params: &DockerImageParams) -> Result<String> {
        self.fetch_auth_token(params, false).await
    }

    async fn refresh_auth_token(&self, params: &DockerImageParams) -> Result<String> {
        self.fetch_auth_token(params, true).await
    }

    async fn fetch_auth_token(
        &self,
        params: &DockerImageParams,
        skip_cache: bool,
    ) -> Result<String> {
        if self.is_local() {
            return Ok(String::new());
        }
        let tag = &params.tag;
        let url = format!("{}/{}/manifests/{tag}", self.registry, params.repo());
        let resp = self
            .http_client
            .head(url.clone())
            .timeout(Duration::from_secs(5 * 60))
            .send()
            .await
            .context(RequestSnafu { url: url.clone() })?;
        if resp.status().as_u16() == StatusCode::UNAUTHORIZED.as_u16() {
            if let Some(value) = resp.headers().get("www-authenticate") {
                let auth_info = parse_auth_info(value.to_str().unwrap_or_default());
                let url = format!(
                    "{}?service={}&scope={}",
                    auth_info.auth, auth_info.service, auth_info.scope
                );
                let creds = resolve_for_registry(
                    &self.registry,
                    params.credentials.as_ref(),
                    self.docker_config.as_deref(),
                );
                let cache_key = format!("{url}#auth={}", auth_scope_key(creds.as_ref()));
                if !skip_cache {
                    if let Some(info) = self.caches.get_token(&cache_key) {
                        return Ok(info.bearer());
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
                    return Err(Error::Other {
                        message: "registry auth returned empty token".to_string(),
                    });
                }
                if resp.issued_at.is_none() {
                    resp.issued_at = Some(Utc::now().to_rfc3339());
                }
                self.caches.set_token(&cache_key, resp);
                tl_info!(url = url, "got token");
                return Ok(bearer);
            }
        }
        Ok(String::new())
    }

    pub(crate) async fn head_manifest_digest(
        &self,
        params: &DockerImageParams,
    ) -> Option<(String, String)> {
        if self.is_local() {
            return None;
        }
        let tag = &params.tag;
        let token = &params.token;
        let url = format!("{}/{}/manifests/{tag}", self.registry, params.repo());
        let mut req = self
            .http_client
            .head(url)
            .timeout(Duration::from_secs(30))
            .header("Accept", MANIFEST_ACCEPTS.join(", "));
        if !token.is_empty() {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        let resp = req.send().await.ok()?;
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
        assert_eq!(auth_scope_key(None), "anon");
        assert_ne!(auth_scope_key(Some(&c1)), auth_scope_key(Some(&c2)));
        assert_ne!(auth_scope_key(Some(&c1)), "anon");

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
