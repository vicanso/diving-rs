//! Root application state (`AppState`) with storage persistence and stale-result guards.

use std::cell::RefCell;
use std::collections::HashSet;
use std::future::Future;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use chrono::Local;
use diving_core::{
    DockerAnalyzeResult, DockerAnalyzeSummary, ImageDiff, ImagePreview, LocalContainerImage,
    LocalRuntime, ProgressEvent, diff_images,
};
use nsh_core::element::Owner;
use nsh_core::file_dialog::{OpenDialog, SaveDialog};
use nsh_core::prelude::*;
use nsh_core::storage::Storage;

use crate::domain::model::{
    AppRoute, DiffFileTab, FilePreviewContent, FileTreeFilter, FileTreeRow, InspectorTab,
    LayerFilterMode, RecentImageEntry, SharedRef, TreemapViewMode, format_bytes, remove_recent,
    upsert_recent,
};
use crate::domain::service::{
    AnalyzeMessage, AnalyzeTaskHandle, DiffMessage, DiffRunResult, DivingService,
};
use crate::domain::treemap::FsNode;
use crate::i18n::{UiLang, t, tf};

const KEY_LANG: &str = "diving.lang";
const KEY_APPEARANCE: &str = "diving.appearance";
const KEY_SIDEBAR_WIDTH: &str = "diving.layers_pane_width";
const KEY_APP_SIDEBAR_OPEN: &str = "diving.app_sidebar_open";
const KEY_RECENT_IMAGES: &str = "diving.recent_images";
const MAX_RECENT_IMAGES: usize = 12;

/// Metadata identifying the currently inspected file in `FilePreviewSheet`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewTarget {
    pub image_ref: String,
    pub layer_index: usize,
    pub layer_digest: String,
    pub media_type: String,
    pub path: String,
    pub size: u64,
    pub mode: String,
    pub uid: u64,
    pub gid: u64,
}

/// Fixed-footer hover details displayed below the `Treemap` visualization.
#[derive(Debug, Clone, PartialEq)]
pub struct TreemapHoverInfo {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub layer: Option<usize>,
    pub share: f32,
}

/// Side-specific error diagnostics reported when `Image Diff` fails to load one or both images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffErrorState {
    pub base: Option<String>,
    pub target: Option<String>,
}

/// Shared reactive state provided via `provide_context`.
#[derive(Clone)]
pub struct AppState {
    pub service: DivingService,
    pub route: Signal<AppRoute>,
    pub lang: Signal<UiLang>,
    pub appearance: Signal<Appearance>,
    pub app_sidebar_open: Signal<bool>,
    pub sidebar_width: Signal<Points>,
    pub recent_images: Signal<Vec<RecentImageEntry>>,
    pub image_input: Signal<String>,
    pub arch_input: Signal<Option<SharedString>>,
    pub header_search_open: Signal<bool>,
    pub current_image: Signal<String>,
    pub pending_image: Signal<Option<String>>,
    pub config_warning: Signal<Option<String>>,
    pub request_id: Signal<u64>,
    pub is_loading: Signal<bool>,
    pub show_preview_card: Signal<bool>,
    pub preview: Signal<Option<SharedRef<ImagePreview>>>,
    pub progress_text: Signal<Option<String>>,
    pub progress_ratio: Signal<f32>,
    pub progress_indeterminate: Signal<bool>,
    pub analyze_error: Signal<Option<String>>,
    pub result: Signal<Option<Arc<DockerAnalyzeResult>>>,
    pub summary: Signal<Option<SharedRef<DockerAnalyzeSummary>>>,
    pub final_tree: Signal<Option<SharedRef<FsNode>>>,
    pub active_tab: Signal<InspectorTab>,
    pub layer_filter: Signal<LayerFilterMode>,
    pub selected_layer: Signal<usize>,
    pub command_expanded: Signal<bool>,
    pub command_wrap: Signal<bool>,
    pub tree_keyword: Signal<String>,
    pub tree_modifications_only: Signal<bool>,
    pub tree_min_size: Signal<u64>,
    pub tree_compact_paths: Signal<bool>,
    pub tree_expand_all: Signal<bool>,
    pub tree_show_meta_cols: Signal<bool>,
    pub tree_expanded: Signal<HashSet<String>>,
    pub tree_collapsed: Signal<HashSet<String>>,
    pub selected_file_path: Signal<Option<String>>,
    pub preview_sheet_open: Signal<bool>,
    pub preview_request_id: Signal<u64>,
    pub preview_target: Signal<Option<PreviewTarget>>,
    pub preview_loading: Signal<bool>,
    pub preview_content: Signal<Option<FilePreviewContent>>,
    pub preview_error: Signal<Option<String>>,
    pub dockerfile_wrap: Signal<bool>,
    pub status_notice: Signal<Option<String>>,
    notice_seq: Signal<u64>,
    pub search_input_ref: ElementRef,
    pub tree_filter_ref: ElementRef,
    pub treemap_path: Signal<Vec<String>>,
    pub treemap_mode: Signal<TreemapViewMode>,
    pub treemap_hovered: Signal<Option<TreemapHoverInfo>>,
    pub compare_dialog_open: Signal<bool>,
    pub compare_base_input: Signal<String>,
    pub compare_base_arch: Signal<Option<SharedString>>,
    pub compare_target_input: Signal<String>,
    pub compare_target_arch: Signal<Option<SharedString>>,
    pub diff_loading: Signal<bool>,
    pub diff_base_progress: Signal<Option<String>>,
    pub diff_target_progress: Signal<Option<String>>,
    pub diff_error: Signal<Option<DiffErrorState>>,
    pub diff_result: Signal<Option<SharedRef<ImageDiff>>>,
    pub diff_base_ref: Signal<String>,
    pub diff_target_ref: Signal<String>,
    pub diff_base_result: Signal<Option<Arc<DockerAnalyzeResult>>>,
    pub diff_target_result: Signal<Option<Arc<DockerAnalyzeResult>>>,
    pub diff_file_tab: Signal<DiffFileTab>,
    pub diff_request_id: Signal<u64>,
    pub local_picker_open: Signal<bool>,
    pub local_runtime: Signal<LocalRuntime>,
    pub local_images: Signal<Vec<LocalContainerImage>>,
    pub local_loading: Signal<bool>,
    pub local_error: Signal<Option<String>>,
    pub local_filter: Signal<String>,
    pub local_selected_ref: Signal<Option<String>>,
    pub local_request_id: Signal<u64>,
    pub command_palette_open: Signal<bool>,
    pub provenance_open: Signal<bool>,
    pub provenance_query: Signal<String>,
    root_owner: Owner,
    active_task: Rc<RefCell<Option<AnalyzeTaskHandle>>>,
    active_diff_task: Rc<RefCell<Option<AnalyzeTaskHandle>>>,
}

impl AppState {
    /// Initializes state, restoring user preferences from `nsh_core::storage::Storage`.
    pub fn new(service: DivingService) -> Self {
        let saved_lang = Storage::get_item(KEY_LANG)
            .map(|raw| UiLang::from_storage_str(&raw))
            .unwrap_or_default();

        let saved_appearance = Storage::get_item(KEY_APPEARANCE)
            .map(|raw| match raw.as_str() {
                "light" => Appearance::Light,
                "dark" => Appearance::Dark,
                _ => Appearance::System,
            })
            .unwrap_or(Appearance::System);

        if matches!(saved_appearance, Appearance::Light | Appearance::Dark) {
            set_color_scheme(saved_appearance);
        }

        let saved_sidebar = Storage::get_item(KEY_SIDEBAR_WIDTH)
            .and_then(|raw| raw.parse::<f32>().ok())
            .map(|w| px(w.clamp(260.0, 480.0)))
            .unwrap_or_else(|| px(340.0));

        let saved_recent = Storage::get_item(KEY_RECENT_IMAGES)
            .and_then(|raw| serde_json::from_str::<Vec<RecentImageEntry>>(&raw).ok())
            .unwrap_or_default();

        let compact_viewport = window().width < px(1080.0);
        let saved_app_sidebar = if compact_viewport {
            false
        } else {
            Storage::get_item(KEY_APP_SIDEBAR_OPEN)
                .map(|raw| raw != "false")
                .unwrap_or(true)
        };

        let config_warning = service.config_warning().map(str::to_owned);

        Self {
            service,
            route: signal(AppRoute::Landing),
            lang: signal(saved_lang),
            appearance: signal(saved_appearance),
            app_sidebar_open: signal(saved_app_sidebar),
            sidebar_width: signal(saved_sidebar),
            recent_images: signal(saved_recent),
            image_input: signal(String::new()),
            arch_input: signal(None),
            header_search_open: signal(false),
            current_image: signal(String::new()),
            pending_image: signal(None),
            config_warning: signal(config_warning),
            request_id: signal(0),
            is_loading: signal(false),
            show_preview_card: signal(false),
            preview: signal(None),
            progress_text: signal(None),
            progress_ratio: signal(0.0),
            progress_indeterminate: signal(true),
            analyze_error: signal(None),
            result: signal(None),
            summary: signal(None),
            final_tree: signal(None),
            active_tab: signal(InspectorTab::FileTree),
            layer_filter: signal(LayerFilterMode::All),
            selected_layer: signal(0),
            command_expanded: signal(false),
            command_wrap: signal(false),
            tree_keyword: signal(String::new()),
            tree_modifications_only: signal(false),
            tree_min_size: signal(0),
            tree_compact_paths: signal(true),
            tree_expand_all: signal(false),
            tree_show_meta_cols: signal(!compact_viewport),
            tree_expanded: signal(HashSet::new()),
            tree_collapsed: signal(HashSet::new()),
            selected_file_path: signal(None),
            preview_sheet_open: signal(false),
            preview_request_id: signal(0),
            preview_target: signal(None),
            preview_loading: signal(false),
            preview_content: signal(None),
            preview_error: signal(None),
            dockerfile_wrap: signal(false),
            status_notice: signal(None),
            notice_seq: signal(0),
            search_input_ref: element_ref(),
            tree_filter_ref: element_ref(),
            treemap_path: signal(Vec::new()),
            treemap_mode: signal(TreemapViewMode::Map),
            treemap_hovered: signal(None),
            compare_dialog_open: signal(false),
            compare_base_input: signal(String::new()),
            compare_base_arch: signal(None),
            compare_target_input: signal(String::new()),
            compare_target_arch: signal(None),
            diff_loading: signal(false),
            diff_base_progress: signal(None),
            diff_target_progress: signal(None),
            diff_error: signal(None),
            diff_result: signal(None),
            diff_base_ref: signal(String::new()),
            diff_target_ref: signal(String::new()),
            diff_base_result: signal(None),
            diff_target_result: signal(None),
            diff_file_tab: signal(DiffFileTab::Added),
            diff_request_id: signal(0),
            local_picker_open: signal(false),
            local_runtime: signal(LocalRuntime::Docker),
            local_images: signal(Vec::new()),
            local_loading: signal(false),
            local_error: signal(None),
            local_filter: signal(String::new()),
            local_selected_ref: signal(None),
            local_request_id: signal(0),
            command_palette_open: signal(false),
            provenance_open: signal(false),
            provenance_query: signal(String::new()),
            root_owner: Owner::current(),
            active_task: Rc::new(RefCell::new(None)),
            active_diff_task: Rc::new(RefCell::new(None)),
        }
    }

    /// Spawns a UI async receiver on the root `App` owner so closing dialogs/menus never cancels it.
    fn spawn_app_task(&self, future: impl Future<Output = ()> + 'static) {
        self.root_owner.run(|| {
            drop(spawn(future));
        });
    }

    fn schedule_app_timeout(&self, duration: Duration, f: impl FnOnce() + 'static) {
        self.root_owner.run(|| {
            set_timeout(duration, f);
        });
    }

    pub fn set_lang(&self, next: UiLang) {
        self.lang.set(next);
        let _ = Storage::set_item(KEY_LANG, next.as_storage_str());
    }

    pub fn set_appearance(&self, next: Appearance) {
        self.appearance.set(next);
        set_color_scheme(next);
        let raw = match next {
            Appearance::System => "system",
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        };
        let _ = Storage::set_item(KEY_APPEARANCE, raw);
    }

    pub fn cycle_appearance(&self) {
        let next = match self.appearance.get_untracked() {
            Appearance::System => Appearance::Light,
            Appearance::Light => Appearance::Dark,
            Appearance::Dark => Appearance::System,
        };
        self.set_appearance(next);
    }

    pub fn set_sidebar_width(&self, width: Points) {
        let clamped = px(width.0.clamp(260.0, 480.0));
        self.sidebar_width.set(clamped);
        let _ = Storage::set_item(KEY_SIDEBAR_WIDTH, format!("{:.0}", clamped.0));
    }

    pub fn toggle_app_sidebar(&self) {
        let next = !self.app_sidebar_open.get_untracked();
        self.app_sidebar_open.set(next);
        let _ = Storage::set_item(KEY_APP_SIDEBAR_OPEN, if next { "true" } else { "false" });
    }

    pub fn show_status_notice(&self, msg: String) {
        let seq = self.notice_seq.get_untracked().wrapping_add(1);
        self.notice_seq.set(seq);
        self.status_notice.set(Some(msg));
        let timer_state = self.clone();
        self.schedule_app_timeout(Duration::from_secs(5), move || {
            if timer_state.notice_seq.get_untracked() == seq {
                timer_state.status_notice.set(None);
            }
        });
    }

    pub fn remove_recent_entry(&self, image: &str, arch: &str) {
        let mut updated = self.recent_images.get_untracked();
        remove_recent(&mut updated, image, arch);
        if let Ok(json) = serde_json::to_string(&updated) {
            let _ = Storage::set_item(KEY_RECENT_IMAGES, json);
        }
        self.recent_images.set(updated);
    }

    pub fn clear_recent_images(&self) {
        let _ = Storage::set_item(KEY_RECENT_IMAGES, "[]");
        self.recent_images.set(Vec::new());
    }

    pub fn dismiss_config_warning(&self) {
        self.config_warning.set(None);
    }

    pub fn dismiss_analyze_error(&self) {
        self.analyze_error.set(None);
    }

    pub fn navigate_home(&self) {
        if self.diff_loading.get_untracked() {
            self.cancel_diff();
        }
        self.preview_sheet_open.set(false);
        self.header_search_open.set(false);
        self.route.set(AppRoute::Landing);
    }

    pub fn open_gallery(&self) {
        self.preview_sheet_open.set(false);
        self.header_search_open.set(false);
        self.route.set(AppRoute::Gallery);
    }

    pub fn open_header_search(&self) {
        self.header_search_open.set(true);
        let input_ref = self.search_input_ref;
        self.schedule_app_timeout(Duration::from_millis(20), move || {
            input_ref.focus();
        });
    }

    pub fn close_header_search(&self) {
        self.header_search_open.set(false);
    }

    pub fn submit_current_input(&self) {
        let raw = self.image_input.get_untracked();
        let trimmed = raw.trim().to_owned();
        if trimmed.is_empty() {
            return;
        }
        self.header_search_open.set(false);
        let arch = self
            .arch_input
            .get_untracked()
            .map(|s| s.as_str().trim().to_owned())
            .filter(|s| !s.is_empty());
        self.analyze(trimmed, arch);
    }

    pub fn analyze(&self, image: String, arch: Option<String>) {
        self.analyze_with_options(image, arch, false);
    }

    pub fn analyze_with_options(
        &self,
        image: String,
        arch: Option<String>,
        skip_memory_cache: bool,
    ) {
        self.analyze_internal(image, arch, skip_memory_cache, true);
    }

    /// Retries the currently loaded stale image directly against the registry without offline fallback.
    pub fn retry_stale_image(&self) {
        let img = self.current_image.get_untracked();
        if img.is_empty() {
            return;
        }
        let arch = self
            .result
            .get_untracked()
            .map(|r| r.arch.clone())
            .filter(|a| !a.is_empty());
        self.analyze_internal(img, arch, true, false);
    }

    fn analyze_internal(
        &self,
        image: String,
        arch: Option<String>,
        skip_memory_cache: bool,
        fallback_to_last_known: bool,
    ) {
        let trimmed = image.trim().to_owned();
        if trimmed.is_empty() {
            return;
        }
        self.abort_active_task();

        let next_id = self.request_id.get_untracked().wrapping_add(1);
        self.request_id.set(next_id);
        self.image_input.set(trimmed.clone());
        self.pending_image.set(Some(trimmed.clone()));
        self.analyze_error.set(None);
        self.preview.set(None);
        self.show_preview_card.set(false);
        self.is_loading.set(true);
        self.progress_ratio.set(0.0);
        self.progress_indeterminate.set(true);
        self.progress_text.set(Some(
            t(self.lang.get_untracked(), "landing.analyzing").to_owned(),
        ));
        self.status_notice.set(None);
        self.header_search_open.set(false);

        let lang = self.lang.get_untracked();
        let (handle, mut rx) = self.service.start_analyze_with_options(
            trimmed.clone(),
            arch.clone(),
            lang.to_core_lang(),
            true,
            skip_memory_cache,
            fallback_to_last_known,
        );
        *self.active_task.borrow_mut() = Some(handle);

        let timer_state = self.clone();
        self.schedule_app_timeout(Duration::from_millis(500), move || {
            if timer_state.request_id.get_untracked() == next_id
                && timer_state.is_loading.get_untracked()
            {
                timer_state.show_preview_card.set(true);
            }
        });

        let state = self.clone();
        self.spawn_app_task(async move {
            let mut total_layers = 0usize;
            let mut finished_layers = 0usize;
            while let Some(msg) = rx.recv().await {
                if state.request_id.get_untracked() != next_id {
                    break;
                }
                match msg {
                    AnalyzeMessage::Progress(ev) => {
                        state.apply_progress_event(&ev, &mut total_layers, &mut finished_layers);
                    }
                    AnalyzeMessage::Preview(prev) => {
                        state.preview.set(Some(SharedRef::new(prev)));
                        state.show_preview_card.set(true);
                    }
                    AnalyzeMessage::Finished(res) => {
                        state.finish_analysis(next_id, &trimmed, arch.as_deref(), res);
                        break;
                    }
                    AnalyzeMessage::Failed(err) => {
                        state.is_loading.set(false);
                        state.pending_image.set(None);
                        state.show_preview_card.set(false);
                        state.progress_text.set(None);
                        state.analyze_error.set(Some(err));
                        break;
                    }
                }
            }
        });
    }

    fn apply_progress_event(
        &self,
        ev: &ProgressEvent,
        total_layers: &mut usize,
        finished_layers: &mut usize,
    ) {
        let lang = self.lang.get_untracked();
        match ev {
            ProgressEvent::Authenticating { .. } | ProgressEvent::FetchingManifest => {
                self.progress_indeterminate.set(true);
            }
            ProgressEvent::LayersDiscovered { count, .. } => {
                *total_layers = (*count).max(1);
                *finished_layers = 0;
                self.progress_indeterminate.set(false);
                self.progress_ratio.set(0.05);
            }
            ProgressEvent::DownloadingLayer { .. } | ProgressEvent::LayerCacheHit { .. } => {
                // Do not advance completion ratio on download start; wait for LayerCompleted.
            }
            ProgressEvent::LayerCompleted { .. } => {
                *finished_layers += 1;
                if *total_layers > 0 {
                    let ratio =
                        ((*finished_layers as f32) / (*total_layers as f32)).clamp(0.05, 0.95);
                    self.progress_indeterminate.set(false);
                    self.progress_ratio.set(ratio);
                }
            }
            ProgressEvent::AnalysisCacheHit { .. } => {
                self.progress_indeterminate.set(false);
                self.progress_ratio.set(1.0);
            }
            ProgressEvent::UsingStaleCache { .. } => {}
        }
        self.progress_text
            .set(Some(format_progress_event_text(lang, ev)));
    }

    fn finish_analysis(
        &self,
        req_id: u64,
        image: &str,
        explicit_arch: Option<&str>,
        res: Arc<DockerAnalyzeResult>,
    ) {
        if self.request_id.get_untracked() != req_id {
            return;
        }
        let summary = SharedRef::new(res.summary());
        let resolved_arch = explicit_arch
            .filter(|a| !a.is_empty())
            .unwrap_or(&res.arch)
            .to_owned();

        let score = summary.score;
        let total_size = res.total_size;
        let is_stale = res.stale_as_of.is_some();

        self.activate_result(image, summary, res);

        let entry = RecentImageEntry {
            image: image.to_owned(),
            arch: resolved_arch,
            score,
            total_size,
            analyzed_at: Local::now().format("%Y-%m-%d %H:%M").to_string(),
            stale: is_stale,
        };
        let mut updated = self.recent_images.get_untracked();
        upsert_recent(&mut updated, entry, MAX_RECENT_IMAGES);
        if let Ok(json) = serde_json::to_string(&updated) {
            let _ = Storage::set_item(KEY_RECENT_IMAGES, json);
        }
        self.recent_images.set(updated);
    }

    fn activate_result(
        &self,
        image: &str,
        summary: SharedRef<DockerAnalyzeSummary>,
        res: Arc<DockerAnalyzeResult>,
    ) {
        let first_non_empty = res.layers.iter().position(|l| !l.empty).unwrap_or(0);
        self.pending_image.set(None);
        self.current_image.set(image.to_owned());
        self.image_input.set(image.to_owned());
        self.result.set(Some(Arc::clone(&res)));
        self.summary.set(Some(summary));
        self.final_tree.set(None);
        self.is_loading.set(false);
        self.show_preview_card.set(false);
        self.preview.set(None);
        self.progress_text.set(None);
        self.progress_ratio.set(1.0);
        self.analyze_error.set(None);

        // Precompute the merged final filesystem tree off the UI thread for instant Treemap tab switches.
        let res_for_tree = Arc::clone(&res);
        let tree_rx = self.service.build_final_tree_async(res);
        let tree_state = self.clone();
        self.spawn_app_task(async move {
            if let Ok(tree) = tree_rx.await {
                let still_current = tree_state
                    .result
                    .get_untracked()
                    .is_some_and(|cur| Arc::ptr_eq(&cur, &res_for_tree));
                if still_current {
                    tree_state.final_tree.set(Some(SharedRef(tree)));
                }
            }
        });

        // Reset per-image inspector state
        self.active_tab.set(InspectorTab::FileTree);
        self.layer_filter.set(LayerFilterMode::All);
        self.selected_layer.set(first_non_empty);
        self.command_expanded.set(false);
        self.tree_keyword.set(String::new());
        self.tree_modifications_only.set(false);
        self.tree_min_size.set(0);
        self.tree_expand_all.set(false);
        self.tree_expanded.set(HashSet::new());
        self.tree_collapsed.set(HashSet::new());
        self.treemap_path.set(Vec::new());
        self.treemap_hovered.set(None);
        self.selected_file_path.set(None);
        self.preview_sheet_open.set(false);
        self.route.set(AppRoute::Inspector);
    }

    pub fn cancel_analysis(&self) {
        self.abort_active_task();
        self.request_id
            .set(self.request_id.get_untracked().wrapping_add(1));
        self.is_loading.set(false);
        self.pending_image.set(None);
        self.show_preview_card.set(false);
        self.progress_text.set(None);
    }

    pub fn cancel_diff(&self) {
        if let Some(handle) = self.active_diff_task.borrow_mut().take() {
            handle.abort();
        }
        self.diff_request_id
            .set(self.diff_request_id.get_untracked().wrapping_add(1));
        self.diff_loading.set(false);
        self.diff_base_progress.set(None);
        self.diff_target_progress.set(None);
    }

    fn abort_active_task(&self) {
        if let Some(handle) = self.active_task.borrow_mut().take() {
            handle.abort();
        }
    }

    pub fn select_layer(&self, layer_index: usize) {
        if self.selected_layer.get_untracked() != layer_index {
            self.selected_layer.set(layer_index);
            self.command_expanded.set(false);
            self.tree_expanded.set(HashSet::new());
            self.tree_collapsed.set(HashSet::new());
            self.selected_file_path.set(None);
        }
    }

    pub fn current_tree_filter(&self) -> FileTreeFilter {
        FileTreeFilter {
            keyword: self.tree_keyword.get(),
            only_modified_removed: self.tree_modifications_only.get(),
            min_size_bytes: self.tree_min_size.get(),
            compact_paths: self.tree_compact_paths.get(),
            expand_all: self.tree_expand_all.get(),
        }
    }

    pub fn toggle_tree_dir(&self, key: &str, currently_expanded: bool) {
        let mut expanded = self.tree_expanded.get_untracked();
        let mut collapsed = self.tree_collapsed.get_untracked();
        if currently_expanded {
            expanded.remove(key);
            collapsed.insert(key.to_owned());
        } else {
            collapsed.remove(key);
            expanded.insert(key.to_owned());
        }
        self.tree_expanded.set(expanded);
        self.tree_collapsed.set(collapsed);
    }

    pub fn expand_all_dirs(&self) {
        self.tree_expand_all.set(true);
        self.tree_collapsed.set(HashSet::new());
    }

    pub fn collapse_all_dirs(&self) {
        self.tree_expand_all.set(false);
        self.tree_expanded.set(HashSet::new());
        self.tree_collapsed.set(HashSet::new());
    }

    pub fn open_file_preview(&self, row: &FileTreeRow) {
        if !row.is_previewable() {
            return;
        }
        let Some(res) = self.result.get_untracked() else {
            return;
        };
        let layer_idx = self.selected_layer.get_untracked();
        let Some(layer) = res.layers.get(layer_idx) else {
            return;
        };

        let image_ref = self.current_image.get_untracked();
        self.selected_file_path.set(Some(row.key.clone()));
        let req_id = self.preview_request_id.get_untracked().wrapping_add(1);
        self.preview_request_id.set(req_id);
        self.preview_target.set(Some(PreviewTarget {
            image_ref: image_ref.clone(),
            layer_index: layer_idx,
            layer_digest: layer.digest.clone(),
            media_type: layer.media_type.clone(),
            path: row.key.clone(),
            size: row.size,
            mode: row.mode.clone(),
            uid: row.uid,
            gid: row.gid,
        }));
        self.preview_loading.set(true);
        self.preview_content.set(None);
        self.preview_error.set(None);
        self.preview_sheet_open.set(true);

        let rx = self.service.load_file_preview(
            image_ref,
            layer.digest.clone(),
            layer.media_type.clone(),
            row.key.clone(),
        );
        let state = self.clone();
        self.spawn_app_task(async move {
            let outcome = match rx.await {
                Ok(res) => res,
                Err(_) => Err(t(state.lang.get_untracked(), "error.preview_cancelled").to_owned()),
            };
            if state.preview_request_id.get_untracked() != req_id {
                return;
            }
            state.preview_loading.set(false);
            match outcome {
                Ok(content) => state.preview_content.set(Some(content)),
                Err(err) => state.preview_error.set(Some(err)),
            }
        });
    }

    pub fn save_preview_file(&self) {
        let Some(target) = self.preview_target.get_untracked() else {
            return;
        };
        self.save_layer_file_for_image(
            target.image_ref,
            target.layer_digest,
            target.media_type,
            target.path,
        );
    }

    pub fn save_layer_file_at(&self, layer_digest: String, media_type: String, file_path: String) {
        let image_ref = self.current_image.get_untracked();
        self.save_layer_file_for_image(image_ref, layer_digest, media_type, file_path);
    }

    fn save_layer_file_for_image(
        &self,
        image_ref: String,
        layer_digest: String,
        media_type: String,
        file_path: String,
    ) {
        let default_name = Path::new(&file_path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("file.bin")
            .to_owned();
        let lang = self.lang.get_untracked();
        let title = t(lang, "common.save_as").to_owned();
        let state = self.clone();
        self.spawn_app_task(async move {
            let picked =
                pick_save_path(SaveDialog::new().file_name(default_name).title(title)).await;
            let Ok(Some(dest)) = picked else {
                return;
            };
            let rx = state.service.export_layer_file(
                image_ref,
                layer_digest,
                media_type,
                file_path,
                dest.clone(),
            );
            match rx.await {
                Ok(Ok(_bytes)) => {
                    let msg = tf(lang, "notice.exported_to", &[&dest.display().to_string()]);
                    state.show_status_notice(msg);
                }
                Ok(Err(err)) => state.show_status_notice(err),
                Err(_) => {}
            }
        });
    }

    pub fn open_local_file_dialog(&self) {
        let lang = self.lang.get_untracked();
        let title = t(lang, "menu.open_local_file").to_owned();
        let state = self.clone();
        self.spawn_app_task(async move {
            let picked = pick_paths(OpenDialog::files_and_folders().title(title)).await;
            if let Ok(Some(paths)) = picked
                && let Some(first) = paths.into_iter().next()
            {
                let ref_str = format!("file://{}", first.display());
                state.analyze(ref_str, None);
            }
        });
    }

    pub fn export_markdown_report(&self) {
        let Some(res) = self.result.get_untracked() else {
            return;
        };
        let lang = self.lang.get_untracked();
        let title = t(lang, "menu.export_markdown").to_owned();
        let state = self.clone();
        self.spawn_app_task(async move {
            let picked =
                pick_save_path(SaveDialog::new().file_name("diving-report.md").title(title)).await;
            if let Ok(Some(dest)) = picked {
                let rx =
                    state
                        .service
                        .export_markdown_report(res, lang.to_core_lang(), dest.clone());
                match rx.await {
                    Ok(Ok(())) => {
                        let msg = tf(lang, "notice.exported_to", &[&dest.display().to_string()]);
                        state.show_status_notice(msg);
                    }
                    Ok(Err(err)) => state.show_status_notice(err),
                    Err(_) => {}
                }
            }
        });
    }

    pub fn export_json_report(&self) {
        let Some(res) = self.result.get_untracked() else {
            return;
        };
        let lang = self.lang.get_untracked();
        let title = t(lang, "menu.export_json").to_owned();
        let state = self.clone();
        self.spawn_app_task(async move {
            let picked = pick_save_path(
                SaveDialog::new()
                    .file_name("diving-report.json")
                    .title(title),
            )
            .await;
            if let Ok(Some(dest)) = picked {
                let rx = state.service.export_json_report(res, dest.clone());
                match rx.await {
                    Ok(Ok(())) => {
                        let msg = tf(lang, "notice.exported_to", &[&dest.display().to_string()]);
                        state.show_status_notice(msg);
                    }
                    Ok(Err(err)) => state.show_status_notice(err),
                    Err(_) => {}
                }
            }
        });
    }

    pub fn export_dockerfile(&self, body: String) {
        let lang = self.lang.get_untracked();
        let title = t(lang, "dockerfile.export").to_owned();
        let state = self.clone();
        self.spawn_app_task(async move {
            let picked =
                pick_save_path(SaveDialog::new().file_name("Dockerfile").title(title)).await;
            if let Ok(Some(dest)) = picked {
                let rx = state.service.write_text_file(dest.clone(), body);
                match rx.await {
                    Ok(Ok(())) => {
                        let msg = tf(lang, "notice.exported_to", &[&dest.display().to_string()]);
                        state.show_status_notice(msg);
                    }
                    Ok(Err(err)) => state.show_status_notice(err),
                    Err(_) => {}
                }
            }
        });
    }

    pub fn clear_cache(&self) {
        let lang = self.lang.get_untracked();
        let rx = self.service.cleanup_caches();
        let state = self.clone();
        self.spawn_app_task(async move {
            match rx.await {
                Ok(Ok(stats)) => {
                    let removed = (stats.removed_blobs + stats.removed_analyses).to_string();
                    let freed = format_bytes(stats.freed_bytes);
                    let msg = tf(lang, "notice.cache_cleaned", &[&removed, &freed]);
                    state.show_status_notice(msg);
                }
                Ok(Err(err)) => state.show_status_notice(err),
                Err(_) => {}
            }
        });
    }

    pub fn focus_search_input(&self) {
        if matches!(
            self.route.get_untracked(),
            AppRoute::Inspector | AppRoute::Diff
        ) {
            self.open_header_search();
        } else {
            self.search_input_ref.focus();
        }
    }

    pub fn focus_tree_filter(&self) {
        if self.route.get_untracked() == AppRoute::Inspector {
            self.active_tab.set(InspectorTab::FileTree);
            self.tree_filter_ref.focus();
        }
    }

    pub fn toggle_command_palette(&self) {
        let next = !self.command_palette_open.get_untracked();
        self.command_palette_open.set(next);
    }

    pub fn open_provenance_search(&self) {
        if self.result.get_untracked().is_some() {
            self.provenance_open.set(true);
        }
    }

    pub fn jump_to_layer_file(&self, layer_index: usize, path: String) {
        self.provenance_open.set(false);
        self.route.set(AppRoute::Inspector);
        self.active_tab.set(InspectorTab::FileTree);
        self.select_layer(layer_index);
        self.tree_keyword.set(path.clone());
        self.selected_file_path.set(Some(path));
    }

    /// Handles a local `.tar` archive or OCI layout directory dropped onto the window.
    pub fn handle_dropped_uri(&self, uri: &str) {
        let trimmed = uri.trim();
        if trimmed.is_empty() {
            return;
        }
        let image_ref = if trimmed.starts_with("file://") {
            percent_decode_file_uri(trimmed)
        } else {
            format!("file://{trimmed}")
        };
        self.analyze(image_ref, None);
    }

    pub fn reanalyze_current_image(&self) {
        match self.route.get_untracked() {
            AppRoute::Diff => {
                let base = self.diff_base_ref.get_untracked();
                let target = self.diff_target_ref.get_untracked();
                if !base.is_empty() && !target.is_empty() {
                    self.start_compare_pair_with_options(
                        base,
                        self.compare_base_arch
                            .get_untracked()
                            .map(|s| s.as_str().to_owned()),
                        target,
                        self.compare_target_arch
                            .get_untracked()
                            .map(|s| s.as_str().to_owned()),
                        true,
                    );
                }
            }
            AppRoute::Inspector | AppRoute::Landing => {
                let img = {
                    let cur = self.current_image.get_untracked();
                    if !cur.is_empty() {
                        cur
                    } else {
                        self.image_input.get_untracked().trim().to_owned()
                    }
                };
                if !img.is_empty() {
                    let arch = self
                        .arch_input
                        .get_untracked()
                        .map(|s| s.as_str().trim().to_owned())
                        .filter(|s| !s.is_empty());
                    self.analyze_with_options(img, arch, true);
                }
            }
            AppRoute::Gallery => {}
        }
    }

    pub fn open_compare_dialog(&self) {
        let current = self.current_image.get_untracked();
        if !current.is_empty() && self.compare_target_input.get_untracked().trim().is_empty() {
            self.compare_target_input.set(current);
            self.compare_target_arch
                .set(self.arch_input.get_untracked());
        }
        self.compare_dialog_open.set(true);
    }

    pub fn submit_compare_dialog(&self) {
        let base = self.compare_base_input.get_untracked().trim().to_owned();
        let target = self.compare_target_input.get_untracked().trim().to_owned();
        if base.is_empty() || target.is_empty() {
            return;
        }
        let base_arch = self
            .compare_base_arch
            .get_untracked()
            .map(|s| s.as_str().trim().to_owned())
            .filter(|s| !s.is_empty());
        let target_arch = self
            .compare_target_arch
            .get_untracked()
            .map(|s| s.as_str().trim().to_owned())
            .filter(|s| !s.is_empty());
        self.start_compare_pair(base, base_arch, target, target_arch);
    }

    pub fn start_compare_pair(
        &self,
        base: String,
        base_arch: Option<String>,
        target: String,
        target_arch: Option<String>,
    ) {
        self.start_compare_pair_with_options(base, base_arch, target, target_arch, false);
    }

    pub fn start_compare_pair_with_options(
        &self,
        base: String,
        base_arch: Option<String>,
        target: String,
        target_arch: Option<String>,
        skip_memory_cache: bool,
    ) {
        let base = base.trim().to_owned();
        let target = target.trim().to_owned();
        if base.is_empty() || target.is_empty() {
            return;
        }
        if let Some(handle) = self.active_diff_task.borrow_mut().take() {
            handle.abort();
        }
        let req_id = self.diff_request_id.get_untracked().wrapping_add(1);
        self.diff_request_id.set(req_id);
        self.compare_base_input.set(base.clone());
        self.compare_target_input.set(target.clone());
        self.compare_base_arch.set(
            base_arch
                .as_deref()
                .map(|a| SharedString::from(a.to_owned())),
        );
        self.compare_target_arch.set(
            target_arch
                .as_deref()
                .map(|a| SharedString::from(a.to_owned())),
        );
        self.diff_base_ref.set(base.clone());
        self.diff_target_ref.set(target.clone());
        self.diff_error.set(None);
        self.diff_base_progress.set(None);
        self.diff_target_progress.set(None);
        self.diff_loading.set(true);
        self.diff_file_tab.set(DiffFileTab::Added);
        self.compare_dialog_open.set(false);
        self.preview_sheet_open.set(false);
        self.route.set(AppRoute::Diff);

        let lang = self.lang.get_untracked().to_core_lang();
        let (handle, mut rx) = self.service.start_diff(
            base,
            base_arch,
            target,
            target_arch,
            lang,
            skip_memory_cache,
        );
        *self.active_diff_task.borrow_mut() = Some(handle);

        let state = self.clone();
        self.spawn_app_task(async move {
            while let Some(msg) = rx.recv().await {
                if state.diff_request_id.get_untracked() != req_id {
                    break;
                }
                match msg {
                    DiffMessage::BaseProgress(ev) => {
                        let ui_lang = state.lang.get_untracked();
                        state
                            .diff_base_progress
                            .set(Some(format_progress_event_text(ui_lang, &ev)));
                    }
                    DiffMessage::TargetProgress(ev) => {
                        let ui_lang = state.lang.get_untracked();
                        state
                            .diff_target_progress
                            .set(Some(format_progress_event_text(ui_lang, &ev)));
                    }
                    DiffMessage::Finished(outcome) => {
                        state.diff_loading.set(false);
                        state.diff_base_progress.set(None);
                        state.diff_target_progress.set(None);
                        match outcome {
                            DiffRunResult::Ready { diff, base, target } => {
                                state.diff_base_result.set(Some(base));
                                state.diff_target_result.set(Some(target));
                                state.diff_result.set(Some(SharedRef(diff)));
                                state.diff_error.set(None);
                            }
                            DiffRunResult::BaseFailed(err) => {
                                state.diff_error.set(Some(DiffErrorState {
                                    base: Some(err),
                                    target: None,
                                }));
                            }
                            DiffRunResult::TargetFailed(err) => {
                                state.diff_error.set(Some(DiffErrorState {
                                    base: None,
                                    target: Some(err),
                                }));
                            }
                            DiffRunResult::BothFailed { base, target } => {
                                state.diff_error.set(Some(DiffErrorState {
                                    base: Some(base),
                                    target: Some(target),
                                }));
                            }
                        }
                        break;
                    }
                }
            }
        });
    }

    pub fn swap_diff(&self) {
        let base_ref = self.diff_base_ref.get_untracked();
        let target_ref = self.diff_target_ref.get_untracked();
        let base_arch = self.compare_base_arch.get_untracked();
        let target_arch = self.compare_target_arch.get_untracked();
        self.diff_base_ref.set(target_ref.clone());
        self.diff_target_ref.set(base_ref.clone());
        self.compare_base_input.set(target_ref.clone());
        self.compare_target_input.set(base_ref.clone());
        self.compare_base_arch.set(target_arch);
        self.compare_target_arch.set(base_arch);

        let base_res = self.diff_base_result.get_untracked();
        let target_res = self.diff_target_result.get_untracked();
        if let (Some(prev_base), Some(prev_target)) = (base_res, target_res) {
            let swapped = Arc::new(diff_images(&prev_target, &prev_base));
            self.diff_base_result.set(Some(prev_target));
            self.diff_target_result.set(Some(prev_base));
            self.diff_result.set(Some(SharedRef(swapped)));
            self.diff_error.set(None);
        } else if !base_ref.is_empty() && !target_ref.is_empty() {
            self.submit_compare_dialog();
        }
    }

    pub fn open_diff_side(&self, is_target: bool) {
        let (image_ref, res) = if is_target {
            (
                self.diff_target_ref.get_untracked(),
                self.diff_target_result.get_untracked(),
            )
        } else {
            (
                self.diff_base_ref.get_untracked(),
                self.diff_base_result.get_untracked(),
            )
        };
        if let Some(res) = res {
            let summary = SharedRef::new(res.summary());
            self.activate_result(&image_ref, summary, res);
        }
    }

    pub fn export_diff_markdown(&self) {
        let Some(diff) = self.diff_result.get_untracked() else {
            return;
        };
        let lang = self.lang.get_untracked();
        let title = t(lang, "diff.export_md").to_owned();
        let state = self.clone();
        self.spawn_app_task(async move {
            let picked =
                pick_save_path(SaveDialog::new().file_name("diving-diff.md").title(title)).await;
            if let Ok(Some(dest)) = picked {
                let rx =
                    state
                        .service
                        .export_diff_markdown(diff.0, lang.to_core_lang(), dest.clone());
                match rx.await {
                    Ok(Ok(())) => {
                        let msg = tf(lang, "notice.exported_to", &[&dest.display().to_string()]);
                        state.show_status_notice(msg);
                    }
                    Ok(Err(err)) => state.show_status_notice(err),
                    Err(_) => {}
                }
            }
        });
    }

    pub fn open_local_picker(&self) {
        self.local_picker_open.set(true);
        self.local_filter.set(String::new());
        self.local_selected_ref.set(None);
        self.refresh_local_images();
    }

    pub fn set_local_runtime(&self, runtime: LocalRuntime) {
        if self.local_runtime.get_untracked() != runtime {
            self.local_runtime.set(runtime);
            self.local_selected_ref.set(None);
            self.refresh_local_images();
        }
    }

    pub fn refresh_local_images(&self) {
        let req_id = self.local_request_id.get_untracked().wrapping_add(1);
        self.local_request_id.set(req_id);
        self.local_loading.set(true);
        self.local_error.set(None);
        let runtime = self.local_runtime.get_untracked();
        let rx = self.service.list_local(runtime);
        let state = self.clone();
        self.spawn_app_task(async move {
            let outcome = match rx.await {
                Ok(res) => res,
                Err(_) => {
                    Err(t(state.lang.get_untracked(), "error.local_list_cancelled").to_owned())
                }
            };
            if state.local_request_id.get_untracked() != req_id {
                return;
            }
            state.local_loading.set(false);
            match outcome {
                Ok(list) => {
                    let first_ref = list.first().map(LocalContainerImage::reference);
                    state.local_images.set(list);
                    state.local_selected_ref.set(first_ref);
                    state.local_error.set(None);
                }
                Err(err) => {
                    state.local_images.set(Vec::new());
                    state.local_selected_ref.set(None);
                    state.local_error.set(Some(err));
                }
            }
        });
    }

    pub fn analyze_selected_local_image(&self) {
        let Some(selected) = self.local_selected_ref.get_untracked() else {
            return;
        };
        self.local_picker_open.set(false);
        self.analyze(selected, None);
    }
}

fn format_progress_event_text(lang: UiLang, ev: &ProgressEvent) -> String {
    match ev {
        ProgressEvent::Authenticating { registry } => tf(lang, "progress.auth", &[registry]),
        ProgressEvent::FetchingManifest => t(lang, "progress.manifest").to_owned(),
        ProgressEvent::LayersDiscovered { count, total_bytes } => tf(
            lang,
            "progress.layers_discovered",
            &[&count.to_string(), &format_bytes(*total_bytes)],
        ),
        ProgressEvent::DownloadingLayer { digest, size, .. } => {
            let short = short_digest(digest);
            tf(
                lang,
                "progress.downloading",
                &[&format!("{short} · {}", format_bytes(*size))],
            )
        }
        ProgressEvent::LayerCacheHit { digest, size, .. } => {
            let short = short_digest(digest);
            tf(
                lang,
                "progress.layer_cached",
                &[&format!("{short} · {}", format_bytes(*size))],
            )
        }
        ProgressEvent::LayerCompleted { digest, size } => {
            let short = short_digest(digest);
            tf(
                lang,
                "progress.layer_done",
                &[&format!("{short} · {}", format_bytes(*size))],
            )
        }
        ProgressEvent::AnalysisCacheHit { .. } => t(lang, "progress.cache_hit").to_owned(),
        ProgressEvent::UsingStaleCache { as_of } => tf(lang, "stale.banner", &[as_of]),
    }
}

fn percent_decode_file_uri(uri: &str) -> String {
    let Some(rest) = uri.strip_prefix("file://") else {
        return uri.to_owned();
    };
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3])
            && let Ok(byte) = u8::from_str_radix(hex, 16)
        {
            out.push(byte);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    let decoded = String::from_utf8(out).unwrap_or_else(|_| rest.to_owned());
    format!("file://{decoded}")
}

fn short_digest(digest: &str) -> &str {
    let trimmed = digest.strip_prefix("sha256:").unwrap_or(digest);
    trimmed.get(..12).unwrap_or(trimmed)
}
