use super::elf::{analyze_runtime_compat, find_unstripped_binaries};
use super::image_ref::{
    is_local_registry, normalize_arch, repository_path, ImageInfo, REGISTRY_LOCAL_DOCKER,
    REGISTRY_LOCAL_FILE, REGISTRY_LOCAL_PODMAN,
};
use super::local::save_local_image_to_file;
use super::model::{DockerAnalyzeResult, ImagePreview};
use super::oci_image::detect_cross_layer_duplicates;
use super::registry::{
    resolve_layer_concurrency, DockerImageParams, RegistryCaches, RegistryClient,
};
use super::registry_auth::{DockerConfig, RegistryCredentials};
use super::scan::{
    build_risk_tags, detect_os_from_history, display_name, extract_base_os_from_layers,
    extract_health_and_ports, extract_image_meta, preview_layers, probe_base_os,
    reconstruct_dockerfile, scan_layers, LayerScan,
};
use crate::error::{Error, IoSnafu, Result};
use crate::i18n::Lang;
use crate::progress::{emit_progress, ProgressCallback, ProgressEvent};
use crate::recommend::build_recommendations;
use crate::sensitive::SensitivePatterns;
use crate::store::{read_analysis, read_last_known, remember_digest, write_analysis, CacheStore};
use crate::tl_info;
use chrono::DateTime;
use snafu::ResultExt;
use std::sync::Arc;
use std::thread;
use std::time::Instant;
use tokio::sync::OnceCell as TokioOnceCell;

pub(crate) struct PipelineContext {
    pub(crate) http_client: reqwest::Client,
    pub(crate) store: Arc<CacheStore>,
    pub(crate) registry_caches: Arc<RegistryCaches>,
    pub(crate) docker_config: Option<Arc<DockerConfig>>,
    pub(crate) sensitive_patterns: Arc<SensitivePatterns>,
    pub(crate) layer_concurrency: Option<usize>,
    pub(crate) block_private_hosts: bool,
}

pub(crate) struct PipelineOptions {
    pub(crate) lang: Lang,
    pub(crate) verify_dup: bool,
    pub(crate) credentials: Option<RegistryCredentials>,
    pub(crate) on_progress: Option<ProgressCallback>,
}

pub(crate) fn image_ref_key(registry: &str, repo: &str, tag: &str) -> String {
    format!("{registry}/{repo}:{tag}")
}

fn new_client(ctx: &PipelineContext, registry: &str) -> RegistryClient {
    RegistryClient {
        registry: registry.to_string(),
        http_client: ctx.http_client.clone(),
        store: ctx.store.clone(),
        caches: ctx.registry_caches.clone(),
        docker_config: ctx.docker_config.clone(),
        layer_concurrency: ctx.layer_concurrency,
        block_private_hosts: ctx.block_private_hosts,
        local_index: Arc::new(TokioOnceCell::new()),
    }
}

fn normalize_supported_archs(archs: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(archs.len());
    for arch in archs.iter().map(|a| normalize_arch(a)) {
        if !out.contains(&arch) {
            out.push(arch);
        }
    }
    out
}

async fn analyze_client(
    ctx: &PipelineContext,
    client: &RegistryClient,
    params: &mut DockerImageParams,
    lang: Lang,
) -> Result<Arc<DockerAnalyzeResult>> {
    if !client.is_local() {
        emit_progress(
            params.on_progress.as_ref(),
            ProgressEvent::Authenticating {
                registry: client.registry.clone(),
            },
        );
    }
    let token = client.get_auth_token(params).await?;
    params.token = token;

    let cache_key: Option<(String, String)> = if params.verify_dup {
        client.head_manifest_digest(params).await
    } else {
        None
    };
    if let Some((digest, cache_arch)) = &cache_key {
        if let Some(mut cached) = read_analysis(&ctx.store, digest, cache_arch).await {
            tl_info!(digest = digest, "analysis cache hit");
            emit_progress(
                params.on_progress.as_ref(),
                ProgressEvent::AnalysisCacheHit {
                    digest: digest.clone(),
                },
            );
            cached.recommendations = build_recommendations(&cached, lang);
            cached.supported_archs = normalize_supported_archs(&cached.supported_archs);
            let image_ref = image_ref_key(&client.registry, &params.repo(), &params.tag);
            remember_digest(&ctx.store, &image_ref, &params.arch, digest).await;
            return Ok(Arc::new(cached));
        }
    }

    if !client.is_local() {
        emit_progress(params.on_progress.as_ref(), ProgressEvent::FetchingManifest);
    }
    let (manifest, supported_archs) = client.get_manifest(params).await?;
    let config = client
        .get_image_config(params, &manifest.config.digest)
        .await?;
    let repo = params.repo();
    let tag = &params.tag;

    tl_info!(repo = repo.as_str(), tag = tag, "analyzing image");

    if !client.is_local() {
        let layer_count = manifest.layers.len();
        let total_bytes: u64 = manifest.layers.iter().map(|l| l.size).sum();
        emit_progress(
            params.on_progress.as_ref(),
            ProgressEvent::LayersDiscovered {
                count: layer_count,
                total_bytes,
            },
        );
    }
    let info_list = client
        .get_all_layer_info(params, manifest.layers.clone())
        .await?;

    tl_info!(repo = repo.as_str(), tag = tag, "analyze image done");
    let image_name = display_name(&repo, tag);
    let image_arch = config.architecture.clone();
    let image_os = config.os.clone();
    let dockerfile = reconstruct_dockerfile(&config.history);
    let (run_user, envs, labels) = extract_image_meta(&config);
    let (has_healthcheck, exposed_ports) = extract_health_and_ports(&config);

    let is_local = client.is_local();
    let verify_dup = params.verify_dup;
    let store_for_probe = ctx.store.clone();
    let sensitive_patterns = ctx.sensitive_patterns.clone();
    let configured_concurrency = ctx.layer_concurrency;
    let (
        run_user,
        base_os,
        tags,
        layers,
        file_tree_list,
        file_summary_list,
        big_modified_file_list,
        sensitive_files,
        image_size,
        image_total_size,
        duplicate_groups,
        runtime_compat,
        unstripped_binaries,
    ) = tokio::task::spawn_blocking(move || {
        let scan = scan_layers(&config, &manifest.layers, &info_list, &sensitive_patterns);
        let base_os = if let Some(found) = extract_base_os_from_layers(&info_list) {
            found
        } else if !is_local {
            let base_os = probe_base_os(&manifest, &store_for_probe);
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

        let concurrency = resolve_layer_concurrency(configured_concurrency, layers.len());
        let (strip_concurrency, dup_concurrency) = if verify_dup {
            let half = concurrency.div_ceil(2).max(1);
            (half, half)
        } else {
            (concurrency, 0)
        };
        let (dup, unstripped) = thread::scope(|scope| {
            let strip = scope.spawn(|| {
                let started = Instant::now();
                let found = find_unstripped_binaries(
                    &layers,
                    &file_tree_list,
                    &store_for_probe,
                    strip_concurrency,
                );
                tl_info!(
                    found = found.len(),
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "unstripped binary scan done"
                );
                found
            });
            let started = Instant::now();
            let dup = if verify_dup {
                detect_cross_layer_duplicates(
                    &layers,
                    &file_tree_list,
                    &store_for_probe,
                    dup_concurrency,
                )
            } else {
                vec![]
            };
            tl_info!(
                groups = dup.len(),
                elapsed_ms = started.elapsed().as_millis() as u64,
                "duplicate scan done"
            );
            (dup, strip.join().unwrap_or_default())
        });
        let rc = analyze_runtime_compat(
            &config,
            &layers,
            &file_tree_list,
            &base_os,
            &store_for_probe,
        )
        .unwrap_or_default();
        (
            run_user,
            base_os,
            tags,
            layers,
            file_tree_list,
            file_summary_list,
            big_modified_file_list,
            sensitive_files,
            image_size,
            image_total_size,
            dup,
            rc,
            unstripped,
        )
    })
    .await
    .map_err(|e| Error::Other {
        message: format!("post-analysis probe task failed: {e}"),
    })?;

    let mut result = DockerAnalyzeResult {
        name: image_name,
        arch: image_arch,
        os: image_os,
        user: run_user,
        envs,
        labels,
        dockerfile,
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
        unstripped_binaries,
        has_healthcheck,
        exposed_ports,
        stale_as_of: None,
    };
    result.recommendations = build_recommendations(&result, lang);
    let result = Arc::new(result);

    if let Some((digest, cache_arch)) = &cache_key {
        write_analysis(&ctx.store, digest, cache_arch, Arc::clone(&result)).await;
        let image_ref = image_ref_key(&client.registry, &params.repo(), &params.tag);
        remember_digest(&ctx.store, &image_ref, &params.arch, digest).await;
    }

    Ok(result)
}

pub(crate) async fn preview_image(
    ctx: &PipelineContext,
    image_info: ImageInfo,
    credentials: Option<RegistryCredentials>,
) -> Result<ImagePreview> {
    if is_local_registry(&image_info.registry) {
        return Err(Error::PreviewUnsupported {
            message: "a preview is only available for registry images".to_string(),
        });
    }
    let client = new_client(ctx, &image_info.registry);
    let mut params = DockerImageParams {
        user: image_info.user,
        img: image_info.name,
        tag: image_info.tag,
        arch: image_info.arch,
        credentials,
        ..Default::default()
    };
    params.token = client.get_auth_token(&params).await?;
    let (manifest, supported_archs) = client.get_manifest(&params).await?;
    let config = client
        .get_image_config(&params, &manifest.config.digest)
        .await?;
    Ok(ImagePreview {
        name: display_name(&params.repo(), &params.tag),
        layers: preview_layers(&config.history, &manifest.layers),
        arch: config.architecture,
        os: config.os,
        size: manifest.layers.iter().map(|l| l.size).sum(),
        supported_archs,
    })
}

pub(crate) async fn analyze_once(
    ctx: &PipelineContext,
    image_info: ImageInfo,
    opts: PipelineOptions,
) -> Result<Arc<DockerAnalyzeResult>> {
    let save_cli = match image_info.registry.as_str() {
        REGISTRY_LOCAL_DOCKER => Some("docker"),
        REGISTRY_LOCAL_PODMAN => Some("podman"),
        _ => None,
    };
    if let Some(cli) = save_cli {
        let tmpfile = tempfile::Builder::new().tempfile().context(IoSnafu {})?;
        let filename = tmpfile.path().to_string_lossy().to_string();
        save_local_image_to_file(cli, &image_info.name, tmpfile.path()).await?;

        let client = new_client(ctx, REGISTRY_LOCAL_FILE);
        let mut params = DockerImageParams {
            img: filename,
            verify_dup: opts.verify_dup,
            credentials: opts.credentials,
            on_progress: opts.on_progress,
            ..Default::default()
        };
        analyze_client(ctx, &client, &mut params, opts.lang).await
    } else {
        let client = new_client(ctx, &image_info.registry);
        let mut params = DockerImageParams {
            user: image_info.user,
            img: image_info.name,
            tag: image_info.tag,
            arch: image_info.arch,
            verify_dup: opts.verify_dup,
            credentials: opts.credentials,
            on_progress: opts.on_progress,
            ..Default::default()
        };
        analyze_client(ctx, &client, &mut params, opts.lang).await
    }
}

pub(crate) async fn analyze_or_last_known(
    ctx: &PipelineContext,
    image_info: ImageInfo,
    opts: PipelineOptions,
) -> Result<Arc<DockerAnalyzeResult>> {
    let image_ref = image_ref_key(
        &image_info.registry,
        &repository_path(&image_info.user, &image_info.name),
        &image_info.tag,
    );
    let arch = image_info.arch.clone();
    let lang = opts.lang;
    let on_progress = opts.on_progress.clone();
    let err = match analyze_once(ctx, image_info, opts).await {
        Err(err) if err.is_registry_unavailable() => err,
        other => return other,
    };
    let Some((mut cached, cached_at)) = read_last_known(&ctx.store, &image_ref, &arch).await else {
        return Err(err);
    };
    let as_of = DateTime::from_timestamp(cached_at, 0)
        .unwrap_or_default()
        .to_rfc3339();
    tl_info!(
        image = image_ref,
        err = err.to_string(),
        as_of,
        "registry unavailable, serving last known analysis"
    );
    emit_progress(
        on_progress.as_ref(),
        ProgressEvent::UsingStaleCache {
            as_of: as_of.clone(),
        },
    );
    cached.recommendations = build_recommendations(&cached, lang);
    cached.supported_archs = normalize_supported_archs(&cached.supported_archs);
    cached.stale_as_of = Some(as_of);
    Ok(Arc::new(cached))
}
