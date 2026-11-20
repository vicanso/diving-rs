use bytesize::ByteSize;
use chrono::DateTime;
use std::cmp::Reverse;

use crate::i18n::{self, Lang};
use crate::image::{final_filesystem, DockerAnalyzeResult, FileTreeItem, Op, WasteChange};

pub(crate) fn icon(severity: &str) -> &'static str {
    match severity {
        "high" => "🔴",
        "medium" => "🟠",
        "low" => "🟡",
        _ => "ℹ️",
    }
}

/// `s` cut to at most `max` bytes on a char boundary, marked with `…` when
/// cut. Layer commands are arbitrary text; slicing one mid-character would
/// panic, and a panic aborts the whole web server.
pub(crate) fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Changes listed per wasted path; the count column covers the rest.
const WASTE_CHANGES_SHOWN: usize = 3;

/// `L1 → L3 deleted, L3 → L5 overwritten` (1-based, like the layer
/// headings). The writing layer is missing from analyses cached by 0.12.0.
fn waste_changes(changes: &[WasteChange], lang: Lang) -> String {
    let mut parts: Vec<String> = changes
        .iter()
        .take(WASTE_CHANGES_SHOWN)
        .map(|c| {
            let from = c
                .written_in
                .map(|l| format!("L{} → ", l + 1))
                .unwrap_or_default();
            let key = if c.removed {
                "md.waste.removed"
            } else {
                "md.waste.overwritten"
            };
            format!("{from}L{} {}", c.layer + 1, i18n::tr(lang, key))
        })
        .collect();
    if changes.len() > WASTE_CHANGES_SHOWN {
        parts.push("…".to_string());
    }
    parts.join(", ")
}

/// Largest files in the final filesystem shown in the summary.
const SUMMARY_TOP_FILES: usize = 5;
/// Recommendations listed in the summary; the rest are counted.
const SUMMARY_RECS: usize = 8;

/// A short Markdown summary — headline numbers, the largest files and the
/// recommendations — sized for a pull-request comment or a CI job summary;
/// the full report runs to tens of KB. `verdict` is the CI gate outcome
/// (see `gate_verdict_md`), when there is one.
pub fn to_summary_markdown(
    result: &DockerAnalyzeResult,
    lang: Lang,
    verdict: Option<&str>,
) -> String {
    let t = |k: &str| -> String { i18n::tr(lang, k).to_string() };
    let f = |k: &str, args: &[&str]| -> String { i18n::fill(i18n::tr(lang, k), args) };
    let summary = result.summary();
    let mut md = format!("### diving · `{}`\n\n", result.name);
    if let Some(verdict) = verdict {
        md.push_str(&format!("{verdict}\n\n"));
    }
    if let Some(as_of) = &result.stale_as_of {
        md.push_str(&format!("> ⚠️ {}\n\n", f("md.stale", &[as_of])));
    }

    let empty = result.layers.iter().filter(|l| l.empty).count();
    let layers = if empty > 0 {
        format!(
            "{} ({})",
            result.layers.len(),
            f("sum.empty", &[&empty.to_string()])
        )
    } else {
        result.layers.len().to_string()
    };
    md.push_str(&format!(
        "| {} | {} | {} | {} |\n|---|---|---|---|\n| {} ({}) | {} | {} % | {} ({:.1} %) |\n\n",
        t("sum.size"),
        t("sum.layers"),
        t("md.f.eff"),
        t("md.f.wasted"),
        ByteSize(result.total_size),
        f("sum.compressed", &[&ByteSize(result.size).to_string()]),
        layers,
        summary.score,
        ByteSize(summary.wasted_size),
        summary.wasted_percent * 100.0,
    ));

    let mut files: Vec<(String, u64, usize)> = final_filesystem(&result.file_tree_list)
        .into_iter()
        .map(|(path, file)| (path, file.size, file.layer))
        .filter(|(_, size, _)| *size > 0)
        .collect();
    files.sort_by_key(|(_, size, _)| Reverse(*size));
    if !files.is_empty() {
        md.push_str(&format!(
            "**{}**\n\n| {} | {} | {} |\n|---:|---:|---|\n",
            t("sum.largest"),
            t("sum.col.size"),
            t("sum.col.layer"),
            t("sum.col.path"),
        ));
        for (path, size, layer) in files.iter().take(SUMMARY_TOP_FILES) {
            md.push_str(&format!(
                "| {} | {} | `{}` |\n",
                ByteSize(*size),
                layer + 1,
                path
            ));
        }
        md.push('\n');
    }

    if !result.recommendations.is_empty() {
        md.push_str(&format!(
            "**{}**\n\n",
            f("sum.recs", &[&result.recommendations.len().to_string()])
        ));
        for r in result.recommendations.iter().take(SUMMARY_RECS) {
            let saved = if r.est_saved_bytes > 0 {
                f("cli.saved", &[&ByteSize(r.est_saved_bytes).to_string()])
            } else {
                String::new()
            };
            md.push_str(&format!(
                "- {} **{}** · {}{}\n",
                icon(&r.severity),
                r.title,
                t(&format!("cat.{}", r.category)),
                saved
            ));
        }
        let rest = result.recommendations.len().saturating_sub(SUMMARY_RECS);
        if rest > 0 {
            md.push_str(&format!("- {}\n", f("sum.more", &[&rest.to_string()])));
        }
    }
    md
}

fn cmd_to_dockerfile_line(cmd: &str) -> Option<String> {
    if cmd.is_empty() {
        return None;
    }
    let cmd = if cmd.starts_with('|') {
        cmd.find("/bin/sh -c ")
            .map(|pos| &cmd[pos..])
            .unwrap_or(cmd)
    } else {
        cmd
    };
    if let Some(rest) = cmd.strip_prefix("/bin/sh -c #(nop) ") {
        Some(rest.trim().to_string())
    } else if let Some(rest) = cmd.strip_prefix("/bin/sh -c ") {
        Some(format!("RUN {rest}"))
    } else {
        Some(cmd.to_string())
    }
}

struct FileEntry {
    path: String,
    size: u64,
    mode: String,
    uid: u64,
    gid: u64,
}

struct LayerFiles {
    added: Vec<FileEntry>,
    modified: Vec<FileEntry>,
    removed: Vec<FileEntry>,
}

fn flatten_tree(items: &[FileTreeItem], prefix: &str, out: &mut LayerFiles) {
    for item in items {
        let path = if prefix.is_empty() {
            item.name.clone()
        } else {
            format!("{}/{}", prefix, item.name)
        };
        if item.children.is_empty() {
            let entry = FileEntry {
                path,
                size: item.size,
                mode: item.mode.clone(),
                uid: item.uid,
                gid: item.gid,
            };
            match item.op {
                Op::Removed => out.removed.push(entry),
                Op::Modified => out.modified.push(entry),
                _ => out.added.push(entry),
            }
        } else {
            flatten_tree(&item.children, &path, out);
        }
    }
}

/// Rows shown per change kind in each layer; the rest collapse into one
/// "N more files" row. Without a cap a single `chown -R` / `apt-get upgrade`
/// layer lists thousands of files and balloons the report — and with it the
/// AI and MCP payloads built from this Markdown.
const LAYER_FILES_LIMIT: usize = 50;

/// Append one change kind's rows for a layer, largest first, capped at
/// [`LAYER_FILES_LIMIT`].
fn push_file_rows(md: &mut String, op: &str, files: &mut [FileEntry], lang: Lang) {
    files.sort_by_key(|e| Reverse(e.size));
    for e in files.iter().take(LAYER_FILES_LIMIT) {
        md.push_str(&format!(
            "| {} | `{}` | {} | `{}` | {}:{} |\n",
            op,
            e.path,
            ByteSize(e.size),
            e.mode,
            e.uid,
            e.gid,
        ));
    }
    if files.len() > LAYER_FILES_LIMIT {
        md.push_str(&format!(
            "| … | {} | | | |\n",
            i18n::fill(
                i18n::tr(lang, "md.moremfiles"),
                &[&(files.len() - LAYER_FILES_LIMIT).to_string()]
            )
        ));
    }
}

/// Find the index of the first "user layer" by locating the largest time gap
/// between consecutive layers. Base image layers were built months/years ago;
/// user layers are all built within the same `docker build` run (seconds apart).
/// Returns None when no significant gap (> 1 hour) is found.
fn find_user_layer_start(result: &DockerAnalyzeResult) -> Option<usize> {
    let timestamps: Vec<i64> = result
        .layers
        .iter()
        .filter_map(|l| DateTime::parse_from_rfc3339(&l.created).ok())
        .map(|d| d.timestamp())
        .collect();

    if timestamps.len() < 2 {
        return None;
    }

    let (max_gap, cutoff) = timestamps
        .windows(2)
        .enumerate()
        .map(|(i, w)| (w[1] - w[0], i + 1))
        .max_by_key(|(gap, _)| *gap)?;

    // Only treat it as a base/user boundary if the gap is at least 1 hour.
    if max_gap > 3600 {
        Some(cutoff)
    } else {
        None
    }
}

pub fn to_markdown(result: &DockerAnalyzeResult, skip_base: bool, lang: Lang) -> String {
    let summary = result.summary();
    let mut md = String::with_capacity(4096);
    // Localization helpers: `t` = static catalog entry, `f` = entry with
    // `{0}`,`{1}`,… placeholders. Markdown punctuation stays in code so the
    // English output is byte-identical to the pre-i18n version.
    let t = |k: &str| -> String { i18n::tr(lang, k).to_string() };
    let f = |k: &str, args: &[&str]| -> String { i18n::fill(i18n::tr(lang, k), args) };

    // Always detect the base/user boundary for stats; only skip layers when requested.
    let user_layer_start = find_user_layer_start(result);
    let auto_start = if skip_base { user_layer_start } else { None };
    let is_base_layer = |i: usize| -> bool { auto_start.is_some_and(|start| i < start) };

    // Title
    md.push_str(&format!("# {}: {}\n\n", t("md.title"), result.name));

    // A fallback result (registry unavailable) must say so before anything
    // else: every number below describes the image as of that time.
    if let Some(as_of) = &result.stale_as_of {
        md.push_str(&format!("> ⚠️ {}\n\n", f("md.stale", &[as_of])));
    }

    // Risk tags
    if !result.tags.is_empty() {
        md.push_str(&format!("## {}\n\n", t("md.risktags")));
        for tag in &result.tags {
            md.push_str(&format!("- `{}`\n", tag));
        }
        md.push('\n');
    }

    // Image info table
    md.push_str(&format!("## {}\n\n", t("md.imginfo")));
    md.push_str(&format!(
        "| {} | {} |\n|-------|-------|\n",
        t("md.col.field"),
        t("md.col.value")
    ));
    let arch_display = if result.supported_archs.is_empty() {
        result.arch.clone()
    } else {
        format!("{} ({})", result.arch, result.supported_archs.join(", "))
    };
    md.push_str(&format!("| {} | {} |\n", t("md.f.arch"), arch_display));
    md.push_str(&format!("| {} | {} |\n", t("md.f.os"), result.os));
    if !result.base_os.is_empty() {
        md.push_str(&format!("| {} | {} |\n", t("md.f.baseos"), result.base_os));
    }
    // Runtime libc fingerprint (glibc / musl / static) + version requirement
    // and how it compares against the base OS's libc. Always shown when the
    // ELF probe successfully classified the entrypoint.
    if !result.runtime_compat.libc.is_empty() {
        let rc = &result.runtime_compat;
        // Lead with the resolved binary path (incl. "(via wrapper)" when
        // we unwrapped a shell entrypoint) so the user sees which file
        // was actually inspected.
        let mut cell = String::new();
        if !rc.entrypoint.is_empty() {
            cell.push_str(&format!("`{}`: ", rc.entrypoint));
        }
        cell.push_str(&rc.libc);
        if !rc.required_glibc.is_empty() {
            cell.push_str(&format!(" (needs {})", rc.required_glibc));
        }
        if !rc.os_glibc.is_empty() {
            cell.push_str(&format!(" → host glibc {}", rc.os_glibc));
        }
        let status = match rc.issue.as_str() {
            "" => t("md.runtimelibc.ok"),
            "glibc-too-old" => t("md.runtimelibc.tooold"),
            "glibc-on-musl" => t("md.runtimelibc.glibconmusl"),
            "musl-on-glibc" => t("md.runtimelibc.muslonglibc"),
            _ => String::new(),
        };
        if !status.is_empty() {
            cell.push_str(&format!(" — {status}"));
        }
        md.push_str(&format!("| {} | {} |\n", t("md.f.runtimelibc"), cell));
    }
    if !result.user.is_empty() {
        md.push_str(&format!("| {} | {} |\n", t("md.f.user"), result.user));
    }
    md.push_str(&format!(
        "| {} | {} |\n",
        t("md.f.csize"),
        ByteSize(result.size)
    ));
    md.push_str(&format!(
        "| {} | {} |\n",
        t("md.f.usize"),
        ByteSize(result.total_size)
    ));
    md.push_str(&format!(
        "| {} | {} / 127 |\n",
        t("md.f.layers"),
        result.layers.len()
    ));
    md.push_str(&format!("| {} | {}% |\n", t("md.f.eff"), summary.score));
    md.push_str(&format!(
        "| {} | {} ({:.1}%) |\n",
        t("md.f.wasted"),
        ByteSize(summary.wasted_size),
        summary.wasted_percent * 100.0
    ));
    if let Some(start) = user_layer_start {
        let base_layers = &result.layers[..start];
        let base_size: u64 = base_layers.iter().map(|l| l.size).sum();
        let base_unpack: u64 = base_layers.iter().map(|l| l.unpack_size).sum();
        md.push_str(&format!(
            "| {} | {} |\n",
            t("md.f.baseimg"),
            f(
                "md.baseimg.val",
                &[
                    &start.to_string(),
                    &ByteSize(base_size).to_string(),
                    &ByteSize(base_unpack).to_string(),
                ]
            )
        ));
    }
    md.push('\n');

    // Reconstructed Dockerfile — when skip_base is active, only show user layers
    let dockerfile = if let Some(start) = auto_start {
        result.layers[start..]
            .iter()
            .filter_map(|l| cmd_to_dockerfile_line(&l.cmd))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        result.dockerfile.clone()
    };
    if !dockerfile.is_empty() {
        md.push_str(&format!("## {}\n\n", t("md.dockerfile")));
        md.push_str("```dockerfile\n");
        md.push_str(&dockerfile);
        md.push_str("\n```\n\n");
    }

    // Environment variables
    if !result.envs.is_empty() {
        md.push_str(&format!("## {}\n\n", t("md.envs")));
        for env in &result.envs {
            md.push_str(&format!("- `{}`\n", env));
        }
        md.push('\n');
    }

    // Labels
    if !result.labels.is_empty() {
        md.push_str(&format!("## {}\n\n", t("md.labels")));
        for label in &result.labels {
            md.push_str(&format!("- `{}`\n", label));
        }
        md.push('\n');
    }

    // Wasted space
    if !summary.wasted_list.is_empty() {
        md.push_str(&format!("## {}\n\n", t("md.wasted")));
        md.push_str(&t("md.wasted.desc"));
        md.push_str("\n\n");
        md.push_str(&format!(
            "| {} | {} | {} | {} |\n|------|-------------|-------------|-------------|\n",
            t("md.col.path"),
            t("md.col.totwasted"),
            t("md.col.occ"),
            t("md.col.layers")
        ));
        for item in summary.wasted_list.iter().take(20) {
            md.push_str(&format!(
                "| `{}` | {} | {} | {} |\n",
                item.path,
                ByteSize(item.total_size),
                item.count,
                waste_changes(&item.changes, lang)
            ));
        }
        md.push('\n');
    }

    // Large modified files
    if !result.big_modified_file_list.is_empty() {
        md.push_str(&format!("## {}\n\n", t("md.bigfiles")));
        md.push_str(&format!(
            "| {} | {} | {} | {} |\n|------|------|------|-------|\n",
            t("md.col.path"),
            t("md.col.size"),
            t("md.col.mode"),
            t("md.col.owner")
        ));
        for item in &result.big_modified_file_list {
            md.push_str(&format!(
                "| `{}` | {} | `{}` | {}:{} |\n",
                item.path,
                ByteSize(item.size),
                item.mode,
                item.uid,
                item.gid,
            ));
        }
        md.push('\n');
    }

    // Security warnings
    if !result.sensitive_files.is_empty() {
        md.push_str(&format!("## {}\n\n", t("md.secwarn")));
        md.push_str(&format!(
            "| {} | {} | {} | {} |\n|------|------|-------|------|\n",
            t("md.col.path"),
            t("md.col.size"),
            t("md.col.layer"),
            t("md.col.risk")
        ));
        const SECRETS_LIMIT: usize = 30;
        for item in result.sensitive_files.iter().take(SECRETS_LIMIT) {
            md.push_str(&format!(
                "| `{}` | {} | {} | {} |\n",
                item.path,
                ByteSize(item.size),
                f("md.cell.layer", &[&(item.layer_index + 1).to_string()]),
                item.reason,
            ));
        }
        if result.sensitive_files.len() > SECRETS_LIMIT {
            md.push_str(&format!(
                "\n{}\n",
                f(
                    "md.secmore",
                    &[&(result.sensitive_files.len() - SECRETS_LIMIT).to_string()]
                )
            ));
        }
        md.push('\n');
    }

    // Optimization recommendations (derived from the analysis data)
    if !result.recommendations.is_empty() {
        md.push_str(&format!("## {}\n\n", t("md.recs")));
        for r in &result.recommendations {
            md.push_str(&format!(
                "### {} {} — {} ({}){}\n\n",
                icon(&r.severity),
                r.title,
                i18n::tr(lang, &format!("md.cat.{}", r.category)),
                i18n::tr(lang, &format!("sev.{}", r.severity)),
                if r.heuristic {
                    t("md.heuristic")
                } else {
                    String::new()
                },
            ));
            md.push_str(&r.detail);
            md.push_str("\n\n");
            if r.est_saved_bytes > 0 {
                md.push_str(&format!(
                    "- **{}:** {}\n",
                    t("md.potsavings"),
                    ByteSize(r.est_saved_bytes)
                ));
            }
            if !r.dockerfile_hint.is_empty() {
                md.push_str(&format!("- **{}:** {}\n", t("md.fix"), r.dockerfile_hint));
            }
            if !r.paths.is_empty() {
                md.push_str(&format!("- **{}:**\n", t("md.affected")));
                for p in &r.paths {
                    md.push_str(&format!("  - `{}`\n", p));
                }
            }
            md.push('\n');
        }
    }

    // Per-layer breakdown
    let skipped = (0..result.layers.len())
        .filter(|&i| is_base_layer(i))
        .count();
    let skip_note = if skipped > 0 {
        f("md.skipnote", &[&skipped.to_string()])
    } else {
        String::new()
    };
    md.push_str(&format!(
        "## {} ({} {}{})\n\n",
        t("md.layers"),
        result.layers.len(),
        t("md.total"),
        skip_note
    ));

    for (i, layer) in result.layers.iter().enumerate() {
        if is_base_layer(i) {
            continue;
        }
        // Empty (metadata-only) layers — ARG/ENV/LABEL/WORKDIR/… — carry no
        // files. A full heading + command + "empty" note per layer (often a
        // long consecutive run of them) bloats the report and the AI payload,
        // so collapse each to a single compact line.
        if layer.empty {
            let mut line = f("md.emptyline", &[&(i + 1).to_string()]);
            if !layer.cmd.is_empty() {
                line.push_str(&format!(" · `{}`", clip(&layer.cmd, 300)));
            }
            md.push_str(&line);
            md.push('\n');
            continue;
        }

        md.push_str(&format!(
            "### {}\n\n",
            f(
                "md.layerhead",
                &[
                    &(i + 1).to_string(),
                    &ByteSize(layer.size).to_string(),
                    &ByteSize(layer.unpack_size).to_string(),
                ]
            )
        ));

        if !layer.cmd.is_empty() {
            md.push_str(&format!(
                "**{}:** `{}`\n\n",
                t("md.command"),
                clip(&layer.cmd, 300)
            ));
        }

        if let Some(tree) = result.file_tree_list.get(i) {
            let mut files = LayerFiles {
                added: vec![],
                modified: vec![],
                removed: vec![],
            };
            flatten_tree(tree, "", &mut files);

            let has_changes =
                !files.added.is_empty() || !files.modified.is_empty() || !files.removed.is_empty();
            if !has_changes {
                md.push_str(&t("md.nochanges"));
                md.push_str("\n\n");
                continue;
            }

            md.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n\
                 |--------|------|------|------|-------|\n",
                t("md.col.change"),
                t("md.col.path"),
                t("md.col.size"),
                t("md.col.mode"),
                t("md.col.owner"),
            ));

            push_file_rows(&mut md, &t("md.op.removed"), &mut files.removed, lang);
            push_file_rows(&mut md, &t("md.op.modified"), &mut files.modified, lang);
            push_file_rows(&mut md, &t("md.op.added"), &mut files.added, lang);

            md.push('\n');
        }
    }

    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::ImageLayer;
    use crate::recommend::Recommendation;

    #[test]
    fn waste_changes_name_both_layers() {
        let change = |written_in, layer, removed| WasteChange {
            written_in,
            layer,
            removed,
        };
        assert_eq!(
            waste_changes(&[change(Some(0), 2, true)], Lang::En),
            "L1 → L3 deleted"
        );
        // Cached by 0.12.0: no writing layer.
        assert_eq!(
            waste_changes(&[change(None, 1, false)], Lang::Zh),
            "L2 覆盖"
        );
        let many: Vec<WasteChange> = (1..6).map(|l| change(Some(l - 1), l, false)).collect();
        assert_eq!(
            waste_changes(&many, Lang::En),
            "L1 → L2 overwritten, L2 → L3 overwritten, L3 → L4 overwritten, …"
        );
    }

    #[test]
    fn clip_never_splits_a_character() {
        assert_eq!(clip("RUN echo hi", 300), "RUN echo hi");
        // "中" is 3 bytes: 4 bytes in is the middle of the second one.
        assert_eq!(clip("中文命令", 4), "中…");
        // 10 ASCII bytes, then 3-byte characters: byte 300 falls inside one.
        let cmd = format!("RUN echo 1{}", "中".repeat(200));
        assert!(!cmd.is_char_boundary(300));
        assert!(to_markdown_cmd_ok(&cmd));
    }

    /// Rendering a layer whose command crosses the 300-byte cut inside a
    /// multi-byte character used to panic.
    fn to_markdown_cmd_ok(cmd: &str) -> bool {
        let result = DockerAnalyzeResult {
            layers: vec![
                ImageLayer {
                    cmd: cmd.to_string(),
                    ..Default::default()
                },
                ImageLayer {
                    cmd: cmd.to_string(),
                    empty: true,
                    ..Default::default()
                },
            ],
            file_tree_list: vec![vec![], vec![]],
            ..Default::default()
        };
        to_markdown(&result, false, Lang::En).contains("中…")
    }

    fn files(prefix: &str, count: u64, op: Op) -> Vec<FileTreeItem> {
        (1..=count)
            .map(|i| FileTreeItem {
                name: format!("{prefix}{i:03}"),
                size: i,
                mode: "-rw-r--r--".to_string(),
                op: op.clone(),
                ..Default::default()
            })
            .collect()
    }

    fn one_layer_result(children: Vec<FileTreeItem>) -> DockerAnalyzeResult {
        DockerAnalyzeResult {
            name: "fixture:latest".to_string(),
            layers: vec![ImageLayer {
                created: "2024-01-01T00:00:00Z".to_string(),
                digest: "sha256:aaa".to_string(),
                cmd: "RUN chown -R app /srv".to_string(),
                ..Default::default()
            }],
            file_tree_list: vec![vec![FileTreeItem {
                name: "srv".to_string(),
                children,
                ..Default::default()
            }]],
            ..Default::default()
        }
    }

    fn rows(md: &str, op: &str) -> usize {
        let prefix = format!("| {op} | `");
        md.lines().filter(|l| l.starts_with(&prefix)).count()
    }

    #[test]
    fn summary_is_short_and_carries_the_verdict() {
        let mut result = one_layer_result(files("big", 8, Op::None));
        result.total_size = 1000;
        result.recommendations = (0..SUMMARY_RECS + 3)
            .map(|i| Recommendation {
                id: format!("r{i}"),
                severity: "low".to_string(),
                category: "size".to_string(),
                title: format!("rec {i}"),
                ..Default::default()
            })
            .collect();
        let md = to_summary_markdown(&result, Lang::En, Some("**CI gate: failed**"));

        assert!(md.starts_with("### diving · `fixture:latest`\n\n**CI gate: failed**"));
        // Only the largest files, biggest first.
        assert_eq!(md.matches("| `srv/big").count(), SUMMARY_TOP_FILES);
        assert!(md.contains("| 8 B | 1 | `srv/big008` |"));
        // The recommendations past the cap are counted, not listed.
        assert_eq!(md.matches("- 🟡 **rec").count(), SUMMARY_RECS);
        assert!(md.contains("- +3 more in the full report"));

        let plain = to_summary_markdown(&result, Lang::En, None);
        assert!(!plain.contains("CI gate"));
    }

    #[test]
    fn layer_file_rows_are_capped_per_change_kind() {
        let mut children = files("del", 55, Op::Removed);
        children.extend(files("mod", 60, Op::Modified));
        children.extend(files("add", 3, Op::None));
        let md = to_markdown(&one_layer_result(children), false, Lang::En);

        assert_eq!(rows(&md, "Removed"), LAYER_FILES_LIMIT);
        assert_eq!(rows(&md, "Modified"), LAYER_FILES_LIMIT);
        assert_eq!(rows(&md, "Added"), 3);
        assert_eq!(md.matches("| … | *5 more files not shown* |").count(), 1);
        assert_eq!(md.matches("| … | *10 more files not shown* |").count(), 1);

        // The cap keeps the largest files: mod011..=mod060 stay, mod010 goes.
        assert!(md.contains("| Modified | `srv/mod060` |"));
        assert!(md.contains("| Modified | `srv/mod011` |"));
        assert!(!md.contains("`srv/mod010`"));
    }

    #[test]
    fn layer_file_rows_under_the_cap_are_all_listed() {
        let mut children = files("del", 2, Op::Removed);
        children.extend(files("mod", LAYER_FILES_LIMIT as u64, Op::Modified));
        let md = to_markdown(&one_layer_result(children), false, Lang::En);

        assert_eq!(rows(&md, "Removed"), 2);
        assert_eq!(rows(&md, "Modified"), LAYER_FILES_LIMIT);
        assert!(!md.contains("more files not shown"));
    }
}
