//! Pure domain computations for layers, virtual file trees, previews, and history.

use std::collections::HashSet;
use std::ops::Deref;
use std::sync::Arc;

use bytesize::ByteSize;
pub use diving_core::{
    ArchiveEntryPreview, ArchivePreviewMeta, ImagePreviewMeta, MediaPreviewMeta, PreviewField,
    StructuredPreviewMeta, StructuredSection,
};
use diving_core::{
    DockerAnalyzeResult, DockerAnalyzeSummary, FileTreeItem, ImageDiff, ImageLayer,
    InspectedPreview, Op, REGISTRY_LOCAL_DOCKER, REGISTRY_LOCAL_FILE, REGISTRY_LOCAL_PODMAN,
    Recommendation, RuntimeCompat, detect_code_language as core_detect_code_language,
    encode_data_uri, format_archive_text, format_media_text, format_structured_text,
    inspect_file_preview, parse_image_info, registry_host,
};
use serde::{Deserialize, Serialize};

use crate::domain::shell_fmt::parse_layer_command;
use crate::i18n::{UiLang, t, tf};
use crate::ui::{BadgeTone, FileOpKind, LayerOpcodeKind};

/// Active top-level route inside the desktop shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppRoute {
    #[default]
    Landing,
    Inspector,
    Diff,
    #[cfg(debug_assertions)]
    Gallery,
}

/// Pointer-equality wrapper enabling `Signal<Option<SharedRef<T>>>` for external structs.
#[derive(Debug)]
pub struct SharedRef<T>(pub Arc<T>);

impl<T> Clone for SharedRef<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> PartialEq for SharedRef<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<T> Deref for SharedRef<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> SharedRef<T> {
    pub fn new(value: T) -> Self {
        Self(Arc::new(value))
    }
}

/// Active tab inside the Image Inspector workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InspectorTab {
    #[default]
    FileTree,
    Treemap,
    Findings,
    Dockerfile,
}

/// Display ordering for `LayerTimeline`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayerSortOrder {
    /// Upper layers first (`#N → #1`), matching `docker history`.
    #[default]
    Desc,
    /// Dockerfile build order (`#1 → #N`).
    Asc,
}

impl LayerSortOrder {
    pub fn as_storage_str(self) -> &'static str {
        match self {
            Self::Desc => "desc",
            Self::Asc => "asc",
        }
    }

    pub fn from_storage_str(raw: &str) -> Self {
        match raw.trim() {
            "asc" => Self::Asc,
            _ => Self::Desc,
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            Self::Desc => Self::Asc,
            Self::Asc => Self::Desc,
        }
    }
}

/// Filter modes for `LayerTimeline`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayerFilterMode {
    #[default]
    All,
    HasFiles,
    Large,
}

/// Unpack-size threshold (`1 MiB`) used by [`LayerFilterMode::Large`].
pub const LARGE_LAYER_BYTES: u64 = 1_048_576;

/// Whether a single image layer matches the active [`LayerFilterMode`].
pub fn layer_matches_filter(layer: &ImageLayer, mode: LayerFilterMode) -> bool {
    match mode {
        LayerFilterMode::All => true,
        LayerFilterMode::HasFiles => !layer.empty,
        LayerFilterMode::Large => layer.unpack_size >= LARGE_LAYER_BYTES,
    }
}

/// Selects the initial layer index according to the active [`LayerSortOrder`].
pub fn default_selected_layer(layers: &[ImageLayer], order: LayerSortOrder) -> usize {
    match order {
        LayerSortOrder::Desc => layers
            .iter()
            .rposition(|l| !l.empty)
            .unwrap_or_else(|| layers.len().saturating_sub(1)),
        LayerSortOrder::Asc => layers.iter().position(|l| !l.empty).unwrap_or(0),
    }
}

/// Returns the 0-based indices of layers visible under `mode` in `order` visual sequence.
pub fn visible_layer_indices(
    layers: &[ImageLayer],
    mode: LayerFilterMode,
    order: LayerSortOrder,
) -> Vec<usize> {
    let mut indices: Vec<usize> = layers
        .iter()
        .enumerate()
        .filter(|(_, layer)| layer_matches_filter(layer, mode))
        .map(|(idx, _)| idx)
        .collect();
    if order == LayerSortOrder::Desc {
        indices.reverse();
    }
    indices
}

/// Re-computes localized recommendation diffs on an existing [`ImageDiff`] after language switch.
pub fn relocalize_image_diff(
    diff: &ImageDiff,
    base_recs: &[Recommendation],
    target_recs: &[Recommendation],
) -> ImageDiff {
    let base_ids: HashSet<&str> = base_recs.iter().map(|r| r.id.as_str()).collect();
    let target_ids: HashSet<&str> = target_recs.iter().map(|r| r.id.as_str()).collect();
    let mut updated = diff.clone();
    updated.new_recommendations = target_recs
        .iter()
        .filter(|r| !base_ids.contains(r.id.as_str()))
        .cloned()
        .collect();
    updated.resolved_recommendations = base_recs
        .iter()
        .filter(|r| !target_ids.contains(r.id.as_str()))
        .cloned()
        .collect();
    updated
}

/// Active visualization mode inside the `Treemap` tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TreemapViewMode {
    #[default]
    Map,
    Table,
}

/// Active file-change bucket tab inside `Image Diff`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DiffFileTab {
    #[default]
    Added,
    Removed,
    Resized,
}

/// Source classification of an analyzed image for the status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageSourceKind {
    Registry,
    LocalFile,
    LocalDocker,
    LocalPodman,
}

impl ImageSourceKind {
    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::Registry => "status.source_registry",
            Self::LocalFile => "status.source_file",
            Self::LocalDocker => "status.source_docker",
            Self::LocalPodman => "status.source_podman",
        }
    }
}

/// Resolves the image source category from the original image reference.
pub fn image_source_kind(image_ref: &str) -> ImageSourceKind {
    let info = parse_image_info(image_ref);
    match info.registry.as_str() {
        r if r == REGISTRY_LOCAL_FILE => ImageSourceKind::LocalFile,
        r if r == REGISTRY_LOCAL_DOCKER => ImageSourceKind::LocalDocker,
        r if r == REGISTRY_LOCAL_PODMAN => ImageSourceKind::LocalPodman,
        _ => ImageSourceKind::Registry,
    }
}

/// Extracts `host[:port]` for remote registry image references (`None` for local sources).
pub fn extract_registry_host_from_image(image_ref: &str) -> Option<String> {
    let trimmed = image_ref.trim();
    if trimmed.is_empty() || image_source_kind(trimmed) != ImageSourceKind::Registry {
        return None;
    }
    let info = parse_image_info(trimmed);
    registry_host(&info.registry)
}

/// Returns `true` when an error message indicates registry authentication or TLS failure.
pub fn is_registry_auth_or_tls_error(raw: &str) -> bool {
    let lower = raw.to_ascii_lowercase();
    lower.contains("401")
        || lower.contains("403")
        || lower.contains("unauthorized")
        || lower.contains("denied")
        || lower.contains("forbidden")
        || lower.contains("authentication required")
        || lower.contains("empty token")
        || lower.contains("certificate")
        || lower.contains("unknownissuer")
        || lower.contains("invalidcertificate")
        || lower.contains("tls")
        || lower.contains("ssl")
        || lower.contains("handshake")
        || lower.contains("corrupt message")
}

/// Formats byte sizes consistently with `diving-core` (`"0 B"` for zero).
pub fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return "0 B".to_owned();
    }
    ByteSize(bytes).to_string()
}

/// Strips container shell/BuildKit noise and classifies a layer instruction opcode.
pub fn classify_layer_opcode(raw_cmd: &str) -> (LayerOpcodeKind, String) {
    let parsed = parse_layer_command(raw_cmd);
    (parsed.opcode, parsed.summary)
}

/// Formats a layer command into quote-aware indented lines for `CommandInspector`.
pub fn format_command_lines(raw_cmd: &str) -> Vec<String> {
    parse_layer_command(raw_cmd).formatted_lines
}

/// Semantic file-type category used to select an icon and color token in file views.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Symlink,
    Image,
    Audio,
    Video,
    Font,
    Code,
    Config,
    Text,
    Archive,
    Script,
    Binary,
    Key,
    Database,
    Dockerfile,
    Generic,
}

fn is_versioned_shared_lib(lower_name: &str) -> bool {
    let Some((_, ver)) = lower_name.split_once(".so.") else {
        return false;
    };
    !ver.is_empty() && ver.bytes().all(|b| b.is_ascii_digit() || b == b'.')
}

/// Classifies a file entry into a [`FileKind`] by symlink status, filename, extension, or mode.
pub fn classify_file_kind(name: &str, mode: &str, is_symlink: bool) -> FileKind {
    if is_symlink || mode.starts_with('l') {
        return FileKind::Symlink;
    }

    let base = name.rsplit('/').next().unwrap_or(name);
    let lower = base.to_ascii_lowercase();

    if lower == "dockerfile"
        || lower == "containerfile"
        || lower.starts_with("dockerfile.")
        || lower.starts_with("containerfile.")
        || matches!(
            lower.as_str(),
            "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml"
        )
    {
        return FileKind::Dockerfile;
    }

    if matches!(
        lower.as_str(),
        "id_rsa" | "id_ed25519" | "id_ecdsa" | "id_dsa" | "authorized_keys" | "known_hosts"
    ) {
        return FileKind::Key;
    }

    if matches!(
        lower.as_str(),
        ".bashrc"
            | ".bash_profile"
            | ".bash_logout"
            | ".profile"
            | ".zshrc"
            | ".zprofile"
            | ".ashrc"
            | "entrypoint"
    ) {
        return FileKind::Script;
    }

    if matches!(
        lower.as_str(),
        "makefile"
            | "gnumakefile"
            | "cmakelists.txt"
            | "justfile"
            | "procfile"
            | "gemfile"
            | "rakefile"
            | "caddyfile"
            | "vagrantfile"
            | ".gitignore"
            | ".gitattributes"
            | ".gitmodules"
            | ".dockerignore"
            | ".npmrc"
            | ".yarnrc"
            | ".nvmrc"
            | ".editorconfig"
            | ".prettierrc"
            | ".eslintrc"
            | ".babelrc"
            | ".env"
    ) || lower.starts_with(".env.")
    {
        return FileKind::Config;
    }

    if matches!(
        lower.as_str(),
        "license"
            | "licence"
            | "copying"
            | "notice"
            | "readme"
            | "changelog"
            | "changes"
            | "authors"
            | "contributors"
            | "3rdpartylicenses.txt"
    ) || lower.starts_with("license.")
        || lower.starts_with("license-")
        || lower.starts_with("licence.")
        || lower.starts_with("readme.")
        || lower.starts_with("changelog.")
    {
        return FileKind::Text;
    }

    if is_versioned_shared_lib(&lower) {
        return FileKind::Binary;
    }

    let trimmed = lower.strip_prefix('.').unwrap_or(&lower);
    if let Some((_, ext)) = trimmed.rsplit_once('.') {
        match ext {
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "ico" | "bmp" | "avif" | "tif"
            | "tiff" | "heic" | "heif" | "icns" | "cur" => return FileKind::Image,
            "mp3" | "wav" | "ogg" | "oga" | "flac" | "m4a" | "aac" | "opus" | "wma" | "mid"
            | "midi" => return FileKind::Audio,
            "mp4" | "m4v" | "webm" | "mkv" | "mov" | "avi" | "flv" | "wmv" | "ogv" | "3gp" => {
                return FileKind::Video;
            }
            "woff" | "woff2" | "ttf" | "otf" | "eot" | "ttc" => return FileKind::Font,
            "dockerfile" | "containerfile" => return FileKind::Dockerfile,
            "tar" | "gz" | "tgz" | "bz2" | "tbz" | "tbz2" | "xz" | "txz" | "zst" | "tzst"
            | "lz4" | "lzma" | "br" | "zip" | "7z" | "rar" | "deb" | "udeb" | "rpm" | "apk"
            | "jar" | "war" | "ear" | "whl" | "gem" | "nupkg" | "crate" | "cab" | "iso" | "dmg"
            | "pkg" | "layer" => return FileKind::Archive,
            "pem" | "crt" | "cer" | "der" | "key" | "pub" | "p12" | "pfx" | "jks" | "keystore"
            | "truststore" | "kdbx" | "csr" | "crl" | "gpg" | "asc" | "age" => {
                return FileKind::Key;
            }
            "db" | "sqlite" | "sqlite3" | "db3" | "rdb" | "mdb" | "ldb" | "parquet" | "duckdb" => {
                return FileKind::Database;
            }
            "sh" | "bash" | "zsh" | "fish" | "ash" | "dash" | "ksh" | "csh" | "ps1" | "psm1"
            | "bat" | "cmd" => return FileKind::Script,
            "json" | "jsonc" | "json5" | "yaml" | "yml" | "toml" | "ini" | "conf" | "cfg"
            | "cnf" | "config" | "xml" | "plist" | "env" | "properties" | "lock" | "hcl" | "tf"
            | "tfvars" | "nix" | "gradle" | "props" | "targets" | "service" | "socket"
            | "timer" | "map" => return FileKind::Config,
            "js" | "mjs" | "cjs" | "jsx" | "ts" | "mts" | "cts" | "tsx" | "vue" | "svelte"
            | "astro" | "html" | "htm" | "xhtml" | "css" | "scss" | "sass" | "less" | "py"
            | "pyi" | "rs" | "go" | "java" | "kt" | "kts" | "scala" | "groovy" | "c" | "h"
            | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "cs" | "fs" | "vb" | "rb" | "erb" | "php"
            | "swift" | "m" | "mm" | "lua" | "pl" | "pm" | "r" | "jl" | "ex" | "exs" | "erl"
            | "hrl" | "hs" | "ml" | "mli" | "clj" | "dart" | "zig" | "nim" | "sol" | "proto"
            | "graphql" | "gql" | "sql" => return FileKind::Code,
            "txt" | "md" | "markdown" | "mdx" | "rst" | "adoc" | "asciidoc" | "org" | "tex"
            | "log" | "out" | "csv" | "tsv" | "pdf" | "doc" | "docx" | "rtf" | "man" => {
                return FileKind::Text;
            }
            "so" | "dylib" | "dll" | "exe" | "bin" | "elf" | "wasm" | "o" | "a" | "lib" | "ko"
            | "pyc" | "pyo" | "pyd" | "class" | "beam" | "rlib" | "rmeta" => {
                return FileKind::Binary;
            }
            _ => {}
        }
    }

    if !mode.starts_with('d') && !mode.starts_with('l') && mode.contains('x') {
        return FileKind::Binary;
    }

    FileKind::Generic
}

/// Active filter options applied to a single layer's `FileTreeItem` slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTreeFilter {
    pub keyword: String,
    pub only_modified_removed: bool,
    pub min_size_bytes: u64,
    pub compact_paths: bool,
    pub expand_all: bool,
}

impl Default for FileTreeFilter {
    fn default() -> Self {
        Self {
            keyword: String::new(),
            only_modified_removed: false,
            min_size_bytes: 0,
            compact_paths: true,
            expand_all: false,
        }
    }
}

/// One flattened virtualized row representing either a directory or a leaf file/symlink.
#[derive(Debug, Clone, PartialEq)]
pub struct FileTreeRow {
    pub key: String,
    pub name: String,
    pub link: String,
    pub mode: String,
    pub uid: u64,
    pub gid: u64,
    pub size: u64,
    pub op: Op,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
    pub ratio: f32,
}

impl FileTreeRow {
    pub fn op_kind(&self) -> Option<FileOpKind> {
        match self.op {
            Op::Added => Some(FileOpKind::Added),
            Op::Modified => Some(FileOpKind::Modified),
            Op::Removed => Some(FileOpKind::Removed),
            Op::None if !self.is_dir => Some(FileOpKind::Added),
            Op::None => None,
        }
    }

    /// Whether this row has real file bytes in the layer (directories and removed whiteouts do not).
    pub fn is_previewable(&self) -> bool {
        !self.is_dir && !matches!(self.op, Op::Removed)
    }

    /// Whether this row represents a symbolic link rather than a regular file.
    pub fn is_symlink(&self) -> bool {
        !self.is_dir && (self.mode.starts_with('l') || !self.link.is_empty())
    }

    /// Semantic icon/color category derived from file name, mode, and symlink status.
    pub fn file_kind(&self) -> FileKind {
        classify_file_kind(&self.name, &self.mode, self.is_symlink())
    }
}

/// Per-layer counts of added, modified, and removed leaf items.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LayerFileCounts {
    pub added: usize,
    pub modified: usize,
    pub removed: usize,
}

impl LayerFileCounts {
    /// Total file entries touched in the layer (`added + modified + removed`).
    pub fn total(self) -> usize {
        self.added + self.modified + self.removed
    }
}

/// Computes the section/card count displayed on the Inspector `Findings` tab.
pub fn findings_total_count(res: &DockerAnalyzeResult, summary: &DockerAnalyzeSummary) -> usize {
    res.recommendations.len()
        + res.sensitive_files.len()
        + res.duplicate_groups.len()
        + usize::from(!summary.wasted_list.is_empty())
        + usize::from(!res.big_modified_file_list.is_empty())
        + usize::from(!res.unstripped_binaries.is_empty())
}

/// Counts leaf file operations across one layer's hierarchical file tree.
pub fn count_layer_file_ops(items: &[FileTreeItem]) -> LayerFileCounts {
    fn walk(nodes: &[FileTreeItem], counts: &mut LayerFileCounts) {
        for item in nodes {
            if item.children.is_empty() {
                match item.op {
                    Op::None | Op::Added => counts.added += 1,
                    Op::Modified => counts.modified += 1,
                    Op::Removed => counts.removed += 1,
                }
            } else {
                walk(&item.children, counts);
            }
        }
    }
    let mut counts = LayerFileCounts::default();
    walk(items, &mut counts);
    counts
}

/// Precomputed per-layer file tree index avoiding repeated `to_ascii_lowercase`
/// allocations and recursive subtree scans on every filter keystroke.
#[derive(Debug, Clone)]
pub struct IndexedTree {
    pub roots: Vec<IndexedTreeNode>,
    pub max_size: u64,
}

/// Precomputed node in an [`IndexedTree`].
#[derive(Debug, Clone)]
pub struct IndexedTreeNode {
    pub key: String,
    pub key_lower: String,
    pub name: String,
    pub name_lower: String,
    pub link: String,
    pub mode: String,
    pub uid: u64,
    pub gid: u64,
    pub size: u64,
    pub op: Op,
    pub has_modified_or_removed: bool,
    pub children: Vec<IndexedTreeNode>,
}

fn join_tree_key(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}/{name}")
    }
}

/// Builds a reusable [`IndexedTree`] once per layer so keyword/filter updates stay O(n).
pub fn build_indexed_tree(roots: &[FileTreeItem]) -> IndexedTree {
    fn index_nodes(
        items: &[FileTreeItem],
        prefix: &str,
        max_size: &mut u64,
    ) -> Vec<IndexedTreeNode> {
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            *max_size = (*max_size).max(item.size);
            let key = join_tree_key(prefix, &item.name);
            let key_lower = key.to_ascii_lowercase();
            let name_lower = item.name.to_ascii_lowercase();
            let children = index_nodes(&item.children, &key, max_size);
            let has_mod = matches!(item.op, Op::Modified | Op::Removed)
                || children.iter().any(|c| c.has_modified_or_removed);
            out.push(IndexedTreeNode {
                key,
                key_lower,
                name: item.name.clone(),
                name_lower,
                link: item.link.clone(),
                mode: item.mode.clone(),
                uid: item.uid,
                gid: item.gid,
                size: item.size,
                op: item.op.clone(),
                has_modified_or_removed: has_mod,
                children,
            });
        }
        out
    }
    let mut max_size = 0_u64;
    let indexed_roots = index_nodes(roots, "", &mut max_size);
    IndexedTree {
        roots: indexed_roots,
        max_size: max_size.max(1),
    }
}

fn indexed_matches_keyword(node: &IndexedTreeNode, keyword_lower: &str) -> bool {
    if node.key_lower.contains(keyword_lower) || node.name_lower.contains(keyword_lower) {
        return true;
    }
    node.children
        .iter()
        .any(|child| indexed_matches_keyword(child, keyword_lower))
}

fn indexed_matches_filter(
    node: &IndexedTreeNode,
    filter: &FileTreeFilter,
    keyword_lower: &str,
) -> bool {
    if filter.min_size_bytes > 0 && node.size < filter.min_size_bytes {
        return false;
    }
    if filter.only_modified_removed && !node.has_modified_or_removed {
        return false;
    }
    if !keyword_lower.is_empty() && !indexed_matches_keyword(node, keyword_lower) {
        return false;
    }
    true
}

/// Flattens a precomputed [`IndexedTree`] into virtualizable rows honoring filter & expand sets.
pub fn flatten_indexed_tree(
    tree: &IndexedTree,
    filter: &FileTreeFilter,
    expanded: &HashSet<String>,
    collapsed: &HashSet<String>,
) -> Vec<FileTreeRow> {
    let keyword_lower = filter.keyword.trim().to_ascii_lowercase();
    let mut out = Vec::new();

    #[allow(clippy::too_many_arguments)]
    fn walk(
        items: &[IndexedTreeNode],
        depth: usize,
        filter: &FileTreeFilter,
        keyword_lower: &str,
        expanded_set: &HashSet<String>,
        collapsed_set: &HashSet<String>,
        max_size: u64,
        out: &mut Vec<FileTreeRow>,
    ) {
        for raw_item in items {
            if !indexed_matches_filter(raw_item, filter, keyword_lower) {
                continue;
            }
            let mut item = raw_item;
            let mut display_name = raw_item.name.clone();
            let mut is_dir = !item.children.is_empty();

            if filter.compact_paths && is_dir && display_name != "." {
                while item.children.len() == 1 {
                    let only_child = &item.children[0];
                    if only_child.children.is_empty()
                        || !only_child.link.is_empty()
                        || !indexed_matches_filter(only_child, filter, keyword_lower)
                    {
                        break;
                    }
                    display_name = format!("{display_name}/{}", only_child.name);
                    item = only_child;
                }
                is_dir = !item.children.is_empty();
            }

            let is_expanded = is_dir
                && !collapsed_set.contains(&item.key)
                && (!keyword_lower.is_empty()
                    || filter.expand_all
                    || expanded_set.contains(&item.key));

            let ratio = ((item.size as f64) / (max_size as f64)).clamp(0.0, 1.0) as f32;
            let insert_idx = out.len();
            out.push(FileTreeRow {
                key: item.key.clone(),
                name: display_name,
                link: item.link.clone(),
                mode: item.mode.clone(),
                uid: item.uid,
                gid: item.gid,
                size: item.size,
                op: item.op.clone(),
                depth,
                is_dir,
                expanded: is_expanded,
                ratio,
            });

            if is_dir && is_expanded {
                let before_children = out.len();
                walk(
                    &item.children,
                    depth + 1,
                    filter,
                    keyword_lower,
                    expanded_set,
                    collapsed_set,
                    max_size,
                    out,
                );
                // Directories can pass `min_size_bytes` via summed descendant sizes even
                // when every individual child is smaller; when expanded, reveal those
                // children instead of dropping the directory itself.
                if out.len() == before_children && filter.min_size_bytes > 0 {
                    let relaxed = FileTreeFilter {
                        min_size_bytes: 0,
                        ..filter.clone()
                    };
                    walk(
                        &item.children,
                        depth + 1,
                        &relaxed,
                        keyword_lower,
                        expanded_set,
                        collapsed_set,
                        max_size,
                        out,
                    );
                }
                if out.len() == before_children && keyword_lower.is_empty() {
                    out.truncate(insert_idx);
                }
            }
        }
    }

    walk(
        &tree.roots,
        0,
        filter,
        &keyword_lower,
        expanded,
        collapsed,
        tree.max_size,
        &mut out,
    );
    out
}

/// Flattens a hierarchical layer file tree into virtualizable rows honoring filter & expand sets.
pub fn flatten_file_tree(
    roots: &[FileTreeItem],
    filter: &FileTreeFilter,
    expanded: &HashSet<String>,
    collapsed: &HashSet<String>,
) -> Vec<FileTreeRow> {
    let indexed = build_indexed_tree(roots);
    flatten_indexed_tree(&indexed, filter, expanded, collapsed)
}

/// Decoded file preview body shown in the File Preview Sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilePreviewContent {
    Text {
        text: String,
        formatted_text: Option<String>,
        language: Option<&'static str>,
        compression: Option<&'static str>,
        shown_bytes: u64,
        total_size: u64,
        truncated: bool,
    },
    Image {
        meta: ImagePreviewMeta,
        data_uri: Option<String>,
        hex_dump: String,
        shown_bytes: u64,
        total_size: u64,
        truncated: bool,
    },
    Svg {
        meta: ImagePreviewMeta,
        source: String,
        formatted_source: Option<String>,
        total_size: u64,
    },
    Archive {
        meta: ArchivePreviewMeta,
        report_text: String,
        hex_dump: String,
        shown_bytes: u64,
        total_size: u64,
    },
    Structured {
        meta: StructuredPreviewMeta,
        report_text: String,
        hex_dump: String,
        shown_bytes: u64,
        total_size: u64,
    },
    Media {
        meta: MediaPreviewMeta,
        report_text: String,
        hex_dump: String,
        shown_bytes: u64,
        total_size: u64,
    },
    BinaryHex {
        hex_dump: String,
        shown_bytes: u64,
        total_size: u64,
        truncated: bool,
    },
    Empty,
}

/// Maps filenames and extensions to `nsh_base::editor` syntax highlight names.
pub fn detect_code_language(path: &str) -> Option<&'static str> {
    core_detect_code_language(path)
}

/// Formats raw bytes into 16-bytes-per-row `Offset | Hex | ASCII` dump.
pub fn format_hex_dump(bytes: &[u8]) -> String {
    let mut out = String::new();
    for (row_idx, chunk) in bytes.chunks(16).enumerate() {
        if row_idx > 0 {
            out.push('\n');
        }
        let offset = row_idx * 16;
        out.push_str(&format!("{offset:08x}  "));
        for i in 0..16 {
            if let Some(byte) = chunk.get(i) {
                out.push_str(&format!("{byte:02x} "));
            } else {
                out.push_str("   ");
            }
            if i == 7 {
                out.push(' ');
            }
        }
        out.push_str(" |");
        for byte in chunk {
            let ch = if byte.is_ascii_graphic() || *byte == b' ' {
                *byte as char
            } else {
                '.'
            };
            out.push(ch);
        }
        out.push('|');
    }
    out
}

/// Builds preview content for a symbolic link row so users see its target instead of `Empty`.
pub fn symlink_preview_content(link: &str, lang: UiLang) -> FilePreviewContent {
    let body = if link.trim().is_empty() {
        t(lang, "sheet.symlink_no_target").to_owned()
    } else {
        tf(lang, "sheet.symlink_target", &[link.trim()])
    };
    let len = body.len() as u64;
    FilePreviewContent::Text {
        text: body,
        formatted_text: None,
        language: None,
        compression: None,
        shown_bytes: len,
        total_size: len,
        truncated: false,
    }
}

/// Classifies a file entry (distinguishing symlinks first) for the File Preview Sheet.
pub fn classify_file_entry(
    path: &str,
    link: &str,
    mode: &str,
    bytes: &[u8],
    total_size: u64,
    lang: UiLang,
) -> FilePreviewContent {
    if mode.starts_with('l') || (!link.is_empty() && bytes.is_empty() && total_size == 0) {
        return symlink_preview_content(link, lang);
    }
    classify_file_preview(path, bytes, total_size)
}

fn build_hex_preview(bytes: &[u8]) -> (String, u64) {
    let slice = &bytes[..bytes.len().min(8 * 1024)];
    (format_hex_dump(slice), slice.len() as u64)
}

/// Classifies raw file bytes read from a layer tar into rich previews, UTF-8 text, hex dump, or empty.
pub fn classify_file_preview(path: &str, bytes: &[u8], total_size: u64) -> FilePreviewContent {
    match inspect_file_preview(path, bytes, total_size) {
        InspectedPreview::Empty => FilePreviewContent::Empty,
        InspectedPreview::Image { meta, truncated } => {
            let (hex_dump, shown_bytes) = build_hex_preview(bytes);
            let data_uri = (!truncated).then(|| encode_data_uri(&meta.mime, bytes));
            FilePreviewContent::Image {
                meta,
                data_uri,
                hex_dump,
                shown_bytes,
                total_size,
                truncated,
            }
        }
        InspectedPreview::Svg {
            meta,
            source,
            formatted_source,
            ..
        } => FilePreviewContent::Svg {
            meta,
            source,
            formatted_source,
            total_size,
        },
        InspectedPreview::Archive { meta } => {
            let (hex_dump, shown_bytes) = build_hex_preview(bytes);
            let report_text = format_archive_text(&meta);
            FilePreviewContent::Archive {
                meta,
                report_text,
                hex_dump,
                shown_bytes,
                total_size,
            }
        }
        InspectedPreview::Structured { meta } => {
            let (hex_dump, shown_bytes) = build_hex_preview(bytes);
            let report_text = format_structured_text(&meta);
            FilePreviewContent::Structured {
                meta,
                report_text,
                hex_dump,
                shown_bytes,
                total_size,
            }
        }
        InspectedPreview::Media { meta, .. } => {
            let (hex_dump, shown_bytes) = build_hex_preview(bytes);
            let report_text = format_media_text(&meta, total_size);
            FilePreviewContent::Media {
                meta,
                report_text,
                hex_dump,
                shown_bytes,
                total_size,
            }
        }
        InspectedPreview::Text {
            text,
            formatted_text,
            language,
            compression,
            shown_bytes,
            total_size,
            truncated,
        } => FilePreviewContent::Text {
            text,
            formatted_text,
            language,
            compression,
            shown_bytes,
            total_size,
            truncated,
        },
        InspectedPreview::Binary { .. } => {
            let (hex_dump, shown_bytes) = build_hex_preview(bytes);
            FilePreviewContent::BinaryHex {
                hex_dump,
                shown_bytes,
                total_size,
                truncated: total_size > shown_bytes,
            }
        }
    }
}

/// Persisted entry in the Landing page "Recent Analyses" list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentImageEntry {
    pub image: String,
    pub arch: String,
    pub score: u64,
    pub total_size: u64,
    pub analyzed_at: String,
    #[serde(default)]
    pub stale: bool,
}

/// Inserts or promotes a recent image entry to the front, bounded at `max_items`.
pub fn upsert_recent(list: &mut Vec<RecentImageEntry>, entry: RecentImageEntry, max_items: usize) {
    list.retain(|existing| !(existing.image == entry.image && existing.arch == entry.arch));
    list.insert(0, entry);
    if list.len() > max_items {
        list.truncate(max_items);
    }
}

/// Removes a single recent image entry matching `(image, arch)`.
pub fn remove_recent(list: &mut Vec<RecentImageEntry>, image: &str, arch: &str) {
    list.retain(|existing| !(existing.image == image && existing.arch == arch));
}

/// Projected image size and efficiency if reclaimable recommendations are applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WhatIfProjection {
    pub saved_bytes: u64,
    pub projected_size: u64,
    pub projected_efficiency: u64,
}

/// Computes a conservative deduplicated What-if savings projection from recommendations.
pub fn compute_what_if_projection(
    res: &DockerAnalyzeResult,
    summary: &DockerAnalyzeSummary,
) -> Option<WhatIfProjection> {
    if res.total_size == 0 {
        return None;
    }
    let saved_raw = res
        .recommendations
        .iter()
        .find(|r| r.id == "netreclaim" && r.est_saved_bytes > 0)
        .map(|r| r.est_saved_bytes)
        .or_else(|| {
            res.recommendations
                .iter()
                .filter(|r| r.est_saved_bytes > 0)
                .map(|r| r.est_saved_bytes)
                .max()
        })?;
    let saved_bytes = saved_raw.min(res.total_size);
    if saved_bytes == 0 {
        return None;
    }
    let projected_size = res.total_size.saturating_sub(saved_bytes);
    let remaining_waste = summary.wasted_size.saturating_sub(saved_bytes);
    let projected_efficiency = if projected_size == 0 {
        100
    } else {
        ((1.0 - (remaining_waste as f64 / projected_size as f64)).clamp(0.0, 1.0) * 100.0).round()
            as u64
    };
    Some(WhatIfProjection {
        saved_bytes,
        projected_size,
        projected_efficiency,
    })
}

/// A single cross-layer provenance event for a file path.
#[derive(Debug, Clone, PartialEq)]
pub struct FileProvenanceHit {
    pub layer_index: usize,
    pub path: String,
    pub size: u64,
    pub op: Op,
    pub layer_cmd: String,
}

/// Searches across every layer's file tree for paths matching `query` (case-insensitive).
pub fn search_file_provenance(
    res: &DockerAnalyzeResult,
    query: &str,
    limit: usize,
) -> Vec<FileProvenanceHit> {
    let q = query.trim().to_ascii_lowercase();
    if q.is_empty() || limit == 0 {
        return Vec::new();
    }
    let mut hits = Vec::new();
    fn walk(
        items: &[FileTreeItem],
        prefix: &str,
        layer_index: usize,
        layer_cmd: &str,
        q: &str,
        limit: usize,
        hits: &mut Vec<FileProvenanceHit>,
    ) {
        for item in items {
            if hits.len() >= limit {
                return;
            }
            let key = join_tree_key(prefix, &item.name);
            if item.children.is_empty() {
                if key.to_ascii_lowercase().contains(q) {
                    hits.push(FileProvenanceHit {
                        layer_index,
                        path: key,
                        size: item.size,
                        op: item.op.clone(),
                        layer_cmd: layer_cmd.to_owned(),
                    });
                }
            } else {
                walk(&item.children, &key, layer_index, layer_cmd, q, limit, hits);
            }
        }
    }
    for (layer_index, tree) in res.file_tree_list.iter().enumerate() {
        if hits.len() >= limit {
            break;
        }
        let cmd = res
            .layers
            .get(layer_index)
            .map(|l| classify_layer_opcode(&l.cmd).1)
            .unwrap_or_default();
        walk(tree, "", layer_index, &cmd, &q, limit, &mut hits);
    }
    hits
}

/// Formats `RuntimeCompat` for KPI strip display with semantic badge tone.
pub fn format_runtime_compat(rc: &RuntimeCompat, lang: UiLang) -> Option<(String, BadgeTone)> {
    if rc.libc.is_empty() {
        return None;
    }
    let mut cell = if rc.entrypoint.is_empty() {
        rc.libc.clone()
    } else {
        format!("{}: {}", rc.entrypoint, rc.libc)
    };
    if !rc.required_glibc.is_empty() {
        cell.push_str(&format!(
            " ({})",
            tf(lang, "kpi.libc_needs", &[&rc.required_glibc])
        ));
    }
    if !rc.os_glibc.is_empty() {
        cell.push_str(&format!(
            " → {}",
            tf(lang, "kpi.libc_host", &[&rc.os_glibc])
        ));
    }

    let (status_key, tone) = match rc.issue.as_str() {
        "" => ("kpi.libc_ok", BadgeTone::Success),
        "glibc-too-old" => ("kpi.libc_too_old", BadgeTone::Danger),
        "glibc-on-musl" => ("kpi.libc_glibc_on_musl", BadgeTone::Danger),
        "musl-on-glibc" => ("kpi.libc_musl_on_glibc", BadgeTone::Warning),
        _ => ("kpi.libc_ok", BadgeTone::Info),
    };
    cell.push_str(&format!(" — {}", t(lang, status_key)));
    Some((cell, tone))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str, size: u64, op: Op) -> FileTreeItem {
        FileTreeItem {
            name: name.to_owned(),
            size,
            op,
            mode: "-rw-r--r--".to_owned(),
            ..Default::default()
        }
    }

    fn dir(name: &str, size: u64, op: Op, children: Vec<FileTreeItem>) -> FileTreeItem {
        FileTreeItem {
            name: name.to_owned(),
            size,
            op,
            mode: "drwxr-xr-x".to_owned(),
            children,
            ..Default::default()
        }
    }

    #[test]
    fn classify_layer_opcode_strips_shells_and_buildkit_prefixes() {
        assert_eq!(
            classify_layer_opcode("/bin/sh -c #(nop)  ENV APP_ENV=prod"),
            (LayerOpcodeKind::Meta, "ENV APP_ENV=prod".to_owned())
        );
        assert_eq!(
            classify_layer_opcode("/bin/sh -c #(nop) CMD [\"nginx\", \"-g\", \"daemon off;\"]"),
            (
                LayerOpcodeKind::Cmd,
                "CMD [\"nginx\", \"-g\", \"daemon off;\"]".to_owned()
            )
        );
        assert_eq!(
            classify_layer_opcode(
                "|2 TARGETARCH=amd64 /bin/sh -c apk add --no-cache curl # buildkit"
            ),
            (LayerOpcodeKind::Run, "apk add --no-cache curl".to_owned())
        );
        assert_eq!(
            classify_layer_opcode("COPY --from=builder /app/bin /usr/local/bin/app"),
            (
                LayerOpcodeKind::Copy,
                "COPY --from=builder /app/bin /usr/local/bin/app".to_owned()
            )
        );
    }

    #[test]
    fn flatten_file_tree_handles_compact_paths_and_nested_modifications() {
        let tree = vec![dir(
            "usr",
            300,
            Op::None,
            vec![dir(
                "local",
                300,
                Op::None,
                vec![
                    file("keep.txt", 100, Op::Added),
                    file("changed.conf", 200, Op::Modified),
                ],
            )],
        )];

        let filter = FileTreeFilter {
            only_modified_removed: true,
            compact_paths: true,
            expand_all: true,
            ..Default::default()
        };
        let rows = flatten_file_tree(&tree, &filter, &HashSet::new(), &HashSet::new());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].key, "usr/local");
        assert_eq!(rows[0].name, "usr/local");
        assert!(rows[0].is_dir);
        assert_eq!(rows[0].op_kind(), None);
        assert!(!rows[0].is_previewable());
        assert!(rows[0].expanded);
        assert_eq!(rows[1].key, "usr/local/changed.conf");
        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[1].op_kind(), Some(FileOpKind::Modified));
        assert!(rows[1].is_previewable());

        let removed_tree = vec![file("gone.bin", 0, Op::Removed)];
        let removed_rows =
            flatten_file_tree(&removed_tree, &filter, &HashSet::new(), &HashSet::new());
        assert_eq!(removed_rows.len(), 1);
        assert_eq!(removed_rows[0].op_kind(), Some(FileOpKind::Removed));
        assert!(!removed_rows[0].is_previewable());

        let all_filter = FileTreeFilter {
            expand_all: true,
            ..Default::default()
        };
        let added_tree = vec![dir(
            "etc",
            16,
            Op::None,
            vec![file("new.conf", 16, Op::None)],
        )];
        let added_rows =
            flatten_file_tree(&added_tree, &all_filter, &HashSet::new(), &HashSet::new());
        assert_eq!(added_rows[0].op_kind(), None);
        assert_eq!(added_rows[1].op_kind(), Some(FileOpKind::Added));
        assert_eq!(
            count_layer_file_ops(&added_tree),
            LayerFileCounts {
                added: 1,
                modified: 0,
                removed: 0,
            }
        );
    }

    #[test]
    fn classify_file_preview_handles_text_binary_and_empty() {
        assert_eq!(
            classify_file_preview("empty.txt", &[], 0),
            FilePreviewContent::Empty
        );

        let text_preview = classify_file_preview("/app/Dockerfile", b"FROM alpine:3.20\n", 17);
        assert_eq!(
            text_preview,
            FilePreviewContent::Text {
                text: "FROM alpine:3.20\n".to_owned(),
                formatted_text: None,
                language: Some("Dockerfile"),
                compression: None,
                shown_bytes: 17,
                total_size: 17,
                truncated: false,
            }
        );

        let bin_preview = classify_file_preview("/bin/app", b"\x7fELF\x00\x01", 4096);
        match bin_preview {
            FilePreviewContent::BinaryHex {
                hex_dump,
                truncated,
                total_size,
                ..
            } => {
                assert!(hex_dump.contains("7f 45 4c 46 00 01"));
                assert!(hex_dump.contains("|.ELF..|"));
                assert!(truncated);
                assert_eq!(total_size, 4096);
            }
            other => panic!("expected BinaryHex, got {other:?}"),
        }
    }

    #[test]
    fn upsert_recent_deduplicates_promotes_and_caps() {
        let mut recent = vec![
            RecentImageEntry {
                image: "alpine:3.20".into(),
                arch: "amd64".into(),
                score: 99,
                total_size: 8_000_000,
                analyzed_at: "t1".into(),
                stale: false,
            },
            RecentImageEntry {
                image: "nginx:alpine".into(),
                arch: "amd64".into(),
                score: 97,
                total_size: 40_000_000,
                analyzed_at: "t2".into(),
                stale: false,
            },
        ];
        upsert_recent(
            &mut recent,
            RecentImageEntry {
                image: "nginx:alpine".into(),
                arch: "amd64".into(),
                score: 98,
                total_size: 41_000_000,
                analyzed_at: "t3".into(),
                stale: false,
            },
            2,
        );
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].image, "nginx:alpine");
        assert_eq!(recent[0].score, 98);

        upsert_recent(
            &mut recent,
            RecentImageEntry {
                image: "redis:alpine".into(),
                arch: "arm64".into(),
                score: 95,
                total_size: 30_000_000,
                analyzed_at: "t4".into(),
                stale: true,
            },
            2,
        );
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].image, "redis:alpine");
        assert_eq!(recent[1].image, "nginx:alpine");

        remove_recent(&mut recent, "redis:alpine", "arm64");
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].image, "nginx:alpine");
    }

    #[test]
    fn flatten_file_tree_preserves_expanded_dir_under_min_size_filter() {
        let tree = vec![dir(
            "bundle",
            180_000,
            Op::Added,
            vec![dir(
                "chunks",
                180_000,
                Op::Added,
                vec![
                    file("a.js", 90_000, Op::Added),
                    file("b.js", 90_000, Op::Added),
                ],
            )],
        )];
        let filter = FileTreeFilter {
            min_size_bytes: 100 * 1024,
            compact_paths: true,
            ..Default::default()
        };

        let collapsed_rows = flatten_file_tree(&tree, &filter, &HashSet::new(), &HashSet::new());
        assert_eq!(collapsed_rows.len(), 1);
        assert_eq!(collapsed_rows[0].key, "bundle/chunks");
        assert!(!collapsed_rows[0].expanded);

        let expanded: HashSet<String> = ["bundle/chunks".to_owned()].into_iter().collect();
        let expanded_rows = flatten_file_tree(&tree, &filter, &expanded, &HashSet::new());
        assert_eq!(expanded_rows.len(), 3);
        assert_eq!(expanded_rows[0].key, "bundle/chunks");
        assert!(expanded_rows[0].expanded);
        assert_eq!(expanded_rows[1].key, "bundle/chunks/a.js");
        assert_eq!(expanded_rows[2].key, "bundle/chunks/b.js");
    }

    #[test]
    fn classify_file_preview_handles_truncated_utf8_boundary() {
        // 131_071 ASCII bytes + first byte of a 3-byte CJK character = 131_072 bytes (128 KiB slice).
        let cjk = "中".as_bytes();
        let mut buf = vec![b'a'; 128 * 1024 - 1];
        buf.push(cjk[0]);
        assert_eq!(buf.len(), 128 * 1024);

        let preview = classify_file_preview("notes/cjk.txt", &buf, (128 * 1024 + 10) as u64);
        match preview {
            FilePreviewContent::Text {
                text,
                shown_bytes,
                truncated,
                ..
            } => {
                assert!(truncated);
                assert_eq!(shown_bytes as usize, 128 * 1024 - 1);
                assert_eq!(text.len(), 128 * 1024 - 1);
            }
            other => panic!("expected Text preview on UTF-8 boundary, got {other:?}"),
        }
    }

    #[test]
    fn classify_file_entry_distinguishes_symlinks() {
        let preview =
            classify_file_entry("usr/bin/sh", "busybox", "lrwxrwxrwx", &[], 0, UiLang::En);
        assert_eq!(
            preview,
            FilePreviewContent::Text {
                text: "Symbolic link → busybox".to_owned(),
                formatted_text: None,
                language: None,
                compression: None,
                shown_bytes: "Symbolic link → busybox".len() as u64,
                total_size: "Symbolic link → busybox".len() as u64,
                truncated: false,
            }
        );

        let zh = classify_file_entry("usr/bin/sh", "", "lrwxrwxrwx", &[], 0, UiLang::Zh);
        assert_eq!(
            zh,
            FilePreviewContent::Text {
                text: "符号链接（未记录目标路径）。".to_owned(),
                formatted_text: None,
                language: None,
                compression: None,
                shown_bytes: "符号链接（未记录目标路径）。".len() as u64,
                total_size: "符号链接（未记录目标路径）。".len() as u64,
                truncated: false,
            }
        );
    }

    #[test]
    fn classify_file_preview_handles_images_svg_json_and_elf() {
        let mut png = Vec::new();
        png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        png.extend_from_slice(&13_u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&32_u32.to_be_bytes());
        png.extend_from_slice(&24_u32.to_be_bytes());
        png.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        match classify_file_preview("assets/icon.png", &png, png.len() as u64) {
            FilePreviewContent::Image {
                meta,
                data_uri,
                truncated,
                ..
            } => {
                assert_eq!(meta.format, "PNG");
                assert_eq!(meta.width, Some(32));
                assert_eq!(meta.height, Some(24));
                assert!(
                    data_uri
                        .as_deref()
                        .unwrap()
                        .starts_with("data:image/png;base64,")
                );
                assert!(!truncated);
            }
            other => panic!("expected Image preview, got {other:?}"),
        }

        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20" viewBox="0 0 40 20"><g fill="none"><circle cx="10" cy="10" r="8"/><rect x="20" y="2" width="12" height="12"/></g></svg>"#;
        match classify_file_preview("assets/badge.svg", svg, svg.len() as u64) {
            FilePreviewContent::Svg {
                meta,
                source,
                formatted_source,
                ..
            } => {
                assert_eq!(meta.format, "SVG");
                assert_eq!(meta.width, Some(40));
                assert!(source.contains("<circle"));
                assert!(
                    formatted_source
                        .as_deref()
                        .unwrap()
                        .contains("\n  <g fill=\"none\">")
                );
            }
            other => panic!("expected Svg preview, got {other:?}"),
        }

        let min_js = br#"import{a as b}from"./c.js";const run=(x)=>{const z=x*2;return z};for(let i=0;i<3;i++){console.log("step",i,run(i))}const extra={enabled:true,retries:3};export{run,extra};"#;
        match classify_file_preview("static/app.module.js", min_js, min_js.len() as u64) {
            FilePreviewContent::Text {
                text,
                formatted_text,
                language,
                ..
            } => {
                assert_eq!(language, Some("javascript"));
                assert_eq!(text.as_bytes(), min_js);
                assert!(formatted_text.as_deref().unwrap().contains('\n'));
            }
            other => panic!("expected minified JS preview, got {other:?}"),
        }

        let min_json = br#"{"a":1,"b":"ok"}"#;
        match classify_file_preview("static/version.json", min_json, min_json.len() as u64) {
            FilePreviewContent::Text {
                formatted_text,
                language,
                ..
            } => {
                assert_eq!(language, Some("json"));
                assert!(
                    formatted_text
                        .as_deref()
                        .unwrap()
                        .contains("\n  \"b\": \"ok\"")
                );
            }
            other => panic!("expected formatted JSON Text preview, got {other:?}"),
        }

        let mut elf = [0_u8; 64];
        elf[0..4].copy_from_slice(b"\x7fELF");
        elf[4] = 2;
        elf[5] = 1;
        elf[16..18].copy_from_slice(&3_u16.to_le_bytes());
        elf[18..20].copy_from_slice(&62_u16.to_le_bytes());
        match classify_file_preview("usr/lib/libdemo.so", &elf, 8192) {
            FilePreviewContent::Structured {
                meta, report_text, ..
            } => {
                assert!(meta.summary.contains("x86_64"));
                assert!(report_text.contains("ELF"));
            }
            other => panic!("expected Structured ELF preview, got {other:?}"),
        }
    }

    #[test]
    fn visible_layer_indices_respects_filter_modes() {
        let layers = vec![
            ImageLayer {
                empty: true,
                unpack_size: 0,
                ..Default::default()
            },
            ImageLayer {
                empty: false,
                unpack_size: 512,
                ..Default::default()
            },
            ImageLayer {
                empty: false,
                unpack_size: LARGE_LAYER_BYTES,
                ..Default::default()
            },
            ImageLayer {
                empty: true,
                unpack_size: 0,
                ..Default::default()
            },
        ];
        assert_eq!(
            visible_layer_indices(&layers, LayerFilterMode::All, LayerSortOrder::Desc),
            [3, 2, 1, 0]
        );
        assert_eq!(
            visible_layer_indices(&layers, LayerFilterMode::HasFiles, LayerSortOrder::Desc),
            [2, 1]
        );
        assert_eq!(
            visible_layer_indices(&layers, LayerFilterMode::Large, LayerSortOrder::Desc),
            [2]
        );
        assert_eq!(
            visible_layer_indices(&layers, LayerFilterMode::All, LayerSortOrder::Asc),
            [0, 1, 2, 3]
        );
        assert_eq!(
            visible_layer_indices(&layers, LayerFilterMode::HasFiles, LayerSortOrder::Asc),
            [1, 2]
        );
        assert_eq!(default_selected_layer(&layers, LayerSortOrder::Desc), 2);
        assert_eq!(default_selected_layer(&layers, LayerSortOrder::Asc), 1);
    }

    #[test]
    fn format_runtime_compat_localizes_needs_and_host() {
        let rc = RuntimeCompat {
            entrypoint: "/app/server".to_owned(),
            libc: "glibc".to_owned(),
            required_glibc: "2.38".to_owned(),
            needed_libs: Vec::new(),
            os_glibc: "2.36".to_owned(),
            issue: "glibc-too-old".to_owned(),
        };
        let (zh, tone) = format_runtime_compat(&rc, UiLang::Zh).expect("zh compat");
        assert_eq!(tone, BadgeTone::Danger);
        assert!(zh.contains("需要 2.38"));
        assert!(zh.contains("宿主 2.36"));
        assert!(zh.contains("宿主 glibc 过低"));
    }

    #[test]
    fn classify_file_kind_covers_media_code_container_and_edge_cases() {
        // Filenames visible in the Inspector FileTree screenshot.
        assert_eq!(
            classify_file_kind("3rdpartylicenses.txt", "-rw-r--r--", false),
            FileKind::Text
        );
        assert_eq!(
            classify_file_kind("account-active.module.d5371b561e3.js", "-rw-r--r--", false),
            FileKind::Code
        );
        for img in ["icon-fail.png", "point.png", "right.png", "HERO.WEBP"] {
            assert_eq!(
                classify_file_kind(img, "-rw-r--r--", false),
                FileKind::Image
            );
        }
        for audio in ["beijiaosuo.mp3", "rzrq.mp3", "xinsanbannewv4.mp3"] {
            assert_eq!(
                classify_file_kind(audio, "-rw-r--r--", false),
                FileKind::Audio
            );
        }
        assert_eq!(
            classify_file_kind("version.json", "-rw-r--r--", false),
            FileKind::Config
        );

        // Symlinks win regardless of target filename extension.
        assert_eq!(
            classify_file_kind("python3.py", "lrwxrwxrwx", false),
            FileKind::Symlink
        );
        assert_eq!(
            classify_file_kind("python3.py", "", true),
            FileKind::Symlink
        );

        // Container runtime, binaries, and versioned shared objects.
        assert_eq!(
            classify_file_kind("/lib/x86_64-linux-gnu/libc.so.6", "-rwxr-xr-x", false),
            FileKind::Binary
        );
        assert_eq!(
            classify_file_kind("/usr/lib/libssl.so.3", "-rw-r--r--", false),
            FileKind::Binary
        );
        assert_eq!(
            classify_file_kind("/usr/lib/libstdc++.so.6.0.32", "-rw-r--r--", false),
            FileKind::Binary
        );
        assert_eq!(
            classify_file_kind("/usr/local/bin/redis-server", "-rwxr-xr-x", false),
            FileKind::Binary
        );
        // Extension beats executable fallback so chmod +x assets keep their type icon.
        assert_eq!(
            classify_file_kind("icon-fail.png", "-rwxr-xr-x", false),
            FileKind::Image
        );

        // Archives, scripts, keys, databases, fonts, video, Dockerfiles, configs, generic.
        for arch in ["layer.tar.gz", "alpine-keys.apk", "app.jar", "pkg.whl"] {
            assert_eq!(
                classify_file_kind(arch, "-rw-r--r--", false),
                FileKind::Archive
            );
        }
        for script in ["docker-entrypoint.sh", ".bashrc", "entrypoint"] {
            assert_eq!(
                classify_file_kind(script, "-rwxr-xr-x", false),
                FileKind::Script
            );
        }
        for key in ["ca-certificates.crt", "server.key", "id_ed25519"] {
            assert_eq!(classify_file_kind(key, "-rw-------", false), FileKind::Key);
        }
        for db in ["data.sqlite3", "dump.rdb"] {
            assert_eq!(
                classify_file_kind(db, "-rw-r--r--", false),
                FileKind::Database
            );
        }
        assert_eq!(
            classify_file_kind("inter.woff2", "-rw-r--r--", false),
            FileKind::Font
        );
        assert_eq!(
            classify_file_kind("demo.mp4", "-rw-r--r--", false),
            FileKind::Video
        );
        for df in ["Dockerfile", "Dockerfile.dev", "compose.yaml"] {
            assert_eq!(
                classify_file_kind(df, "-rw-r--r--", false),
                FileKind::Dockerfile
            );
        }
        assert_eq!(
            classify_file_kind(".env.production", "-rw-r--r--", false),
            FileKind::Config
        );
        assert_eq!(
            classify_file_kind("UNKNOWN_DATA", "-rw-r--r--", false),
            FileKind::Generic
        );
    }

    #[test]
    fn extract_registry_host_and_auth_error_detection_cover_private_registries() {
        assert_eq!(
            extract_registry_host_from_image("harbor.corp.com/org/app:v1").as_deref(),
            Some("harbor.corp.com")
        );
        assert_eq!(
            extract_registry_host_from_image("http://10.0.1.5:5000/svc:latest").as_deref(),
            Some("10.0.1.5:5000")
        );
        assert_eq!(
            extract_registry_host_from_image("redis:alpine").as_deref(),
            Some("index.docker.io")
        );
        assert_eq!(extract_registry_host_from_image("file:///tmp/a.tar"), None);
        assert_eq!(
            extract_registry_host_from_image("docker://redis:alpine"),
            None
        );
        assert_eq!(extract_registry_host_from_image(""), None);

        assert!(is_registry_auth_or_tls_error(
            "Request https://harbor.corp.com/v2/app/manifests/latest code: UNAUTHORIZED fail: authentication required"
        ));
        assert!(is_registry_auth_or_tls_error("registry returned HTTP 403"));
        assert!(is_registry_auth_or_tls_error(
            "invalid peer certificate: UnknownIssuer"
        ));
        assert!(is_registry_auth_or_tls_error(
            "error sending request for url: corrupt message"
        ));
        assert!(!is_registry_auth_or_tls_error("Tar fail: unexpected EOF"));
        assert!(!is_registry_auth_or_tls_error(
            "docker images failed: daemon not running"
        ));
    }
}
