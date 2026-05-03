use bytesize::ByteSize;
use chrono::DateTime;
use std::cmp::Reverse;

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

struct LayerFiles {
    added: Vec<(String, u64)>,
    modified: Vec<(String, u64)>,
    removed: Vec<(String, u64)>,
}

fn flatten_tree(items: &[FileTreeItem], prefix: &str, out: &mut LayerFiles) {
    for item in items {
        let path = if prefix.is_empty() {
            item.name.clone()
        } else {
            format!("{}/{}", prefix, item.name)
        };
        if item.children.is_empty() {
            match item.op {
                Op::Removed => out.removed.push((path, item.size)),
                Op::Modified => out.modified.push((path, item.size)),
                _ => out.added.push((path, item.size)),
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

pub fn to_markdown(result: &DockerAnalyzeResult, skip_base: bool) -> String {
    let summary = result.summary();
    let mut md = String::with_capacity(4096);

    // Always detect the base/user boundary for stats; only skip layers when requested.
    let user_layer_start = find_user_layer_start(result);
    let auto_start = if skip_base { user_layer_start } else { None };
    let is_base_layer = |i: usize| -> bool { auto_start.is_some_and(|start| i < start) };

    // Title
    md.push_str(&format!("# Image Analysis: {}\n\n", result.name));

    // Image info table
    md.push_str("## Image Info\n\n");
    md.push_str("| Field | Value |\n|-------|-------|\n");
    let arch_display = if result.supported_archs.is_empty() {
        result.arch.clone()
    } else {
        format!("{} ({})", result.arch, result.supported_archs.join(", "))
    };
    md.push_str(&format!("| Architecture | {} |\n", arch_display));
    md.push_str(&format!("| OS | {} |\n", result.os));
    if !result.user.is_empty() {
        md.push_str(&format!("| User | {} |\n", result.user));
    }
    md.push_str(&format!(
        "| Compressed size | {} |\n",
        ByteSize(result.size)
    ));
    md.push_str(&format!(
        "| Uncompressed size | {} |\n",
        ByteSize(result.total_size)
    ));
    md.push_str(&format!("| Efficiency | {}% |\n", summary.score));
    md.push_str(&format!(
        "| Wasted space | {} ({:.1}%) |\n",
        ByteSize(summary.wasted_size),
        summary.wasted_percent * 100.0
    ));
    if let Some(start) = user_layer_start {
        let base_layers = &result.layers[..start];
        let base_size: u64 = base_layers.iter().map(|l| l.size).sum();
        let base_unpack: u64 = base_layers.iter().map(|l| l.unpack_size).sum();
        md.push_str(&format!(
            "| Base image | {} layers, {} compressed / {} uncompressed |\n",
            start,
            ByteSize(base_size),
            ByteSize(base_unpack),
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
        md.push_str("## Dockerfile (reconstructed)\n\n");
        md.push_str("```dockerfile\n");
        md.push_str(&dockerfile);
        md.push_str("\n```\n\n");
    }

    // Environment variables
    if !result.envs.is_empty() {
        md.push_str("## Environment Variables\n\n");
        for env in &result.envs {
            md.push_str(&format!("- `{}`\n", env));
        }
        md.push('\n');
    }

    // Labels
    if !result.labels.is_empty() {
        md.push_str("## Labels\n\n");
        for label in &result.labels {
            md.push_str(&format!("- `{}`\n", label));
        }
        md.push('\n');
    }

    // Wasted space
    if !summary.wasted_list.is_empty() {
        md.push_str("## Wasted Space\n\n");
        md.push_str("Files overwritten or deleted in a later layer (top 20 by size):\n\n");
        md.push_str(
            "| Path | Total Wasted | Occurrences |\n|------|-------------|-------------|\n",
        );
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
        md.push_str("## Large Files Added in Recent Layers\n\n");
        md.push_str("| Path | Size |\n|------|------|\n");
        for item in &result.big_modified_file_list {
            md.push_str(&format!("| `{}` | {} |\n", item.path, ByteSize(item.size)));
        }
        md.push('\n');
    }

    // Security warnings
    if !result.sensitive_files.is_empty() {
        md.push_str("## ⚠️ Security Warnings (Potential Secrets)\n\n");
        md.push_str("| Path | Size | Layer | Risk |\n|------|------|-------|------|\n");
        const SECRETS_LIMIT: usize = 30;
        for item in result.sensitive_files.iter().take(SECRETS_LIMIT) {
            md.push_str(&format!(
                "| `{}` | {} | Layer {} | {} |\n",
                item.path,
                ByteSize(item.size),
                item.layer_index + 1,
                item.reason,
            ));
        }
        if result.sensitive_files.len() > SECRETS_LIMIT {
            md.push_str(&format!(
                "\n*… and {} more — see JSON output for the full list.*\n",
                result.sensitive_files.len() - SECRETS_LIMIT
            ));
        }
        md.push('\n');
    }

    // Per-layer breakdown
    let skipped = (0..result.layers.len())
        .filter(|&i| is_base_layer(i))
        .count();
    let skip_note = if skipped > 0 {
        format!(" — {} base layers auto-detected and hidden", skipped)
    } else {
        String::new()
    };
    md.push_str(&format!(
        "## Layers ({} total{})\n\n",
        result.layers.len(),
        skip_note
    ));

    for (i, layer) in result.layers.iter().enumerate() {
        if is_base_layer(i) {
            continue;
        }
        md.push_str(&format!(
            "### Layer {} — {} compressed / {} uncompressed\n\n",
            i + 1,
            ByteSize(layer.size),
            ByteSize(layer.unpack_size),
        ));

        if !layer.cmd.is_empty() {
            let cmd = if layer.cmd.len() > 300 {
                format!("{}…", &layer.cmd[..300])
            } else {
                layer.cmd.clone()
            };
            md.push_str(&format!("**Command:** `{}`\n\n", cmd));
        }

        if layer.empty {
            md.push_str("*Empty layer — no file changes.*\n\n");
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
                md.push_str("*No file changes recorded for this layer.*\n\n");
                continue;
            }

            md.push_str("| Change | Path | Size |\n|--------|------|------|\n");

            for (path, size) in &files.removed {
                md.push_str(&format!("| Removed | `{}` | {} |\n", path, ByteSize(*size)));
            }
            for (path, size) in &files.modified {
                md.push_str(&format!(
                    "| Modified | `{}` | {} |\n",
                    path,
                    ByteSize(*size)
                ));
            }

            // Sort added files by size descending; cap at 50 to keep output readable
            files.added.sort_by_key(|(_, s)| Reverse(*s));
            const ADDED_LIMIT: usize = 50;
            for (path, size) in files.added.iter().take(ADDED_LIMIT) {
                md.push_str(&format!("| Added | `{}` | {} |\n", path, ByteSize(*size)));
            }
            if files.added.len() > ADDED_LIMIT {
                md.push_str(&format!(
                    "| … | *{} more files not shown* | |\n",
                    files.added.len() - ADDED_LIMIT
                ));
            }

            md.push('\n');
        }
    }

    md
}
