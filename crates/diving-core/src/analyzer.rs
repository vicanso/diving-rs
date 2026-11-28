use crate::diff::{diff_images_with_opener, DiffOptions, ImageDiff};
use crate::error::{Error, IoSnafu, Result};
use crate::i18n::Lang;
use crate::image::{
    analyze_once, analyze_or_last_known, get_file_content_from_layer, get_file_head_from_layer,
    hash_files_from_layer, is_local_registry, is_private_or_loopback_host, load_docker_config,
    parse_image_info, preview_image, registry_host, stream_file_from_layer, DockerAnalyzeResult,
    DockerConfig, ImageInfo, ImagePreview, LocalManifest, PipelineContext, PipelineOptions,
    RegistryCaches, RegistryCredentials, TarIndex, REGISTRY_LOCAL_DOCKER, REGISTRY_LOCAL_FILE,
    REGISTRY_LOCAL_PODMAN,
};
use crate::progress::ProgressCallback;
use crate::recommend::build_recommendations;
use crate::sensitive::SensitivePatterns;
use crate::store::{
    is_safe_blob_id, sha256_hex, sha256_hex_of_reader, touch_blob_times, CacheConfig, CacheStore,
    CleanupStats, RecentCache,
};
use crate::trace::{clone_value_from_task_local, TRACE_ID};
use futures::future::{try_join, FutureExt, Shared};
use rustls::crypto::CryptoProvider;
use serde_json::from_slice;
use snafu::ResultExt;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::future::Future;
use std::io::{BufReader, Read, Result as IoResult, Take};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::pin::Pin;
use std::result::Result as StdResult;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex as TokioMutex, Semaphore};

const DEFAULT_MEMORY_CACHE_CAPACITY: usize = 4;

enum LocalLayerReader {
    Blob(File),
    Entry(Take<File>),
}

impl Read for LocalLayerReader {
    fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
        match self {
            Self::Blob(f) => f.read(buf),
            Self::Entry(t) => t.read(buf),
        }
    }
}

fn open_tar_index_layer(index: &TarIndex, layer_digest: &str) -> Result<Take<File>> {
    match index.open_reader(layer_digest) {
        Ok((reader, _)) => Ok(reader),
        Err(err) => {
            if let Some(hex) = layer_digest.strip_prefix("sha256:") {
                let oci_entry = format!("blobs/sha256/{hex}");
                if let Ok((reader, _)) = index.open_reader(&oci_entry) {
                    return Ok(reader);
                }
                if let Ok(manifest_bytes) = index.read("manifest.json") {
                    if let Ok(manifests) = from_slice::<Vec<LocalManifest>>(&manifest_bytes) {
                        let mut seen = HashSet::new();
                        for entry_name in manifests.iter().flat_map(|m| m.layers.iter()) {
                            if !seen.insert(entry_name.as_str()) {
                                continue;
                            }
                            let key = if index.size_of(entry_name).is_some() {
                                entry_name.as_str()
                            } else {
                                let trimmed = entry_name.trim_start_matches("./");
                                if index.size_of(trimmed).is_some() {
                                    trimmed
                                } else {
                                    continue;
                                }
                            };
                            let Ok((reader, _)) = index.open_reader(key) else {
                                continue;
                            };
                            if let Ok(actual) = sha256_hex_of_reader(reader) {
                                if actual.eq_ignore_ascii_case(hex) {
                                    if let Ok((matched, _)) = index.open_reader(key) {
                                        return Ok(matched);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Err(err)
        }
    }
}

type FlightOutput = StdResult<(Arc<DockerAnalyzeResult>, Lang), Arc<Error>>;
type AnalysisFuture = Shared<Pin<Box<dyn Future<Output = FlightOutput> + Send>>>;
type RecentAnalysisCache = RecentCache<(Arc<DockerAnalyzeResult>, Lang)>;

/// Options controlling a single [`Analyzer::analyze`] or [`Analyzer::diff`] call.
#[derive(Clone)]
pub struct AnalyzeOptions {
    pub lang: Lang,
    pub verify_dup: bool,
    pub fallback_to_last_known: bool,
    pub skip_memory_cache: bool,
    pub credentials: Option<RegistryCredentials>,
    pub on_progress: Option<ProgressCallback>,
}

impl Default for AnalyzeOptions {
    fn default() -> Self {
        Self {
            lang: Lang::En,
            verify_dup: true,
            fallback_to_last_known: false,
            skip_memory_cache: false,
            credentials: None,
            on_progress: None,
        }
    }
}

impl AnalyzeOptions {
    pub fn lang(mut self, lang: Lang) -> Self {
        self.lang = lang;
        self
    }

    pub fn verify_dup(mut self, verify_dup: bool) -> Self {
        self.verify_dup = verify_dup;
        self
    }

    pub fn fallback_to_last_known(mut self, fallback: bool) -> Self {
        self.fallback_to_last_known = fallback;
        self
    }

    pub fn skip_memory_cache(mut self, skip: bool) -> Self {
        self.skip_memory_cache = skip;
        self
    }

    pub fn credentials(mut self, credentials: Option<RegistryCredentials>) -> Self {
        self.credentials = credentials;
        self
    }

    pub fn on_progress(mut self, cb: ProgressCallback) -> Self {
        self.on_progress = Some(cb);
        self
    }
}

/// Options controlling a [`Analyzer::preview`] call.
#[derive(Default, Clone)]
pub struct PreviewOptions {
    pub credentials: Option<RegistryCredentials>,
}

impl PreviewOptions {
    pub fn credentials(mut self, credentials: Option<RegistryCredentials>) -> Self {
        self.credentials = credentials;
        self
    }
}

/// Builder for configuring an isolated [`Analyzer`] instance.
///
/// Defaults follow a secure-by-default server posture:
/// - Ephemeral `TempDir` cache (no implicit `~/.diving` I/O)
/// - Local sources (`file://`, `docker://`, `podman://`) disabled
/// - `~/.docker/config.json` / credential helpers disabled
/// - Singleflight deduplication enabled
#[derive(Clone)]
pub struct AnalyzerBuilder {
    cache: CacheConfig,
    http_client: Option<reqwest::Client>,
    layer_concurrency: Option<usize>,
    sensitive_patterns: SensitivePatterns,
    allow_local_file: Option<bool>,
    allow_local_docker: Option<bool>,
    allow_local_podman: Option<bool>,
    use_docker_config: bool,
    docker_config_path: Option<PathBuf>,
    registry_allowlist: Vec<String>,
    block_private_registries: bool,
    singleflight: bool,
    max_concurrent_analyses: Option<usize>,
    memory_cache_ttl: Duration,
    memory_cache_capacity: usize,
}

impl Default for AnalyzerBuilder {
    fn default() -> Self {
        Self {
            cache: CacheConfig::ephemeral(),
            http_client: None,
            layer_concurrency: None,
            sensitive_patterns: SensitivePatterns::default(),
            allow_local_file: None,
            allow_local_docker: None,
            allow_local_podman: None,
            use_docker_config: false,
            docker_config_path: None,
            registry_allowlist: vec![],
            block_private_registries: false,
            singleflight: true,
            max_concurrent_analyses: None,
            memory_cache_ttl: Duration::ZERO,
            memory_cache_capacity: DEFAULT_MEMORY_CACHE_CAPACITY,
        }
    }
}

impl AnalyzerBuilder {
    pub fn cache(mut self, cache: CacheConfig) -> Self {
        self.cache = cache;
        self
    }

    pub fn http_client(mut self, client: reqwest::Client) -> Self {
        self.http_client = Some(client);
        self
    }

    pub fn layer_concurrency(mut self, concurrency: usize) -> Self {
        self.layer_concurrency = (concurrency > 0).then_some(concurrency);
        self
    }

    pub fn sensitive_patterns(mut self, patterns: SensitivePatterns) -> Self {
        self.sensitive_patterns = patterns;
        self
    }

    /// Enable or disable all local image sources (`file://`, `docker://`, `podman://`).
    pub fn allow_local_sources(mut self, allow: bool) -> Self {
        self.allow_local_file = Some(allow);
        self.allow_local_docker = Some(allow);
        self.allow_local_podman = Some(allow);
        self
    }

    pub fn allow_local_file(mut self, allow: bool) -> Self {
        self.allow_local_file = Some(allow);
        self
    }

    pub fn allow_local_docker(mut self, allow: bool) -> Self {
        self.allow_local_docker = Some(allow);
        self
    }

    pub fn allow_local_podman(mut self, allow: bool) -> Self {
        self.allow_local_podman = Some(allow);
        self
    }

    /// Enable reading `~/.docker/config.json` (or `$DOCKER_CONFIG/config.json`)
    /// and Docker credential helpers when explicit credentials are not provided.
    pub fn use_docker_config(mut self, use_cfg: bool) -> Self {
        self.use_docker_config = use_cfg;
        self
    }

    /// Supply an explicit `config.json` path for registry authentication.
    pub fn docker_config_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.docker_config_path = Some(path.into());
        self.use_docker_config = true;
        self
    }

    /// Restrict permitted registry hosts (and optionally `local-file`,
    /// `local-docker`, `local-podman`). Empty means unrestricted for remote registries.
    pub fn registry_allowlist<I, S>(mut self, hosts: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.registry_allowlist = hosts
            .into_iter()
            .map(|s| s.as_ref().trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        self
    }

    /// Deduplicate concurrent in-flight analyses of the same image.
    pub fn singleflight(mut self, enabled: bool) -> Self {
        self.singleflight = enabled;
        self
    }

    /// Cap how many different images may be analyzed concurrently (`0` = unlimited).
    pub fn max_concurrent_analyses(mut self, max: usize) -> Self {
        self.max_concurrent_analyses = (max > 0).then_some(max);
        self
    }

    /// Keep recently completed registry analyses in memory for `ttl` (`Duration::ZERO` = off).
    pub fn memory_cache_ttl(mut self, ttl: Duration) -> Self {
        self.memory_cache_ttl = ttl;
        self
    }

    pub fn memory_cache_capacity(mut self, capacity: usize) -> Self {
        self.memory_cache_capacity = capacity.max(1);
        self
    }

    /// Reject private, loopback, link-local, and `.local`/`.internal` registry
    /// and token-realm hosts when no explicit `registry_allowlist` is configured.
    pub fn block_private_registries(mut self, block: bool) -> Self {
        self.block_private_registries = block;
        self
    }

    pub fn build(self) -> Result<Analyzer> {
        let store = Arc::new(CacheStore::from_config(self.cache)?);
        let http_client = match self.http_client {
            Some(c) => c,
            None => build_default_http_client()?,
        };
        let recent_analyses = (!self.memory_cache_ttl.is_zero()).then(|| {
            let cap = NonZeroUsize::new(self.memory_cache_capacity.max(1)).unwrap();
            StdMutex::new(RecentCache::new(cap, self.memory_cache_ttl))
        });

        let has_in_allowlist = |id: &str| self.registry_allowlist.iter().any(|h| h == id);
        let allow_local_file = self
            .allow_local_file
            .unwrap_or_else(|| has_in_allowlist(REGISTRY_LOCAL_FILE));
        let allow_local_docker = self
            .allow_local_docker
            .unwrap_or_else(|| has_in_allowlist(REGISTRY_LOCAL_DOCKER));
        let allow_local_podman = self
            .allow_local_podman
            .unwrap_or_else(|| has_in_allowlist(REGISTRY_LOCAL_PODMAN));

        let analysis_slots = self.max_concurrent_analyses.map(Semaphore::new);

        Ok(Analyzer {
            inner: Arc::new(AnalyzerInner {
                store,
                http_client,
                registry_caches: Arc::new(RegistryCaches::default()),
                sensitive_patterns: Arc::new(self.sensitive_patterns),
                layer_concurrency: self.layer_concurrency,
                allow_local_file,
                allow_local_docker,
                allow_local_podman,
                use_docker_config: self.use_docker_config,
                docker_config_path: self.docker_config_path,
                docker_config: OnceLock::new(),
                registry_allowlist: self.registry_allowlist,
                block_private_registries: self.block_private_registries,
                singleflight_enabled: self.singleflight,
                analysis_slots,
                in_flight: TokioMutex::new(HashMap::new()),
                recent_analyses,
                memory_cache_ttl: self.memory_cache_ttl,
            }),
        })
    }
}

struct AnalyzerInner {
    store: Arc<CacheStore>,
    http_client: reqwest::Client,
    registry_caches: Arc<RegistryCaches>,
    sensitive_patterns: Arc<SensitivePatterns>,
    layer_concurrency: Option<usize>,
    allow_local_file: bool,
    allow_local_docker: bool,
    allow_local_podman: bool,
    use_docker_config: bool,
    docker_config_path: Option<PathBuf>,
    docker_config: OnceLock<Option<Arc<DockerConfig>>>,
    registry_allowlist: Vec<String>,
    block_private_registries: bool,
    singleflight_enabled: bool,
    analysis_slots: Option<Semaphore>,
    in_flight: TokioMutex<HashMap<String, AnalysisFuture>>,
    recent_analyses: Option<StdMutex<RecentAnalysisCache>>,
    memory_cache_ttl: Duration,
}

impl AnalyzerInner {
    fn docker_config(&self) -> Option<Arc<DockerConfig>> {
        if !self.use_docker_config {
            return None;
        }
        self.docker_config
            .get_or_init(|| load_docker_config(self.docker_config_path.as_deref()).map(Arc::new))
            .clone()
    }

    fn pipeline_context(&self) -> PipelineContext {
        PipelineContext {
            http_client: self.http_client.clone(),
            store: self.store.clone(),
            registry_caches: self.registry_caches.clone(),
            docker_config: self.docker_config(),
            sensitive_patterns: self.sensitive_patterns.clone(),
            layer_concurrency: self.layer_concurrency,
            block_private_hosts: self.block_private_registries
                && self.registry_allowlist.is_empty(),
        }
    }

    async fn run_in_slot<T>(&self, task: impl Future<Output = T>) -> T {
        let _permit = match &self.analysis_slots {
            Some(slots) => slots.acquire().await.ok(),
            None => None,
        };
        task.await
    }
}

/// Isolated, cloneable handle for analyzing container images.
///
/// Cloning an `Analyzer` shares the underlying cache store, connection pool,
/// singleflight coordinator, concurrency limiter, and in-memory cache.
#[derive(Clone)]
pub struct Analyzer {
    inner: Arc<AnalyzerInner>,
}

impl Analyzer {
    pub fn builder() -> AnalyzerBuilder {
        AnalyzerBuilder::default()
    }

    pub fn http_client(&self) -> &reqwest::Client {
        &self.inner.http_client
    }

    /// Validate whether `info` is permitted by this `Analyzer`'s local-source
    /// flags and `registry_allowlist`.
    pub fn check_allowed(&self, info: &ImageInfo) -> Result<()> {
        if is_local_registry(&info.registry) {
            let local_enabled = match info.registry.as_str() {
                REGISTRY_LOCAL_FILE => self.inner.allow_local_file,
                REGISTRY_LOCAL_DOCKER => self.inner.allow_local_docker,
                REGISTRY_LOCAL_PODMAN => self.inner.allow_local_podman,
                _ => false,
            };
            if !local_enabled {
                return Err(Error::LocalSourceDisabled {
                    source: info.registry.clone(),
                });
            }
        }
        if self.inner.registry_allowlist.is_empty() {
            if self.inner.block_private_registries && !is_local_registry(&info.registry) {
                let host = registry_host(&info.registry).unwrap_or_default();
                if is_private_or_loopback_host(&host) {
                    return Err(Error::DisallowedRegistry { registry: host });
                }
            }
            return Ok(());
        }
        let id = if is_local_registry(&info.registry) {
            info.registry.clone()
        } else {
            registry_host(&info.registry).unwrap_or_default()
        }
        .to_ascii_lowercase();
        if self.inner.registry_allowlist.iter().any(|a| a == &id) {
            return Ok(());
        }
        Err(Error::DisallowedRegistry { registry: id })
    }

    /// Analyze an image string (registry ref, `file://`, `docker://`, `podman://`).
    pub async fn analyze(
        &self,
        image: &str,
        opts: AnalyzeOptions,
    ) -> Result<Arc<DockerAnalyzeResult>> {
        let info = parse_image_info(image);
        self.analyze_info(info, opts).await
    }

    /// Analyze a pre-parsed [`ImageInfo`].
    pub async fn analyze_info(
        &self,
        info: ImageInfo,
        opts: AnalyzeOptions,
    ) -> Result<Arc<DockerAnalyzeResult>> {
        self.check_allowed(&info)?;

        let is_local = is_local_registry(&info.registry);
        let auth_scope = match &opts.credentials {
            Some(c) if !c.is_empty() => {
                sha256_hex(format!("{}:{}", c.username, c.password).as_bytes())
            }
            _ => "anon".to_string(),
        };
        let key = format!(
            "{}|{}|{}|{}|{}|{}|{}",
            info.registry, info.user, info.name, info.tag, info.arch, opts.verify_dup, auth_scope
        );

        if !is_local && !opts.skip_memory_cache {
            if let Some(recent) = &self.inner.recent_analyses {
                let hit = recent
                    .lock()
                    .ok()
                    .and_then(|mut cache| cache.get(&key, Instant::now()));
                if let Some((result, result_lang)) = hit {
                    return Ok(in_lang(result, result_lang, opts.lang));
                }
            }
        }

        let use_singleflight = self.inner.singleflight_enabled
            && !opts.skip_memory_cache
            && opts.on_progress.is_none();

        if !use_singleflight {
            let lang = opts.lang;
            let ctx = self.inner.pipeline_context();
            let pipeline_opts = PipelineOptions {
                lang,
                verify_dup: opts.verify_dup,
                credentials: opts.credentials,
                on_progress: opts.on_progress,
            };
            let result = self
                .inner
                .run_in_slot(async move {
                    if opts.fallback_to_last_known {
                        analyze_or_last_known(&ctx, info, pipeline_opts).await
                    } else {
                        analyze_once(&ctx, info, pipeline_opts).await
                    }
                })
                .await?;
            if !is_local {
                remember_analysis(&self.inner, key, result.clone(), lang);
            }
            return Ok(result);
        }

        let fut = {
            let mut map = self.inner.in_flight.lock().await;
            if let Some(existing) = map.get(&key) {
                existing.clone()
            } else {
                let inner = self.inner.clone();
                let key_for_cleanup = key.clone();
                let lang = opts.lang;
                let fallback = opts.fallback_to_last_known;
                let verify_dup = opts.verify_dup;
                let credentials = opts.credentials;
                let tid = TRACE_ID
                    .try_with(clone_value_from_task_local)
                    .unwrap_or_default();
                let handle = tokio::spawn(TRACE_ID.scope(tid.clone(), async move {
                    let work_inner = inner.clone();
                    let work_key = key_for_cleanup.clone();
                    let work_handle = tokio::spawn(TRACE_ID.scope(tid, async move {
                        let ctx = work_inner.pipeline_context();
                        let pipeline_opts = PipelineOptions {
                            lang,
                            verify_dup,
                            credentials,
                            on_progress: None,
                        };
                        let res = work_inner
                            .run_in_slot(async move {
                                if fallback {
                                    analyze_or_last_known(&ctx, info, pipeline_opts).await
                                } else {
                                    analyze_once(&ctx, info, pipeline_opts).await
                                }
                            })
                            .await
                            .map(|r| (r, lang))
                            .map_err(|e| Arc::new(e.clone_for_waiter()));
                        if let (Ok((analysis, _)), false) = (&res, is_local) {
                            remember_analysis(&work_inner, work_key, analysis.clone(), lang);
                        }
                        res
                    }));
                    let res = match work_handle.await {
                        Ok(res) => res,
                        Err(join_err) => Err(Arc::new(Error::Other {
                            message: format!("analysis task failed: {join_err}"),
                        })),
                    };
                    inner.in_flight.lock().await.remove(&key_for_cleanup);
                    res
                }));
                let shared = async move {
                    match handle.await {
                        Ok(res) => res,
                        Err(join_err) => Err(Arc::new(Error::Other {
                            message: format!("singleflight coordinator failed: {join_err}"),
                        })),
                    }
                }
                .boxed()
                .shared();
                map.insert(key, shared.clone());
                shared
            }
        };

        let (result, flight_lang) = fut.await.map_err(|e| e.clone_for_waiter())?;
        Ok(in_lang(result, flight_lang, opts.lang))
    }

    /// Lightweight preview of a registry image from manifest + config alone.
    pub async fn preview(&self, image: &str, opts: PreviewOptions) -> Result<ImagePreview> {
        let info = parse_image_info(image);
        self.preview_info(info, opts).await
    }

    pub async fn preview_info(
        &self,
        info: ImageInfo,
        opts: PreviewOptions,
    ) -> Result<ImagePreview> {
        self.check_allowed(&info)?;
        let ctx = self.inner.pipeline_context();
        preview_image(&ctx, info, opts.credentials).await
    }

    /// Compare two images concurrently (singleflight still deduplicates any
    /// shared layers) and compute their [`ImageDiff`].
    /// Compare two images (fetching/analyzing them in parallel) with default [`DiffOptions`].
    pub async fn diff(&self, base: &str, target: &str, opts: AnalyzeOptions) -> Result<ImageDiff> {
        self.diff_with_options(base, target, opts, DiffOptions::default())
            .await
    }

    /// Compare two images (fetching/analyzing them in parallel) with explicit [`DiffOptions`].
    pub async fn diff_with_options(
        &self,
        base: &str,
        target: &str,
        analyze_opts: AnalyzeOptions,
        diff_opts: DiffOptions,
    ) -> Result<ImageDiff> {
        let base_info = parse_image_info(base);
        let target_info = parse_image_info(target);
        self.check_allowed(&base_info)?;
        self.check_allowed(&target_info)?;

        let (base_res, target_res) = try_join(
            self.analyze_info(base_info, analyze_opts.clone()),
            self.analyze_info(target_info, analyze_opts),
        )
        .await?;
        self.compute_diff(base, target, base_res, target_res, diff_opts)
            .await
    }

    /// Recompute [`ImageDiff`] from already-analyzed results while verifying
    /// same-size regular file contents against locally available layer blobs or
    /// `file://` archives.
    pub async fn compute_diff(
        &self,
        base_ref: &str,
        target_ref: &str,
        base_res: Arc<DockerAnalyzeResult>,
        target_res: Arc<DockerAnalyzeResult>,
        diff_opts: DiffOptions,
    ) -> Result<ImageDiff> {
        let base_info = parse_image_info(base_ref);
        let target_info = parse_image_info(target_ref);
        let base_tar = (base_info.registry == REGISTRY_LOCAL_FILE
            && self.check_allowed(&base_info).is_ok())
        .then_some(base_info.name);
        let target_tar = (target_info.registry == REGISTRY_LOCAL_FILE
            && self.check_allowed(&target_info).is_ok())
        .then_some(target_info.name);
        let store = self.inner.store.clone();

        tokio::task::spawn_blocking(move || {
            let mut base_index: Option<Option<TarIndex>> = None;
            let mut target_index: Option<Option<TarIndex>> = None;
            diff_images_with_opener(
                &base_res,
                &target_res,
                &diff_opts,
                true,
                |is_target, layer, paths| {
                    if is_safe_blob_id(&layer.digest) {
                        let blob = store.blob_path(&layer.digest);
                        if let Ok(file) = File::open(&blob) {
                            touch_blob_times(&blob);
                            return hash_files_from_layer(
                                BufReader::new(file),
                                &layer.media_type,
                                paths,
                            )
                            .ok();
                        }
                    }
                    let (tar_path, index_cache) = if is_target {
                        (&target_tar, &mut target_index)
                    } else {
                        (&base_tar, &mut base_index)
                    };
                    if let Some(archive_path) = tar_path {
                        let index = index_cache
                            .get_or_insert_with(|| TarIndex::build(archive_path).ok())
                            .as_ref()?;
                        let reader = open_tar_index_layer(index, &layer.digest).ok()?;
                        return hash_files_from_layer(
                            BufReader::new(reader),
                            &layer.media_type,
                            paths,
                        )
                        .ok();
                    }
                    None
                },
            )
        })
        .await
        .map_err(|e| Error::Other {
            message: format!("diff task failed: {e}"),
        })
    }

    /// Read a file's full bytes (up to `max_bytes`) from a cached layer blob.
    pub fn read_file(
        &self,
        layer_digest: &str,
        media_type: &str,
        file_path: &str,
        max_bytes: u64,
    ) -> Result<Vec<u8>> {
        let path = self.layer_blob_path(layer_digest)?;
        let file = File::open(&path).context(IoSnafu {})?;
        touch_blob_times(&path);
        get_file_content_from_layer(BufReader::new(file), media_type, file_path, max_bytes)
    }

    /// Stream a file's bytes (up to `max_bytes`) in chunks from a cached layer blob.
    pub fn stream_file<F>(
        &self,
        layer_digest: &str,
        media_type: &str,
        file_path: &str,
        max_bytes: u64,
        on_chunk: F,
    ) -> Result<()>
    where
        F: FnMut(&[u8]) -> Result<()>,
    {
        let path = self.layer_blob_path(layer_digest)?;
        let file = File::open(&path).context(IoSnafu {})?;
        touch_blob_times(&path);
        stream_file_from_layer(
            BufReader::new(file),
            media_type,
            file_path,
            max_bytes,
            on_chunk,
        )
    }

    /// Read up to `limit` bytes from the start of a file inside a cached layer blob.
    pub fn read_file_head(
        &self,
        layer_digest: &str,
        media_type: &str,
        file_path: &str,
        limit: u64,
    ) -> Result<(Vec<u8>, u64)> {
        let path = self.layer_blob_path(layer_digest)?;
        let file = File::open(&path).context(IoSnafu {})?;
        touch_blob_times(&path);
        get_file_head_from_layer(BufReader::new(file), media_type, file_path, limit)
    }

    fn open_local_layer_reader(
        &self,
        archive_path: &str,
        layer_digest: &str,
    ) -> Result<BufReader<LocalLayerReader>> {
        let info = ImageInfo {
            registry: REGISTRY_LOCAL_FILE.to_string(),
            name: archive_path.to_string(),
            ..Default::default()
        };
        self.check_allowed(&info)?;
        if is_safe_blob_id(layer_digest) {
            let blob = self.inner.store.blob_path(layer_digest);
            if let Ok(file) = File::open(&blob) {
                touch_blob_times(&blob);
                return Ok(BufReader::new(LocalLayerReader::Blob(file)));
            }
        }
        let index = TarIndex::build(archive_path)?;
        let reader = open_tar_index_layer(&index, layer_digest)?;
        Ok(BufReader::new(LocalLayerReader::Entry(reader)))
    }

    /// Read a file's full bytes (up to `max_bytes`) from a layer inside a local `file://` archive or directory.
    pub fn read_local_file(
        &self,
        archive_path: &str,
        layer_digest: &str,
        media_type: &str,
        file_path: &str,
        max_bytes: u64,
    ) -> Result<Vec<u8>> {
        let reader = self.open_local_layer_reader(archive_path, layer_digest)?;
        get_file_content_from_layer(
            reader,
            media_type,
            file_path.trim_start_matches('/'),
            max_bytes,
        )
    }

    /// Read up to `limit` bytes from the start of a file inside a local `file://` archive or directory layer.
    pub fn read_local_file_head(
        &self,
        archive_path: &str,
        layer_digest: &str,
        media_type: &str,
        file_path: &str,
        limit: u64,
    ) -> Result<(Vec<u8>, u64)> {
        let reader = self.open_local_layer_reader(archive_path, layer_digest)?;
        get_file_head_from_layer(reader, media_type, file_path.trim_start_matches('/'), limit)
    }

    /// Check whether a layer blob file currently exists in this analyzer's cache.
    pub fn has_layer_blob(&self, layer_digest: &str) -> bool {
        is_safe_blob_id(layer_digest) && self.inner.store.blob_path(layer_digest).is_file()
    }

    /// Resolve a validated layer blob path inside this analyzer's cache.
    pub fn layer_blob_path(&self, layer_digest: &str) -> Result<PathBuf> {
        if !is_safe_blob_id(layer_digest) {
            return Err(Error::InvalidDigest {
                digest: layer_digest.to_string(),
            });
        }
        Ok(self.inner.store.blob_path(layer_digest))
    }

    /// Invalidate cached manifest and in-memory analysis entries for an image reference.
    pub fn invalidate(&self, image: &str) {
        let info = parse_image_info(image);
        self.invalidate_info(&info);
    }

    pub fn invalidate_info(&self, info: &ImageInfo) {
        self.inner.registry_caches.invalidate_manifest(info);
        if let Some(recent) = &self.inner.recent_analyses {
            let prefix = format!(
                "{}|{}|{}|{}|",
                info.registry, info.user, info.name, info.tag
            );
            if let Ok(mut cache) = recent.lock() {
                cache.remove_prefix(&prefix);
            }
        }
    }

    /// Run TTL expiration and layer size-limit eviction on this analyzer's disk caches.
    pub async fn cleanup_caches(&self) -> Result<CleanupStats> {
        self.inner.store.cleanup().await
    }
}

fn remember_analysis(
    inner: &Arc<AnalyzerInner>,
    key: String,
    result: Arc<DockerAnalyzeResult>,
    lang: Lang,
) {
    let Some(recent) = &inner.recent_analyses else {
        return;
    };
    if let Ok(mut cache) = recent.lock() {
        cache.put(key, (result, lang), Instant::now());
    }
    let ttl = inner.memory_cache_ttl;
    let weak = Arc::downgrade(inner);
    tokio::spawn(async move {
        tokio::time::sleep(ttl).await;
        if let Some(inner) = weak.upgrade() {
            if let Some(recent) = &inner.recent_analyses {
                if let Ok(mut cache) = recent.lock() {
                    cache.purge_expired(Instant::now());
                }
            }
        }
    });
}

fn in_lang(
    result: Arc<DockerAnalyzeResult>,
    result_lang: Lang,
    lang: Lang,
) -> Arc<DockerAnalyzeResult> {
    if result_lang == lang {
        return result;
    }
    let mut owned = result.as_ref().clone();
    owned.recommendations = build_recommendations(&owned, lang);
    Arc::new(owned)
}

fn ensure_rustls_provider() {
    if CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

fn build_default_http_client() -> Result<reqwest::Client> {
    ensure_rustls_provider();
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(32)
        .build()
        .map_err(|e| Error::Other {
            message: format!("failed to build HTTP client: {e}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_analyzer_blocks_local_sources() {
        let analyzer = Analyzer::builder().build().unwrap();
        assert!(analyzer
            .check_allowed(&parse_image_info("file:///tmp/a.tar"))
            .unwrap_err()
            .is_forbidden());
        assert!(analyzer
            .check_allowed(&parse_image_info("docker://redis:alpine"))
            .unwrap_err()
            .is_forbidden());
        assert!(analyzer
            .check_allowed(&parse_image_info("podman://app:dev"))
            .unwrap_err()
            .is_forbidden());
        assert!(analyzer
            .check_allowed(&parse_image_info("redis:alpine"))
            .is_ok());
    }

    #[test]
    fn allowlist_and_local_source_policies_compose() {
        let analyzer = Analyzer::builder()
            .registry_allowlist(["ghcr.io", "local-file"])
            .build()
            .unwrap();
        assert!(analyzer
            .check_allowed(&parse_image_info("ghcr.io/org/app:1"))
            .is_ok());
        assert!(analyzer
            .check_allowed(&parse_image_info("file:///tmp/a.tar"))
            .is_ok());
        assert!(matches!(
            analyzer.check_allowed(&parse_image_info("redis:alpine")),
            Err(Error::DisallowedRegistry { .. })
        ));
        assert!(matches!(
            analyzer.check_allowed(&parse_image_info("docker://redis:alpine")),
            Err(Error::LocalSourceDisabled { .. })
        ));
    }

    #[test]
    fn read_file_rejects_unsafe_digest() {
        let analyzer = Analyzer::builder().build().unwrap();
        let err = analyzer
            .read_file("../../etc/passwd", "tar", "etc/passwd", 1024)
            .unwrap_err();
        assert!(matches!(err, Error::InvalidDigest { .. }));
    }

    #[tokio::test]
    async fn run_in_slot_caps_how_many_tasks_run_at_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        async fn peak_concurrency(limit: Option<usize>) -> usize {
            let mut builder = Analyzer::builder();
            if let Some(n) = limit {
                builder = builder.max_concurrent_analyses(n);
            }
            let analyzer = builder.build().unwrap();
            let running = Arc::new(AtomicUsize::new(0));
            let peak = Arc::new(AtomicUsize::new(0));
            let tasks: Vec<_> = (0..8)
                .map(|_| {
                    let (inner, running, peak) =
                        (analyzer.inner.clone(), running.clone(), peak.clone());
                    tokio::spawn(async move {
                        inner
                            .run_in_slot(async {
                                let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                                peak.fetch_max(now, Ordering::SeqCst);
                                tokio::time::sleep(Duration::from_millis(20)).await;
                                running.fetch_sub(1, Ordering::SeqCst);
                            })
                            .await;
                    })
                })
                .collect();
            for task in tasks {
                task.await.unwrap();
            }
            peak.load(Ordering::SeqCst)
        }

        assert_eq!(peak_concurrency(Some(1)).await, 1);
        assert_eq!(peak_concurrency(Some(3)).await, 3);
        assert_eq!(peak_concurrency(None).await, 8);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dropped_singleflight_caller_does_not_leak_in_flight_or_slot() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            drop(listener);
            let mut buf = [0u8; 512];
            let _ = socket.read(&mut buf).await;
            tokio::time::sleep(Duration::from_millis(30)).await;
            let _ = socket
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
                )
                .await;
        });

        let analyzer = Analyzer::builder()
            .singleflight(true)
            .max_concurrent_analyses(1)
            .build()
            .unwrap();
        let info = ImageInfo {
            registry: format!("http://{addr}/v2"),
            user: "org".into(),
            name: "app".into(),
            tag: "v1".into(),
            arch: "amd64".into(),
        };

        // Caller drops the future before the server answers.
        let timed_out = tokio::time::timeout(
            Duration::from_millis(5),
            analyzer.analyze_info(info, AnalyzeOptions::default()),
        )
        .await;
        assert!(timed_out.is_err());

        server.await.unwrap();
        for _ in 0..100 {
            if analyzer.inner.in_flight.lock().await.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(analyzer.inner.in_flight.lock().await.is_empty());

        // The single concurrency slot must have been released too.
        let acquired = tokio::time::timeout(
            Duration::from_millis(100),
            analyzer.inner.run_in_slot(async { true }),
        )
        .await;
        assert!(acquired.unwrap());
    }

    #[test]
    fn block_private_registries_rejects_ssrf_targets_and_allows_public_hosts() {
        let analyzer = Analyzer::builder()
            .block_private_registries(true)
            .build()
            .unwrap();

        for blocked in [
            "169.254.169.254/latest/meta-data:v1",
            "10.0.0.5:5000/app:latest",
            "127.0.0.1:5000/app:latest",
            "localhost:5000/app:latest",
            "metadata.google.internal/app:v1",
            "192.168.1.10/app:v1",
        ] {
            let err = analyzer
                .check_allowed(&parse_image_info(blocked))
                .unwrap_err();
            assert!(err.is_forbidden(), "expected forbidden for {blocked}");
        }

        for allowed in ["redis:alpine", "ghcr.io/org/app:v1", "quay.io/org/app:v1"] {
            assert!(
                analyzer.check_allowed(&parse_image_info(allowed)).is_ok(),
                "expected {allowed} to be allowed"
            );
        }
    }
}
