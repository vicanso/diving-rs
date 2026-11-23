//! Image diff: what a target image adds, removes and grows relative to a
//! base image — typically the same repository one release or one commit
//! earlier. A pure function of the two analyses, like `recommend`.

use bytesize::ByteSize;
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::i18n::{self, Lang};
use crate::image::{final_filesystem, normalize_arch, DockerAnalyzeResult, FinalFile, ImageLayer};
use crate::markdown::{clip, icon};
use crate::recommend::Recommendation;

/// Files kept per change kind (largest first); the counts and byte totals
/// always cover every file.
pub const DIFF_MAX_FILES: usize = 50;
/// Directories kept, largest size change first.
pub const DIFF_MAX_DIRS: usize = 20;
/// Maximum file size eligible for same-size blake3 content verification.
pub const DIFF_CONTENT_HASH_MAX_BYTES: u64 = 1024 * 1024;
/// Rows per table in the Markdown rendering.
const MD_ROWS: usize = 10;
/// Files count toward their directory at most this deep (`usr/local/lib`),
/// so every byte lands in exactly one directory row.
const DIR_DEPTH: usize = 3;
/// Minimum seconds between consecutive layer `created` timestamps to detect a
/// base-to-app boundary (matching `markdown::find_user_layer_start`).
const BASE_LAYER_GAP_SECS: i64 = 3600;
/// Epoch-0 flattened builds report `created` around 1970-01-01; ignore anything
/// within the first day so synthetic timestamps never trigger gap detection.
const MIN_VALID_TIMESTAMP_SECS: i64 = 86400;

/// How diff decides whether to skip leading base OS/runtime layers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BaseLayerMode {
    /// Use shared `diff_id`/`digest` prefix when layers match; otherwise skip
    /// detected base layers automatically when architectures differ.
    #[default]
    Auto,
    /// Skip detected base layers using the timestamp-gap / first-layer heuristic
    /// regardless of architecture or shared prefix.
    ForceAuto,
    /// Disable base-layer skipping heuristics (exact shared prefix still applies).
    Off,
    /// Always skip the first `N` file-carrying layers on each side.
    Skip(usize),
}

/// Options controlling [`diff_images_with_options`] and [`crate::Analyzer::compute_diff`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffOptions {
    pub base_layer_mode: BaseLayerMode,
    pub verify_content: bool,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            base_layer_mode: BaseLayerMode::Auto,
            verify_content: true,
        }
    }
}

impl DiffOptions {
    pub fn base_layer_mode(mut self, mode: BaseLayerMode) -> Self {
        self.base_layer_mode = mode;
        self
    }

    pub fn verify_content(mut self, verify: bool) -> Self {
        self.verify_content = verify;
        self
    }
}

/// Why base layers were (or were not) skipped for a diff.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BaseLayerSkipReason {
    #[default]
    None,
    SharedPrefix,
    CrossArchCreatedGap,
    CrossArchFirstLayer,
    ForcedCreatedGap,
    ForcedFirstLayer,
    ManualCount,
}

/// The headline numbers of one side of the diff.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffSide {
    pub name: String,
    pub arch: String,
    /// Compressed (pull) size.
    pub size: u64,
    /// Uncompressed size, the SIZE `docker images` shows.
    pub total_size: u64,
    /// Layers that carry files; metadata-only layers are left out.
    pub layer_count: usize,
    pub efficiency_score: u64,
    pub wasted_size: u64,
    /// Entries in the compared filesystem (upper layers when base layers are ignored).
    pub file_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_as_of: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLayer {
    /// 1-based, as numbered in that image's own report.
    pub layer: usize,
    pub digest: String,
    pub cmd: String,
    pub size: u64,
    pub unpack_size: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    /// 0 for an added file.
    pub base_size: u64,
    /// 0 for a removed file.
    pub target_size: u64,
    /// 1-based layer holding the copy: the target's for added and changed
    /// files, the base's for removed ones.
    pub layer: usize,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base_mode: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target_mode: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base_link: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub target_link: String,
    #[serde(default)]
    pub is_binary: bool,
    #[serde(default)]
    pub content_changed: bool,
}

impl FileChange {
    pub fn delta(&self) -> i64 {
        self.target_size as i64 - self.base_size as i64
    }

    /// True when the file exists on both sides, looks like a compiled binary or
    /// shared library, and keeps the same permissions and link state.
    pub fn is_expected_binary_only(&self) -> bool {
        self.is_binary
            && self.base_size > 0
            && self.target_size > 0
            && self.base_mode == self.target_mode
            && self.base_link == self.target_link
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeSet {
    /// Every file of this kind.
    pub count: usize,
    /// Net bytes over all of them, target − base.
    pub bytes: i64,
    /// The largest of them, at most [`DIFF_MAX_FILES`].
    pub files: Vec<FileChange>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirChange {
    /// `""` for files at the image root.
    pub path: String,
    pub base_size: u64,
    pub target_size: u64,
}

impl DirChange {
    pub fn delta(&self) -> i64 {
        self.target_size as i64 - self.base_size as i64
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyValueDiff {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScalarChange<T> {
    pub base: T,
    pub target: T,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigDiff {
    pub same: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_os: Option<ScalarChange<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<ScalarChange<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<ScalarChange<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<ScalarChange<Vec<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cmd: Option<ScalarChange<Vec<String>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_healthcheck: Option<ScalarChange<bool>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added_ports: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_ports: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_changes: Vec<KeyValueDiff>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub label_changes: Vec<KeyValueDiff>,
}

impl ConfigDiff {
    pub fn change_count(&self) -> usize {
        [
            self.base_os.is_some(),
            self.user.is_some(),
            self.working_dir.is_some(),
            self.entrypoint.is_some(),
            self.cmd.is_some(),
            self.has_healthcheck.is_some(),
        ]
        .into_iter()
        .filter(|&changed| changed)
        .count()
            + self.added_ports.len()
            + self.removed_ports.len()
            + self.env_changes.len()
            + self.label_changes.len()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDiff {
    pub same: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<ScalarChange<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub libc: Option<ScalarChange<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_glibc: Option<ScalarChange<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_glibc: Option<ScalarChange<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue: Option<ScalarChange<String>>,
    pub base_unstripped_count: usize,
    pub target_unstripped_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added_unstripped: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed_unstripped: Vec<String>,
}

impl RuntimeDiff {
    pub fn change_count(&self) -> usize {
        [
            self.entrypoint.is_some(),
            self.libc.is_some(),
            self.required_glibc.is_some(),
            self.os_glibc.is_some(),
            self.issue.is_some(),
        ]
        .into_iter()
        .filter(|&changed| changed)
        .count()
            + self.added_unstripped.len()
            + self.removed_unstripped.len()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ParityStatus {
    #[default]
    Consistent,
    BinaryDiffsOnly,
    ConfigOrFileDrift,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParitySummary {
    pub status: ParityStatus,
    pub expected_binary_changes: usize,
    pub unexpected_file_changes: usize,
    pub one_sided_files: usize,
    pub non_binary_size_changes: usize,
    pub mode_changes: usize,
    pub link_changes: usize,
    pub same_size_content_changes: usize,
    pub config_changes: usize,
    pub runtime_changes: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageDiff {
    pub base: DiffSide,
    pub target: DiffSide,
    /// Uncompressed size change across full images, target − base.
    pub size_delta: i64,
    /// Uncompressed size change across non-skipped upper layers, target − base.
    pub upper_size_delta: i64,
    pub base_layers_ignored: bool,
    pub base_skip_reason: BaseLayerSkipReason,
    pub base_skipped_layers: usize,
    pub target_skipped_layers: usize,
    pub base_skipped_size: u64,
    pub target_skipped_size: u64,
    /// File-carrying layers both images start with (same digests, same
    /// order) — usually the common base image.
    pub shared_layers: usize,
    /// Uncompressed size of the shared layers.
    pub shared_size: u64,
    /// The base's layers after the shared (or skipped) ones.
    pub base_layers: Vec<DiffLayer>,
    /// The target's layers after the shared (or skipped) ones.
    pub target_layers: Vec<DiffLayer>,
    pub added: ChangeSet,
    pub removed: ChangeSet,
    /// Files in both images whose size, mode, symlink target, or verified content differs.
    pub changed: ChangeSet,
    /// Directories whose size changed, largest change first.
    pub dirs: Vec<DirChange>,
    pub config_diff: ConfigDiff,
    pub runtime_diff: RuntimeDiff,
    pub parity: ParitySummary,
    pub content_verified: bool,
    pub content_verified_files: usize,
    pub content_skipped_files: usize,
    /// Recommendations (by id) the target has and the base does not.
    pub new_recommendations: Vec<Recommendation>,
    /// Recommendations (by id) the base had and the target no longer does.
    pub resolved_recommendations: Vec<Recommendation>,
}

fn side(result: &DockerAnalyzeResult, file_count: usize) -> DiffSide {
    let summary = result.summary();
    DiffSide {
        name: result.name.clone(),
        arch: result.arch.clone(),
        size: result.size,
        total_size: result.total_size,
        layer_count: result.layers.iter().filter(|l| !l.empty).count(),
        efficiency_score: summary.score,
        wasted_size: summary.wasted_size,
        file_count,
        stale_as_of: result.stale_as_of.clone(),
    }
}

/// File-carrying layers with their 1-based numbers.
fn file_layers(layers: &[ImageLayer]) -> Vec<(usize, &ImageLayer)> {
    layers
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.empty)
        .map(|(i, l)| (i + 1, l))
        .collect()
}

/// Whether two layers hold the same files: by diff id (the uncompressed
/// tar's digest, recorded by every source) when both have one, else by a
/// content digest. A `docker save` tar may name its layers by path only,
/// which says nothing about their contents.
fn same_layer(a: &ImageLayer, b: &ImageLayer) -> bool {
    if !a.diff_id.is_empty() && !b.diff_id.is_empty() {
        return a.diff_id == b.diff_id;
    }
    a.digest.starts_with("sha256:") && a.digest == b.digest
}

fn diff_layer((layer, l): &(usize, &ImageLayer)) -> DiffLayer {
    DiffLayer {
        layer: *layer,
        digest: l.digest.clone(),
        cmd: l.cmd.clone(),
        size: l.size,
        unpack_size: l.unpack_size,
    }
}

/// Parse RFC3339 timestamps on non-empty layers and return the number of leading
/// base layers to skip along with whether a `> 1h` gap was detected.
fn detect_auto_base_skip(file_layers: &[(usize, &ImageLayer)]) -> (usize, bool) {
    if file_layers.len() < 2 {
        return (0, false);
    }
    let timestamps: Vec<Option<i64>> = file_layers
        .iter()
        .map(|(_, l)| {
            DateTime::parse_from_rfc3339(&l.created)
                .ok()
                .map(|dt| dt.timestamp())
                .filter(|&ts| ts > MIN_VALID_TIMESTAMP_SECS)
        })
        .collect();
    let mut best_gap: Option<(i64, usize)> = None;
    for i in 0..file_layers.len() - 1 {
        if let (Some(prev), Some(next)) = (timestamps[i], timestamps[i + 1]) {
            let gap = next - prev;
            if gap > BASE_LAYER_GAP_SECS && best_gap.is_none_or(|(best, _)| gap > best) {
                best_gap = Some((gap, i + 1));
            }
        }
    }
    match best_gap {
        Some((_, skip)) => (skip, true),
        None => (1, false),
    }
}

struct LayerPartition {
    shared: usize,
    base_skip: usize,
    target_skip: usize,
    base_layers_ignored: bool,
    reason: BaseLayerSkipReason,
}

fn partition_layers(
    base: &DockerAnalyzeResult,
    target: &DockerAnalyzeResult,
    base_layers: &[(usize, &ImageLayer)],
    target_layers: &[(usize, &ImageLayer)],
    mode: BaseLayerMode,
) -> LayerPartition {
    let shared_prefix = || {
        base_layers
            .iter()
            .zip(target_layers)
            .take_while(|((_, a), (_, b))| same_layer(a, b))
            .count()
    };

    match mode {
        BaseLayerMode::Off | BaseLayerMode::Skip(0) => {
            let shared = shared_prefix();
            LayerPartition {
                shared,
                base_skip: 0,
                target_skip: 0,
                base_layers_ignored: false,
                reason: if shared > 0 {
                    BaseLayerSkipReason::SharedPrefix
                } else {
                    BaseLayerSkipReason::None
                },
            }
        }
        BaseLayerMode::Skip(n) => {
            let base_skip = n.min(base_layers.len());
            let target_skip = n.min(target_layers.len());
            let ignored = base_skip > 0 || target_skip > 0;
            LayerPartition {
                shared: 0,
                base_skip,
                target_skip,
                base_layers_ignored: ignored,
                reason: if ignored {
                    BaseLayerSkipReason::ManualCount
                } else {
                    BaseLayerSkipReason::None
                },
            }
        }
        BaseLayerMode::ForceAuto => {
            let (b_skip, b_gap) = detect_auto_base_skip(base_layers);
            let (t_skip, t_gap) = detect_auto_base_skip(target_layers);
            let ignored = b_skip > 0 || t_skip > 0;
            if !ignored {
                let shared = shared_prefix();
                return LayerPartition {
                    shared,
                    base_skip: 0,
                    target_skip: 0,
                    base_layers_ignored: false,
                    reason: if shared > 0 {
                        BaseLayerSkipReason::SharedPrefix
                    } else {
                        BaseLayerSkipReason::None
                    },
                };
            }
            LayerPartition {
                shared: 0,
                base_skip: b_skip,
                target_skip: t_skip,
                base_layers_ignored: true,
                reason: if b_gap || t_gap {
                    BaseLayerSkipReason::ForcedCreatedGap
                } else {
                    BaseLayerSkipReason::ForcedFirstLayer
                },
            }
        }
        BaseLayerMode::Auto => {
            let shared = shared_prefix();
            let base_arch = normalize_arch(&base.arch);
            let target_arch = normalize_arch(&target.arch);
            let cross_arch =
                !base_arch.is_empty() && !target_arch.is_empty() && base_arch != target_arch;
            if shared == 0 && cross_arch {
                let (b_skip, b_gap) = detect_auto_base_skip(base_layers);
                let (t_skip, t_gap) = detect_auto_base_skip(target_layers);
                if b_skip > 0 || t_skip > 0 {
                    return LayerPartition {
                        shared: 0,
                        base_skip: b_skip,
                        target_skip: t_skip,
                        base_layers_ignored: true,
                        reason: if b_gap || t_gap {
                            BaseLayerSkipReason::CrossArchCreatedGap
                        } else {
                            BaseLayerSkipReason::CrossArchFirstLayer
                        },
                    };
                }
            }
            LayerPartition {
                shared,
                base_skip: 0,
                target_skip: 0,
                base_layers_ignored: false,
                reason: if shared > 0 {
                    BaseLayerSkipReason::SharedPrefix
                } else {
                    BaseLayerSkipReason::None
                },
            }
        }
    }
}

fn tree_start_for_skipped(layers: &[ImageLayer], skip_file_layers: usize) -> usize {
    if skip_file_layers == 0 {
        return 0;
    }
    layers
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.empty)
        .nth(skip_file_layers - 1)
        .map_or(layers.len(), |(idx, _)| idx + 1)
}

fn scoped_filesystem(
    result: &DockerAnalyzeResult,
    tree_start: usize,
) -> BTreeMap<String, FinalFile> {
    if tree_start == 0 {
        return final_filesystem(&result.file_tree_list);
    }
    let slice = result.file_tree_list.get(tree_start..).unwrap_or(&[]);
    let mut fs = final_filesystem(slice);
    for file in fs.values_mut() {
        file.layer += tree_start;
    }
    fs
}

/// The directory a file's bytes count toward: its parent, cut to
/// [`DIR_DEPTH`] segments.
fn dir_bucket(path: &str) -> &str {
    let mut end = 0;
    for (depth, (i, _)) in path.match_indices('/').enumerate() {
        if depth == DIR_DEPTH {
            break;
        }
        end = i;
    }
    &path[..end]
}

fn change_set<R: Ord, D: Ord>(
    mut files: Vec<FileChange>,
    retain_key: impl Fn(&FileChange) -> R,
    display_key: impl Fn(&FileChange) -> D,
) -> ChangeSet {
    let bytes = files.iter().map(FileChange::delta).sum();
    let count = files.len();
    if files.len() > DIFF_MAX_FILES {
        files.sort_by(|a, b| {
            retain_key(b)
                .cmp(&retain_key(a))
                .then_with(|| a.path.cmp(&b.path))
        });
        files.truncate(DIFF_MAX_FILES);
    }
    files.sort_by(|a, b| {
        display_key(b)
            .cmp(&display_key(a))
            .then_with(|| a.path.cmp(&b.path))
    });
    ChangeSet {
        count,
        bytes,
        files,
    }
}

fn rec_ids(recs: &[Recommendation]) -> HashSet<&str> {
    recs.iter().map(|r| r.id.as_str()).collect()
}

fn is_executable_mode(mode: &str) -> bool {
    let c: Vec<char> = mode.chars().collect();
    c.len() >= 10 && [3, 6, 9].iter().any(|&i| matches!(c[i], 'x' | 's' | 't'))
}

fn has_script_or_config_ext(lower_name: &str) -> bool {
    const EXTENSIONS: &[&str] = &[
        ".sh", ".bash", ".zsh", ".ash", ".ksh", ".py", ".rb", ".pl", ".php", ".js", ".mjs", ".cjs",
        ".ts", ".mts", ".lua", ".awk", ".ps1", ".bat", ".cmd", ".yml", ".yaml", ".json", ".toml",
        ".conf", ".cfg", ".ini", ".env", ".txt", ".md", ".html", ".htm", ".css", ".scss", ".svg",
        ".xml", ".sql", ".crt", ".pem", ".key", ".pub", ".log", ".lock",
    ];
    EXTENSIONS.iter().any(|ext| lower_name.ends_with(ext))
}

fn has_binary_ext(lower_name: &str) -> bool {
    const EXTENSIONS: &[&str] = &[
        ".so", ".dll", ".dylib", ".a", ".o", ".node", ".wasm", ".exe", ".bin", ".elf", ".ko",
    ];
    if EXTENSIONS.iter().any(|ext| lower_name.ends_with(ext)) {
        return true;
    }
    if let Some((_, ver)) = lower_name.split_once(".so.") {
        return ver.as_bytes().first().is_some_and(u8::is_ascii_digit);
    }
    false
}

fn in_binary_dir(path: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "bin/",
        "sbin/",
        "usr/bin/",
        "usr/sbin/",
        "usr/local/bin/",
        "usr/local/sbin/",
    ];
    PREFIXES.iter().any(|prefix| path.starts_with(prefix))
}

fn is_binary_file(path: &str, mode: &str, link: &str) -> bool {
    if !link.is_empty() || mode.starts_with('d') || mode.starts_with('l') {
        return false;
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    let lower = name.to_ascii_lowercase();
    if has_binary_ext(&lower) {
        return true;
    }
    if has_script_or_config_ext(&lower) {
        return false;
    }
    is_executable_mode(mode) || in_binary_dir(path)
}

fn is_hashable_regular_file(file: &FinalFile) -> bool {
    file.size > 0
        && file.size <= DIFF_CONTENT_HASH_MAX_BYTES
        && file.link.is_empty()
        && !file.mode.starts_with('d')
        && !file.mode.starts_with('l')
}

fn scalar_change<T: PartialEq + Clone>(base: &T, target: &T) -> Option<ScalarChange<T>> {
    if base != target {
        Some(ScalarChange {
            base: base.clone(),
            target: target.clone(),
        })
    } else {
        None
    }
}

fn parse_kv_list(items: &[String]) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for item in items {
        if let Some((k, v)) = item.split_once('=') {
            map.insert(k.to_string(), v.to_string());
        } else if !item.is_empty() {
            map.insert(item.clone(), String::new());
        }
    }
    map
}

fn diff_kv_lists(base: &[String], target: &[String]) -> Vec<KeyValueDiff> {
    let b = parse_kv_list(base);
    let t = parse_kv_list(target);
    let keys: BTreeSet<&str> = b.keys().chain(t.keys()).map(String::as_str).collect();
    let mut diffs = Vec::new();
    for key in keys {
        match (b.get(key), t.get(key)) {
            (Some(bv), Some(tv)) if bv == tv => {}
            (bv, tv) => diffs.push(KeyValueDiff {
                key: key.to_string(),
                base: bv.cloned(),
                target: tv.cloned(),
            }),
        }
    }
    diffs
}

fn diff_config(base: &DockerAnalyzeResult, target: &DockerAnalyzeResult) -> ConfigDiff {
    let base_os = scalar_change(
        &base.base_os.trim().to_string(),
        &target.base_os.trim().to_string(),
    );
    let user = scalar_change(&base.user, &target.user);
    let working_dir = scalar_change(&base.working_dir, &target.working_dir);
    let entrypoint = scalar_change(&base.entrypoint, &target.entrypoint);
    let cmd = scalar_change(&base.cmd, &target.cmd);
    let has_healthcheck = scalar_change(&base.has_healthcheck, &target.has_healthcheck);

    let b_ports: BTreeSet<&str> = base.exposed_ports.iter().map(String::as_str).collect();
    let t_ports: BTreeSet<&str> = target.exposed_ports.iter().map(String::as_str).collect();
    let added_ports: Vec<String> = t_ports
        .difference(&b_ports)
        .map(|s| (*s).to_string())
        .collect();
    let removed_ports: Vec<String> = b_ports
        .difference(&t_ports)
        .map(|s| (*s).to_string())
        .collect();

    let env_changes = diff_kv_lists(&base.envs, &target.envs);
    let label_changes = diff_kv_lists(&base.labels, &target.labels);

    let same = base_os.is_none()
        && user.is_none()
        && working_dir.is_none()
        && entrypoint.is_none()
        && cmd.is_none()
        && has_healthcheck.is_none()
        && added_ports.is_empty()
        && removed_ports.is_empty()
        && env_changes.is_empty()
        && label_changes.is_empty();

    ConfigDiff {
        same,
        base_os,
        user,
        working_dir,
        entrypoint,
        cmd,
        has_healthcheck,
        added_ports,
        removed_ports,
        env_changes,
        label_changes,
    }
}

fn diff_runtime(base: &DockerAnalyzeResult, target: &DockerAnalyzeResult) -> RuntimeDiff {
    let bc = &base.runtime_compat;
    let tc = &target.runtime_compat;
    let entrypoint = scalar_change(&bc.entrypoint, &tc.entrypoint);
    let libc = scalar_change(&bc.libc, &tc.libc);
    let required_glibc = scalar_change(&bc.required_glibc, &tc.required_glibc);
    let os_glibc = scalar_change(&bc.os_glibc, &tc.os_glibc);
    let issue = scalar_change(&bc.issue, &tc.issue);

    let b_unstripped: BTreeSet<&str> = base
        .unstripped_binaries
        .iter()
        .map(|u| u.path.as_str())
        .collect();
    let t_unstripped: BTreeSet<&str> = target
        .unstripped_binaries
        .iter()
        .map(|u| u.path.as_str())
        .collect();
    let added_unstripped: Vec<String> = t_unstripped
        .difference(&b_unstripped)
        .map(|s| (*s).to_string())
        .collect();
    let removed_unstripped: Vec<String> = b_unstripped
        .difference(&t_unstripped)
        .map(|s| (*s).to_string())
        .collect();

    let same = entrypoint.is_none()
        && libc.is_none()
        && required_glibc.is_none()
        && os_glibc.is_none()
        && issue.is_none()
        && added_unstripped.is_empty()
        && removed_unstripped.is_empty();

    RuntimeDiff {
        same,
        entrypoint,
        libc,
        required_glibc,
        os_glibc,
        issue,
        base_unstripped_count: base.unstripped_binaries.len(),
        target_unstripped_count: target.unstripped_binaries.len(),
        added_unstripped,
        removed_unstripped,
    }
}

fn build_parity_summary(
    added_count: usize,
    removed_count: usize,
    changed: &[FileChange],
    config_diff: &ConfigDiff,
    runtime_diff: &RuntimeDiff,
) -> ParitySummary {
    let mut expected_binary_changes = 0usize;
    let mut non_binary_size_changes = 0usize;
    let mut mode_changes = 0usize;
    let mut link_changes = 0usize;
    let mut same_size_content_changes = 0usize;
    let mut unexpected_changed = 0usize;

    for change in changed {
        if change.is_expected_binary_only() {
            expected_binary_changes += 1;
            continue;
        }
        unexpected_changed += 1;
        if change.base_mode != change.target_mode {
            mode_changes += 1;
        }
        if change.base_link != change.target_link {
            link_changes += 1;
        }
        if change.content_changed {
            same_size_content_changes += 1;
        } else if !change.is_binary && change.base_size != change.target_size {
            non_binary_size_changes += 1;
        }
    }

    let one_sided_files = added_count + removed_count;
    let unexpected_file_changes = one_sided_files + unexpected_changed;
    let config_changes = config_diff.change_count();
    let runtime_changes = runtime_diff.change_count();

    let status = if unexpected_file_changes == 0 && config_changes == 0 && runtime_changes == 0 {
        if expected_binary_changes > 0 {
            ParityStatus::BinaryDiffsOnly
        } else {
            ParityStatus::Consistent
        }
    } else {
        ParityStatus::ConfigOrFileDrift
    };

    ParitySummary {
        status,
        expected_binary_changes,
        unexpected_file_changes,
        one_sided_files,
        non_binary_size_changes,
        mode_changes,
        link_changes,
        same_size_content_changes,
        config_changes,
        runtime_changes,
    }
}

fn changed_retention_key(c: &FileChange) -> (bool, bool, bool, u64, u64) {
    (
        !c.is_expected_binary_only(),
        c.content_changed,
        c.base_mode != c.target_mode || c.base_link != c.target_link,
        c.delta().unsigned_abs(),
        c.target_size.max(c.base_size),
    )
}

fn changed_sort_key(c: &FileChange) -> (u64, bool, bool, bool, u64) {
    (
        c.delta().unsigned_abs(),
        !c.is_expected_binary_only(),
        c.content_changed,
        c.base_mode != c.target_mode || c.base_link != c.target_link,
        c.target_size.max(c.base_size),
    )
}

#[derive(Clone, Copy)]
struct HashCandidate<'a> {
    path: &'a str,
    base: &'a FinalFile,
    target: &'a FinalFile,
}

fn shared_layer_ordinals(
    file_layers: &[(usize, &ImageLayer)],
    shared: usize,
) -> HashMap<usize, usize> {
    let mut ordinals = HashMap::with_capacity(shared);
    for (ord, (layer_no, _)) in file_layers.iter().take(shared).enumerate() {
        ordinals.insert(*layer_no - 1, ord);
    }
    ordinals
}

/// Compare `target` against `base` using default [`DiffOptions`] without opening layer blobs.
pub fn diff_images(base: &DockerAnalyzeResult, target: &DockerAnalyzeResult) -> ImageDiff {
    diff_images_with_options(base, target, &DiffOptions::default())
}

/// Compare `target` against `base` using explicit [`DiffOptions`] without opening layer blobs.
pub fn diff_images_with_options(
    base: &DockerAnalyzeResult,
    target: &DockerAnalyzeResult,
    opts: &DiffOptions,
) -> ImageDiff {
    diff_images_with_opener(base, target, opts, false, |_, _, _| None)
}

/// Compare `target` against `base` and hash common equal-size regular files
/// via `hash_layer(side_is_target, layer, target_paths)`.
pub(crate) fn diff_images_with_opener<F>(
    base: &DockerAnalyzeResult,
    target: &DockerAnalyzeResult,
    opts: &DiffOptions,
    blobs_available: bool,
    mut hash_layer: F,
) -> ImageDiff
where
    F: FnMut(bool, &ImageLayer, &HashSet<String>) -> Option<HashMap<String, String>>,
{
    let base_layers_all = file_layers(&base.layers);
    let target_layers_all = file_layers(&target.layers);
    let part = partition_layers(
        base,
        target,
        &base_layers_all,
        &target_layers_all,
        opts.base_layer_mode,
    );

    let base_tree_start = tree_start_for_skipped(&base.layers, part.base_skip);
    let target_tree_start = tree_start_for_skipped(&target.layers, part.target_skip);
    let base_fs = scoped_filesystem(base, base_tree_start);
    let target_fs = scoped_filesystem(target, target_tree_start);

    let base_shared_ord = shared_layer_ordinals(&base_layers_all, part.shared);
    let target_shared_ord = shared_layer_ordinals(&target_layers_all, part.shared);

    let mut added = vec![];
    let mut changed = vec![];
    let mut hash_candidates = vec![];
    let mut skipped_by_cap = 0usize;
    let mut dirs: BTreeMap<&str, (u64, u64)> = BTreeMap::new();

    for (path, file) in &target_fs {
        dirs.entry(dir_bucket(path)).or_default().1 += file.size;
        match base_fs.get(path) {
            None => added.push(file_change(path, None, Some(file), false)),
            Some(old)
                if old.size != file.size || old.mode != file.mode || old.link != file.link =>
            {
                changed.push(file_change(path, Some(old), Some(file), false));
            }
            Some(old) => {
                let in_same_shared = matches!(
                    (base_shared_ord.get(&old.layer), target_shared_ord.get(&file.layer)),
                    (Some(a), Some(b)) if a == b
                );
                if !in_same_shared
                    && old.size > 0
                    && old.link.is_empty()
                    && file.link.is_empty()
                    && !old.mode.starts_with('d')
                    && !old.mode.starts_with('l')
                {
                    if is_hashable_regular_file(old) && is_hashable_regular_file(file) {
                        hash_candidates.push(HashCandidate {
                            path: path.as_str(),
                            base: old,
                            target: file,
                        });
                    } else {
                        skipped_by_cap += 1;
                    }
                }
            }
        }
    }

    let mut removed = vec![];
    for (path, file) in &base_fs {
        dirs.entry(dir_bucket(path)).or_default().0 += file.size;
        if !target_fs.contains_key(path) {
            removed.push(file_change(path, Some(file), None, false));
        }
    }

    let mut content_verified = opts.verify_content && blobs_available;
    let mut content_verified_files = 0usize;
    let mut content_skipped_files = skipped_by_cap;

    if !opts.verify_content || !blobs_available {
        content_verified = false;
        content_skipped_files += hash_candidates.len();
    } else if !hash_candidates.is_empty() {
        let mut base_by_layer: BTreeMap<usize, HashSet<String>> = BTreeMap::new();
        let mut target_by_layer: BTreeMap<usize, HashSet<String>> = BTreeMap::new();
        for c in &hash_candidates {
            base_by_layer
                .entry(c.base.layer)
                .or_default()
                .insert(c.path.to_string());
            target_by_layer
                .entry(c.target.layer)
                .or_default()
                .insert(c.path.to_string());
        }

        let mut base_hashes: HashMap<(usize, String), String> = HashMap::new();
        let mut target_hashes: HashMap<(usize, String), String> = HashMap::new();
        for (layer_idx, paths) in base_by_layer {
            if let Some(layer) = base.layers.get(layer_idx) {
                if let Some(hashes) = hash_layer(false, layer, &paths) {
                    for (p, h) in hashes {
                        base_hashes.insert((layer_idx, p), h);
                    }
                } else {
                    content_verified = false;
                }
            } else {
                content_verified = false;
            }
        }
        for (layer_idx, paths) in target_by_layer {
            if let Some(layer) = target.layers.get(layer_idx) {
                if let Some(hashes) = hash_layer(true, layer, &paths) {
                    for (p, h) in hashes {
                        target_hashes.insert((layer_idx, p), h);
                    }
                } else {
                    content_verified = false;
                }
            } else {
                content_verified = false;
            }
        }

        for c in hash_candidates {
            match (
                base_hashes.get(&(c.base.layer, c.path.to_string())),
                target_hashes.get(&(c.target.layer, c.path.to_string())),
            ) {
                (Some(bh), Some(th)) => {
                    content_verified_files += 1;
                    if bh != th {
                        changed.push(file_change(c.path, Some(c.base), Some(c.target), true));
                    }
                }
                _ => {
                    content_verified = false;
                    content_skipped_files += 1;
                }
            }
        }
    }

    let mut dirs: Vec<DirChange> = dirs
        .into_iter()
        .filter(|(_, (b, t))| b != t)
        .map(|(path, (base_size, target_size))| DirChange {
            path: path.to_string(),
            base_size,
            target_size,
        })
        .collect();
    dirs.sort_by_key(|d| Reverse(d.delta().unsigned_abs()));
    dirs.truncate(DIFF_MAX_DIRS);

    let config_diff = diff_config(base, target);
    let runtime_diff = diff_runtime(base, target);
    let parity = build_parity_summary(
        added.len(),
        removed.len(),
        &changed,
        &config_diff,
        &runtime_diff,
    );

    let base_drop = if part.base_layers_ignored {
        part.base_skip
    } else {
        part.shared
    };
    let target_drop = if part.base_layers_ignored {
        part.target_skip
    } else {
        part.shared
    };
    let base_skipped_size: u64 = if part.base_layers_ignored {
        base_layers_all[..part.base_skip]
            .iter()
            .map(|(_, l)| l.unpack_size)
            .sum()
    } else {
        0
    };
    let target_skipped_size: u64 = if part.base_layers_ignored {
        target_layers_all[..part.target_skip]
            .iter()
            .map(|(_, l)| l.unpack_size)
            .sum()
    } else {
        0
    };
    let base_upper: Vec<DiffLayer> = base_layers_all[base_drop..]
        .iter()
        .map(diff_layer)
        .collect();
    let target_upper: Vec<DiffLayer> = target_layers_all[target_drop..]
        .iter()
        .map(diff_layer)
        .collect();
    let base_upper_size: u64 = base_upper.iter().map(|l| l.unpack_size).sum();
    let target_upper_size: u64 = target_upper.iter().map(|l| l.unpack_size).sum();
    let upper_size_delta = if part.base_layers_ignored || part.shared > 0 {
        target_upper_size as i64 - base_upper_size as i64
    } else {
        target.total_size as i64 - base.total_size as i64
    };

    let base_ids = rec_ids(&base.recommendations);
    let target_ids = rec_ids(&target.recommendations);
    ImageDiff {
        base: side(base, base_fs.len()),
        target: side(target, target_fs.len()),
        size_delta: target.total_size as i64 - base.total_size as i64,
        upper_size_delta,
        base_layers_ignored: part.base_layers_ignored,
        base_skip_reason: part.reason,
        base_skipped_layers: part.base_skip,
        target_skipped_layers: part.target_skip,
        base_skipped_size,
        target_skipped_size,
        shared_layers: part.shared,
        shared_size: base_layers_all[..part.shared]
            .iter()
            .map(|(_, l)| l.unpack_size)
            .sum(),
        base_layers: base_upper,
        target_layers: target_upper,
        added: change_set(added, |f| f.target_size, |f| f.target_size),
        removed: change_set(removed, |f| f.base_size, |f| f.base_size),
        changed: change_set(changed, changed_retention_key, changed_sort_key),
        dirs,
        config_diff,
        runtime_diff,
        parity,
        content_verified,
        content_verified_files,
        content_skipped_files,
        new_recommendations: target
            .recommendations
            .iter()
            .filter(|r| !base_ids.contains(r.id.as_str()))
            .cloned()
            .collect(),
        resolved_recommendations: base
            .recommendations
            .iter()
            .filter(|r| !target_ids.contains(r.id.as_str()))
            .cloned()
            .collect(),
    }
}

fn file_change(
    path: &str,
    base: Option<&FinalFile>,
    target: Option<&FinalFile>,
    content_changed: bool,
) -> FileChange {
    let ref_file = target.or(base);
    let is_binary = ref_file.is_some_and(|f| is_binary_file(path, &f.mode, &f.link))
        || base.is_some_and(|f| is_binary_file(path, &f.mode, &f.link));
    FileChange {
        path: path.to_string(),
        base_size: base.map_or(0, |f| f.size),
        target_size: target.map_or(0, |f| f.size),
        layer: ref_file.map_or(0, |f| f.layer + 1),
        base_mode: base.map_or_else(String::new, |f| f.mode.clone()),
        target_mode: target.map_or_else(String::new, |f| f.mode.clone()),
        base_link: base.map_or_else(String::new, |f| f.link.clone()),
        target_link: target.map_or_else(String::new, |f| f.link.clone()),
        is_binary,
        content_changed,
    }
}

/// `+12.0 MiB` / `-3.1 KiB` / `0 B`.
fn signed_bytes(delta: i64) -> String {
    let size = ByteSize(delta.unsigned_abs());
    match delta {
        d if d > 0 => format!("+{size}"),
        d if d < 0 => format!("-{size}"),
        _ => size.to_string(),
    }
}

fn signed(delta: i64) -> String {
    if delta > 0 {
        format!("+{delta}")
    } else {
        delta.to_string()
    }
}

/// Inline code that survives a table cell: whitespace runs (the tabs and
/// newlines of a `RUN` script) folded to one space, pipes escaped.
fn cell_code(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("`{}`", clip(&flat.replace('|', "\\|"), 120))
}

fn format_changed_tags(c: &FileChange, t: &dyn Fn(&str) -> String) -> String {
    let mut tags: Vec<String> = Vec::new();
    if c.is_binary {
        tags.push(t("diff.tag.binary"));
    }
    if c.content_changed {
        tags.push(t("diff.tag.content"));
    }
    if c.base_mode != c.target_mode {
        tags.push(format!(
            "{}: {}→{}",
            t("diff.tag.mode"),
            if c.base_mode.is_empty() {
                "-"
            } else {
                &c.base_mode
            },
            if c.target_mode.is_empty() {
                "-"
            } else {
                &c.target_mode
            }
        ));
    }
    if c.base_link != c.target_link {
        tags.push(format!(
            "{}: {}→{}",
            t("diff.tag.link"),
            if c.base_link.is_empty() {
                "-"
            } else {
                &c.base_link
            },
            if c.target_link.is_empty() {
                "-"
            } else {
                &c.target_link
            }
        ));
    }
    if tags.is_empty() {
        String::new()
    } else {
        format!(" (`{}`)", tags.join("`, `"))
    }
}

fn skip_reason_label(reason: BaseLayerSkipReason, t: &dyn Fn(&str) -> String) -> String {
    match reason {
        BaseLayerSkipReason::CrossArchCreatedGap => t("diff.skip.created_gap"),
        BaseLayerSkipReason::CrossArchFirstLayer => t("diff.skip.first_layer"),
        BaseLayerSkipReason::ForcedCreatedGap => t("diff.skip.forced_gap"),
        BaseLayerSkipReason::ForcedFirstLayer => t("diff.skip.forced_first"),
        BaseLayerSkipReason::ManualCount => t("diff.skip.manual"),
        BaseLayerSkipReason::None | BaseLayerSkipReason::SharedPrefix => String::new(),
    }
}

fn push_config_runtime_section(
    md: &mut String,
    diff: &ImageDiff,
    lang: Lang,
    t: &dyn Fn(&str) -> String,
) {
    if diff.config_diff.same && diff.runtime_diff.same {
        return;
    }
    let mut rows: Vec<(String, String, String)> = Vec::new();
    let cd = &diff.config_diff;
    let show_opt = |s: &str| {
        if s.is_empty() {
            "—".to_string()
        } else {
            cell_code(s)
        }
    };
    let show_cmd = |v: &[String]| {
        if v.is_empty() {
            "—".to_string()
        } else {
            cell_code(&v.join(" "))
        }
    };
    if let Some(c) = &cd.base_os {
        rows.push((
            "Base OS".to_string(),
            show_opt(&c.base),
            show_opt(&c.target),
        ));
    }
    if let Some(c) = &cd.user {
        rows.push(("USER".to_string(), show_opt(&c.base), show_opt(&c.target)));
    }
    if let Some(c) = &cd.working_dir {
        rows.push((
            "WORKDIR".to_string(),
            show_opt(&c.base),
            show_opt(&c.target),
        ));
    }
    if let Some(c) = &cd.entrypoint {
        rows.push((
            "ENTRYPOINT".to_string(),
            show_cmd(&c.base),
            show_cmd(&c.target),
        ));
    }
    if let Some(c) = &cd.cmd {
        rows.push(("CMD".to_string(), show_cmd(&c.base), show_cmd(&c.target)));
    }
    if let Some(c) = &cd.has_healthcheck {
        rows.push((
            "HEALTHCHECK".to_string(),
            c.base.to_string(),
            c.target.to_string(),
        ));
    }
    if !cd.added_ports.is_empty() || !cd.removed_ports.is_empty() {
        rows.push((
            "EXPOSE".to_string(),
            if cd.removed_ports.is_empty() {
                "—".to_string()
            } else {
                cell_code(&cd.removed_ports.join(", "))
            },
            if cd.added_ports.is_empty() {
                "—".to_string()
            } else {
                cell_code(&cd.added_ports.join(", "))
            },
        ));
    }
    for env in &cd.env_changes {
        rows.push((
            format!("ENV {}", env.key),
            env.base
                .as_deref()
                .map_or_else(|| "—".to_string(), cell_code),
            env.target
                .as_deref()
                .map_or_else(|| "—".to_string(), cell_code),
        ));
    }
    for label in &cd.label_changes {
        rows.push((
            format!("LABEL {}", label.key),
            label
                .base
                .as_deref()
                .map_or_else(|| "—".to_string(), cell_code),
            label
                .target
                .as_deref()
                .map_or_else(|| "—".to_string(), cell_code),
        ));
    }
    let rd = &diff.runtime_diff;
    if let Some(c) = &rd.entrypoint {
        rows.push((
            "Runtime entrypoint".to_string(),
            show_opt(&c.base),
            show_opt(&c.target),
        ));
    }
    if let Some(c) = &rd.libc {
        rows.push((
            "Runtime libc".to_string(),
            show_opt(&c.base),
            show_opt(&c.target),
        ));
    }
    if let Some(c) = &rd.required_glibc {
        rows.push((
            "Min glibc".to_string(),
            show_opt(&c.base),
            show_opt(&c.target),
        ));
    }
    if let Some(c) = &rd.os_glibc {
        rows.push((
            "OS glibc".to_string(),
            show_opt(&c.base),
            show_opt(&c.target),
        ));
    }
    if let Some(c) = &rd.issue {
        rows.push((
            "Runtime issue".to_string(),
            show_opt(&c.base),
            show_opt(&c.target),
        ));
    }
    if !rd.added_unstripped.is_empty() || !rd.removed_unstripped.is_empty() {
        rows.push((
            "Unstripped binaries".to_string(),
            rd.base_unstripped_count.to_string(),
            rd.target_unstripped_count.to_string(),
        ));
    }
    if rows.is_empty() {
        return;
    }
    md.push_str(&format!(
        "**{}**\n\n| {} | {} | {} |\n|---|---|---|\n",
        t("diff.config_runtime"),
        t("diff.col.item"),
        t("diff.col.base"),
        t("diff.col.target"),
    ));
    for (item, b, tgt) in rows.iter().take(MD_ROWS) {
        md.push_str(&format!("| {item} | {b} | {tgt} |\n"));
    }
    push_more(md, rows.len(), lang);
}

/// The diff as Markdown: headline table, parity verdict, layers, where the
/// size went, the largest file changes, config/runtime diffs, and the
/// recommendation changes. `verdict` is the CI outcome, when there is one.
pub fn to_diff_markdown(diff: &ImageDiff, lang: Lang, verdict: Option<&str>) -> String {
    let t = |k: &str| -> String { i18n::tr(lang, k).to_string() };
    let f = |k: &str, args: &[&str]| -> String { i18n::fill(i18n::tr(lang, k), args) };
    let (base, target) = (&diff.base, &diff.target);
    let mut md = format!("### diving diff · `{}` → `{}`\n\n", base.name, target.name);
    if let Some(verdict) = verdict {
        md.push_str(&format!("{verdict}\n\n"));
    }
    for side in [base, target] {
        if let Some(as_of) = &side.stale_as_of {
            md.push_str(&format!(
                "> ⚠️ `{}`: {}\n\n",
                side.name,
                f("md.stale", &[as_of])
            ));
        }
    }

    let pct = if base.total_size > 0 {
        format!(
            " ({:+.1} %)",
            diff.size_delta as f64 / base.total_size as f64 * 100.0
        )
    } else {
        String::new()
    };
    md.push_str(&format!(
        "| | {} | {} | {} |\n|---|---:|---:|---:|\n",
        t("diff.col.base"),
        t("diff.col.target"),
        t("diff.col.change"),
    ));
    let mut rows: Vec<(String, String, String, String)> = vec![(
        t("sum.size"),
        ByteSize(base.total_size).to_string(),
        ByteSize(target.total_size).to_string(),
        format!("**{}**{pct}", signed_bytes(diff.size_delta)),
    )];
    if diff.base_layers_ignored {
        let base_upper = diff.base_layers.iter().map(|l| l.unpack_size).sum();
        let target_upper = diff.target_layers.iter().map(|l| l.unpack_size).sum();
        rows.push((
            t("diff.upper_size"),
            ByteSize(base_upper).to_string(),
            ByteSize(target_upper).to_string(),
            format!("**{}**", signed_bytes(diff.upper_size_delta)),
        ));
    }
    rows.extend([
        (
            t("diff.compressed"),
            ByteSize(base.size).to_string(),
            ByteSize(target.size).to_string(),
            signed_bytes(target.size as i64 - base.size as i64),
        ),
        (
            t("sum.layers"),
            base.layer_count.to_string(),
            target.layer_count.to_string(),
            signed(target.layer_count as i64 - base.layer_count as i64),
        ),
        (
            t("md.f.eff"),
            format!("{} %", base.efficiency_score),
            format!("{} %", target.efficiency_score),
            signed(target.efficiency_score as i64 - base.efficiency_score as i64),
        ),
        (
            t("md.f.wasted"),
            ByteSize(base.wasted_size).to_string(),
            ByteSize(target.wasted_size).to_string(),
            signed_bytes(target.wasted_size as i64 - base.wasted_size as i64),
        ),
        (
            t("diff.files"),
            base.file_count.to_string(),
            target.file_count.to_string(),
            signed(target.file_count as i64 - base.file_count as i64),
        ),
    ]);
    for (label, b, tgt, change) in rows {
        md.push_str(&format!("| {label} | {b} | {tgt} | {change} |\n"));
    }
    md.push('\n');

    let parity_line = match diff.parity.status {
        ParityStatus::Consistent => t("diff.parity.consistent"),
        ParityStatus::BinaryDiffsOnly => f(
            "diff.parity.binary_only",
            &[&diff.parity.expected_binary_changes.to_string()],
        ),
        ParityStatus::ConfigOrFileDrift => f(
            "diff.parity.drift",
            &[
                &diff.parity.unexpected_file_changes.to_string(),
                &diff.parity.config_changes.to_string(),
                &diff.parity.runtime_changes.to_string(),
                &diff.parity.expected_binary_changes.to_string(),
            ],
        ),
    };
    md.push_str(&format!("{parity_line}\n\n"));

    if diff.content_verified_files > 0 || diff.content_skipped_files > 0 {
        let content_line = if diff.content_verified {
            f(
                "diff.content.verified",
                &[
                    &diff.content_verified_files.to_string(),
                    &diff.content_skipped_files.to_string(),
                ],
            )
        } else {
            f(
                "diff.content.unverified",
                &[&diff.content_skipped_files.to_string()],
            )
        };
        md.push_str(&format!("_{content_line}_\n\n"));
    }

    if diff.base_layers_ignored {
        let reason = skip_reason_label(diff.base_skip_reason, &t);
        md.push_str(&f(
            "diff.skipped",
            &[
                &diff.base_skipped_layers.to_string(),
                &diff.target_skipped_layers.to_string(),
                &ByteSize(diff.base_skipped_size).to_string(),
                &ByteSize(diff.target_skipped_size).to_string(),
                &reason,
            ],
        ));
    } else if diff.shared_layers > 0 {
        md.push_str(&f(
            "diff.shared",
            &[
                &diff.shared_layers.to_string(),
                &ByteSize(diff.shared_size).to_string(),
            ],
        ));
    } else {
        md.push_str(&t("diff.noshared"));
    }
    md.push_str("\n\n");

    push_config_runtime_section(&mut md, diff, lang, &t);

    for (key, layers) in [
        ("diff.layers.base", &diff.base_layers),
        ("diff.layers.target", &diff.target_layers),
    ] {
        if layers.is_empty() {
            continue;
        }
        md.push_str(&format!(
            "**{}**\n\n| {} | {} | {} |\n|---:|---:|---|\n",
            f(key, &[&layers.len().to_string()]),
            t("sum.col.layer"),
            t("sum.col.size"),
            t("md.command"),
        ));
        for l in layers.iter().take(MD_ROWS) {
            md.push_str(&format!(
                "| {} | {} | {} |\n",
                l.layer,
                ByteSize(l.unpack_size),
                cell_code(&l.cmd)
            ));
        }
        push_more(&mut md, layers.len(), lang);
    }

    if !diff.dirs.is_empty() {
        md.push_str(&format!(
            "**{}**\n\n| {} | {} | {} | {} |\n|---:|---:|---:|---|\n",
            t("diff.dirs"),
            t("diff.col.change"),
            t("diff.col.base"),
            t("diff.col.target"),
            t("diff.col.dir"),
        ));
        for d in diff.dirs.iter().take(MD_ROWS) {
            md.push_str(&format!(
                "| {} | {} | {} | `/{}` |\n",
                signed_bytes(d.delta()),
                ByteSize(d.base_size),
                ByteSize(d.target_size),
                d.path
            ));
        }
        md.push('\n');
    }

    for (key, set, size_of) in [
        (
            "diff.added",
            &diff.added,
            (|c| c.target_size) as fn(&FileChange) -> u64,
        ),
        ("diff.removed", &diff.removed, |c| c.base_size),
    ] {
        if set.count == 0 {
            continue;
        }
        md.push_str(&format!(
            "**{}**\n\n| {} | {} | {} |\n|---:|---:|---|\n",
            f(key, &[&set.count.to_string(), &signed_bytes(set.bytes)]),
            t("sum.col.size"),
            t("sum.col.layer"),
            t("sum.col.path"),
        ));
        for c in set.files.iter().take(MD_ROWS) {
            md.push_str(&format!(
                "| {} | {} | `{}` |\n",
                ByteSize(size_of(c)),
                c.layer,
                c.path
            ));
        }
        push_more(&mut md, set.count, lang);
    }
    if diff.changed.count > 0 {
        md.push_str(&format!(
            "**{}**\n\n| {} | {} | {} | {} |\n|---:|---:|---:|---|\n",
            f(
                "diff.changed",
                &[
                    &diff.changed.count.to_string(),
                    &signed_bytes(diff.changed.bytes)
                ]
            ),
            t("diff.col.change"),
            t("diff.col.base"),
            t("diff.col.target"),
            t("sum.col.path"),
        ));
        for c in diff.changed.files.iter().take(MD_ROWS) {
            md.push_str(&format!(
                "| {} | {} | {} | `{}`{} |\n",
                signed_bytes(c.delta()),
                ByteSize(c.base_size),
                ByteSize(c.target_size),
                c.path,
                format_changed_tags(c, &t),
            ));
        }
        push_more(&mut md, diff.changed.count, lang);
    }

    for (key, recs) in [
        ("diff.recs.new", &diff.new_recommendations),
        ("diff.recs.resolved", &diff.resolved_recommendations),
    ] {
        if recs.is_empty() {
            continue;
        }
        md.push_str(&format!("**{}**\n\n", f(key, &[&recs.len().to_string()])));
        for r in recs {
            md.push_str(&format!(
                "- {} **{}** · {} (`{}`)\n",
                icon(&r.severity),
                r.title,
                t(&format!("cat.{}", r.category)),
                r.id
            ));
        }
        md.push('\n');
    }
    md
}

/// Close a table, noting the rows it left out.
fn push_more(md: &mut String, total: usize, lang: Lang) {
    if total > MD_ROWS {
        md.push_str(&format!(
            "\n_{}_\n",
            i18n::fill(
                i18n::tr(lang, "diff.more"),
                &[&(total - MD_ROWS).to_string()]
            )
        ));
    }
    md.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::{FileTreeItem, Op};
    use crate::UnstrippedBinary;

    fn leaf(name: &str, size: u64) -> FileTreeItem {
        FileTreeItem {
            name: name.to_string(),
            size,
            ..Default::default()
        }
    }

    fn node(name: &str, children: Vec<FileTreeItem>) -> FileTreeItem {
        FileTreeItem {
            name: name.to_string(),
            size: children.iter().map(|c| c.size).sum(),
            children,
            ..Default::default()
        }
    }

    fn layer(digest: &str, unpack_size: u64) -> ImageLayer {
        ImageLayer {
            digest: digest.to_string(),
            cmd: format!("RUN build {digest}"),
            unpack_size,
            size: unpack_size / 2,
            ..Default::default()
        }
    }

    fn rec(id: &str) -> Recommendation {
        Recommendation {
            id: id.to_string(),
            category: "size".to_string(),
            severity: "medium".to_string(),
            title: format!("{id} title"),
            ..Default::default()
        }
    }

    fn image(
        name: &str,
        layers: Vec<ImageLayer>,
        trees: Vec<Vec<FileTreeItem>>,
        recs: Vec<Recommendation>,
    ) -> DockerAnalyzeResult {
        DockerAnalyzeResult {
            name: name.to_string(),
            total_size: layers.iter().map(|l| l.unpack_size).sum(),
            size: layers.iter().map(|l| l.size).sum(),
            layers,
            file_tree_list: trees,
            recommendations: recs,
            ..Default::default()
        }
    }

    fn base_os() -> Vec<FileTreeItem> {
        vec![node(
            "usr",
            vec![node(
                "lib",
                vec![leaf("libc.so", 1000), leaf("libz.so", 100)],
            )],
        )]
    }

    fn sample() -> ImageDiff {
        let env = ImageLayer {
            empty: true,
            ..Default::default()
        };
        let base = image(
            "app:1",
            vec![
                layer("sha256:os", 1100),
                env.clone(),
                layer("sha256:app1", 600),
            ],
            vec![
                base_os(),
                vec![],
                vec![node(
                    "app",
                    vec![
                        leaf("server", 500),
                        leaf("old.txt", 100),
                        node("assets", vec![leaf("logo.png", 0)]),
                    ],
                )],
            ],
            vec![rec("runasroot"), rec("pkgcache")],
        );
        let target = image(
            "app:2",
            vec![layer("sha256:os", 1100), layer("sha256:app2", 5900), env],
            vec![
                base_os(),
                vec![
                    node(
                        "app",
                        vec![
                            leaf("server", 800),
                            node("node_modules", vec![node("big", vec![leaf("x.js", 5000)])]),
                            node("assets", vec![leaf("logo.png", 0)]),
                        ],
                    ),
                    node("usr", vec![node("lib", vec![leaf("libz.so", 0)])]),
                    // a whiteout, so it never shows up as added
                    FileTreeItem {
                        op: Op::Removed,
                        ..leaf("tmp", 0)
                    },
                ],
                vec![],
            ],
            vec![rec("runasroot"), rec("unstripped")],
        );
        diff_images(&base, &target)
    }

    #[test]
    fn layers_match_by_diff_id_then_by_content_digest() {
        let with = |digest: &str, diff_id: &str| ImageLayer {
            digest: digest.to_string(),
            diff_id: diff_id.to_string(),
            ..Default::default()
        };
        // docker save paths say nothing; the diff ids decide
        assert!(same_layer(
            &with("layer1/layer.tar", "sha256:aa"),
            &with("blobs/sha256/aa", "sha256:aa")
        ));
        assert!(!same_layer(
            &with("layer1/layer.tar", "sha256:aa"),
            &with("layer1/layer.tar", "sha256:bb")
        ));
        // a cache from before diff ids were recorded: registry digests
        assert!(same_layer(
            &with("sha256:cc", ""),
            &with("sha256:cc", "sha256:x")
        ));
        assert!(!same_layer(
            &with("layer1/layer.tar", ""),
            &with("layer1/layer.tar", "")
        ));
    }

    #[test]
    fn dir_bucket_cuts_to_the_parent_at_most_three_deep() {
        assert_eq!(dir_bucket("README"), "");
        assert_eq!(dir_bucket("app/server"), "app");
        assert_eq!(dir_bucket("usr/local/lib/libpython.so"), "usr/local/lib");
        assert_eq!(
            dir_bucket("app/node_modules/big/lib/x.js"),
            "app/node_modules/big"
        );
    }

    #[test]
    fn diff_aligns_layers_and_compares_final_files() {
        let diff = sample();
        assert_eq!(diff.size_delta, 7000 - 1700);
        assert_eq!(diff.shared_layers, 1);
        assert_eq!(diff.shared_size, 1100);
        // Layer numbers are each image's own (the base has an ENV layer
        // in between).
        assert_eq!(diff.base_layers[0].layer, 3);
        assert_eq!(diff.target_layers[0].layer, 2);
        assert_eq!((diff.base.layer_count, diff.target.layer_count), (2, 2));

        assert_eq!(diff.added.count, 1);
        assert_eq!(diff.added.files[0].path, "app/node_modules/big/x.js");
        assert_eq!(diff.added.files[0].layer, 2);
        assert_eq!(diff.added.bytes, 5000);
        assert_eq!(diff.removed.count, 1);
        assert_eq!(diff.removed.files[0].path, "app/old.txt");
        assert_eq!(diff.removed.files[0].layer, 3);
        assert_eq!(diff.removed.bytes, -100);
        // server grew, libz.so was overwritten with an empty copy; the
        // unchanged zero-byte logo is not a change.
        let changed: Vec<(&str, i64)> = diff
            .changed
            .files
            .iter()
            .map(|c| (c.path.as_str(), c.delta()))
            .collect();
        assert_eq!(changed, [("app/server", 300), ("usr/lib/libz.so", -100)]);
        assert_eq!(diff.changed.bytes, 200);

        let dirs: Vec<(&str, i64)> = diff
            .dirs
            .iter()
            .map(|d| (d.path.as_str(), d.delta()))
            .collect();
        assert_eq!(
            dirs,
            [
                ("app/node_modules/big", 5000),
                ("app", 200),
                ("usr/lib", -100)
            ]
        );

        let ids = |recs: &[Recommendation]| recs.iter().map(|r| r.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&diff.new_recommendations), ["unstripped"]);
        assert_eq!(ids(&diff.resolved_recommendations), ["pkgcache"]);
    }

    #[test]
    fn diff_markdown_lists_the_changes() {
        let md = to_diff_markdown(&sample(), Lang::En, Some("**PASS**"));
        assert!(md.starts_with("### diving diff · `app:1` → `app:2`\n\n**PASS**"));
        assert!(md.contains("| Image size | 1.7 KiB | 6.8 KiB | **+5.2 KiB** (+311.8 %) |"));
        assert!(md.contains("the first 1 (1.1 KiB) are identical"));
        assert!(md.contains("`RUN build sha256:app2`"));
        assert!(md.contains("| +4.9 KiB | 0 B | 4.9 KiB | `/app/node_modules/big` |"));
        assert!(md.contains("`app/node_modules/big/x.js`"));
        assert!(md.contains("`app/old.txt`"));
        assert!(md.contains("(`unstripped`)"));
        assert!(md.contains("(`pkgcache`)"));
        let zh = to_diff_markdown(&sample(), Lang::Zh, None);
        assert!(zh.contains("新增文件"));
    }

    #[test]
    fn identical_images_have_no_changes() {
        let a = image(
            "a",
            vec![layer("sha256:os", 1100)],
            vec![base_os()],
            vec![rec("runasroot")],
        );
        let diff = diff_images(&a, &a);
        assert_eq!(diff.size_delta, 0);
        assert_eq!(diff.shared_layers, 1);
        assert!(diff.base_layers.is_empty() && diff.target_layers.is_empty());
        assert_eq!(
            (diff.added.count, diff.removed.count, diff.changed.count),
            (0, 0, 0)
        );
        assert!(diff.dirs.is_empty());
        assert!(diff.new_recommendations.is_empty());
    }

    #[test]
    fn cell_code_keeps_a_table_row_intact() {
        assert_eq!(cell_code("RUN a |\tb\n\t\tc"), "`RUN a \\| b c`");
    }

    #[test]
    fn cross_arch_auto_skips_by_created_gap_and_falls_back_to_first_layer() {
        let mut l0 = layer("sha256:os-amd", 4000);
        l0.created = "2024-01-01T00:00:00Z".into();
        let mut l1 = layer("sha256:pkg-amd", 1000);
        l1.created = "2024-01-01T00:05:00Z".into();
        let mut l2 = layer("sha256:app-amd", 500);
        l2.created = "2024-06-01T00:00:00Z".into();

        let mut base = image(
            "app:amd64",
            vec![l0, l1, l2],
            vec![
                vec![node("lib", vec![leaf("libc.so.6", 4000)])],
                vec![node("usr", vec![leaf("pkg.dat", 1000)])],
                vec![node(
                    "app",
                    vec![FileTreeItem {
                        mode: "-rwxr-xr-x".into(),
                        ..leaf("server", 500)
                    }],
                )],
            ],
            vec![],
        );
        base.arch = "amd64".into();
        base.base_os = "Debian 12".into();

        let mut t0 = layer("sha256:os-arm", 4200);
        t0.created = "2024-01-01T00:00:00Z".into();
        let mut t1 = layer("sha256:pkg-arm", 1100);
        t1.created = "2024-01-01T00:05:00Z".into();
        let mut t2 = layer("sha256:app-arm", 620);
        t2.created = "2024-06-01T00:00:00Z".into();

        let mut target = image(
            "app:arm64",
            vec![t0, t1, t2],
            vec![
                vec![node("lib", vec![leaf("libc.so.6", 4200)])],
                vec![node("usr", vec![leaf("pkg.dat", 1100)])],
                vec![node(
                    "app",
                    vec![FileTreeItem {
                        mode: "-rwxr-xr-x".into(),
                        ..leaf("server", 620)
                    }],
                )],
            ],
            vec![],
        );
        target.arch = "arm64".into();
        target.base_os = "Debian 12".into();

        let diff = diff_images(&base, &target);
        assert!(diff.base_layers_ignored);
        assert_eq!(
            diff.base_skip_reason,
            BaseLayerSkipReason::CrossArchCreatedGap
        );
        assert_eq!(
            (diff.base_skipped_layers, diff.target_skipped_layers),
            (2, 2)
        );
        assert_eq!(
            (diff.base_skipped_size, diff.target_skipped_size),
            (5000, 5300)
        );
        assert_eq!(diff.upper_size_delta, 120);
        assert_eq!(diff.size_delta, 420);
        assert_eq!(diff.changed.count, 1);
        assert_eq!(diff.changed.files[0].path, "app/server");
        assert!(diff.changed.files[0].is_expected_binary_only());
        assert_eq!(diff.parity.status, ParityStatus::BinaryDiffsOnly);
        assert_eq!(diff.parity.expected_binary_changes, 1);
        assert_eq!(diff.parity.unexpected_file_changes, 0);

        // No timestamps -> fallback to skipping 1st non-empty rootfs layer
        for l in &mut base.layers {
            l.created.clear();
        }
        for l in &mut target.layers {
            l.created.clear();
        }
        let fallback = diff_images(&base, &target);
        assert!(fallback.base_layers_ignored);
        assert_eq!(
            fallback.base_skip_reason,
            BaseLayerSkipReason::CrossArchFirstLayer
        );
        assert_eq!(
            (fallback.base_skipped_layers, fallback.target_skipped_layers),
            (1, 1)
        );
        assert!(fallback
            .changed
            .files
            .iter()
            .any(|c| c.path == "usr/pkg.dat"));
        assert!(!fallback
            .changed
            .files
            .iter()
            .any(|c| c.path == "lib/libc.so.6"));

        // Epoch-0 flattened timestamps must NOT look like a >1h gap vs build gap:
        for l in &mut base.layers {
            l.created = "1970-01-01T00:00:00Z".into();
        }
        base.layers[2].created = "2024-06-01T00:00:00Z".into();
        let epoch_check = diff_images(&base, &target);
        assert_eq!(
            epoch_check.base_skip_reason,
            BaseLayerSkipReason::CrossArchFirstLayer
        );

        // Manual Skip clamps to layer count and Off disables base skipping
        let manual = diff_images_with_options(
            &base,
            &target,
            &DiffOptions::default().base_layer_mode(BaseLayerMode::Skip(99)),
        );
        assert_eq!(manual.base_skip_reason, BaseLayerSkipReason::ManualCount);
        assert_eq!(
            (manual.base_skipped_layers, manual.target_skipped_layers),
            (3, 3)
        );
        assert!(manual.base_layers.is_empty());

        let off = diff_images_with_options(
            &base,
            &target,
            &DiffOptions::default().base_layer_mode(BaseLayerMode::Off),
        );
        assert!(!off.base_layers_ignored);
        assert!(off.changed.files.iter().any(|c| c.path == "lib/libc.so.6"));
    }

    #[test]
    fn parity_detects_mode_link_same_size_content_config_and_runtime_drift() {
        let mut base = image(
            "svc:1",
            vec![layer("sha256:a", 100)],
            vec![vec![
                FileTreeItem {
                    mode: "-rw-r--r--".into(),
                    ..leaf("cfg.json", 12)
                },
                FileTreeItem {
                    mode: "-rwxr-xr-x".into(),
                    ..leaf("perm.sh", 10)
                },
                FileTreeItem {
                    mode: "lrwxrwxrwx".into(),
                    link: "/usr/bin/a".into(),
                    ..leaf("cur", 0)
                },
            ]],
            vec![],
        );
        base.user = "app".into();
        base.working_dir = "/app".into();
        base.entrypoint = vec!["/srv".into()];
        base.cmd = vec!["--fast".into()];
        base.has_healthcheck = true;
        base.exposed_ports = vec!["8080/tcp".into()];
        base.envs = vec!["A=1".into(), "DROP=x".into()];
        base.labels = vec!["v=1".into()];
        base.runtime_compat.libc = "gnu".into();
        base.unstripped_binaries = vec![UnstrippedBinary {
            path: "bin/a".into(),
            ..Default::default()
        }];

        let mut target = image(
            "svc:2",
            vec![layer("sha256:b", 100)],
            vec![vec![
                FileTreeItem {
                    mode: "-rw-r--r--".into(),
                    ..leaf("cfg.json", 12)
                },
                FileTreeItem {
                    mode: "-rw-r--r--".into(),
                    ..leaf("perm.sh", 10)
                },
                FileTreeItem {
                    mode: "lrwxrwxrwx".into(),
                    link: "/usr/bin/b".into(),
                    ..leaf("cur", 0)
                },
            ]],
            vec![],
        );
        target.user = "root".into();
        target.working_dir = "/srv".into();
        target.entrypoint = vec!["/srv2".into()];
        target.cmd = vec![];
        target.has_healthcheck = false;
        target.exposed_ports = vec!["8080/tcp".into(), "9090/tcp".into()];
        target.envs = vec!["A=2".into(), "ADD=y".into()];
        target.labels = vec!["v=1".into()];
        target.runtime_compat.libc = "musl".into();
        target.unstripped_binaries = vec![UnstrippedBinary {
            path: "bin/b".into(),
            ..Default::default()
        }];

        let diff = diff_images_with_opener(
            &base,
            &target,
            &DiffOptions::default(),
            true,
            |is_target, _, _| {
                let mut map = HashMap::new();
                map.insert(
                    "cfg.json".to_string(),
                    if is_target { "hash-b" } else { "hash-a" }.to_string(),
                );
                Some(map)
            },
        );
        assert!(diff.content_verified);
        assert_eq!(diff.content_verified_files, 1);
        assert_eq!(diff.changed.count, 3);
        assert_eq!(diff.parity.status, ParityStatus::ConfigOrFileDrift);
        assert_eq!(diff.parity.mode_changes, 1);
        assert_eq!(diff.parity.link_changes, 1);
        assert_eq!(diff.parity.same_size_content_changes, 1);
        assert!(!diff.config_diff.same);
        assert_eq!(diff.config_diff.added_ports, ["9090/tcp"]);
        assert_eq!(diff.config_diff.env_changes.len(), 3);
        assert!(!diff.runtime_diff.same);
        assert_eq!(diff.runtime_diff.added_unstripped, ["bin/b"]);
        assert_eq!(diff.runtime_diff.removed_unstripped, ["bin/a"]);

        let md = to_diff_markdown(&diff, Lang::En, None);
        assert!(md.contains("Config & runtime differences"));
        assert!(md.contains("Content hash check: verified 1"));
    }

    #[test]
    fn changed_retains_same_size_content_changed_behind_many_resized_files() {
        let mut base_items = Vec::new();
        let mut target_items = Vec::new();
        for i in 0..60 {
            let name = format!("bin-{i:02}.so");
            base_items.push(FileTreeItem {
                mode: "-rwxr-xr-x".into(),
                ..leaf(&name, 1000 + i as u64 * 10)
            });
            target_items.push(FileTreeItem {
                mode: "-rwxr-xr-x".into(),
                ..leaf(&name, 2000 + i as u64 * 20)
            });
        }
        // Add >50 unexpected non-binary size changes too so both cases are exercised.
        for i in 0..10 {
            let name = format!("doc-{i:02}.txt");
            base_items.push(FileTreeItem {
                mode: "-rw-r--r--".into(),
                ..leaf(&name, 100 + i as u64)
            });
            target_items.push(FileTreeItem {
                mode: "-rw-r--r--".into(),
                ..leaf(&name, 300 + i as u64)
            });
        }
        base_items.push(FileTreeItem {
            mode: "-rw-r--r--".into(),
            ..leaf("app.json", 42)
        });
        target_items.push(FileTreeItem {
            mode: "-rw-r--r--".into(),
            ..leaf("app.json", 42)
        });

        let base = image(
            "app:1",
            vec![layer("sha256:a", 100)],
            vec![base_items],
            vec![],
        );
        let target = image(
            "app:2",
            vec![layer("sha256:b", 100)],
            vec![target_items],
            vec![],
        );

        let diff = diff_images_with_opener(
            &base,
            &target,
            &DiffOptions::default(),
            true,
            |is_target, _, _| {
                let mut map = HashMap::new();
                map.insert(
                    "app.json".to_string(),
                    if is_target { "new" } else { "old" }.to_string(),
                );
                Some(map)
            },
        );
        assert_eq!(diff.changed.count, 71);
        assert_eq!(diff.changed.files.len(), DIFF_MAX_FILES);
        assert_eq!(diff.parity.same_size_content_changes, 1);
        assert!(diff
            .changed
            .files
            .iter()
            .any(|c| c.path == "app.json" && c.content_changed));

        // Even when >50 unexpected non-binary size changes exist, same-size content_changed survives.
        let mut base_docs = Vec::new();
        let mut target_docs = Vec::new();
        for i in 0..60 {
            let name = format!("note-{i:02}.txt");
            base_docs.push(FileTreeItem {
                mode: "-rw-r--r--".into(),
                ..leaf(&name, 100)
            });
            target_docs.push(FileTreeItem {
                mode: "-rw-r--r--".into(),
                ..leaf(&name, 500 + i as u64)
            });
        }
        base_docs.push(FileTreeItem {
            mode: "-rw-r--r--".into(),
            ..leaf("secret.yaml", 32)
        });
        target_docs.push(FileTreeItem {
            mode: "-rw-r--r--".into(),
            ..leaf("secret.yaml", 32)
        });
        let b2 = image(
            "docs:1",
            vec![layer("sha256:a", 100)],
            vec![base_docs],
            vec![],
        );
        let t2 = image(
            "docs:2",
            vec![layer("sha256:b", 100)],
            vec![target_docs],
            vec![],
        );
        let diff2 = diff_images_with_opener(
            &b2,
            &t2,
            &DiffOptions::default(),
            true,
            |is_target, _, _| {
                let mut map = HashMap::new();
                map.insert(
                    "secret.yaml".to_string(),
                    if is_target { "b" } else { "a" }.to_string(),
                );
                Some(map)
            },
        );
        assert_eq!(diff2.changed.count, 61);
        assert_eq!(diff2.changed.files.len(), DIFF_MAX_FILES);
        assert!(diff2
            .changed
            .files
            .iter()
            .any(|c| c.path == "secret.yaml" && c.content_changed));

        // Deterministic tie-breaking by path ascending.
        let diff2_repeat = diff_images_with_opener(
            &b2,
            &t2,
            &DiffOptions::default(),
            true,
            |is_target, _, _| {
                let mut map = HashMap::new();
                map.insert(
                    "secret.yaml".to_string(),
                    if is_target { "b" } else { "a" }.to_string(),
                );
                Some(map)
            },
        );
        assert_eq!(diff2.changed.files, diff2_repeat.changed.files);
    }
}
