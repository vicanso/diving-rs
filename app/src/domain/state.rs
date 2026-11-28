//! Root application state (`AppState`) with storage persistence and stale-result guards.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashSet};
use std::env::consts;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime};

use chrono::Local;
use diving_app::config::merge_registry_rules;
use diving_core::{
    BaseLayerMode, DiffOptions, DockerAnalyzeResult, DockerAnalyzeSummary, FileTreeItem, ImageDiff,
    ImagePreview, LocalContainerImage, LocalRuntime, ProgressEvent, RegistryRule,
    build_recommendations, normalize_registry_host_pattern, registry_host_matches,
};
use nsh_core::element::Owner;
use nsh_core::element::desktop::quit;
use nsh_core::file_dialog::{OpenDialog, SaveDialog};
use nsh_core::prelude::*;
use nsh_core::storage::Storage;
use tokio::sync::{mpsc::unbounded_channel, oneshot};

use crate::domain::model::{
    AppRoute, DiffFileTab, FilePreviewContent, FileTreeFilter, FileTreeRow, ImageSourceKind,
    InspectorTab, LayerFilterMode, LayerSortOrder, RecentImageEntry, SharedRef, TreemapViewMode,
    default_selected_layer, format_bytes, image_source_kind, relocalize_image_diff, remove_recent,
    symlink_preview_content, upsert_recent,
};
use crate::domain::service::{
    AnalyzeMessage, AnalyzeTaskHandle, DiffMessage, DiffRunResult, DivingService,
};
use crate::domain::treemap::FsNode;
use crate::domain::updater::{
    CURRENT_VERSION, InstallOutcome, UpdateCheck, asset_name, auto_check_disabled_by_env,
    auto_check_due, check_latest_blocking, clear_skipped_version, download_and_verify_blocking,
    install as install_update_package, nudge_notes, nudge_version, record_outcome, relaunch,
    skip_version, skipped_version, sweep_installer_mounts,
};
use crate::i18n::{UiLang, format_user_error, t, tf};

const KEY_LANG: &str = "diving.lang";
const KEY_APPEARANCE: &str = "diving.appearance";
const KEY_SIDEBAR_WIDTH: &str = "diving.layers_pane_width";
const KEY_APP_SIDEBAR_OPEN: &str = "diving.app_sidebar_open";
const KEY_RECENT_IMAGES: &str = "diving.recent_images";
const KEY_LOCAL_RUNTIME: &str = "diving.local_runtime";
const KEY_LAYER_SORT_ORDER: &str = "diving.layer_sort_order";
const KEY_COMMAND_WRAP: &str = "diving.command_wrap";
const KEY_COMMAND_RAW: &str = "diving.command_raw";
const KEY_DOCKERFILE_WRAP: &str = "diving.dockerfile_wrap";
const KEY_DOCKERFILE_RAW: &str = "diving.dockerfile_raw";
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

/// Action to automatically trigger once private registry settings are saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RegistryRetryTarget {
    #[default]
    None,
    Analyze,
    Diff,
}

/// One editable HTTP header row inside the Private Registries dialog.
#[derive(Clone, PartialEq)]
pub struct RegistryHeaderDraft {
    pub id: u64,
    pub key: Signal<String>,
    pub value: Signal<String>,
}

/// Editable draft entry backing one domain rule in the Private Registries dialog.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegistryDraftRule {
    pub host: String,
    pub username: String,
    pub password: String,
    pub token: String,
    pub headers: Vec<(String, String)>,
    pub insecure_http: bool,
    pub insecure_skip_tls_verify: bool,
    pub ca_cert: String,
    pub from_config_yml: bool,
}

impl RegistryDraftRule {
    fn from_rule(rule: &RegistryRule, from_config_yml: bool) -> Self {
        Self {
            host: normalize_registry_host_pattern(&rule.host),
            username: rule.username.clone().unwrap_or_default(),
            password: rule.password.clone().unwrap_or_default(),
            token: rule.token.clone().unwrap_or_default(),
            headers: rule
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            insecure_http: rule.insecure_http,
            insecure_skip_tls_verify: rule.insecure_skip_tls_verify,
            ca_cert: rule.ca_cert.clone().unwrap_or_default(),
            from_config_yml,
        }
    }

    fn to_rule(&self) -> RegistryRule {
        let mut headers = BTreeMap::new();
        for (k, v) in &self.headers {
            let clean_key = k.trim();
            if !clean_key.is_empty() {
                headers.insert(clean_key.to_owned(), v.clone());
            }
        }
        RegistryRule {
            host: normalize_registry_host_pattern(&self.host),
            username: (!self.username.is_empty()).then(|| self.username.clone()),
            password: (!self.password.is_empty()).then(|| self.password.clone()),
            token: (!self.token.is_empty()).then(|| self.token.clone()),
            headers,
            insecure_http: self.insecure_http,
            insecure_skip_tls_verify: self.insecure_skip_tls_verify,
            ca_cert: (!self.ca_cert.trim().is_empty()).then(|| self.ca_cert.trim().to_owned()),
        }
    }
}

/// Lifecycle state for the GitHub Releases self-updater UI.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdatePhase {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available {
        version: String,
        notes: String,
    },
    Downloading {
        version: String,
        received: u64,
        total: u64,
    },
    Installing {
        version: String,
    },
    ReadyToRestart {
        version: String,
        target: PathBuf,
    },
    OpenedInstaller {
        version: String,
        path: PathBuf,
    },
    Failed {
        version: Option<String>,
        message: String,
    },
}

/// Returns `true` when the updater has an actionable update or restart waiting for the user.
pub fn has_pending_update_indicator(phase: &UpdatePhase) -> bool {
    matches!(
        phase,
        UpdatePhase::Available { .. } | UpdatePhase::ReadyToRestart { .. }
    )
}

enum UpdateTaskEvent {
    Progress { received: u64, total: u64 },
    Installing,
    Finished(Result<InstallOutcome, String>),
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
    pub layer_order: Signal<LayerSortOrder>,
    pub selected_layer: Signal<usize>,
    pub command_expanded: Signal<bool>,
    pub command_wrap: Signal<bool>,
    pub command_raw: Signal<bool>,
    pub tree_keyword: Signal<String>,
    pub tree_modifications_only: Signal<bool>,
    pub tree_min_size: Signal<u64>,
    pub tree_compact_paths: Signal<bool>,
    pub tree_expand_all: Signal<bool>,
    pub tree_show_meta_cols: Signal<bool>,
    pub tree_expanded: Signal<HashSet<String>>,
    pub tree_collapsed: Signal<HashSet<String>>,
    pub selected_file_path: Signal<Option<String>>,
    pub file_tree_focused: Signal<bool>,
    pub preview_sheet_open: Signal<bool>,
    pub preview_request_id: Signal<u64>,
    pub preview_target: Signal<Option<PreviewTarget>>,
    pub preview_loading: Signal<bool>,
    pub preview_content: Signal<Option<FilePreviewContent>>,
    pub preview_error: Signal<Option<String>>,
    pub dockerfile_wrap: Signal<bool>,
    pub dockerfile_raw: Signal<bool>,
    pub status_notice: Signal<Option<String>>,
    pub status_notice_is_update: Signal<bool>,
    notice_seq: Signal<u64>,
    pub update_phase: Signal<UpdatePhase>,
    pub update_skipped: Signal<Option<String>>,
    last_compact: Signal<bool>,
    wide_sidebar_pref: Signal<bool>,
    wide_meta_pref: Signal<bool>,
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
    pub diff_base_arch: Signal<Option<SharedString>>,
    pub diff_target_ref: Signal<String>,
    pub diff_target_arch: Signal<Option<SharedString>>,
    pub diff_base_result: Signal<Option<Arc<DockerAnalyzeResult>>>,
    pub diff_target_result: Signal<Option<Arc<DockerAnalyzeResult>>>,
    pub diff_base_layer_mode: Signal<BaseLayerMode>,
    pub diff_recomputing: Signal<bool>,
    pub diff_unexpected_only: Signal<bool>,
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
    pub failed_analyze_image: Signal<Option<String>>,
    pub registry_dialog_open: Signal<bool>,
    pub registry_drafts: Signal<Vec<RegistryDraftRule>>,
    pub registry_config_base: Signal<Vec<RegistryRule>>,
    pub registry_selected_index: Signal<Option<usize>>,
    pub registry_dialog_error: Signal<Option<String>>,
    pub registry_retry_target: Signal<RegistryRetryTarget>,
    pub reg_host_input: Signal<String>,
    pub reg_username_input: Signal<String>,
    pub reg_password_input: Signal<String>,
    pub reg_token_input: Signal<String>,
    pub reg_headers: Signal<Vec<RegistryHeaderDraft>>,
    pub reg_insecure_http: Signal<bool>,
    pub reg_skip_tls: Signal<bool>,
    pub reg_ca_cert_input: Signal<String>,
    reg_next_header_id: Signal<u64>,
    result_seq: Signal<u64>,
    compare_synced_source: Signal<(String, Option<SharedString>)>,
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

        let saved_local_runtime = Storage::get_item(KEY_LOCAL_RUNTIME)
            .map(|raw| match raw.as_str() {
                "podman" => LocalRuntime::Podman,
                "container" => LocalRuntime::Container,
                _ => LocalRuntime::Docker,
            })
            .unwrap_or(LocalRuntime::Docker);

        let compact_viewport = untracked(|| window().width) < px(1080.0);
        let wide_sidebar_pref = Storage::get_item(KEY_APP_SIDEBAR_OPEN)
            .map(|raw| raw != "false")
            .unwrap_or(true);
        let saved_app_sidebar = if compact_viewport {
            false
        } else {
            wide_sidebar_pref
        };

        let config_warning = service.config_warning();
        let initial_skipped = if auto_check_disabled_by_env() {
            None
        } else {
            skipped_version()
        };
        let saved_command_wrap = Storage::get_item(KEY_COMMAND_WRAP)
            .map(|raw| raw != "false")
            .unwrap_or(true);
        let saved_command_raw = Storage::get_item(KEY_COMMAND_RAW)
            .map(|raw| raw == "true")
            .unwrap_or(false);
        let saved_dockerfile_wrap = Storage::get_item(KEY_DOCKERFILE_WRAP)
            .map(|raw| raw != "false")
            .unwrap_or(true);
        let saved_dockerfile_raw = Storage::get_item(KEY_DOCKERFILE_RAW)
            .map(|raw| raw == "true")
            .unwrap_or(false);
        let saved_layer_order = Storage::get_item(KEY_LAYER_SORT_ORDER)
            .map(|raw| LayerSortOrder::from_storage_str(&raw))
            .unwrap_or_default();

        let state = Self {
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
            layer_order: signal(saved_layer_order),
            selected_layer: signal(0),
            command_expanded: signal(false),
            command_wrap: signal(saved_command_wrap),
            command_raw: signal(saved_command_raw),
            tree_keyword: signal(String::new()),
            tree_modifications_only: signal(false),
            tree_min_size: signal(0),
            tree_compact_paths: signal(true),
            tree_expand_all: signal(false),
            tree_show_meta_cols: signal(!compact_viewport),
            tree_expanded: signal(HashSet::new()),
            tree_collapsed: signal(HashSet::new()),
            selected_file_path: signal(None),
            file_tree_focused: signal(false),
            preview_sheet_open: signal(false),
            preview_request_id: signal(0),
            preview_target: signal(None),
            preview_loading: signal(false),
            preview_content: signal(None),
            preview_error: signal(None),
            dockerfile_wrap: signal(saved_dockerfile_wrap),
            dockerfile_raw: signal(saved_dockerfile_raw),
            status_notice: signal(None),
            status_notice_is_update: signal(false),
            notice_seq: signal(0),
            update_phase: signal(UpdatePhase::Idle),
            update_skipped: signal(initial_skipped),
            last_compact: signal(compact_viewport),
            wide_sidebar_pref: signal(wide_sidebar_pref),
            wide_meta_pref: signal(true),
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
            diff_base_arch: signal(None),
            diff_target_ref: signal(String::new()),
            diff_target_arch: signal(None),
            diff_base_result: signal(None),
            diff_target_result: signal(None),
            diff_base_layer_mode: signal(BaseLayerMode::Auto),
            diff_recomputing: signal(false),
            diff_unexpected_only: signal(false),
            diff_file_tab: signal(DiffFileTab::Added),
            diff_request_id: signal(0),
            local_picker_open: signal(false),
            local_runtime: signal(saved_local_runtime),
            local_images: signal(Vec::new()),
            local_loading: signal(false),
            local_error: signal(None),
            local_filter: signal(String::new()),
            local_selected_ref: signal(None),
            local_request_id: signal(0),
            command_palette_open: signal(false),
            provenance_open: signal(false),
            provenance_query: signal(String::new()),
            failed_analyze_image: signal(None),
            registry_dialog_open: signal(false),
            registry_drafts: signal(Vec::new()),
            registry_config_base: signal(Vec::new()),
            registry_selected_index: signal(None),
            registry_dialog_error: signal(None),
            registry_retry_target: signal(RegistryRetryTarget::None),
            reg_host_input: signal(String::new()),
            reg_username_input: signal(String::new()),
            reg_password_input: signal(String::new()),
            reg_token_input: signal(String::new()),
            reg_headers: signal(Vec::new()),
            reg_insecure_http: signal(false),
            reg_skip_tls: signal(false),
            reg_ca_cert_input: signal(String::new()),
            reg_next_header_id: signal(1),
            result_seq: signal(0),
            compare_synced_source: signal((String::new(), None)),
            root_owner: Owner::current(),
            active_task: Rc::new(RefCell::new(None)),
            active_diff_task: Rc::new(RefCell::new(None)),
        };
        state.install_responsive_sync();
        state
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
        let core_lang = next.to_core_lang();

        if let Some(cur) = self.result.get_untracked() {
            let mut updated = (*cur).clone();
            updated.recommendations = build_recommendations(&updated, core_lang);
            let updated = Arc::new(updated);
            let summary = SharedRef::new(updated.summary());
            self.result.set(Some(updated));
            self.summary.set(Some(summary));
        }

        let mut base_recs = None;
        if let Some(base) = self.diff_base_result.get_untracked() {
            let mut updated = (*base).clone();
            updated.recommendations = build_recommendations(&updated, core_lang);
            base_recs = Some(updated.recommendations.clone());
            self.diff_base_result.set(Some(Arc::new(updated)));
        }

        let mut target_recs = None;
        if let Some(target) = self.diff_target_result.get_untracked() {
            let mut updated = (*target).clone();
            updated.recommendations = build_recommendations(&updated, core_lang);
            target_recs = Some(updated.recommendations.clone());
            self.diff_target_result.set(Some(Arc::new(updated)));
        }

        if let (Some(diff), Some(b), Some(t)) = (
            self.diff_result.get_untracked(),
            base_recs.as_deref(),
            target_recs.as_deref(),
        ) {
            self.diff_result
                .set(Some(SharedRef::new(relocalize_image_diff(&diff.0, b, t))));
        }

        if self.preview_sheet_open.get_untracked()
            && let Some(target) = self.preview_target.get_untracked()
            && target.mode.starts_with('l')
            && let Some(res) = self.result.get_untracked()
            && let Some(tree) = res.file_tree_list.get(target.layer_index)
            && let Some(link) = find_tree_item_link(tree, &target.path)
        {
            self.preview_content
                .set(Some(symlink_preview_content(&link, next)));
        }
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
        let wide = untracked(|| window().width) >= px(1080.0);
        if wide {
            self.wide_sidebar_pref.set(next);
            let _ = Storage::set_item(KEY_APP_SIDEBAR_OPEN, if next { "true" } else { "false" });
        }
    }

    /// Updates `tree_show_meta_cols`, persisting wide-mode preference when viewport `>= 1080pt`.
    pub fn set_tree_show_meta_cols(&self, next: bool) {
        self.tree_show_meta_cols.set(next);
        if untracked(|| window().width) >= px(1080.0) {
            self.wide_meta_pref.set(next);
        }
    }

    /// Toggles line wrapping inside `CommandInspector` and persists the preference.
    pub fn toggle_command_wrap(&self) {
        let next = !self.command_wrap.get_untracked();
        self.command_wrap.set(next);
        let _ = Storage::set_item(KEY_COMMAND_WRAP, if next { "true" } else { "false" });
    }

    /// Toggles between auto-formatted and raw `created_by` output in `CommandInspector`.
    pub fn toggle_command_raw(&self) {
        let next = !self.command_raw.get_untracked();
        self.command_raw.set(next);
        let _ = Storage::set_item(KEY_COMMAND_RAW, if next { "true" } else { "false" });
    }

    /// Updates `dockerfile_wrap` and persists the preference.
    pub fn set_dockerfile_wrap(&self, next: bool) {
        self.dockerfile_wrap.set(next);
        let _ = Storage::set_item(KEY_DOCKERFILE_WRAP, if next { "true" } else { "false" });
    }

    /// Updates `dockerfile_raw` and persists the preference.
    pub fn set_dockerfile_raw(&self, next: bool) {
        self.dockerfile_raw.set(next);
        let _ = Storage::set_item(KEY_DOCKERFILE_RAW, if next { "true" } else { "false" });
    }

    /// Toggles layer list display order (`#N→1` vs `#1→N`) and persists the preference.
    pub fn toggle_layer_order(&self) {
        let next = self.layer_order.get_untracked().toggle();
        self.layer_order.set(next);
        let _ = Storage::set_item(KEY_LAYER_SORT_ORDER, next.as_storage_str());
    }

    fn install_responsive_sync(&self) {
        let sync_state = self.clone();
        effect(move || {
            let compact = window().width < px(1080.0);
            let prev = sync_state.last_compact.get_untracked();
            if compact != prev {
                sync_state.last_compact.set(compact);
                if compact {
                    sync_state.app_sidebar_open.set(false);
                    sync_state.tree_show_meta_cols.set(false);
                } else {
                    sync_state
                        .app_sidebar_open
                        .set(sync_state.wide_sidebar_pref.get_untracked());
                    sync_state
                        .tree_show_meta_cols
                        .set(sync_state.wide_meta_pref.get_untracked());
                }
            }
        });
    }

    pub fn show_status_notice(&self, msg: String) {
        self.show_status_notice_with_kind(msg, false);
    }

    fn show_status_notice_with_kind(&self, msg: String, is_update: bool) {
        let seq = self.notice_seq.get_untracked().wrapping_add(1);
        self.notice_seq.set(seq);
        self.status_notice_is_update.set(is_update);
        self.status_notice.set(Some(msg));
        let timer_state = self.clone();
        self.schedule_app_timeout(
            Duration::from_secs(if is_update { 10 } else { 5 }),
            move || {
                if timer_state.notice_seq.get_untracked() == seq {
                    timer_state.status_notice.set(None);
                    timer_state.status_notice_is_update.set(false);
                }
            },
        );
    }

    /// Dismisses the active floating status notice immediately.
    pub fn dismiss_status_notice(&self) {
        self.notice_seq
            .set(self.notice_seq.get_untracked().wrapping_add(1));
        self.status_notice.set(None);
        self.status_notice_is_update.set(false);
    }

    /// Whether any modal dialog, sheet, or command palette is currently open.
    pub fn any_overlay_open(&self) -> bool {
        self.command_palette_open.get_untracked()
            || self.registry_dialog_open.get_untracked()
            || self.provenance_open.get_untracked()
            || self.compare_dialog_open.get_untracked()
            || self.local_picker_open.get_untracked()
            || self.preview_sheet_open.get_untracked()
    }

    /// Closes all modal dialogs, sheets, and header search overlays.
    pub fn close_all_modals(&self) {
        self.command_palette_open.set(false);
        self.registry_dialog_open.set(false);
        self.provenance_open.set(false);
        self.compare_dialog_open.set(false);
        self.local_picker_open.set(false);
        self.preview_sheet_open.set(false);
        self.header_search_open.set(false);
    }

    /// Dismisses only the single top-most open modal/overlay or active search/filter input.
    pub fn dismiss_top_overlay_or_search(&self) -> bool {
        if self.command_palette_open.get_untracked() {
            self.command_palette_open.set(false);
            return true;
        }
        if self.registry_dialog_open.get_untracked() {
            self.registry_dialog_open.set(false);
            return true;
        }
        if self.provenance_open.get_untracked() {
            self.provenance_open.set(false);
            return true;
        }
        if self.compare_dialog_open.get_untracked() {
            self.compare_dialog_open.set(false);
            return true;
        }
        if self.local_picker_open.get_untracked() {
            self.local_picker_open.set(false);
            return true;
        }
        if self.preview_sheet_open.get_untracked() {
            self.preview_sheet_open.set(false);
            return true;
        }
        if self.header_search_open.get_untracked() {
            self.close_header_search();
            return true;
        }
        if self.route.get_untracked() == AppRoute::Inspector
            && self.active_tab.get_untracked() == InspectorTab::FileTree
            && !self.tree_keyword.get_untracked().is_empty()
        {
            self.tree_keyword.set(String::new());
            return true;
        }
        false
    }

    /// Whether `Mod+R` / menu "Re-analyze" can run in the current route state.
    pub fn can_reanalyze(&self) -> bool {
        match self.route.get() {
            AppRoute::Diff => {
                !self.diff_base_ref.get().is_empty() && !self.diff_target_ref.get().is_empty()
            }
            AppRoute::Inspector | AppRoute::Landing => {
                !self.current_image.get().is_empty() || !self.image_input.get().trim().is_empty()
            }
            #[cfg(debug_assertions)]
            AppRoute::Gallery => false,
        }
    }

    /// Whether `Mod+E` / menu "Export Markdown report…" can run in the current route state.
    pub fn can_export_markdown(&self) -> bool {
        match self.route.get() {
            AppRoute::Diff => self.diff_result.get().is_some(),
            _ => self.result.get().is_some(),
        }
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
        self.close_all_modals();
        self.route.set(AppRoute::Landing);
    }

    #[cfg(debug_assertions)]
    pub fn open_gallery(&self) {
        self.close_all_modals();
        self.route.set(AppRoute::Gallery);
    }

    pub fn open_header_search(&self) {
        match self.route.get_untracked() {
            AppRoute::Inspector => {
                self.header_search_open.set(true);
                let input_ref = self.search_input_ref;
                self.schedule_app_timeout(Duration::from_millis(20), move || {
                    input_ref.focus();
                });
            }
            AppRoute::Diff => self.open_compare_dialog(),
            AppRoute::Landing => {
                self.search_input_ref.focus();
            }
            #[cfg(debug_assertions)]
            AppRoute::Gallery => {}
        }
    }

    pub fn close_header_search(&self) {
        self.header_search_open.set(false);
        self.search_input_ref.blur();
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
        let synced_arch = arch
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_owned)
            .or_else(|| parse_explicit_arch_query(&trimmed))
            .map(SharedString::from);
        self.arch_input.set(synced_arch);
        self.image_input.set(trimmed.clone());
        self.pending_image.set(Some(trimmed.clone()));
        self.failed_analyze_image.set(Some(trimmed.clone()));
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
        let initial_layer = default_selected_layer(&res.layers, self.layer_order.get_untracked());
        let next_result_seq = self.result_seq.get_untracked().wrapping_add(1);
        self.result_seq.set(next_result_seq);
        self.close_all_modals();
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
        self.failed_analyze_image.set(None);

        // Precompute the merged final filesystem tree off the UI thread for instant Treemap tab switches.
        let tree_rx = self.service.build_final_tree_async(res);
        let tree_state = self.clone();
        self.spawn_app_task(async move {
            if let Ok(tree) = tree_rx.await
                && tree_state.result_seq.get_untracked() == next_result_seq
            {
                tree_state.final_tree.set(Some(SharedRef(tree)));
            }
        });

        // Reset per-image inspector state
        self.active_tab.set(InspectorTab::FileTree);
        self.layer_filter.set(LayerFilterMode::All);
        self.selected_layer.set(initial_layer);
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
        self.diff_recomputing.set(false);
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
        if row.is_symlink() {
            self.preview_loading.set(false);
            self.preview_content.set(Some(symlink_preview_content(
                &row.link,
                self.lang.get_untracked(),
            )));
            self.preview_error.set(None);
            self.preview_sheet_open.set(true);
            return;
        }
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
                Err(_) => Err("error.preview_cancelled".to_owned()),
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
                Ok(Err(err)) => state.show_status_notice(format_user_error(lang, &err)),
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
                let path_str = first.display().to_string();
                let ref_str = format!("file://{}", strip_windows_drive_leading_slash(&path_str));
                state.analyze(ref_str, None);
            }
        });
    }

    /// Opens a folder picker dialog so Windows and Linux users can select OCI layout directories.
    pub fn open_local_folder(&self) {
        let lang = self.lang.get_untracked();
        let title = t(lang, "action.open_folder").to_owned();
        let state = self.clone();
        self.spawn_app_task(async move {
            let picked = pick_paths(OpenDialog::folders().title(title)).await;
            if let Ok(Some(paths)) = picked
                && let Some(first) = paths.into_iter().next()
            {
                let path_str = first.display().to_string();
                let ref_str = format!("file://{}", strip_windows_drive_leading_slash(&path_str));
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
                    Ok(Err(err)) => state.show_status_notice(format_user_error(lang, &err)),
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
                    Ok(Err(err)) => state.show_status_notice(format_user_error(lang, &err)),
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
                    Ok(Err(err)) => state.show_status_notice(format_user_error(lang, &err)),
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
                Ok(Err(err)) => state.show_status_notice(format_user_error(lang, &err)),
                Err(_) => {}
            }
        });
    }

    pub fn focus_search_input(&self) {
        if self.any_overlay_open() {
            return;
        }
        self.open_header_search();
    }

    pub fn focus_tree_filter(&self) {
        if self.any_overlay_open() || self.route.get_untracked() != AppRoute::Inspector {
            return;
        }
        self.active_tab.set(InspectorTab::FileTree);
        let filter_ref = self.tree_filter_ref;
        self.schedule_app_timeout(Duration::from_millis(20), move || {
            filter_ref.focus();
        });
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
        self.tree_modifications_only.set(false);
        self.tree_min_size.set(0);
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
            format!("file://{}", strip_windows_drive_leading_slash(trimmed))
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
                        self.diff_base_arch
                            .get_untracked()
                            .map(|s| s.as_str().to_owned()),
                        target,
                        self.diff_target_arch
                            .get_untracked()
                            .map(|s| s.as_str().to_owned()),
                        true,
                    );
                }
            }
            AppRoute::Inspector | AppRoute::Landing => {
                let img = if self.analyze_error.get_untracked().is_some() {
                    self.failed_analyze_image
                        .get_untracked()
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| self.image_input.get_untracked().trim().to_owned())
                } else {
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
            #[cfg(debug_assertions)]
            AppRoute::Gallery => {}
        }
    }

    pub fn open_compare_dialog(&self) {
        if self.route.get_untracked() == AppRoute::Diff {
            let active_base = self.diff_base_ref.get_untracked();
            let active_target = self.diff_target_ref.get_untracked();
            if !active_base.is_empty() && !active_target.is_empty() {
                self.compare_base_input.set(active_base);
                self.compare_target_input.set(active_target);
                self.compare_base_arch
                    .set(self.diff_base_arch.get_untracked());
                self.compare_target_arch
                    .set(self.diff_target_arch.get_untracked());
            }
            self.compare_dialog_open.set(true);
            return;
        }
        let current = self.current_image.get_untracked();
        let current_arch = self.arch_input.get_untracked();
        let source = (current.clone(), current_arch.clone());
        if !current.is_empty()
            && (self.compare_target_input.get_untracked().trim().is_empty()
                || self.compare_synced_source.get_untracked() != source)
        {
            self.compare_target_input.set(current.clone());
            self.compare_target_arch.set(current_arch);
            if self.compare_base_input.get_untracked().trim() == current {
                self.compare_base_input.set(String::new());
                self.compare_base_arch.set(None);
            }
            self.compare_synced_source.set(source);
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
        let base_arch_sig = base_arch
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(|a| SharedString::from(a.to_owned()));
        let target_arch_sig = target_arch
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(|a| SharedString::from(a.to_owned()));
        self.compare_base_input.set(base.clone());
        self.compare_target_input.set(target.clone());
        self.compare_base_arch.set(base_arch_sig.clone());
        self.compare_target_arch.set(target_arch_sig.clone());
        self.diff_base_ref.set(base.clone());
        self.diff_base_arch.set(base_arch_sig);
        self.diff_target_ref.set(target.clone());
        self.diff_target_arch.set(target_arch_sig);
        self.diff_result.set(None);
        self.diff_base_result.set(None);
        self.diff_target_result.set(None);
        self.diff_error.set(None);
        self.diff_base_progress.set(None);
        self.diff_target_progress.set(None);
        self.diff_recomputing.set(false);
        self.diff_unexpected_only.set(false);
        self.diff_loading.set(true);
        self.diff_file_tab.set(DiffFileTab::Added);
        self.compare_dialog_open.set(false);
        self.preview_sheet_open.set(false);
        self.route.set(AppRoute::Diff);

        let lang = self.lang.get_untracked().to_core_lang();
        let diff_opts =
            DiffOptions::default().base_layer_mode(self.diff_base_layer_mode.get_untracked());
        let (handle, mut rx) = self.service.start_diff(
            base,
            base_arch,
            target,
            target_arch,
            diff_opts,
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

    pub fn set_diff_base_layer_mode(&self, next: BaseLayerMode) {
        if self.diff_base_layer_mode.get_untracked() == next {
            return;
        }
        self.diff_base_layer_mode.set(next);
        self.diff_unexpected_only.set(false);
        self.recompute_active_diff();
    }

    fn recompute_active_diff(&self) {
        let Some(base_res) = self.diff_base_result.get_untracked() else {
            return;
        };
        let Some(target_res) = self.diff_target_result.get_untracked() else {
            return;
        };
        if let Some(handle) = self.active_diff_task.borrow_mut().take() {
            handle.abort();
        }
        let req_id = self.diff_request_id.get_untracked().wrapping_add(1);
        self.diff_request_id.set(req_id);
        self.diff_recomputing.set(true);

        let base_ref = self.diff_base_ref.get_untracked();
        let target_ref = self.diff_target_ref.get_untracked();
        let base_arch = self
            .diff_base_arch
            .get_untracked()
            .map(|s| s.as_str().to_owned());
        let target_arch = self
            .diff_target_arch
            .get_untracked()
            .map(|s| s.as_str().to_owned());
        let diff_opts =
            DiffOptions::default().base_layer_mode(self.diff_base_layer_mode.get_untracked());

        let (handle, rx) = self.service.recompute_diff(
            base_ref,
            base_arch,
            target_ref,
            target_arch,
            base_res,
            target_res,
            diff_opts,
        );
        *self.active_diff_task.borrow_mut() = Some(handle);

        let state = self.clone();
        self.spawn_app_task(async move {
            let outcome = rx.await;
            if state.diff_request_id.get_untracked() != req_id {
                return;
            }
            state.diff_recomputing.set(false);
            if let Ok(Ok(diff)) = outcome {
                state.diff_result.set(Some(SharedRef(diff)));
                state.diff_error.set(None);
            }
        });
    }

    pub fn swap_diff(&self) {
        let base_ref = self.diff_base_ref.get_untracked();
        let target_ref = self.diff_target_ref.get_untracked();
        let base_arch = self.diff_base_arch.get_untracked();
        let target_arch = self.diff_target_arch.get_untracked();
        self.diff_base_ref.set(target_ref.clone());
        self.diff_target_ref.set(base_ref.clone());
        self.diff_base_arch.set(target_arch.clone());
        self.diff_target_arch.set(base_arch.clone());
        self.compare_base_input.set(target_ref.clone());
        self.compare_target_input.set(base_ref.clone());
        self.compare_base_arch.set(target_arch.clone());
        self.compare_target_arch.set(base_arch.clone());

        let base_res = self.diff_base_result.get_untracked();
        let target_res = self.diff_target_result.get_untracked();
        if let (Some(prev_base), Some(prev_target)) = (base_res, target_res) {
            self.diff_base_result.set(Some(prev_target));
            self.diff_target_result.set(Some(prev_base));
            self.diff_error.set(None);
            self.recompute_active_diff();
        } else if !base_ref.is_empty() && !target_ref.is_empty() {
            self.start_compare_pair(
                target_ref,
                target_arch.map(|s| s.as_str().to_owned()),
                base_ref,
                base_arch.map(|s| s.as_str().to_owned()),
            );
        }
    }

    pub fn open_diff_side(&self, is_target: bool) {
        let (image_ref, explicit_arch, res) = if is_target {
            (
                self.diff_target_ref.get_untracked(),
                self.diff_target_arch.get_untracked(),
                self.diff_target_result.get_untracked(),
            )
        } else {
            (
                self.diff_base_ref.get_untracked(),
                self.diff_base_arch.get_untracked(),
                self.diff_base_result.get_untracked(),
            )
        };
        if let Some(res) = res {
            self.abort_active_task();
            self.request_id
                .set(self.request_id.get_untracked().wrapping_add(1));
            let synced_arch = explicit_arch
                .filter(|a| !a.as_str().trim().is_empty())
                .or_else(|| parse_explicit_arch_query(&image_ref).map(SharedString::from))
                .or_else(|| {
                    (image_source_kind(&image_ref) == ImageSourceKind::Registry
                        && !res.arch.is_empty())
                    .then(|| SharedString::from(res.arch.clone()))
                });
            self.arch_input.set(synced_arch);
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
                    Ok(Err(err)) => state.show_status_notice(format_user_error(lang, &err)),
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
            let _ = Storage::set_item(KEY_LOCAL_RUNTIME, runtime.cli());
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

    /// Opens the Private Registries configuration modal populated from `config.yml` and `registries.yml`.
    pub fn open_registry_settings(&self) {
        self.open_registry_settings_inner(None, RegistryRetryTarget::None);
    }

    /// Opens the Private Registries modal pre-focused on `host`, optionally retrying after save.
    pub fn open_registry_settings_for_host(&self, host: &str, retry: RegistryRetryTarget) {
        self.open_registry_settings_inner(Some(host), retry);
    }

    fn open_registry_settings_inner(&self, prefill_host: Option<&str>, retry: RegistryRetryTarget) {
        let (base_rules, user_rules) = match self.service.load_registry_sources() {
            Ok(pair) => pair,
            Err(err) => {
                self.registry_dialog_error.set(Some(err));
                (Vec::new(), Vec::new())
            }
        };
        let base_hosts: HashSet<String> = base_rules
            .iter()
            .map(|r| normalize_registry_host_pattern(&r.host))
            .filter(|h| !h.is_empty())
            .collect();
        let merged = merge_registry_rules(&base_rules, &user_rules);
        let mut drafts: Vec<RegistryDraftRule> = merged
            .iter()
            .map(|rule| {
                let norm = normalize_registry_host_pattern(&rule.host);
                RegistryDraftRule::from_rule(rule, base_hosts.contains(&norm))
            })
            .collect();

        let selected_idx = if let Some(raw_host) = prefill_host {
            let target = normalize_registry_host_pattern(raw_host);
            let exact = drafts
                .iter()
                .position(|d| normalize_registry_host_pattern(&d.host) == target);
            let matched = exact.or_else(|| {
                drafts
                    .iter()
                    .position(|d| registry_host_matches(&d.host, &target))
            });
            if let Some(idx) = matched {
                Some(idx)
            } else if !target.is_empty() {
                drafts.push(RegistryDraftRule {
                    host: target,
                    ..Default::default()
                });
                Some(drafts.len() - 1)
            } else {
                (!drafts.is_empty()).then_some(0)
            }
        } else {
            (!drafts.is_empty()).then_some(0)
        };

        self.registry_config_base.set(base_rules);
        self.registry_drafts.set(drafts);
        self.registry_retry_target.set(retry);
        self.registry_dialog_error.set(None);
        self.registry_selected_index.set(selected_idx);
        self.load_registry_editor(selected_idx);
        self.registry_dialog_open.set(true);
    }

    fn alloc_header_draft(&self, key: String, value: String) -> RegistryHeaderDraft {
        let next_id = self.reg_next_header_id.get_untracked();
        self.reg_next_header_id.set(next_id.wrapping_add(1));
        self.root_owner.run(|| RegistryHeaderDraft {
            id: next_id,
            key: signal(key),
            value: signal(value),
        })
    }

    fn load_registry_editor(&self, idx: Option<usize>) {
        let draft = idx.and_then(|i| {
            self.registry_drafts
                .with_untracked(|list| list.get(i).cloned())
        });
        if let Some(draft) = draft {
            self.reg_host_input.set(draft.host);
            self.reg_username_input.set(draft.username);
            self.reg_password_input.set(draft.password);
            self.reg_token_input.set(draft.token);
            let headers = draft
                .headers
                .into_iter()
                .map(|(k, v)| self.alloc_header_draft(k, v))
                .collect();
            self.reg_headers.set(headers);
            self.reg_insecure_http.set(draft.insecure_http);
            self.reg_skip_tls.set(draft.insecure_skip_tls_verify);
            self.reg_ca_cert_input.set(draft.ca_cert);
        } else {
            self.reg_host_input.set(String::new());
            self.reg_username_input.set(String::new());
            self.reg_password_input.set(String::new());
            self.reg_token_input.set(String::new());
            self.reg_headers.set(Vec::new());
            self.reg_insecure_http.set(false);
            self.reg_skip_tls.set(false);
            self.reg_ca_cert_input.set(String::new());
        }
    }

    /// Commits current right-pane editor signal values back into `registry_drafts[selected]`.
    pub fn sync_active_registry_draft(&self) {
        let Some(idx) = self.registry_selected_index.get_untracked() else {
            return;
        };
        let host = self.reg_host_input.get_untracked();
        let username = self.reg_username_input.get_untracked();
        let password = self.reg_password_input.get_untracked();
        let token = self.reg_token_input.get_untracked();
        let headers = self.reg_headers.with_untracked(|rows| {
            rows.iter()
                .map(|row| (row.key.get_untracked(), row.value.get_untracked()))
                .collect()
        });
        let insecure_http = self.reg_insecure_http.get_untracked();
        let insecure_skip_tls_verify = self.reg_skip_tls.get_untracked();
        let ca_cert = self.reg_ca_cert_input.get_untracked();

        let mut list = self.registry_drafts.get_untracked();
        if let Some(entry) = list.get_mut(idx) {
            entry.host = host;
            entry.username = username;
            entry.password = password;
            entry.token = token;
            entry.headers = headers;
            entry.insecure_http = insecure_http;
            entry.insecure_skip_tls_verify = insecure_skip_tls_verify;
            entry.ca_cert = ca_cert;
            self.registry_drafts.set(list);
        }
    }

    /// Switches selection to draft rule `idx` after saving edits on the previously active rule.
    pub fn select_registry_draft(&self, idx: usize) {
        self.sync_active_registry_draft();
        let len = self.registry_drafts.with_untracked(Vec::len);
        let next = (idx < len).then_some(idx);
        self.registry_selected_index.set(next);
        self.load_registry_editor(next);
        self.registry_dialog_error.set(None);
    }

    /// Appends a blank private registry rule and selects it for editing.
    pub fn add_registry_draft(&self) {
        self.sync_active_registry_draft();
        let mut list = self.registry_drafts.get_untracked();
        list.push(RegistryDraftRule::default());
        let next_idx = list.len() - 1;
        self.registry_drafts.set(list);
        self.registry_selected_index.set(Some(next_idx));
        self.load_registry_editor(Some(next_idx));
        self.registry_dialog_error.set(None);
    }

    /// Deletes the active registry rule (or resets an overriding rule if restored separately).
    pub fn remove_selected_registry_draft(&self) {
        self.sync_active_registry_draft();
        let Some(idx) = self.registry_selected_index.get_untracked() else {
            return;
        };
        let mut list = self.registry_drafts.get_untracked();
        if idx >= list.len() {
            return;
        }
        list.remove(idx);
        let next_idx = if list.is_empty() {
            None
        } else {
            Some(idx.min(list.len() - 1))
        };
        self.registry_drafts.set(list);
        self.registry_selected_index.set(next_idx);
        self.load_registry_editor(next_idx);
        self.registry_dialog_error.set(None);
    }

    /// Resets the currently selected `config.yml`-backed rule back to its `config.yml` defaults.
    pub fn reset_selected_registry_draft(&self) {
        self.sync_active_registry_draft();
        let Some(idx) = self.registry_selected_index.get_untracked() else {
            return;
        };
        let mut list = self.registry_drafts.get_untracked();
        let Some(current) = list.get(idx) else {
            return;
        };
        let norm_host = normalize_registry_host_pattern(&current.host);
        let base_rule = self.registry_config_base.with_untracked(|base| {
            base.iter()
                .find(|r| normalize_registry_host_pattern(&r.host) == norm_host)
                .cloned()
        });
        if let Some(base_rule) = base_rule {
            list[idx] = RegistryDraftRule::from_rule(&base_rule, true);
            self.registry_drafts.set(list);
            self.load_registry_editor(Some(idx));
            self.registry_dialog_error.set(None);
        }
    }

    /// Appends an empty `(key, value)` row to the active rule's custom headers list.
    pub fn add_registry_header_row(&self) {
        let mut rows = self.reg_headers.get_untracked();
        rows.push(self.alloc_header_draft(String::new(), String::new()));
        self.reg_headers.set(rows);
    }

    /// Removes a custom header row by its stable row `id`.
    pub fn remove_registry_header_row(&self, id: u64) {
        let mut rows = self.reg_headers.get_untracked();
        rows.retain(|row| row.id != id);
        self.reg_headers.set(rows);
    }

    /// Opens an OS file dialog to pick a custom PEM CA certificate file for the active rule.
    pub fn browse_registry_ca_cert(&self) {
        let lang = self.lang.get_untracked();
        let title = t(lang, "registry.ca_cert_dialog").to_owned();
        let state = self.clone();
        self.spawn_app_task(async move {
            let picked = pick_paths(OpenDialog::files().title(title)).await;
            if let Ok(Some(paths)) = picked
                && let Some(first) = paths.into_iter().next()
            {
                let path_str = first.display().to_string();
                state
                    .reg_ca_cert_input
                    .set(strip_windows_drive_leading_slash(&path_str).to_owned());
            }
        });
    }

    /// Validates drafts, writes `registries.yml`, hot-reloads `Analyzer`, and optionally retries.
    pub fn save_registry_settings(&self) {
        self.sync_active_registry_draft();
        let lang = self.lang.get_untracked();
        let drafts = self.registry_drafts.get_untracked();
        for draft in &drafts {
            if normalize_registry_host_pattern(&draft.host).is_empty() {
                self.registry_dialog_error
                    .set(Some(t(lang, "registry.error_empty_host").to_owned()));
                return;
            }
        }

        let base_rules = self.registry_config_base.get_untracked();
        let mut user_rules = Vec::new();
        for draft in &drafts {
            let candidate = draft.to_rule();
            let matching_base = base_rules
                .iter()
                .find(|r| normalize_registry_host_pattern(&r.host) == candidate.host)
                .map(|r| RegistryDraftRule::from_rule(r, true).to_rule());
            if matching_base.as_ref() != Some(&candidate) {
                user_rules.push(candidate);
            }
        }

        if let Err(err) = self.service.save_user_registry_rules(&user_rules) {
            self.registry_dialog_error.set(Some(err));
            return;
        }

        let retry = self.registry_retry_target.get_untracked();
        self.registry_dialog_error.set(None);
        self.config_warning.set(self.service.config_warning());
        self.registry_dialog_open.set(false);
        self.registry_retry_target.set(RegistryRetryTarget::None);
        self.show_status_notice(t(lang, "registry.saved_notice").to_owned());

        match retry {
            RegistryRetryTarget::None => {}
            RegistryRetryTarget::Analyze | RegistryRetryTarget::Diff => {
                self.reanalyze_current_image();
            }
        }
    }

    /// Runs startup DMG mount cleanup and silent 48-hour background update check (no-op in tests).
    pub fn startup_update_tasks(&self) {
        if auto_check_disabled_by_env() {
            return;
        }
        thread::spawn(sweep_installer_mounts);
        if let Some(version) = nudge_version() {
            let notes = nudge_notes().unwrap_or_default();
            self.update_phase
                .set(UpdatePhase::Available { version, notes });
        }
        if !auto_check_due(SystemTime::now()) {
            return;
        }

        let (tx, rx) = oneshot::channel();
        thread::spawn(move || {
            let outcome = check_latest_blocking();
            record_outcome(SystemTime::now(), &outcome);
            let _ = tx.send(outcome);
        });
        let state = self.clone();
        self.spawn_app_task(async move {
            let Ok(outcome) = rx.await else {
                return;
            };
            if let UpdateCheck::Newer { version, notes } = outcome {
                if skipped_version().as_deref() == Some(version.as_str()) {
                    state.update_skipped.set(Some(version));
                } else if matches!(
                    state.update_phase.get_untracked(),
                    UpdatePhase::Idle | UpdatePhase::UpToDate
                ) {
                    state
                        .update_phase
                        .set(UpdatePhase::Available { version, notes });
                }
            }
        });
    }

    /// Triggers an immediate manual check against GitHub Releases and surfaces a status toast.
    pub fn check_for_updates(&self) {
        if matches!(
            self.update_phase.get_untracked(),
            UpdatePhase::Checking
                | UpdatePhase::Downloading { .. }
                | UpdatePhase::Installing { .. }
        ) {
            return;
        }
        let lang = self.lang.get_untracked();
        self.update_phase.set(UpdatePhase::Checking);
        self.show_status_notice_with_kind(t(lang, "update.checking").to_owned(), true);

        let (tx, rx) = oneshot::channel();
        thread::spawn(move || {
            let outcome = check_latest_blocking();
            if !auto_check_disabled_by_env() {
                record_outcome(SystemTime::now(), &outcome);
            }
            let _ = tx.send(outcome);
        });

        let state = self.clone();
        self.spawn_app_task(async move {
            let outcome = match rx.await {
                Ok(out) => out,
                Err(_) => UpdateCheck::Failed("update check task cancelled".to_owned()),
            };
            let cur_lang = state.lang.get_untracked();
            match outcome {
                UpdateCheck::Newer { version, notes } => {
                    let notice = tf(
                        cur_lang,
                        "update.available_notice",
                        &[CURRENT_VERSION, &version],
                    );
                    state
                        .update_phase
                        .set(UpdatePhase::Available { version, notes });
                    state.show_status_notice_with_kind(notice, true);
                }
                UpdateCheck::UpToDate => {
                    state.update_phase.set(UpdatePhase::UpToDate);
                    let notice = tf(cur_lang, "update.up_to_date", &[CURRENT_VERSION]);
                    state.show_status_notice_with_kind(notice, true);
                }
                UpdateCheck::Failed(message) => {
                    state.update_phase.set(UpdatePhase::Failed {
                        version: None,
                        message: message.clone(),
                    });
                    let notice = tf(cur_lang, "update.failed", &[&message]);
                    state.show_status_notice_with_kind(notice, true);
                }
            }
        });
    }

    /// Downloads, verifies SHA-256, installs in place, and relaunches when ready.
    pub fn start_update(&self) {
        let version = match self.update_phase.get_untracked() {
            UpdatePhase::Available { version, .. }
            | UpdatePhase::Failed {
                version: Some(version),
                ..
            } => version,
            _ => return,
        };
        let lang = self.lang.get_untracked();
        if asset_name().is_none() {
            let msg = tf(lang, "update.unsupported", &[consts::OS, consts::ARCH]);
            self.update_phase.set(UpdatePhase::Failed {
                version: Some(version),
                message: msg.clone(),
            });
            self.show_status_notice_with_kind(msg, true);
            return;
        }

        self.update_phase.set(UpdatePhase::Downloading {
            version: version.clone(),
            received: 0,
            total: 0,
        });
        let (tx, mut rx) = unbounded_channel();
        let dl_version = version.clone();
        thread::spawn(move || {
            let mut last_sent = 0_u64;
            let downloaded = download_and_verify_blocking(&dl_version, |received, total| {
                if received == 0
                    || received == total
                    || received.saturating_sub(last_sent) >= 64 * 1024
                {
                    last_sent = received;
                    let _ = tx.send(UpdateTaskEvent::Progress { received, total });
                }
            });
            let path = match downloaded {
                Ok(path) => path,
                Err(err) => {
                    let _ = tx.send(UpdateTaskEvent::Finished(Err(err)));
                    return;
                }
            };
            let _ = tx.send(UpdateTaskEvent::Installing);
            let outcome = install_update_package(&path);
            let _ = tx.send(UpdateTaskEvent::Finished(outcome));
        });

        let state = self.clone();
        self.spawn_app_task(async move {
            while let Some(ev) = rx.recv().await {
                match ev {
                    UpdateTaskEvent::Progress { received, total } => {
                        state.update_phase.set(UpdatePhase::Downloading {
                            version: version.clone(),
                            received,
                            total,
                        });
                    }
                    UpdateTaskEvent::Installing => {
                        state.update_phase.set(UpdatePhase::Installing {
                            version: version.clone(),
                        });
                    }
                    UpdateTaskEvent::Finished(Ok(outcome)) => {
                        let cur_lang = state.lang.get_untracked();
                        match outcome {
                            InstallOutcome::Relaunch { target }
                            | InstallOutcome::QuitToInstall { target } => {
                                state.update_phase.set(UpdatePhase::ReadyToRestart {
                                    version: version.clone(),
                                    target: target.clone(),
                                });
                                let notice = tf(cur_lang, "update.ready_restart", &[&version]);
                                state.show_status_notice_with_kind(notice, true);
                                relaunch(Some(&target));
                                quit();
                            }
                            InstallOutcome::OpenedInstaller { path } => {
                                state.update_phase.set(UpdatePhase::OpenedInstaller {
                                    version: version.clone(),
                                    path,
                                });
                                let notice = tf(cur_lang, "update.opened_installer", &[&version]);
                                state.show_status_notice_with_kind(notice, true);
                            }
                        }
                        break;
                    }
                    UpdateTaskEvent::Finished(Err(message)) => {
                        let cur_lang = state.lang.get_untracked();
                        state.update_phase.set(UpdatePhase::Failed {
                            version: Some(version.clone()),
                            message: message.clone(),
                        });
                        let notice = tf(cur_lang, "update.failed", &[&message]);
                        state.show_status_notice_with_kind(notice, true);
                        break;
                    }
                }
            }
        });
    }

    /// Marks the currently offered update version as skipped so background checks stay quiet.
    pub fn skip_available_update(&self) {
        let version = match self.update_phase.get_untracked() {
            UpdatePhase::Available { version, .. }
            | UpdatePhase::Failed {
                version: Some(version),
                ..
            } => version,
            _ => return,
        };
        if !auto_check_disabled_by_env() {
            skip_version(&version);
        }
        self.update_skipped.set(Some(version.clone()));
        self.update_phase.set(UpdatePhase::Idle);
        let lang = self.lang.get_untracked();
        self.show_status_notice_with_kind(tf(lang, "update.skipped_notice", &[&version]), false);
    }

    /// Clears any skipped version marker and re-checks for updates immediately.
    pub fn resume_skipped_update(&self) {
        if !auto_check_disabled_by_env() {
            clear_skipped_version();
        }
        self.update_skipped.set(None);
        self.check_for_updates();
    }

    /// Triggers relaunch if an update is already staged and awaiting restart.
    pub fn relaunch_after_update(&self) {
        if let UpdatePhase::ReadyToRestart { target, .. } = self.update_phase.get_untracked() {
            relaunch(Some(&target));
            quit();
        }
    }

    /// Returns the concise localized label for the primary updater command across menus/palette.
    pub fn update_menu_label(&self, lang: UiLang) -> String {
        match self.update_phase.get() {
            UpdatePhase::Checking => t(lang, "update.checking").to_owned(),
            UpdatePhase::Available { version, .. } => {
                tf(lang, "update.action_install_version", &[&version])
            }
            UpdatePhase::Downloading {
                version,
                received,
                total,
            } => {
                let progress = if total > 0 {
                    let pct = ((received as f64 / total as f64) * 100.0).clamp(0.0, 100.0);
                    format!("{pct:.0}%")
                } else {
                    format_bytes(received)
                };
                tf(lang, "update.downloading", &[&version, &progress])
            }
            UpdatePhase::Installing { version } => tf(lang, "update.installing", &[&version]),
            UpdatePhase::ReadyToRestart { .. } => t(lang, "update.restart_now").to_owned(),
            UpdatePhase::Failed {
                version: Some(_), ..
            } => t(lang, "update.retry").to_owned(),
            UpdatePhase::Idle
            | UpdatePhase::UpToDate
            | UpdatePhase::OpenedInstaller { .. }
            | UpdatePhase::Failed { version: None, .. } => t(lang, "menu.check_updates").to_owned(),
        }
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
    let normalized = strip_windows_drive_leading_slash(&decoded);
    format!("file://{normalized}")
}

fn strip_windows_drive_leading_slash(path: &str) -> &str {
    let bytes = path.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        &path[1..]
    } else {
        path
    }
}

fn parse_explicit_arch_query(image_ref: &str) -> Option<String> {
    if matches!(
        image_source_kind(image_ref),
        ImageSourceKind::LocalDocker | ImageSourceKind::LocalPodman
    ) {
        return None;
    }
    let query = image_ref.split_once('?')?.1;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=')
            && k.trim() == "arch"
        {
            let arch = v.trim();
            if !arch.is_empty() {
                return Some(arch.to_owned());
            }
        }
    }
    None
}

fn find_tree_item_link(items: &[FileTreeItem], path: &str) -> Option<String> {
    fn walk(items: &[FileTreeItem], prefix: &str, target: &str) -> Option<String> {
        for item in items {
            let key = if prefix.is_empty() {
                item.name.clone()
            } else {
                format!("{prefix}/{}", item.name)
            };
            if key == target && item.children.is_empty() {
                return Some(item.link.clone());
            }
            if target.starts_with(&key)
                && target.as_bytes().get(key.len()) == Some(&b'/')
                && let Some(found) = walk(&item.children, &key, target)
            {
                return Some(found);
            }
        }
        None
    }
    walk(items, "", path)
}

fn short_digest(digest: &str) -> &str {
    let trimmed = digest.strip_prefix("sha256:").unwrap_or(digest);
    trimmed.get(..12).unwrap_or(trimmed)
}

#[cfg(test)]
mod tests {
    use diving_core::Analyzer;
    use nsh_engine::{EngineConfig, Environment, Idiom, Os, Points, Size};
    use nsh_testing::{Headless, windowed_root};

    use super::*;
    use crate::ui::app_tokens;

    #[test]
    fn windows_uri_and_explicit_arch_helpers_work() {
        assert_eq!(
            percent_decode_file_uri("file:///C:/Users/demo/my%20image.tar"),
            "file://C:/Users/demo/my image.tar"
        );
        assert_eq!(
            percent_decode_file_uri("file:///D%3A/oci-layout"),
            "file://D:/oci-layout"
        );
        assert_eq!(
            percent_decode_file_uri("file:///Users/demo/image.tar"),
            "file:///Users/demo/image.tar"
        );
        assert_eq!(
            parse_explicit_arch_query("redis:alpine?arch=arm64"),
            Some("arm64".to_owned())
        );
        assert_eq!(
            parse_explicit_arch_query("file://C:/tmp/oci?arch=amd64"),
            Some("amd64".to_owned())
        );
        assert_eq!(
            parse_explicit_arch_query("docker://redis:alpine?arch=arm64"),
            None
        );
        assert_eq!(
            parse_explicit_arch_query("container://redis:alpine?arch=arm64"),
            Some("arm64".to_owned())
        );
    }

    #[component]
    fn CancelledCompareDialogHarness() -> impl IntoElement {
        let analyzer = Analyzer::builder().build().expect("ephemeral analyzer");
        let state = AppState::new(DivingService::with_analyzer(analyzer));
        state.diff_base_ref.set("redis:alpine".to_owned());
        state.diff_base_arch.set(Some(SharedString::from("amd64")));
        state.diff_target_ref.set("redis:alpine".to_owned());
        state
            .diff_target_arch
            .set(Some(SharedString::from("arm64")));
        state.compare_base_input.set("redis:alpine".to_owned());
        state
            .compare_base_arch
            .set(Some(SharedString::from("amd64")));
        state.compare_target_input.set("redis:alpine".to_owned());
        state
            .compare_target_arch
            .set(Some(SharedString::from("arm64")));
        let base_res = Arc::new(DockerAnalyzeResult {
            arch: "amd64".to_owned(),
            ..Default::default()
        });
        let target_res = Arc::new(DockerAnalyzeResult {
            arch: "arm64".to_owned(),
            ..Default::default()
        });
        state.diff_base_result.set(Some(base_res));
        state.diff_target_result.set(Some(target_res));
        state.route.set(AppRoute::Diff);

        // User opens Compare dialog, mutates inputs/archs, then cancels without submitting.
        state.open_compare_dialog();
        state.compare_base_input.set("nginx:alpine".to_owned());
        state
            .compare_base_arch
            .set(Some(SharedString::from("arm64")));
        state.compare_target_input.set("busybox:latest".to_owned());
        state.compare_target_arch.set(None);
        state.dismiss_top_overlay_or_search();
        assert!(!state.compare_dialog_open.get_untracked());

        // Active diff archs stay intact for Mod+R and Swap.
        assert_eq!(
            state
                .diff_base_arch
                .get_untracked()
                .as_ref()
                .map(SharedString::as_str),
            Some("amd64")
        );
        assert_eq!(
            state
                .diff_target_arch
                .get_untracked()
                .as_ref()
                .map(SharedString::as_str),
            Some("arm64")
        );

        state.swap_diff();
        assert_eq!(
            state
                .diff_base_arch
                .get_untracked()
                .as_ref()
                .map(SharedString::as_str),
            Some("arm64")
        );
        assert_eq!(
            state
                .diff_target_arch
                .get_untracked()
                .as_ref()
                .map(SharedString::as_str),
            Some("amd64")
        );
        assert_eq!(state.compare_base_input.get_untracked(), "redis:alpine");
        assert_eq!(state.compare_target_input.get_untracked(), "redis:alpine");
        assert_eq!(
            state
                .compare_base_arch
                .get_untracked()
                .as_ref()
                .map(SharedString::as_str),
            Some("arm64")
        );
        assert_eq!(
            state
                .compare_target_arch
                .get_untracked()
                .as_ref()
                .map(SharedString::as_str),
            Some("amd64")
        );

        // Mutate and cancel again, then verify reopening restores active diff params.
        state.open_compare_dialog();
        state.compare_base_input.set("other:v1".to_owned());
        state.compare_base_arch.set(None);
        state.dismiss_top_overlay_or_search();
        state.open_compare_dialog();
        assert_eq!(state.compare_base_input.get_untracked(), "redis:alpine");
        assert_eq!(
            state
                .compare_base_arch
                .get_untracked()
                .as_ref()
                .map(SharedString::as_str),
            Some("arm64")
        );
        text("compare-dialog-state-ok")
    }

    #[test]
    fn cancelled_compare_dialog_edits_do_not_leak_into_swap_reanalyze_or_reopen() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(960.0), Points(640.0)),
            ..EngineConfig::new(
                windowed_root::<CancelledCompareDialogHarness>().tokens(app_tokens()),
            )
        };
        let mut app = Headless::start(config).expect("start compare dialog state harness");
        app.wait_for_within("compare-dialog-state-ok", Duration::from_secs(2))
            .expect("compare dialog state assertions passed");
    }

    #[component]
    fn ResponsiveSyncHarness() -> impl IntoElement {
        use crate::ui::{ButtonVariant, UiSize, ui_button};

        let state = AppState::new(DivingService::new());
        let sidebar = state.app_sidebar_open;
        let meta = state.tree_show_meta_cols;
        let toggle_meta = state.clone();

        view()
            .flex_col()
            .child(text(move || {
                format!("sidebar={};meta={}", sidebar.get(), meta.get())
            }))
            .child(ui_button(
                "ToggleMeta",
                ButtonVariant::Ghost,
                UiSize::Xs,
                None,
                false,
                false,
                move || {
                    toggle_meta
                        .set_tree_show_meta_cols(!toggle_meta.tree_show_meta_cols.get_untracked())
                },
            ))
    }

    #[test]
    fn responsive_viewport_syncs_sidebar_and_meta_columns_across_960_and_1440() {
        let config = EngineConfig {
            environment: Environment {
                os: Os::MacOs,
                idiom: Idiom::Desktop,
                ..Default::default()
            },
            viewport: Size::new(Points(960.0), Points(640.0)),
            ..EngineConfig::new(windowed_root::<ResponsiveSyncHarness>().tokens(app_tokens()))
        };
        let mut app = Headless::start(config).expect("start responsive sync harness");
        app.wait_for_within("sidebar=false;meta=false", Duration::from_secs(2))
            .expect("960pt starts with compact sidebar=false & meta=false");

        app.set_viewport(Size::new(Points(1440.0), Points(900.0)))
            .expect("resize to 1440x900");
        app.wait_for_within("sidebar=true;meta=true", Duration::from_secs(2))
            .expect("1440pt restores sidebar=true & meta=true");

        app.set_viewport(Size::new(Points(960.0), Points(640.0)))
            .expect("resize back to 960x640");
        app.wait_for_within("sidebar=false;meta=false", Duration::from_secs(2))
            .expect("960pt collapses sidebar=false & meta=false again");
    }
}
