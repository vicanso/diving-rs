use super::elf::{
    analyze_runtime_compat, check_unstripped_binary, sort_unstripped_binaries,
    unstripped_candidates, UnstrippedBinary, MAX_STRIP_CANDIDATE_BYTES,
};
use super::image_ref::{
    is_local_registry, normalize_arch, repository_path, ImageInfo, REGISTRY_LOCAL_CONTAINER,
    REGISTRY_LOCAL_DOCKER, REGISTRY_LOCAL_FILE, REGISTRY_LOCAL_PODMAN,
};
use super::layer::{is_os_release_entry, scan_blob_targets};
use super::local::{save_container_image_to_file, save_local_image_to_file};
use super::model::{DockerAnalyzeResult, ImagePreview};
use super::oci_image::{plan_cross_layer_duplicates, DuplicateGroup, FileTreeItem, ImageLayer};
use super::registry::{
    resolve_layer_concurrency, DockerImageParams, RegistryCaches, RegistryClient,
};
use super::registry_auth::{DockerConfig, RegistryCredentials, RegistryRuleSet};
use super::scan::{
    build_risk_tags, detect_os_from_history, display_name, extract_base_os_from_layers,
    extract_health_and_ports, extract_image_exec_meta, extract_image_meta, preview_layers,
    probe_base_os, reconstruct_dockerfile, scan_layers, LayerScan,
};
use crate::error::{Error, IoSnafu, Result};
use crate::i18n::Lang;
use crate::progress::{emit_progress, ProgressCallback, ProgressEvent};
use crate::recommend::build_recommendations;
use crate::sensitive::SensitivePatterns;
use crate::store::{
    read_analysis, read_last_known, remember_digest, touch_blob_times, write_analysis, CacheStore,
};
use crate::tl_info;
use chrono::DateTime;
use snafu::ResultExt;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;
use tempfile::{Builder as TempBuilder, NamedTempFile};
use tokio::sync::OnceCell as TokioOnceCell;

pub(crate) struct PipelineContext {
    pub(crate) http_client: reqwest::Client,
    pub(crate) store: Arc<CacheStore>,
    pub(crate) registry_caches: Arc<RegistryCaches>,
    pub(crate) docker_config: Option<Arc<DockerConfig>>,
    pub(crate) sensitive_patterns: Arc<SensitivePatterns>,
    pub(crate) layer_concurrency: Option<usize>,
    pub(crate) block_private_hosts: bool,
    pub(crate) registries: Arc<RegistryRuleSet>,
    pub(crate) rule_clients: Arc<HashMap<usize, reqwest::Client>>,
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
    new_client_with_local_cache(ctx, registry, false)
}

fn new_client_with_local_cache(
    ctx: &PipelineContext,
    registry: &str,
    cache_local_blobs: bool,
) -> RegistryClient {
    RegistryClient {
        registry: registry.to_string(),
        http_client: ctx.http_client.clone(),
        store: ctx.store.clone(),
        caches: ctx.registry_caches.clone(),
        docker_config: ctx.docker_config.clone(),
        layer_concurrency: ctx.layer_concurrency,
        block_private_hosts: ctx.block_private_hosts,
        registries: ctx.registries.clone(),
        rule_clients: ctx.rule_clients.clone(),
        local_index: Arc::new(TokioOnceCell::new()),
        cache_local_blobs,
    }
}

fn scan_unstripped_and_duplicates(
    layers: &[ImageLayer],
    file_tree_list: &[Vec<FileTreeItem>],
    store: &CacheStore,
    concurrency: usize,
    verify_dup: bool,
) -> (Vec<DuplicateGroup>, Vec<UnstrippedBinary>) {
    let strip_by_layer = unstripped_candidates(file_tree_list);
    let dup_plan = verify_dup
        .then(|| plan_cross_layer_duplicates(file_tree_list))
        .flatten();
    let mut job_set: BTreeSet<usize> = strip_by_layer.keys().copied().collect();
    if let Some(plan) = &dup_plan {
        job_set.extend(plan.by_layer.keys().copied());
    }
    let jobs: Vec<usize> = job_set.into_iter().collect();
    if jobs.is_empty() {
        return (vec![], vec![]);
    }

    let workers = concurrency.max(1).min(jobs.len());
    let next_job = AtomicUsize::new(0);
    let empty_set = HashSet::new();
    let found_strip = Mutex::new(Vec::new());
    let found_hashes = Mutex::new(HashMap::new());

    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                let mut local_strip = Vec::new();
                let mut local_hashes = Vec::new();
                while let Some(&layer_idx) = jobs.get(next_job.fetch_add(1, Ordering::Relaxed)) {
                    let Some(layer) = layers.get(layer_idx) else {
                        continue;
                    };
                    let blob = store.blob_path(&layer.digest);
                    let Ok(file) = File::open(&blob) else {
                        continue;
                    };
                    touch_blob_times(&blob);
                    let strip_targets = strip_by_layer.get(&layer_idx).unwrap_or(&empty_set);
                    let dup_targets = dup_plan
                        .as_ref()
                        .and_then(|p| p.by_layer.get(&layer_idx))
                        .unwrap_or(&empty_set);
                    if let Ok(hashes) = scan_blob_targets(
                        BufReader::new(file),
                        &layer.media_type,
                        strip_targets,
                        dup_targets,
                        MAX_STRIP_CANDIDATE_BYTES,
                        |path, bytes| {
                            if let Some(entry) = check_unstripped_binary(layer_idx, path, bytes) {
                                local_strip.push(entry);
                            }
                        },
                    ) {
                        local_hashes.extend(hashes.into_iter().map(|(p, h)| ((layer_idx, p), h)));
                    }
                }
                if !local_strip.is_empty() {
                    if let Ok(mut guard) = found_strip.lock() {
                        guard.extend(local_strip);
                    }
                }
                if !local_hashes.is_empty() {
                    if let Ok(mut guard) = found_hashes.lock() {
                        guard.extend(local_hashes);
                    }
                }
            });
        }
    });

    let mut unstripped = found_strip.into_inner().unwrap_or_default();
    sort_unstripped_binaries(&mut unstripped);
    let dup = match dup_plan {
        Some(plan) => plan.finish(&found_hashes.into_inner().unwrap_or_default()),
        None => vec![],
    };
    (dup, unstripped)
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
    let auth = client.get_auth(params).await?;
    params.auth = auth;

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
    let (mut manifest, supported_archs) = client.get_manifest(params).await?;
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
        .get_all_layer_info(params, &mut manifest.layers)
        .await?;

    tl_info!(repo = repo.as_str(), tag = tag, "analyze image done");
    let image_name = display_name(&repo, tag);
    let image_arch = config.architecture.clone();
    let image_os = config.os.clone();
    let dockerfile = reconstruct_dockerfile(&config.history);
    let (run_user, envs, labels) = extract_image_meta(&config);
    let (entrypoint, cmd, working_dir) = extract_image_exec_meta(&config);
    let (has_healthcheck, exposed_ports) = extract_health_and_ports(&config);

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
        } else {
            let needs_probe = info_list.iter().take(3).any(|info| {
                info.os_release.is_none()
                    && info.files.iter().any(|f| {
                        f.is_whiteout.is_none()
                            && is_os_release_entry(&f.path).is_some()
                            && f.size > 0
                    })
            });
            let probed = if needs_probe {
                probe_base_os(&manifest, &store_for_probe)
            } else {
                String::new()
            };
            if probed.is_empty() {
                detect_os_from_history(&scan.layers)
                    .unwrap_or_else(|| "Scratch / Distroless (no OS identifier found)".to_string())
            } else {
                probed
            }
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
        let started = Instant::now();
        let (dup, unstripped) = scan_unstripped_and_duplicates(
            &layers,
            &file_tree_list,
            &store_for_probe,
            concurrency,
            verify_dup,
        );
        tl_info!(
            groups = dup.len(),
            unstripped = unstripped.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "post-scan probes done"
        );
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
        entrypoint,
        cmd,
        working_dir,
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
    params.auth = client.get_auth(&params).await?;
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

async fn analyze_saved_tar(
    ctx: &PipelineContext,
    tmpfile: NamedTempFile,
    arch: &str,
    opts: PipelineOptions,
) -> Result<Arc<DockerAnalyzeResult>> {
    let filename = tmpfile.path().to_string_lossy().to_string();
    let client = new_client_with_local_cache(ctx, REGISTRY_LOCAL_FILE, true);
    let mut params = DockerImageParams {
        img: filename,
        arch: arch.to_string(),
        verify_dup: opts.verify_dup,
        credentials: opts.credentials,
        on_progress: opts.on_progress,
        ..Default::default()
    };
    let result = analyze_client(ctx, &client, &mut params, opts.lang).await;
    drop(tmpfile);
    result
}

pub(crate) async fn analyze_once(
    ctx: &PipelineContext,
    image_info: ImageInfo,
    opts: PipelineOptions,
) -> Result<Arc<DockerAnalyzeResult>> {
    if image_info.registry == REGISTRY_LOCAL_CONTAINER {
        let tmpfile = TempBuilder::new().tempfile().context(IoSnafu {})?;
        save_container_image_to_file(&image_info.name, &image_info.arch, tmpfile.path()).await?;
        return analyze_saved_tar(ctx, tmpfile, &image_info.arch, opts).await;
    }
    let save_cli = match image_info.registry.as_str() {
        REGISTRY_LOCAL_DOCKER => Some("docker"),
        REGISTRY_LOCAL_PODMAN => Some("podman"),
        _ => None,
    };
    if let Some(cli) = save_cli {
        let tmpfile = TempBuilder::new().tempfile().context(IoSnafu {})?;
        save_local_image_to_file(cli, &image_info.name, tmpfile.path()).await?;
        analyze_saved_tar(ctx, tmpfile, &image_info.arch, opts).await
    } else {
        let cache_local = image_info.registry == REGISTRY_LOCAL_FILE;
        let client = new_client_with_local_cache(ctx, &image_info.registry, cache_local);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::DiffOptions;
    use crate::store::{is_safe_blob_id, sha256_hex, CacheConfig};
    use crate::Analyzer;
    use serde_json::{json, to_vec};
    use std::fs::{self, write as write_bytes};
    use tar::{Builder as TarBuilder, Header as TarHeader};
    use tempfile::tempdir;

    fn make_tar(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = TarBuilder::new(Vec::new());
        for &(path, data) in entries {
            let mut header = TarHeader::new_gnu();
            header.set_path(path).unwrap();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_uid(0);
            header.set_gid(0);
            header.set_cksum();
            builder.append(&header, data).unwrap();
        }
        builder.into_inner().unwrap()
    }

    fn make_docker_save_tar(layer2_diff_id: &str, settings_json: &[u8]) -> Vec<u8> {
        let dup_payload = [0x5au8; 64 * 1024];
        let layer1 = make_tar(&[
            ("etc/os-release", b"PRETTY_NAME=\"Test Linux\"\n"),
            ("a/shared.dat", &dup_payload),
        ]);
        let layer2 = make_tar(&[
            ("app/settings.json", settings_json),
            ("b/shared.dat", &dup_payload),
        ]);
        let config = json!({
            "architecture": "amd64",
            "os": "linux",
            "created": "2024-06-01T00:00:00Z",
            "history": [
                {"created": "2024-01-01T00:00:00Z", "created_by": "/bin/sh -c #(nop) ADD rootfs.tar /"},
                {"created": "2024-06-01T00:00:00Z", "created_by": "/bin/sh -c #(nop) ENV APP=1", "empty_layer": true},
                {"created": "2024-06-01T00:00:00Z", "created_by": "/bin/sh -c COPY app /app"}
            ],
            "rootfs": {"type": "layers", "diff_ids": ["sha256:l1", layer2_diff_id]}
        });
        let manifest = json!([{
            "Config": "config.json",
            "RepoTags": ["svc:latest"],
            "Layers": ["layer1/layer.tar", "layer2/layer.tar"]
        }]);
        make_tar(&[
            ("manifest.json", manifest.to_string().as_bytes()),
            ("config.json", config.to_string().as_bytes()),
            ("layer1/layer.tar", &layer1),
            ("layer2/layer.tar", &layer2),
        ])
    }

    fn test_ctx(store: Arc<CacheStore>) -> PipelineContext {
        PipelineContext {
            http_client: reqwest::Client::new(),
            store,
            registry_caches: Arc::new(RegistryCaches::default()),
            docker_config: None,
            sensitive_patterns: Arc::new(SensitivePatterns::default()),
            layer_concurrency: Some(2),
            block_private_hosts: false,
            registries: Arc::new(RegistryRuleSet::default()),
            rule_clients: Arc::new(HashMap::new()),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn analyze_saved_tar_caches_non_empty_layers_for_preview_export_and_diff() {
        let dir = tempdir().unwrap();
        let layer_dir = dir.path().join("layers");
        let analysis_dir = dir.path().join("analysis");
        let cfg = CacheConfig::persistent(&layer_dir, &analysis_dir);
        let store = Arc::new(CacheStore::from_config(cfg.clone()).unwrap());
        let analyzer = Analyzer::builder()
            .cache(cfg)
            .allow_local_sources(true)
            .build()
            .unwrap();
        let ctx = test_ctx(store);

        let base_tmp = TempBuilder::new().suffix(".tar").tempfile().unwrap();
        let base_path = base_tmp.path().to_path_buf();
        write_bytes(
            &base_path,
            make_docker_save_tar("sha256:l2-a", br#"{"id":"A"}"#),
        )
        .unwrap();
        let base_res = analyze_saved_tar(
            &ctx,
            base_tmp,
            "",
            PipelineOptions {
                lang: Lang::En,
                verify_dup: true,
                credentials: None,
                on_progress: None,
            },
        )
        .await
        .expect("analyze base saved tar");
        assert!(!base_path.exists(), "temp tar should be dropped");
        assert_eq!(base_res.duplicate_groups.len(), 1);

        let target_tmp = TempBuilder::new().suffix(".tar").tempfile().unwrap();
        let target_path = target_tmp.path().to_path_buf();
        write_bytes(
            &target_path,
            make_docker_save_tar("sha256:l2-b", br#"{"id":"B"}"#),
        )
        .unwrap();
        let target_res = analyze_saved_tar(
            &ctx,
            target_tmp,
            "",
            PipelineOptions {
                lang: Lang::En,
                verify_dup: true,
                credentials: None,
                on_progress: None,
            },
        )
        .await
        .expect("analyze target saved tar");
        assert!(!target_path.exists(), "temp tar should be dropped");

        // Non-empty layers are atomically cached under sha256 digests; empty ENV layer is untouched.
        assert_eq!(base_res.layers.len(), 3);
        assert!(base_res.layers[1].empty);
        assert!(base_res.layers[1].digest.is_empty());
        for idx in [0, 2] {
            let digest = &base_res.layers[idx].digest;
            assert!(
                digest.starts_with("sha256:") && is_safe_blob_id(digest),
                "unexpected digest {digest}"
            );
            assert!(analyzer.has_layer_blob(digest));
        }

        // No orphaned temp files left in layer_dir.
        for entry in fs::read_dir(&layer_dir).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().to_string();
            assert!(!name.contains(".tmp-"), "leftover temp file {name}");
        }

        // FilePreviewSheet (read_file_head) and Save file as... (stream_file) work after temp tar cleanup.
        let (head, total) = analyzer
            .read_file_head(
                &base_res.layers[2].digest,
                &base_res.layers[2].media_type,
                "app/settings.json",
                64,
            )
            .expect("preview cached docker/podman layer file");
        assert_eq!(head, br#"{"id":"A"}"#);
        assert_eq!(total, 10);

        let mut exported = Vec::new();
        analyzer
            .stream_file(
                &target_res.layers[2].digest,
                &target_res.layers[2].media_type,
                "app/settings.json",
                1024,
                |chunk| {
                    exported.extend_from_slice(chunk);
                    Ok(())
                },
            )
            .expect("stream cached docker/podman layer file");
        assert_eq!(exported, br#"{"id":"B"}"#);

        // compute_diff hashes same-size files from cached blobs for docker:// and podman:// images.
        let diff = analyzer
            .compute_diff(
                "docker://svc:1",
                "podman://svc:2",
                base_res,
                target_res,
                DiffOptions::default(),
            )
            .await
            .expect("compute diff over cached local blobs");
        assert!(diff.content_verified);
        assert_eq!(diff.parity.same_size_content_changes, 1);
        assert!(diff
            .changed
            .files
            .iter()
            .any(|c| c.path == "app/settings.json" && c.content_changed));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn analyze_saved_tar_caches_oci_blobs_and_skips_zero_byte_layers() {
        let dir = tempdir().unwrap();
        let layer_dir = dir.path().join("layers");
        let analysis_dir = dir.path().join("analysis");
        let cfg = CacheConfig::persistent(&layer_dir, &analysis_dir);
        let store = Arc::new(CacheStore::from_config(cfg.clone()).unwrap());
        let analyzer = Analyzer::builder()
            .cache(cfg)
            .allow_local_sources(true)
            .build()
            .unwrap();
        let ctx = test_ctx(store);

        let layer_bytes = make_tar(&[("etc/app.conf", b"port=8080\n")]);
        let layer_digest = format!("sha256:{}", sha256_hex(&layer_bytes));
        let layer_blob_rel = format!("blobs/{}", layer_digest.replacen(':', "/", 1));

        let empty_blob_rel =
            "blobs/sha256/0000000000000000000000000000000000000000000000000000000000000000";
        let config_bytes = to_vec(&json!({
            "architecture": "amd64",
            "os": "linux",
            "created": "2024-06-01T00:00:00Z",
            "history": [
                {"created": "2024-06-01T00:00:00Z", "created_by": "COPY app.conf /etc/app.conf"},
                {"created": "2024-06-01T00:00:00Z", "created_by": "COPY empty /"}
            ],
            "rootfs": {"type": "layers", "diff_ids": ["sha256:aaa", "sha256:bbb"]}
        }))
        .unwrap();
        let config_digest = format!("sha256:{}", sha256_hex(&config_bytes));
        let config_blob_rel = format!("blobs/{}", config_digest.replacen(':', "/", 1));

        let manifest_bytes = to_vec(&json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "config": {
                "mediaType": "application/vnd.oci.image.config.v1+json",
                "digest": config_digest,
                "size": config_bytes.len()
            },
            "layers": [
                {
                    "mediaType": "application/vnd.oci.image.layer.v1.tar",
                    "digest": layer_digest,
                    "size": layer_bytes.len()
                },
                {
                    "mediaType": "application/vnd.oci.image.layer.v1.tar",
                    "digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
                    "size": 0
                }
            ]
        }))
        .unwrap();
        let manifest_digest = format!("sha256:{}", sha256_hex(&manifest_bytes));
        let manifest_blob_rel = format!("blobs/{}", manifest_digest.replacen(':', "/", 1));
        let index_bytes = to_vec(&json!({
            "schemaVersion": 2,
            "manifests": [{
                "mediaType": "application/vnd.oci.image.manifest.v1+json",
                "digest": manifest_digest,
                "size": manifest_bytes.len(),
                "platform": {"architecture": "amd64", "os": "linux"}
            }]
        }))
        .unwrap();

        let oci_tar = make_tar(&[
            ("oci-layout", br#"{"imageLayoutVersion":"1.0.0"}"#),
            ("index.json", &index_bytes),
            (&manifest_blob_rel, &manifest_bytes),
            (&config_blob_rel, &config_bytes),
            (&layer_blob_rel, &layer_bytes),
            (empty_blob_rel, b""),
        ]);
        let tmp = TempBuilder::new().suffix(".tar").tempfile().unwrap();
        write_bytes(tmp.path(), oci_tar).unwrap();

        let res = analyze_saved_tar(
            &ctx,
            tmp,
            "",
            PipelineOptions {
                lang: Lang::En,
                verify_dup: false,
                credentials: None,
                on_progress: None,
            },
        )
        .await
        .expect("analyze saved OCI tar");

        assert_eq!(res.layers[0].digest, layer_digest);
        assert!(analyzer.has_layer_blob(&layer_digest));
        let content = analyzer
            .read_file(
                &layer_digest,
                &res.layers[0].media_type,
                "etc/app.conf",
                1024,
            )
            .expect("read cached OCI layer");
        assert_eq!(content, b"port=8080\n");

        // Zero-byte layer must not be cached.
        assert!(!analyzer.has_layer_blob(
            "sha256:0000000000000000000000000000000000000000000000000000000000000000"
        ));
    }
}
