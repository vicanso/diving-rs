//! Pure domain computations for layers, virtual file trees, previews, and history.

use std::collections::HashSet;
use std::ops::Deref;
use std::str::from_utf8;
use std::sync::Arc;

use bytesize::ByteSize;
use diving_core::{
    DockerAnalyzeResult, DockerAnalyzeSummary, FileTreeItem, Op, REGISTRY_LOCAL_DOCKER,
    REGISTRY_LOCAL_FILE, REGISTRY_LOCAL_PODMAN, RuntimeCompat, parse_image_info,
};
use serde::{Deserialize, Serialize};

use crate::i18n::{UiLang, t};
use crate::ui::{BadgeTone, FileOpKind, LayerOpcodeKind};

/// Active top-level route inside the desktop shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppRoute {
    #[default]
    Landing,
    Inspector,
    Diff,
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

/// Filter modes for `LayerTimeline`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayerFilterMode {
    #[default]
    All,
    HasFiles,
    Large,
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

/// Formats byte sizes consistently with `diving-core` (`"0 B"` for zero).
pub fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return "0 B".to_owned();
    }
    ByteSize(bytes).to_string()
}

/// Strips container shell/BuildKit noise and classifies a layer instruction opcode.
pub fn classify_layer_opcode(raw_cmd: &str) -> (LayerOpcodeKind, String) {
    let mut s = raw_cmd.trim();
    if let Some(stripped) = s.strip_suffix("# buildkit") {
        s = stripped.trim_end();
    }

    let mut had_nop = false;
    let mut had_sh = false;
    if let Some(rest) = s.strip_prefix("/bin/sh -c ") {
        had_sh = true;
        s = rest.trim_start();
    }
    if let Some(rest) = s.strip_prefix("#(nop) ") {
        had_nop = true;
        s = rest.trim_start();
    } else if let Some(rest) = s.strip_prefix("#(nop)") {
        had_nop = true;
        s = rest.trim_start();
    }

    // BuildKit prefixed runs like `|2 A=1 B=2 /bin/sh -c cmd`
    if let Some(after_pipe) = s.strip_prefix('|') {
        let mut digits_end = 0;
        for (idx, ch) in after_pipe.char_indices() {
            if ch.is_ascii_digit() {
                digits_end = idx + ch.len_utf8();
            } else {
                break;
            }
        }
        if digits_end > 0
            && after_pipe[digits_end..].starts_with(' ')
            && let Some(sh_pos) = after_pipe.find("/bin/sh -c ")
        {
            had_sh = true;
            s = after_pipe[sh_pos + "/bin/sh -c ".len()..].trim_start();
        }
    }

    let cleaned = collapse_spaces(s);
    if cleaned.is_empty() {
        return (LayerOpcodeKind::Meta, "(metadata)".to_owned());
    }

    let first_word = cleaned
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();

    let kind = match first_word.as_str() {
        "FROM" => LayerOpcodeKind::From,
        "RUN" => LayerOpcodeKind::Run,
        "COPY" | "ADD" => LayerOpcodeKind::Copy,
        "CMD" | "ENTRYPOINT" => LayerOpcodeKind::Cmd,
        "ENV" | "LABEL" | "WORKDIR" | "USER" | "EXPOSE" | "VOLUME" | "ARG" | "SHELL"
        | "STOPSIGNAL" | "HEALTHCHECK" | "ONBUILD" | "MAINTAINER" => LayerOpcodeKind::Meta,
        _ if had_nop => LayerOpcodeKind::Meta,
        _ if had_sh => LayerOpcodeKind::Run,
        _ => LayerOpcodeKind::Run,
    };

    (kind, cleaned)
}

fn collapse_spaces(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_space = false;
    for ch in input.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out.trim().to_owned()
}

/// Splits chained shell `&&` segments onto readable indented lines for `CommandInspector`.
pub fn format_command_lines(raw_cmd: &str) -> Vec<String> {
    let (_, cleaned) = classify_layer_opcode(raw_cmd);
    if !cleaned.contains(" && ") {
        return vec![cleaned];
    }
    let parts: Vec<&str> = cleaned.split(" && ").collect();
    if parts.len() <= 1 {
        return vec![cleaned];
    }
    parts
        .iter()
        .enumerate()
        .map(|(idx, part)| {
            let trimmed = part.trim();
            if idx == 0 {
                format!("{trimmed} &&")
            } else if idx + 1 < parts.len() {
                format!("  {trimmed} &&")
            } else {
                format!("  {trimmed}")
            }
        })
        .collect()
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

fn has_modified_or_removed(item: &FileTreeItem) -> bool {
    if matches!(item.op, Op::Modified | Op::Removed) {
        return true;
    }
    item.children.iter().any(has_modified_or_removed)
}

fn matches_keyword(item: &FileTreeItem, key: &str, keyword_lower: &str) -> bool {
    if key.to_ascii_lowercase().contains(keyword_lower)
        || item.name.to_ascii_lowercase().contains(keyword_lower)
    {
        return true;
    }
    item.children.iter().any(|child| {
        let child_key = join_tree_key(key, &child.name);
        matches_keyword(child, &child_key, keyword_lower)
    })
}

fn matches_filter(
    item: &FileTreeItem,
    key: &str,
    filter: &FileTreeFilter,
    keyword_lower: &str,
) -> bool {
    if filter.min_size_bytes > 0 && item.size < filter.min_size_bytes {
        return false;
    }
    if filter.only_modified_removed && !has_modified_or_removed(item) {
        return false;
    }
    if !keyword_lower.is_empty() && !matches_keyword(item, key, keyword_lower) {
        return false;
    }
    true
}

fn join_tree_key(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}/{name}")
    }
}

fn max_entry_size(items: &[FileTreeItem]) -> u64 {
    let mut max_sz = 0_u64;
    for item in items {
        max_sz = max_sz.max(item.size);
        if !item.children.is_empty() {
            max_sz = max_sz.max(max_entry_size(&item.children));
        }
    }
    max_sz
}

/// Flattens a hierarchical layer file tree into virtualizable rows honoring filter & expand sets.
pub fn flatten_file_tree(
    roots: &[FileTreeItem],
    filter: &FileTreeFilter,
    expanded: &HashSet<String>,
    collapsed: &HashSet<String>,
) -> Vec<FileTreeRow> {
    let max_size = max_entry_size(roots).max(1);
    let keyword_lower = filter.keyword.trim().to_ascii_lowercase();
    let mut out = Vec::new();

    #[allow(clippy::too_many_arguments)]
    fn walk(
        items: &[FileTreeItem],
        prefix: &str,
        depth: usize,
        filter: &FileTreeFilter,
        keyword_lower: &str,
        expanded_set: &HashSet<String>,
        collapsed_set: &HashSet<String>,
        max_size: u64,
        out: &mut Vec<FileTreeRow>,
    ) {
        for raw_item in items {
            let mut key = join_tree_key(prefix, &raw_item.name);
            if !matches_filter(raw_item, &key, filter, keyword_lower) {
                continue;
            }
            let mut item = raw_item;
            let mut display_name = raw_item.name.clone();
            let mut is_dir = !item.children.is_empty();

            if filter.compact_paths && is_dir && display_name != "." {
                while item.children.len() == 1 {
                    let only_child = &item.children[0];
                    let child_key = join_tree_key(&key, &only_child.name);
                    if only_child.children.is_empty()
                        || !only_child.link.is_empty()
                        || !matches_filter(only_child, &child_key, filter, keyword_lower)
                    {
                        break;
                    }
                    display_name = format!("{display_name}/{}", only_child.name);
                    key = child_key;
                    item = only_child;
                }
                is_dir = !item.children.is_empty();
            }

            let is_expanded = is_dir
                && !collapsed_set.contains(&key)
                && (!keyword_lower.is_empty() || filter.expand_all || expanded_set.contains(&key));

            let ratio = ((item.size as f64) / (max_size as f64)).clamp(0.0, 1.0) as f32;
            let insert_idx = out.len();
            out.push(FileTreeRow {
                key: key.clone(),
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
                    &key,
                    depth + 1,
                    filter,
                    keyword_lower,
                    expanded_set,
                    collapsed_set,
                    max_size,
                    out,
                );
                if out.len() == before_children && keyword_lower.is_empty() {
                    out.truncate(insert_idx);
                }
            }
        }
    }

    walk(
        roots,
        "",
        0,
        filter,
        &keyword_lower,
        expanded,
        collapsed,
        max_size,
        &mut out,
    );
    out
}

/// Decoded file preview body shown in the File Preview Sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilePreviewContent {
    Text {
        text: String,
        language: Option<&'static str>,
        shown_bytes: u64,
        total_size: u64,
        truncated: bool,
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
    let basename = path.rsplit('/').next().unwrap_or(path);
    if basename.eq_ignore_ascii_case("Dockerfile") || basename.starts_with("Dockerfile.") {
        return Some("Dockerfile");
    }
    if basename.eq_ignore_ascii_case("Makefile") {
        return Some("makefile");
    }
    let ext = basename.rsplit_once('.')?.1.to_ascii_lowercase();
    match ext.as_str() {
        "rs" => Some("rust"),
        "ts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "js" | "mjs" | "cjs" => Some("javascript"),
        "jsx" => Some("jsx"),
        "json" => Some("json"),
        "yml" | "yaml" => Some("yaml"),
        "toml" => Some("toml"),
        "sh" | "bash" | "zsh" => Some("bash"),
        "py" => Some("python"),
        "go" => Some("go"),
        "md" | "markdown" => Some("markdown"),
        "xml" | "svg" => Some("xml"),
        "html" | "htm" => Some("html"),
        "css" => Some("css"),
        "sql" => Some("sql"),
        "conf" | "ini" | "env" => Some("ini"),
        "c" | "h" => Some("c"),
        "cc" | "cpp" | "cxx" | "hpp" => Some("cpp"),
        _ => None,
    }
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

/// Classifies raw file bytes read from a layer tar into UTF-8 text, hex dump, or empty.
pub fn classify_file_preview(path: &str, bytes: &[u8], total_size: u64) -> FilePreviewContent {
    if bytes.is_empty() && total_size == 0 {
        return FilePreviewContent::Empty;
    }
    let shown_bytes = bytes.len() as u64;
    let truncated = total_size > shown_bytes;
    let looks_binary = bytes.contains(&0);

    if !looks_binary && let Ok(text) = from_utf8(bytes) {
        return FilePreviewContent::Text {
            text: text.to_owned(),
            language: detect_code_language(path),
            shown_bytes,
            total_size,
            truncated,
        };
    }

    let hex_slice = &bytes[..bytes.len().min(8 * 1024)];
    FilePreviewContent::BinaryHex {
        hex_dump: format_hex_dump(hex_slice),
        shown_bytes: hex_slice.len() as u64,
        total_size,
        truncated: total_size > hex_slice.len() as u64,
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
}

/// Inserts or promotes a recent image entry to the front, bounded at `max_items`.
pub fn upsert_recent(list: &mut Vec<RecentImageEntry>, entry: RecentImageEntry, max_items: usize) {
    list.retain(|existing| !(existing.image == entry.image && existing.arch == entry.arch));
    list.insert(0, entry);
    if list.len() > max_items {
        list.truncate(max_items);
    }
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
        cell.push_str(&format!(" (needs {})", rc.required_glibc));
    }
    if !rc.os_glibc.is_empty() {
        cell.push_str(&format!(" → host {}", rc.os_glibc));
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
                language: Some("Dockerfile"),
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
            },
            RecentImageEntry {
                image: "nginx:alpine".into(),
                arch: "amd64".into(),
                score: 97,
                total_size: 40_000_000,
                analyzed_at: "t2".into(),
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
            },
            2,
        );
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].image, "redis:alpine");
        assert_eq!(recent[1].image, "nginx:alpine");
    }
}
