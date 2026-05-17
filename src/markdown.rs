use bytesize::ByteSize;
use chrono::DateTime;
use std::cmp::Reverse;

use crate::i18n::{self, Lang};
use crate::image::{DockerAnalyzeResult, FileTreeItem, Op};

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
            "| {} | {} | {} |\n|------|-------------|-------------|\n",
            t("md.col.path"),
            t("md.col.totwasted"),
            t("md.col.occ")
        ));
        for item in summary.wasted_list.iter().take(20) {
            md.push_str(&format!(
                "| `{}` | {} | {} |\n",
                item.path,
                ByteSize(item.total_size),
                item.count
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
        let icon = |sev: &str| match sev {
            "high" => "🔴",
            "medium" => "🟠",
            "low" => "🟡",
            _ => "ℹ️",
        };
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
            let cmd = if layer.cmd.len() > 300 {
                format!("{}…", &layer.cmd[..300])
            } else {
                layer.cmd.clone()
            };
            md.push_str(&format!("**{}:** `{}`\n\n", t("md.command"), cmd));
        }

        if layer.empty {
            md.push_str(&t("md.emptylayer"));
            md.push_str("\n\n");
            continue;
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

            for e in &files.removed {
                md.push_str(&format!(
                    "| {} | `{}` | {} | `{}` | {}:{} |\n",
                    t("md.op.removed"),
                    e.path,
                    ByteSize(e.size),
                    e.mode,
                    e.uid,
                    e.gid,
                ));
            }
            for e in &files.modified {
                md.push_str(&format!(
                    "| {} | `{}` | {} | `{}` | {}:{} |\n",
                    t("md.op.modified"),
                    e.path,
                    ByteSize(e.size),
                    e.mode,
                    e.uid,
                    e.gid,
                ));
            }

            // Sort added files by size descending; cap at 50 to keep output readable
            files.added.sort_by_key(|e| Reverse(e.size));
            const ADDED_LIMIT: usize = 50;
            for e in files.added.iter().take(ADDED_LIMIT) {
                md.push_str(&format!(
                    "| {} | `{}` | {} | `{}` | {}:{} |\n",
                    t("md.op.added"),
                    e.path,
                    ByteSize(e.size),
                    e.mode,
                    e.uid,
                    e.gid,
                ));
            }
            if files.added.len() > ADDED_LIMIT {
                md.push_str(&format!(
                    "| … | {} | | | |\n",
                    f(
                        "md.moremfiles",
                        &[&(files.added.len() - ADDED_LIMIT).to_string()]
                    )
                ));
            }

            md.push('\n');
        }
    }

    md
}
