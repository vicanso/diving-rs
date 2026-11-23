//! Async analysis bridge running `diving-core` pipelines on a multi-threaded Tokio runtime.

use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use diving_app::config::{build_desktop_analyzer, build_http_client, load_config};
use diving_core::{
    AnalyzeOptions, Analyzer, CleanupStats, DiffOptions, DockerAnalyzeResult, Error as CoreError,
    ImageDiff, ImagePreview, Lang, LocalContainerImage, LocalRuntime, PreviewOptions,
    ProgressEvent, REGISTRY_LOCAL_FILE, diff_images_with_options, list_local_images,
    parse_image_info, to_diff_markdown, to_markdown,
};
use tokio::runtime::{Builder, Runtime};
use tokio::sync::{mpsc, oneshot};
use tokio::task::AbortHandle;

use crate::domain::model::{
    FilePreviewContent, ImageSourceKind, classify_file_preview, image_source_kind,
};
use crate::domain::treemap::{FsNode, build_final_tree};

const FILE_PREVIEW_LIMIT: u64 = 128 * 1024;
const FILE_EXPORT_LIMIT: u64 = 100 * 1024 * 1024;

/// Progress, preview, completion, or failure event streamed from `DivingService::start_analyze`.
#[derive(Debug, Clone)]
pub enum AnalyzeMessage {
    Progress(ProgressEvent),
    Preview(ImagePreview),
    Finished(Arc<DockerAnalyzeResult>),
    Failed(String),
}

/// Outcome returned when running concurrent base/target analysis for `Image Diff`.
#[derive(Debug, Clone)]
pub enum DiffRunResult {
    Ready {
        diff: Arc<ImageDiff>,
        base: Arc<DockerAnalyzeResult>,
        target: Arc<DockerAnalyzeResult>,
    },
    BaseFailed(String),
    TargetFailed(String),
    BothFailed {
        base: String,
        target: String,
    },
}

/// Progress or completion event streamed from `DivingService::start_diff`.
#[derive(Debug, Clone)]
pub enum DiffMessage {
    BaseProgress(ProgressEvent),
    TargetProgress(ProgressEvent),
    Finished(DiffRunResult),
}

/// Handle cancelling both the analysis task and its delayed 500ms preview task.
pub struct AnalyzeTaskHandle {
    tasks: Vec<AbortHandle>,
}

impl AnalyzeTaskHandle {
    pub fn abort(self) {
        for handle in self.tasks {
            handle.abort();
        }
    }
}

struct ServiceInner {
    rt: Runtime,
    analyzer: Analyzer,
    config_warning: Option<String>,
}

/// Thread-safe handle wrapping the desktop `Analyzer` and shared Tokio runtime.
#[derive(Clone)]
pub struct DivingService {
    inner: Arc<ServiceInner>,
}

impl Default for DivingService {
    fn default() -> Self {
        Self::new()
    }
}

impl DivingService {
    /// Initializes config from `~/.diving/config.yml` (falling back safely on error)
    /// and starts the dedicated multi-threaded Tokio runtime.
    pub fn new() -> Self {
        let mut config_warning = None;
        let loaded = match load_config(None) {
            Ok(cfg) => Some(cfg),
            Err(err) => {
                log::warn!("falling back to default desktop config: {err}");
                config_warning = Some(err);
                None
            }
        };
        let worker_threads = loaded
            .as_ref()
            .map_or(4, |cfg| cfg.config.worker_threads().clamp(1, 8));
        let rt = Builder::new_multi_thread()
            .worker_threads(worker_threads)
            .enable_all()
            .build()
            .expect("create Tokio runtime for DivingService");

        let analyzer = loaded
            .as_ref()
            .map(|cfg| {
                build_http_client().and_then(|client| {
                    build_desktop_analyzer(cfg, client).map_err(|err| err.to_string())
                })
            })
            .transpose()
            .unwrap_or_else(|err| {
                log::warn!("falling back to ephemeral desktop analyzer: {err}");
                if config_warning.is_none() {
                    config_warning = Some(err);
                }
                None
            })
            .unwrap_or_else(|| {
                Analyzer::builder()
                    .allow_local_sources(true)
                    .use_docker_config(true)
                    .build()
                    .expect("build ephemeral desktop analyzer")
            });

        Self {
            inner: Arc::new(ServiceInner {
                rt,
                analyzer,
                config_warning,
            }),
        }
    }

    /// Creates an isolated service backed by a custom `Analyzer` (used in tests).
    pub fn with_analyzer(analyzer: Analyzer) -> Self {
        let rt = Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("create test Tokio runtime");
        Self {
            inner: Arc::new(ServiceInner {
                rt,
                analyzer,
                config_warning: None,
            }),
        }
    }

    /// Returns any non-fatal warning captured while loading `~/.diving/config.yml`.
    pub fn config_warning(&self) -> Option<&str> {
        self.inner.config_warning.as_deref()
    }

    /// Starts analyzing `image` (optionally scoped to `arch`) and streams progress + result.
    pub fn start_analyze(
        &self,
        image: String,
        arch: Option<String>,
        lang: Lang,
        verify_dup: bool,
    ) -> (AnalyzeTaskHandle, mpsc::UnboundedReceiver<AnalyzeMessage>) {
        self.start_analyze_with_options(image, arch, lang, verify_dup, false, true)
    }

    /// Starts analyzing `image` with explicit memory-cache bypass and offline fallback controls.
    pub fn start_analyze_with_options(
        &self,
        image: String,
        arch: Option<String>,
        lang: Lang,
        verify_dup: bool,
        skip_memory_cache: bool,
        fallback_to_last_known: bool,
    ) -> (AnalyzeTaskHandle, mpsc::UnboundedReceiver<AnalyzeMessage>) {
        let target = format_image_target(&image, arch.as_deref());
        let (tx, rx) = mpsc::unbounded_channel();
        let analyzer = self.inner.analyzer.clone();
        let done = Arc::new(AtomicBool::new(false));
        let is_registry = image_source_kind(&target) == ImageSourceKind::Registry;

        let mut handles = Vec::with_capacity(2);

        let main_tx = tx.clone();
        let main_done = Arc::clone(&done);
        let main_target = target.clone();
        let main_analyzer = analyzer.clone();
        let main_handle = self.inner.rt.spawn(async move {
            let progress_tx = main_tx.clone();
            let opts = AnalyzeOptions::default()
                .lang(lang)
                .verify_dup(verify_dup)
                .skip_memory_cache(skip_memory_cache)
                .fallback_to_last_known(fallback_to_last_known)
                .on_progress(Arc::new(move |ev| {
                    let _ = progress_tx.send(AnalyzeMessage::Progress(ev));
                }));

            let outcome = main_analyzer.analyze(&main_target, opts).await;
            main_done.store(true, Ordering::SeqCst);
            match outcome {
                Ok(result) => {
                    let _ = main_tx.send(AnalyzeMessage::Finished(result));
                }
                Err(err) => {
                    let _ = main_tx.send(AnalyzeMessage::Failed(err.to_string()));
                }
            }
        });
        handles.push(main_handle.abort_handle());

        if is_registry {
            let preview_handle = self.inner.rt.spawn(async move {
                tokio::time::sleep(Duration::from_millis(500)).await;
                if done.load(Ordering::SeqCst) {
                    return;
                }
                if let Ok(preview) = analyzer.preview(&target, PreviewOptions::default()).await
                    && !done.load(Ordering::SeqCst)
                {
                    let _ = tx.send(AnalyzeMessage::Preview(preview));
                }
            });
            handles.push(preview_handle.abort_handle());
        }

        (AnalyzeTaskHandle { tasks: handles }, rx)
    }

    /// Analyzes base and target images concurrently, streaming progress and returning `ImageDiff`.
    /// Analyzes `base` and `target` concurrently and builds an `ImageDiff`.
    #[allow(clippy::too_many_arguments)]
    pub fn start_diff(
        &self,
        base_image: String,
        base_arch: Option<String>,
        target_image: String,
        target_arch: Option<String>,
        diff_opts: DiffOptions,
        lang: Lang,
        skip_memory_cache: bool,
    ) -> (AnalyzeTaskHandle, mpsc::UnboundedReceiver<DiffMessage>) {
        let base_ref = format_image_target(&base_image, base_arch.as_deref());
        let target_ref = format_image_target(&target_image, target_arch.as_deref());
        let (tx, rx) = mpsc::unbounded_channel();
        let analyzer = self.inner.analyzer.clone();

        let handle = self.inner.rt.spawn(async move {
            let base_tx = tx.clone();
            let target_tx = tx.clone();
            let base_opts = AnalyzeOptions::default()
                .lang(lang)
                .verify_dup(true)
                .skip_memory_cache(skip_memory_cache)
                .fallback_to_last_known(true)
                .on_progress(Arc::new(move |ev| {
                    let _ = base_tx.send(DiffMessage::BaseProgress(ev));
                }));
            let target_opts = AnalyzeOptions::default()
                .lang(lang)
                .verify_dup(true)
                .skip_memory_cache(skip_memory_cache)
                .fallback_to_last_known(true)
                .on_progress(Arc::new(move |ev| {
                    let _ = target_tx.send(DiffMessage::TargetProgress(ev));
                }));

            let (base_res, target_res) = tokio::join!(
                analyzer.analyze(&base_ref, base_opts),
                analyzer.analyze(&target_ref, target_opts),
            );

            let outcome = match (base_res, target_res) {
                (Ok(base), Ok(target)) => {
                    let diff = analyzer
                        .compute_diff(
                            &base_ref,
                            &target_ref,
                            Arc::clone(&base),
                            Arc::clone(&target),
                            diff_opts,
                        )
                        .await
                        .unwrap_or_else(|_| diff_images_with_options(&base, &target, &diff_opts));
                    DiffRunResult::Ready {
                        diff: Arc::new(diff),
                        base,
                        target,
                    }
                }
                (Err(b_err), Ok(_)) => DiffRunResult::BaseFailed(b_err.to_string()),
                (Ok(_), Err(t_err)) => DiffRunResult::TargetFailed(t_err.to_string()),
                (Err(b_err), Err(t_err)) => DiffRunResult::BothFailed {
                    base: b_err.to_string(),
                    target: t_err.to_string(),
                },
            };
            let _ = tx.send(DiffMessage::Finished(outcome));
        });

        (
            AnalyzeTaskHandle {
                tasks: vec![handle.abort_handle()],
            },
            rx,
        )
    }

    /// Recomputes `ImageDiff` from cached analyses when toggling base-layer mode or swapping sides.
    #[allow(clippy::too_many_arguments)]
    pub fn recompute_diff(
        &self,
        base_image: String,
        base_arch: Option<String>,
        target_image: String,
        target_arch: Option<String>,
        base: Arc<DockerAnalyzeResult>,
        target: Arc<DockerAnalyzeResult>,
        diff_opts: DiffOptions,
    ) -> (
        AnalyzeTaskHandle,
        oneshot::Receiver<Result<Arc<ImageDiff>, String>>,
    ) {
        let base_ref = format_image_target(&base_image, base_arch.as_deref());
        let target_ref = format_image_target(&target_image, target_arch.as_deref());
        let (tx, rx) = oneshot::channel();
        let analyzer = self.inner.analyzer.clone();

        let handle = self.inner.rt.spawn(async move {
            let outcome = analyzer
                .compute_diff(
                    &base_ref,
                    &target_ref,
                    Arc::clone(&base),
                    Arc::clone(&target),
                    diff_opts,
                )
                .await
                .map(Arc::new)
                .or_else(|_| {
                    Ok(Arc::new(diff_images_with_options(
                        &base, &target, &diff_opts,
                    )))
                });
            let _ = tx.send(outcome);
        });

        (
            AnalyzeTaskHandle {
                tasks: vec![handle.abort_handle()],
            },
            rx,
        )
    }

    /// Reads up to 128 KiB from a cached layer blob or local archive for File Preview Sheet.
    pub fn load_file_preview(
        &self,
        image_ref: String,
        layer_digest: String,
        media_type: String,
        file_path: String,
    ) -> oneshot::Receiver<Result<FilePreviewContent, String>> {
        let (tx, rx) = oneshot::channel();
        let analyzer = self.inner.analyzer.clone();
        self.inner.rt.spawn_blocking(move || {
            let info = parse_image_info(&image_ref);
            let outcome = if info.registry == REGISTRY_LOCAL_FILE {
                analyzer.read_local_file_head(
                    &info.name,
                    &layer_digest,
                    &media_type,
                    &file_path,
                    FILE_PREVIEW_LIMIT,
                )
            } else {
                analyzer.read_file_head(&layer_digest, &media_type, &file_path, FILE_PREVIEW_LIMIT)
            };
            let result = outcome
                .map(|(bytes, total_size)| classify_file_preview(&file_path, &bytes, total_size))
                .map_err(|err| err.to_string());
            let _ = tx.send(result);
        });
        rx
    }

    /// Exports an entire file (up to 100 MiB) from a cached layer blob or local archive to `dest`.
    /// Streams registry blobs in 64 KiB chunks to avoid buffering large files in memory.
    pub fn export_layer_file(
        &self,
        image_ref: String,
        layer_digest: String,
        media_type: String,
        file_path: String,
        dest: PathBuf,
    ) -> oneshot::Receiver<Result<u64, String>> {
        let (tx, rx) = oneshot::channel();
        let analyzer = self.inner.analyzer.clone();
        self.inner.rt.spawn_blocking(move || {
            let info = parse_image_info(&image_ref);
            let result = if info.registry == REGISTRY_LOCAL_FILE {
                analyzer
                    .read_local_file(
                        &info.name,
                        &layer_digest,
                        &media_type,
                        &file_path,
                        FILE_EXPORT_LIMIT,
                    )
                    .map_err(|err| err.to_string())
                    .and_then(|bytes| {
                        let len = bytes.len() as u64;
                        fs::write(&dest, &bytes)
                            .map(|()| len)
                            .map_err(|err| err.to_string())
                    })
            } else {
                let mut writer: Option<BufWriter<File>> = None;
                let mut written: u64 = 0;
                let stream_res = analyzer.stream_file(
                    &layer_digest,
                    &media_type,
                    &file_path,
                    FILE_EXPORT_LIMIT,
                    |chunk| {
                        let w = match writer.as_mut() {
                            Some(w) => w,
                            None => {
                                let file = File::create(&dest).map_err(|e| CoreError::Other {
                                    message: e.to_string(),
                                })?;
                                writer.insert(BufWriter::new(file))
                            }
                        };
                        w.write_all(chunk).map_err(|e| CoreError::Other {
                            message: e.to_string(),
                        })?;
                        written = written.saturating_add(chunk.len() as u64);
                        Ok(())
                    },
                );
                match stream_res {
                    Ok(()) => {
                        if let Some(mut w) = writer {
                            w.flush().map(|()| written).map_err(|e| e.to_string())
                        } else {
                            // Empty file
                            fs::write(&dest, []).map(|()| 0).map_err(|e| e.to_string())
                        }
                    }
                    Err(err) => {
                        drop(writer);
                        let _ = fs::remove_file(&dest);
                        Err(err.to_string())
                    }
                }
            };
            let _ = tx.send(result);
        });
        rx
    }

    /// Computes the final merged filesystem tree on the Tokio blocking pool for `Treemap`.
    pub fn build_final_tree_async(
        &self,
        res: Arc<DockerAnalyzeResult>,
    ) -> oneshot::Receiver<Arc<FsNode>> {
        let (tx, rx) = oneshot::channel();
        self.inner.rt.spawn_blocking(move || {
            let tree = Arc::new(build_final_tree(&res.file_tree_list));
            let _ = tx.send(tree);
        });
        rx
    }

    /// Renders and writes a Markdown analysis report off the UI thread.
    pub fn export_markdown_report(
        &self,
        res: Arc<DockerAnalyzeResult>,
        lang: Lang,
        dest: PathBuf,
    ) -> oneshot::Receiver<Result<(), String>> {
        let (tx, rx) = oneshot::channel();
        self.inner.rt.spawn_blocking(move || {
            let md = to_markdown(&res, false, lang);
            let outcome = fs::write(&dest, md).map_err(|e| e.to_string());
            let _ = tx.send(outcome);
        });
        rx
    }

    /// Serializes and writes an `AnalyzeReport` JSON document off the UI thread.
    pub fn export_json_report(
        &self,
        res: Arc<DockerAnalyzeResult>,
        dest: PathBuf,
    ) -> oneshot::Receiver<Result<(), String>> {
        let (tx, rx) = oneshot::channel();
        self.inner.rt.spawn_blocking(move || {
            let summary = res.summary();
            let outcome = serde_json::to_string_pretty(&res.report(&summary))
                .map_err(|e| e.to_string())
                .and_then(|json| fs::write(&dest, json).map_err(|e| e.to_string()));
            let _ = tx.send(outcome);
        });
        rx
    }

    /// Renders and writes a Markdown diff report off the UI thread.
    pub fn export_diff_markdown(
        &self,
        diff: Arc<ImageDiff>,
        lang: Lang,
        dest: PathBuf,
    ) -> oneshot::Receiver<Result<(), String>> {
        let (tx, rx) = oneshot::channel();
        self.inner.rt.spawn_blocking(move || {
            let md = to_diff_markdown(&diff, lang, None);
            let outcome = fs::write(&dest, md).map_err(|e| e.to_string());
            let _ = tx.send(outcome);
        });
        rx
    }

    /// Writes a text document (such as reconstructed Dockerfile) off the UI thread.
    pub fn write_text_file(
        &self,
        dest: PathBuf,
        body: String,
    ) -> oneshot::Receiver<Result<(), String>> {
        let (tx, rx) = oneshot::channel();
        self.inner.rt.spawn_blocking(move || {
            let outcome = fs::write(&dest, body).map_err(|e| e.to_string());
            let _ = tx.send(outcome);
        });
        rx
    }

    /// Lists local images from Docker or Podman daemon CLI.
    pub fn list_local(
        &self,
        runtime: LocalRuntime,
    ) -> oneshot::Receiver<Result<Vec<LocalContainerImage>, String>> {
        let (tx, rx) = oneshot::channel();
        self.inner.rt.spawn(async move {
            let result = list_local_images(runtime)
                .await
                .map_err(|err| err.to_string());
            let _ = tx.send(result);
        });
        rx
    }

    /// Runs expired cache cleanup on layer blobs and saved analyses.
    pub fn cleanup_caches(&self) -> oneshot::Receiver<Result<CleanupStats, String>> {
        let (tx, rx) = oneshot::channel();
        let analyzer = self.inner.analyzer.clone();
        self.inner.rt.spawn(async move {
            let result = analyzer
                .cleanup_caches()
                .await
                .map_err(|err| err.to_string());
            let _ = tx.send(result);
        });
        rx
    }
}

fn format_image_target(image: &str, arch: Option<&str>) -> String {
    let trimmed = image.trim();
    let kind = image_source_kind(trimmed);
    if matches!(
        kind,
        ImageSourceKind::LocalDocker | ImageSourceKind::LocalPodman
    ) {
        return trimmed.to_owned();
    }
    let Some(arch) = arch.map(str::trim).filter(|a| !a.is_empty()) else {
        return trimmed.to_owned();
    };
    if kind == ImageSourceKind::LocalFile {
        let path = trimmed
            .rsplit_once("?arch=")
            .map_or(trimmed, |(path, _)| path);
        return format!("{path}?arch={arch}");
    }
    let Some((base, query)) = trimmed.split_once('?') else {
        return format!("{trimmed}?arch={arch}");
    };
    let mut replaced = false;
    let mut params: Vec<String> = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            if let Some((key, _)) = pair.split_once('=')
                && key.trim() == "arch"
            {
                if replaced {
                    None
                } else {
                    replaced = true;
                    Some(format!("arch={arch}"))
                }
            } else {
                Some(pair.to_owned())
            }
        })
        .collect();
    if !replaced {
        params.push(format!("arch={arch}"));
    }
    format!("{base}?{}", params.join("&"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_image_target_sets_replaces_and_preserves_query_params() {
        assert_eq!(format_image_target("redis:alpine", None), "redis:alpine");
        assert_eq!(
            format_image_target("redis:alpine", Some("   ")),
            "redis:alpine"
        );
        assert_eq!(
            format_image_target("redis:alpine", Some("arm64")),
            "redis:alpine?arch=arm64"
        );
        assert_eq!(
            format_image_target("redis:alpine?arch=amd64", Some("arm64")),
            "redis:alpine?arch=arm64"
        );
        assert_eq!(
            format_image_target("redis:alpine?arch=amd64", None),
            "redis:alpine?arch=amd64"
        );
        assert_eq!(
            format_image_target("redis:alpine?foo=1&arch=amd64&bar=2", Some("arm64")),
            "redis:alpine?foo=1&arch=arm64&bar=2"
        );
        assert_eq!(
            format_image_target("redis:alpine?foo=1", Some("arm64")),
            "redis:alpine?foo=1&arch=arm64"
        );
        assert_eq!(
            format_image_target("redis:alpine?arch=amd64&arch=386", Some("arm64")),
            "redis:alpine?arch=arm64"
        );
        assert_eq!(
            format_image_target("redis:alpine?", Some("arm64")),
            "redis:alpine?arch=arm64"
        );
        assert_eq!(
            format_image_target("file:///tmp/oci?arch=amd64", Some("arm64")),
            "file:///tmp/oci?arch=arm64"
        );
        assert_eq!(
            format_image_target("file:///tmp/dir?name/oci", Some("arm64")),
            "file:///tmp/dir?name/oci?arch=arm64"
        );
        assert_eq!(
            format_image_target("docker://redis:alpine", Some("arm64")),
            "docker://redis:alpine"
        );
        assert_eq!(
            format_image_target("podman://redis:alpine", Some("arm64")),
            "podman://redis:alpine"
        );
    }

    #[test]
    fn start_analyze_emits_raw_core_error_without_preformatting() {
        let analyzer = Analyzer::builder().build().expect("ephemeral analyzer");
        let service = DivingService::with_analyzer(analyzer);
        let (_handle, mut rx) = service.start_analyze_with_options(
            "file:///nonexistent/diving-service-missing.tar".to_owned(),
            None,
            Lang::En,
            false,
            true,
            false,
        );
        let err = service.inner.rt.block_on(async {
            while let Some(msg) = rx.recv().await {
                if let AnalyzeMessage::Failed(err) = msg {
                    return err;
                }
            }
            panic!("expected AnalyzeMessage::Failed");
        });
        assert!(
            !err.starts_with("Unable to read local archive:"),
            "analyze error must stay raw for live EN/ZH formatting, got: {err}"
        );
        assert!(
            !err.starts_with("无法读取本地镜像归档："),
            "analyze error must stay raw for live EN/ZH formatting, got: {err}"
        );
    }
}
