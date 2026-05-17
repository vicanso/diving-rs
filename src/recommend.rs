//! Optimization advisor.
//!
//! Pure derived layer: turns the data already present in a
//! [`DockerAnalyzeResult`] into actionable recommendations across three
//! dimensions — image size, necessity of newly added files, and security.
//! It never re-fetches layers or changes collection; everything here is
//! computed from the analysis result that is already in memory.
//!
//! Limitations (documented on purpose): necessity checks are path/extension
//! heuristics, not runtime-reachability analysis, so they are flagged with
//! `heuristic = true`. Content-level secret scanning and CVE/package
//! vulnerability scanning are out of scope here because they need new data
//! collection.

use crate::image::{DockerAnalyzeResult, DockerAnalyzeSummary, FileTreeItem, Op};
use chrono::DateTime;
use serde::{Deserialize, Serialize};

pub const CATEGORY_SIZE: &str = "size";
pub const CATEGORY_NECESSITY: &str = "necessity";
pub const CATEGORY_SECURITY: &str = "security";

pub const SEVERITY_HIGH: &str = "high";
pub const SEVERITY_MEDIUM: &str = "medium";
pub const SEVERITY_LOW: &str = "low";
pub const SEVERITY_INFO: &str = "info";

/// One optimization suggestion derived from the analysis result.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    /// `size` | `necessity` | `security`
    pub category: String,
    /// `high` | `medium` | `low` | `info`
    pub severity: String,
    pub title: String,
    pub detail: String,
    /// Concrete Dockerfile-level fix, empty when there is no one-liner.
    pub dockerfile_hint: String,
    /// Bytes that could be reclaimed if acted on; 0 when not quantifiable.
    pub est_saved_bytes: u64,
    /// True when this is a path/extension heuristic that needs human judgment
    /// (cannot prove the file is truly unused at runtime).
    pub heuristic: bool,
    /// A bounded sample of affected paths (full list lives in the raw data).
    pub paths: Vec<String>,
}

const PATH_SAMPLE_LIMIT: usize = 8;

fn severity_rank(s: &str) -> u8 {
    match s {
        SEVERITY_HIGH => 0,
        SEVERITY_MEDIUM => 1,
        SEVERITY_LOW => 2,
        _ => 3,
    }
}

/// A leaf file collected from the per-layer file trees.
struct Leaf {
    path: String,
    size: u64,
    mode: String,
}

fn collect_leaves(items: &[FileTreeItem], prefix: &str, out: &mut Vec<Leaf>) {
    for item in items {
        let path = if prefix.is_empty() {
            item.name.clone()
        } else {
            format!("{}/{}", prefix, item.name)
        };
        if item.children.is_empty() {
            // Removed entries are whiteouts, not real content — skip them.
            if item.op != Op::Removed {
                out.push(Leaf {
                    path,
                    size: item.size,
                    mode: item.mode.clone(),
                });
            }
        } else {
            collect_leaves(&item.children, &path, out);
        }
    }
}

fn is_pkg_cache(path: &str) -> bool {
    path.starts_with("var/cache/apt/")
        || path.starts_with("var/lib/apt/lists/")
        || path.starts_with("var/cache/apk/")
        || path.starts_with("var/cache/yum/")
        || path.starts_with("var/cache/dnf/")
        || path.starts_with("var/cache/pacman/")
}

fn is_dev_artifact(path: &str) -> bool {
    let p = path;
    p.starts_with("node_modules/.cache/")
        || p.contains("/node_modules/.cache/")
        || p.starts_with(".git/")
        || p.contains("/.git/")
        || p.starts_with("target/debug/")
        || p.contains("/target/debug/")
        || p.starts_with("__pycache__/")
        || p.contains("/__pycache__/")
        || p.starts_with(".gradle/")
        || p.contains("/.gradle/")
        || p.starts_with(".m2/")
        || p.contains("/.m2/")
}

/// Build-time-only artifacts that almost never need to ship in a runtime image.
fn is_build_only(path: &str) -> bool {
    let f = path.rsplit('/').next().unwrap_or(path);
    f.ends_with(".a")
        || f.ends_with(".o")
        || f.ends_with(".la")
        || f.ends_with(".h")
        || f.ends_with(".hpp")
        || f.ends_with(".pyc")
        || f.ends_with(".pyo")
        || f.ends_with(".map") // JS source maps
}

fn is_doc_or_locale(path: &str) -> bool {
    path.starts_with("usr/share/doc/")
        || path.starts_with("usr/share/man/")
        || path.starts_with("usr/share/info/")
        || path.starts_with("usr/share/locale/")
}

fn is_log_or_temp(path: &str) -> bool {
    path.starts_with("var/log/")
        || path == "tmp"
        || path.starts_with("tmp/")
        || path.starts_with("var/tmp/")
        || path.ends_with(".log")
}

/// Compiler / build toolchain binaries that indicate a non-multi-stage build.
fn is_toolchain_binary(path: &str) -> bool {
    let f = path.rsplit('/').next().unwrap_or(path);
    let in_bin = path.starts_with("usr/bin/")
        || path.starts_with("usr/local/bin/")
        || path.starts_with("bin/");
    in_bin
        && matches!(
            f,
            "gcc" | "g++" | "cc" | "c++" | "ld" | "as" | "make" | "cmake" | "clang" | "rustc"
        )
}

fn mode_chars(mode: &str) -> Option<Vec<char>> {
    if mode.len() >= 10 {
        Some(mode.chars().take(10).collect())
    } else {
        None
    }
}

fn is_setuid_or_setgid(mode: &str) -> bool {
    if let Some(c) = mode_chars(mode) {
        // owner-exec slot (idx 3) or group-exec slot (idx 6) showing s/S
        matches!(c[3], 's' | 'S') || matches!(c[6], 's' | 'S')
    } else {
        false
    }
}

fn is_world_writable_file(mode: &str) -> bool {
    if let Some(c) = mode_chars(mode) {
        // regular file, "other" write bit set, not a sticky-bit dir
        c[0] == '-' && c[8] == 'w'
    } else {
        false
    }
}

/// Looks for an AWS access key id (`AKIA` + 16 uppercase alphanumerics).
fn has_aws_access_key(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    while let Some(pos) = s[i..].find("AKIA") {
        let start = i + pos;
        let tail = &bytes[start + 4..];
        if tail.len() >= 16
            && tail[..16]
                .iter()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            return true;
        }
        i = start + 4;
        if i >= s.len() {
            break;
        }
    }
    false
}

/// Truncate to `n` chars (char-boundary safe) with an ellipsis.
fn clip(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
    } else {
        s.to_string()
    }
}

/// Shannon entropy (bits/byte) of a string.
fn shannon_entropy(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    let mut counts = std::collections::HashMap::new();
    for b in s.bytes() {
        *counts.entry(b).or_insert(0u32) += 1;
    }
    let len = s.len() as f64;
    counts
        .values()
        .map(|&c| {
            let p = c as f64 / len;
            -p * p.log2()
        })
        .sum()
}

/// Classify a value that looks like a hardcoded credential. Returns a short
/// label or None — never echoes the value itself.
fn token_kind(value: &str) -> Option<&'static str> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    if has_aws_access_key(v) {
        return Some("AWS access key");
    }
    if v.contains("-----BEGIN") {
        return Some("private key block");
    }
    // GitHub: ghp_/gho_/ghu_/ghs_/ghr_ + >=20 base62
    for p in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"] {
        if let Some(idx) = v.find(p) {
            let rest: String = v[idx + p.len()..].chars().take(40).collect();
            if rest.len() >= 20 && rest.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                return Some("GitHub token");
            }
        }
    }
    // Slack: xox[baprs]-...
    let b = v.as_bytes();
    if v.len() >= 15
        && v.starts_with("xox")
        && b.get(3)
            .map(|c| matches!(c, b'b' | b'a' | b'p' | b'r' | b's'))
            .unwrap_or(false)
        && b.get(4) == Some(&b'-')
    {
        return Some("Slack token");
    }
    // JWT: three non-empty base64url segments, starts with eyJ
    if v.starts_with("eyJ") {
        let parts: Vec<&str> = v.split('.').collect();
        if parts.len() == 3
            && parts.iter().all(|p| {
                !p.is_empty()
                    && p.chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '='))
            })
        {
            return Some("JWT");
        }
    }
    // Generic high-entropy token. Require BOTH upper and lower letters so that
    // lowercase hex/base32 checksums (sha256 sums etc.) are NOT flagged.
    let has_upper = v.chars().any(|c| c.is_ascii_uppercase());
    let has_lower = v.chars().any(|c| c.is_ascii_lowercase());
    let has_digit = v.chars().any(|c| c.is_ascii_digit());
    if v.len() >= 40
        && !v.contains(char::is_whitespace)
        && has_upper
        && has_lower
        && has_digit
        && shannon_entropy(v) > 4.3
    {
        return Some("high-entropy token");
    }
    None
}

/// Others-read bit set (`ls -l` slot 7).
fn is_world_readable(mode: &str) -> bool {
    mode_chars(mode).map(|c| c[7] == 'r').unwrap_or(false)
}

/// Editor / OS junk that belongs in `.dockerignore`.
fn is_editor_os_junk(path: &str) -> bool {
    let f = path.rsplit('/').next().unwrap_or(path);
    f == ".DS_Store"
        || f == "Thumbs.db"
        || f == "desktop.ini"
        || f.ends_with(".swp")
        || f.ends_with(".swo")
        || f.ends_with('~')
        || path.starts_with(".vscode/")
        || path.contains("/.vscode/")
        || path.starts_with(".idea/")
        || path.contains("/.idea/")
}

/// Index of the first user layer, found via the largest (>1h) gap between
/// consecutive layer timestamps. None when no clear base/user boundary.
fn find_base_layer_split(result: &DockerAnalyzeResult) -> Option<usize> {
    let ts: Vec<i64> = result
        .layers
        .iter()
        .filter_map(|l| DateTime::parse_from_rfc3339(&l.created).ok())
        .map(|d| d.timestamp())
        .collect();
    if ts.len() < 2 {
        return None;
    }
    let (gap, cut) = ts
        .windows(2)
        .enumerate()
        .map(|(i, w)| (w[1] - w[0], i + 1))
        .max_by_key(|(g, _)| *g)?;
    if gap > 3600 {
        Some(cut)
    } else {
        None
    }
}

/// Lint the reconstructed Dockerfile for size / cache anti-patterns.
fn lint_dockerfile(df: &str) -> Vec<String> {
    let mut issues: Vec<String> = Vec::new();
    let mut run_streak = 0u32;
    let mut max_streak = 0u32;
    for raw in df.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let lower = line.to_lowercase();
        let is_run = lower.starts_with("run ");
        if is_run {
            run_streak += 1;
            max_streak = max_streak.max(run_streak);
        } else {
            run_streak = 0;
        }
        let snip = clip(line, 80);

        if lower.starts_with("add ")
            && (lower.contains("http://")
                || lower.contains("https://")
                || lower.contains(".tar")
                || lower.contains(".tgz"))
        {
            // Skip the base-image rootfs bootstrap (`ADD <rootfs>.tar.* /`):
            // it always uses ADD by design and is not user-controlled.
            let body = line.split('#').next().unwrap_or(line);
            let is_rootfs_bootstrap =
                lower.contains("rootfs") || body.split_whitespace().last() == Some("/");
            if !is_rootfs_bootstrap {
                issues.push(format!(
                    "ADD with URL/tarball — prefer COPY or explicit download+extract: {snip}"
                ));
            }
        }
        if is_run
            && (lower.contains("apt-get install") || lower.contains("apt install"))
            && !lower.contains("rm -rf /var/lib/apt/lists")
        {
            issues.push(format!(
                "apt install without `rm -rf /var/lib/apt/lists/*` in the same RUN: {snip}"
            ));
        }
        if is_run && (lower.contains("apt-get upgrade") || lower.contains("dist-upgrade")) {
            issues.push(format!(
                "apt-get upgrade in a layer (non-reproducible, bloats image): {snip}"
            ));
        }
        if is_run && lower.contains("pip install") && !lower.contains("--no-cache-dir") {
            issues.push(format!("pip install without `--no-cache-dir`: {snip}"));
        }
        if is_run
            && (lower.contains("npm install")
                || lower.contains("npm i ")
                || lower.contains("yarn install"))
            && !lower.contains("cache clean")
            && !lower.contains("--production")
            && !lower.contains("npm ci")
        {
            issues.push(format!("npm/yarn install without cache cleanup: {snip}"));
        }
        if is_run && (lower.contains("chown -r") || lower.contains("chmod -r")) {
            issues.push(format!(
                "recursive chown/chmod in RUN duplicates the tree — use `COPY --chown`: {snip}"
            ));
        }
    }
    if max_streak >= 3 {
        issues.push(format!(
            "{max_streak} consecutive RUN instructions — merge with `&&` to cut layers"
        ));
    }
    issues
}

/// Detects a secret-looking `KEY=VALUE` assignment without echoing the value.
/// Returns the key name when it matches.
fn secret_env_key(entry: &str) -> Option<String> {
    let (key, value) = entry.split_once('=')?;
    if value.trim().is_empty() {
        return None;
    }
    let ku = key.to_uppercase();
    let sensitive = [
        "PASSWORD",
        "PASSWD",
        "SECRET",
        "TOKEN",
        "API_KEY",
        "APIKEY",
        "ACCESS_KEY",
        "ACCESS_TOKEN",
        "PRIVATE_KEY",
        "CREDENTIAL",
        "AUTH",
    ];
    if sensitive.iter().any(|s| ku.contains(s)) {
        return Some(key.to_string());
    }
    if token_kind(value).is_some() {
        return Some(key.to_string());
    }
    None
}

fn truncate_sample(mut paths: Vec<String>) -> (Vec<String>, usize) {
    let total = paths.len();
    if total > PATH_SAMPLE_LIMIT {
        paths.truncate(PATH_SAMPLE_LIMIT);
    }
    (paths, total)
}

fn more_note(shown: usize, total: usize) -> String {
    if total > shown {
        format!(" (+{} more)", total - shown)
    } else {
        String::new()
    }
}

/// Produce all recommendations for an analysis result, ordered by severity
/// (high → info) so the most important items render first.
pub fn build_recommendations(result: &DockerAnalyzeResult) -> Vec<Recommendation> {
    let mut out: Vec<Recommendation> = Vec::new();

    let mut leaves: Vec<Leaf> = Vec::new();
    for tree in &result.file_tree_list {
        collect_leaves(tree, "", &mut leaves);
    }

    // ---- SIZE ---------------------------------------------------------------

    // `summary()` divides by total_size; skip on degenerate/empty results.
    let summary = if result.total_size > 0 {
        result.summary()
    } else {
        DockerAnalyzeSummary::default()
    };
    if summary.wasted_size > 0 {
        let severity = if summary.wasted_percent > 0.10 {
            SEVERITY_HIGH
        } else {
            SEVERITY_MEDIUM
        };
        let paths: Vec<String> = summary
            .wasted_list
            .iter()
            .take(PATH_SAMPLE_LIMIT)
            .map(|w| w.path.clone())
            .collect();
        out.push(Recommendation {
            category: CATEGORY_SIZE.into(),
            severity: severity.into(),
            title: "Reclaim wasted space".into(),
            detail: format!(
                "{} ({:.1}% of the image) is occupied by files that a later \
                 layer overwrites or deletes. Because each layer is immutable, \
                 those bytes still ship.",
                bytesize::ByteSize(summary.wasted_size),
                summary.wasted_percent * 100.0
            ),
            dockerfile_hint: "Create and clean up the data in the *same* RUN instruction so \
                 the bytes never enter a layer."
                .into(),
            est_saved_bytes: summary.wasted_size,
            heuristic: false,
            paths,
        });
    }

    // Package manager cache.
    {
        let mut size = 0u64;
        let mut paths: Vec<String> = Vec::new();
        for l in &leaves {
            if is_pkg_cache(&l.path) {
                size += l.size;
                paths.push(l.path.clone());
            }
        }
        if !paths.is_empty() {
            let (sample, total) = truncate_sample(paths);
            out.push(Recommendation {
                category: CATEGORY_SIZE.into(),
                severity: SEVERITY_MEDIUM.into(),
                title: "Remove package manager cache".into(),
                detail: format!(
                    "{} of apt/apk/yum/dnf/pacman cache is baked into the image \
                     across {} file(s){}.",
                    bytesize::ByteSize(size),
                    total,
                    more_note(sample.len(), total)
                ),
                dockerfile_hint: "apt: `rm -rf /var/lib/apt/lists/*` in the same RUN; \
                     apk: `apk add --no-cache`; yum/dnf: `yum clean all`."
                    .into(),
                est_saved_bytes: size,
                heuristic: false,
                paths: sample,
            });
        }
    }

    // Development / build artifacts.
    {
        let mut size = 0u64;
        let mut paths: Vec<String> = Vec::new();
        for l in &leaves {
            if is_dev_artifact(&l.path) {
                size += l.size;
                paths.push(l.path.clone());
            }
        }
        if !paths.is_empty() {
            let (sample, total) = truncate_sample(paths);
            out.push(Recommendation {
                category: CATEGORY_SIZE.into(),
                severity: SEVERITY_MEDIUM.into(),
                title: "Exclude development artifacts".into(),
                detail: format!(
                    "{} of build/SCM artifacts ({} file(s){}) such as .git, \
                     build caches or debug output is shipped.",
                    bytesize::ByteSize(size),
                    total,
                    more_note(sample.len(), total)
                ),
                dockerfile_hint: "Add a .dockerignore and/or use a multi-stage build so these \
                     never reach the final stage."
                    .into(),
                est_saved_bytes: size,
                heuristic: false,
                paths: sample,
            });
        }
    }

    // High layer count.
    if result.layers.len() > 30 {
        out.push(Recommendation {
            category: CATEGORY_SIZE.into(),
            severity: SEVERITY_LOW.into(),
            title: "Reduce layer count".into(),
            detail: format!(
                "The image has {} layers (limit is 127). Many small layers add \
                 metadata overhead and slow pulls.",
                result.layers.len()
            ),
            dockerfile_hint: "Merge consecutive RUN instructions with `&&`.".into(),
            est_saved_bytes: 0,
            heuristic: false,
            paths: vec![],
        });
    }

    // Large files in recent layers (informational pointer to size hot spots).
    if !result.big_modified_file_list.is_empty() {
        let total_big: u64 = result.big_modified_file_list.iter().map(|f| f.size).sum();
        let mut sorted = result.big_modified_file_list.clone();
        sorted.sort_by_key(|b| std::cmp::Reverse(b.size));
        let paths: Vec<String> = sorted
            .iter()
            .take(PATH_SAMPLE_LIMIT)
            .map(|f| f.path.clone())
            .collect();
        out.push(Recommendation {
            category: CATEGORY_SIZE.into(),
            severity: SEVERITY_INFO.into(),
            title: "Large files added in recent layers".into(),
            detail: format!(
                "{} across {} file(s) was added in the most recent layers — \
                 review whether each is required at runtime.",
                bytesize::ByteSize(total_big),
                result.big_modified_file_list.len()
            ),
            dockerfile_hint: String::new(),
            est_saved_bytes: 0,
            heuristic: false,
            paths,
        });
    }

    // Editor / OS junk that should be in .dockerignore.
    {
        let mut size = 0u64;
        let mut paths: Vec<String> = Vec::new();
        for l in &leaves {
            if is_editor_os_junk(&l.path) {
                size += l.size;
                paths.push(l.path.clone());
            }
        }
        if !paths.is_empty() {
            let (sample, total) = truncate_sample(paths);
            out.push(Recommendation {
                category: CATEGORY_SIZE.into(),
                severity: SEVERITY_LOW.into(),
                title: "Editor/OS junk files".into(),
                detail: format!(
                    "{} of editor/OS junk ({} file(s){}) — .DS_Store, Thumbs.db, \
                     .vscode/.idea, vim swap files — is shipped.",
                    bytesize::ByteSize(size),
                    total,
                    more_note(sample.len(), total)
                ),
                dockerfile_hint: "Add these patterns to .dockerignore.".into(),
                est_saved_bytes: size,
                heuristic: false,
                paths: sample,
            });
        }
    }

    // Slimmer base image opportunity.
    {
        let bo = result.base_os.to_lowercase();
        let full_distro = (bo.contains("debian")
            || bo.contains("ubuntu")
            || bo.contains("centos")
            || bo.contains("red hat")
            || bo.contains("fedora")
            || bo.contains("amazon linux")
            || bo.contains("opensuse"))
            && !bo.contains("slim")
            && !bo.contains("alpine")
            && !bo.contains("distroless");
        if full_distro {
            let mut detail = format!(
                "Base OS is `{}`, a full distribution. A slimmer base cuts both \
                 image size and CVE surface substantially.",
                result.base_os
            );
            if let Some(cut) = find_base_layer_split(result) {
                let n = cut.min(result.layers.len());
                let base_sz: u64 = result.layers[..n].iter().map(|l| l.size).sum();
                detail.push_str(&format!(
                    " Detected base ≈ {} across {} layer(s).",
                    bytesize::ByteSize(base_sz),
                    n
                ));
            }
            out.push(Recommendation {
                category: CATEGORY_SIZE.into(),
                severity: SEVERITY_LOW.into(),
                title: "Consider a slimmer base image".into(),
                detail,
                dockerfile_hint: "Switch to a `-slim`, `alpine`, or distroless base \
                     (verify glibc / runtime needs first)."
                    .into(),
                est_saved_bytes: 0,
                heuristic: true,
                paths: vec![],
            });
        }
    }

    // Oversized single layer(s).
    {
        let total = result.size.max(1);
        let threshold = std::cmp::max(50_000_000u64, total * 30 / 100);
        let mut big: Vec<(usize, u64, String)> = Vec::new();
        for (i, l) in result.layers.iter().enumerate() {
            if !l.empty && l.size >= threshold {
                big.push((i, l.size, l.cmd.clone()));
            }
        }
        if !big.is_empty() {
            big.sort_by_key(|(_, s, _)| std::cmp::Reverse(*s));
            let worst = big[0].1;
            let sev = if worst * 2 >= total {
                SEVERITY_MEDIUM
            } else {
                SEVERITY_LOW
            };
            let paths: Vec<String> = big
                .iter()
                .take(PATH_SAMPLE_LIMIT)
                .map(|(i, s, c)| {
                    format!(
                        "Layer {} — {}: {}",
                        i + 1,
                        bytesize::ByteSize(*s),
                        clip(c, 80)
                    )
                })
                .collect();
            out.push(Recommendation {
                category: CATEGORY_SIZE.into(),
                severity: sev.into(),
                title: "Oversized layer(s)".into(),
                detail: format!(
                    "{} layer(s) each exceed {} (or 30% of the image). Slimming \
                     the dominant instruction has the biggest size impact.",
                    big.len(),
                    bytesize::ByteSize(threshold)
                ),
                dockerfile_hint: "Audit the command that builds this layer; remove \
                     caches/intermediate files within the same RUN."
                    .into(),
                est_saved_bytes: 0,
                heuristic: false,
                paths,
            });
        }
    }

    // Dockerfile anti-patterns (reconstructed from image history).
    {
        let issues = lint_dockerfile(&result.dockerfile);
        if !issues.is_empty() {
            let severity = if issues.iter().any(|i| i.contains("apt install without")) {
                SEVERITY_MEDIUM
            } else {
                SEVERITY_LOW
            };
            let (sample, total) = truncate_sample(issues);
            out.push(Recommendation {
                category: CATEGORY_SIZE.into(),
                severity: severity.into(),
                title: "Dockerfile anti-patterns".into(),
                detail: format!(
                    "{} build-instruction issue(s){} that inflate image size or \
                     break layer caching (reconstructed from history — verify \
                     against the real Dockerfile).",
                    total,
                    more_note(sample.len(), total)
                ),
                dockerfile_hint: "Apply the fix noted inline on each item below.".into(),
                est_saved_bytes: 0,
                heuristic: true,
                paths: sample,
            });
        }
    }

    // ---- NECESSITY (heuristic) ---------------------------------------------

    {
        let mut size = 0u64;
        let mut paths: Vec<String> = Vec::new();
        for l in &leaves {
            if is_build_only(&l.path) {
                size += l.size;
                paths.push(l.path.clone());
            }
        }
        if !paths.is_empty() {
            let (sample, total) = truncate_sample(paths);
            out.push(Recommendation {
                category: CATEGORY_NECESSITY.into(),
                severity: SEVERITY_LOW.into(),
                title: "Build-only files in runtime image".into(),
                detail: format!(
                    "{} of likely build-time-only files ({} file(s){}) — static \
                     libs (.a/.o), headers (.h), .pyc, JS source maps. Verify \
                     they are not loaded at runtime before stripping.",
                    bytesize::ByteSize(size),
                    total,
                    more_note(sample.len(), total)
                ),
                dockerfile_hint: "Move compilation to a builder stage and copy only runtime \
                     outputs into the final stage."
                    .into(),
                est_saved_bytes: size,
                heuristic: true,
                paths: sample,
            });
        }
    }

    {
        let mut size = 0u64;
        let mut paths: Vec<String> = Vec::new();
        for l in &leaves {
            if is_doc_or_locale(&l.path) {
                size += l.size;
                paths.push(l.path.clone());
            }
        }
        if size > 1_000_000 {
            let (sample, total) = truncate_sample(paths);
            out.push(Recommendation {
                category: CATEGORY_NECESSITY.into(),
                severity: SEVERITY_LOW.into(),
                title: "Documentation / man / locale data".into(),
                detail: format!(
                    "{} of docs, man pages, info and locale files ({} file(s){}) \
                     is rarely needed in a container.",
                    bytesize::ByteSize(size),
                    total,
                    more_note(sample.len(), total)
                ),
                dockerfile_hint: "Use distro minimization (e.g. dpkg `path-exclude`, \
                     `apk --no-cache`, or a -slim/distroless base)."
                    .into(),
                est_saved_bytes: size,
                heuristic: true,
                paths: sample,
            });
        }
    }

    {
        let mut size = 0u64;
        let mut paths: Vec<String> = Vec::new();
        for l in &leaves {
            if is_log_or_temp(&l.path) {
                size += l.size;
                paths.push(l.path.clone());
            }
        }
        if !paths.is_empty() {
            let (sample, total) = truncate_sample(paths);
            out.push(Recommendation {
                category: CATEGORY_NECESSITY.into(),
                severity: SEVERITY_LOW.into(),
                title: "Log / temp files baked into image".into(),
                detail: format!(
                    "{} of log or temporary files ({} file(s){}) is shipped; \
                     these belong to runtime, not the image.",
                    bytesize::ByteSize(size),
                    total,
                    more_note(sample.len(), total)
                ),
                dockerfile_hint: "Clean /var/log and /tmp at the end of the RUN that creates them."
                    .into(),
                est_saved_bytes: size,
                heuristic: true,
                paths: sample,
            });
        }
    }

    {
        let mut size = 0u64;
        let mut paths: Vec<String> = Vec::new();
        for l in &leaves {
            if is_toolchain_binary(&l.path) {
                size += l.size;
                paths.push(l.path.clone());
            }
        }
        if !paths.is_empty() {
            let (sample, total) = truncate_sample(paths);
            out.push(Recommendation {
                category: CATEGORY_NECESSITY.into(),
                severity: SEVERITY_LOW.into(),
                title: "Build toolchain present in final image".into(),
                detail: format!(
                    "{} of compiler/build tools ({} found{}) are present. They \
                     enlarge the image and widen the attack surface if the app \
                     does not compile at runtime.",
                    bytesize::ByteSize(size),
                    total,
                    more_note(sample.len(), total)
                ),
                dockerfile_hint: "Install build deps in a builder stage; keep only runtime \
                     packages in the final stage."
                    .into(),
                est_saved_bytes: size,
                heuristic: true,
                paths: sample,
            });
        }
    }

    // ---- SECURITY -----------------------------------------------------------

    if !result.sensitive_files.is_empty() {
        let paths: Vec<String> = result
            .sensitive_files
            .iter()
            .take(PATH_SAMPLE_LIMIT)
            .map(|s| format!("{} ({})", s.path, s.reason))
            .collect();
        out.push(Recommendation {
            category: CATEGORY_SECURITY.into(),
            severity: SEVERITY_HIGH.into(),
            title: "Potential secrets in image".into(),
            detail: format!(
                "{} suspected secret/credential file(s) detected. Deleting them \
                 in a later layer does NOT help — the earlier layer still \
                 contains the bytes and is recoverable.",
                result.sensitive_files.len()
            ),
            dockerfile_hint: "Never COPY secrets in; use BuildKit `--mount=type=secret` or \
                 runtime env/secret managers, then rebuild (squashing alone is \
                 not enough if the secret was committed upstream)."
                .into(),
            est_saved_bytes: 0,
            heuristic: false,
            paths,
        });
    }

    // Hardcoded secrets in ENV / labels / Dockerfile (names only, no values).
    {
        let mut hits: Vec<String> = Vec::new();
        for e in &result.envs {
            if let Some(k) = secret_env_key(e) {
                hits.push(format!("ENV {}", k));
            }
        }
        for l in &result.labels {
            if let Some(k) = secret_env_key(l) {
                hits.push(format!("LABEL {}", k));
            }
        }
        let df_secret = has_aws_access_key(&result.dockerfile)
            || result.dockerfile.contains("-----BEGIN")
            || result.dockerfile.split_whitespace().any(|t| {
                matches!(
                    token_kind(t),
                    Some("GitHub token") | Some("Slack token") | Some("JWT")
                )
            });
        if df_secret {
            hits.push("Dockerfile (inline credential)".into());
        }
        if !hits.is_empty() {
            let (sample, total) = truncate_sample(hits);
            out.push(Recommendation {
                category: CATEGORY_SECURITY.into(),
                severity: SEVERITY_HIGH.into(),
                title: "Secrets in image metadata".into(),
                detail: format!(
                    "{} environment variable/label/Dockerfile entr{} look like \
                     hardcoded credentials. Image metadata is world-readable via \
                     `docker inspect`.",
                    total,
                    if total == 1 { "y" } else { "ies" }
                ),
                dockerfile_hint: "Pass secrets at runtime (env/secret manager); do not bake \
                     them into ENV/ARG/LABEL."
                    .into(),
                est_saved_bytes: 0,
                heuristic: false,
                paths: sample,
            });
        }
    }

    // Secret files that are world-readable (others can read them).
    {
        let mut wr: Vec<String> = Vec::new();
        for s in &result.sensitive_files {
            if let Some(l) = leaves.iter().find(|l| l.path == s.path) {
                if is_world_readable(&l.mode) {
                    wr.push(format!("{} ({}, {})", s.path, s.reason, l.mode));
                }
            }
        }
        if !wr.is_empty() {
            let (sample, total) = truncate_sample(wr);
            out.push(Recommendation {
                category: CATEGORY_SECURITY.into(),
                severity: SEVERITY_HIGH.into(),
                title: "World-readable secret files".into(),
                detail: format!(
                    "{} suspected secret file(s) are readable by every user and \
                     process in the container (others-read bit set).",
                    total
                ),
                dockerfile_hint: "Restrict permissions (`chmod 600`) or, better, \
                     don't ship the secret at all."
                    .into(),
                est_saved_bytes: 0,
                heuristic: false,
                paths: sample,
            });
        }
    }

    if result.user.is_empty() || result.user == "root" || result.user == "0" {
        out.push(Recommendation {
            category: CATEGORY_SECURITY.into(),
            severity: SEVERITY_MEDIUM.into(),
            title: "Container runs as root".into(),
            detail: "No non-root USER is set, so the process runs as root by \
                     default — a privilege-escalation risk if compromised."
                .into(),
            dockerfile_hint: "Add a dedicated user and `USER nonroot` before CMD.".into(),
            est_saved_bytes: 0,
            heuristic: false,
            paths: vec![],
        });
    }

    {
        let mut setid: Vec<String> = Vec::new();
        let mut ww: Vec<String> = Vec::new();
        for l in &leaves {
            if is_setuid_or_setgid(&l.mode) {
                setid.push(format!("{} ({})", l.path, l.mode));
            }
            if is_world_writable_file(&l.mode) {
                ww.push(format!("{} ({})", l.path, l.mode));
            }
        }
        if !setid.is_empty() {
            let (sample, total) = truncate_sample(setid);
            out.push(Recommendation {
                category: CATEGORY_SECURITY.into(),
                severity: SEVERITY_MEDIUM.into(),
                title: "setuid/setgid binaries".into(),
                detail: format!(
                    "{} setuid/setgid binar{} present — common privilege-\
                     escalation targets.",
                    total,
                    if total == 1 { "y" } else { "ies" }
                ),
                dockerfile_hint: "Strip the bits you don't need: \
                     `RUN find / -perm /6000 -type f -exec chmod a-s {} +`."
                    .into(),
                est_saved_bytes: 0,
                heuristic: false,
                paths: sample,
            });
        }
        if !ww.is_empty() {
            let (sample, total) = truncate_sample(ww);
            out.push(Recommendation {
                category: CATEGORY_SECURITY.into(),
                severity: SEVERITY_MEDIUM.into(),
                title: "World-writable files".into(),
                detail: format!(
                    "{} world-writable file(s) — any process/user in the \
                     container can tamper with them.",
                    total
                ),
                dockerfile_hint: "Tighten permissions (e.g. `chmod o-w`).".into(),
                est_saved_bytes: 0,
                heuristic: false,
                paths: sample,
            });
        }
    }

    // Net reclaimable estimate. Several size/necessity rules can match the
    // same file (e.g. a .pyc is both a dev artifact and a build-only file),
    // so naively summing each card's `estSavedBytes` over-counts. This rolls
    // every reclaimable byte up once, keyed by path, as a de-duplicated upper
    // bound — explicitly NOT the sum of the cards above.
    {
        let contributing = out
            .iter()
            .filter(|r| {
                (r.category == CATEGORY_SIZE || r.category == CATEGORY_NECESSITY)
                    && r.est_saved_bytes > 0
            })
            .count();
        if contributing >= 2 {
            let mut reclaim: std::collections::HashMap<&str, u64> =
                std::collections::HashMap::new();
            for l in &leaves {
                if is_pkg_cache(&l.path)
                    || is_dev_artifact(&l.path)
                    || is_build_only(&l.path)
                    || is_doc_or_locale(&l.path)
                    || is_log_or_temp(&l.path)
                    || is_editor_os_junk(&l.path)
                {
                    let e = reclaim.entry(l.path.as_str()).or_insert(0);
                    *e = (*e).max(l.size);
                }
            }
            for w in &summary.wasted_list {
                let e = reclaim.entry(w.path.as_str()).or_insert(0);
                *e = (*e).max(w.total_size);
            }
            let net: u64 = reclaim.values().sum();
            if net > 0 {
                out.push(Recommendation {
                    category: CATEGORY_SIZE.into(),
                    severity: SEVERITY_INFO.into(),
                    title: "Net reclaimable estimate (de-duplicated)".into(),
                    detail: format!(
                        "Roughly {} is reclaimable in total once overlap between \
                         the recommendations above is removed. This is the \
                         de-duplicated upper bound — do NOT add up the \
                         individual cards (the same file is often counted by \
                         several rules).",
                        bytesize::ByteSize(net)
                    ),
                    dockerfile_hint: String::new(),
                    est_saved_bytes: net,
                    heuristic: true,
                    paths: vec![],
                });
            }
        }
    }

    // Stable, severity-first ordering.
    out.sort_by(|a, b| {
        severity_rank(&a.severity)
            .cmp(&severity_rank(&b.severity))
            .then_with(|| a.category.cmp(&b.category))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aws_key_detection() {
        assert!(has_aws_access_key("x AKIAIOSFODNN7EXAMPLE y"));
        assert!(!has_aws_access_key("AKIAshort"));
        assert!(!has_aws_access_key("no key here"));
    }

    #[test]
    fn secret_env_key_matches_name_not_value() {
        assert_eq!(
            secret_env_key("DB_PASSWORD=hunter2"),
            Some("DB_PASSWORD".to_string())
        );
        assert_eq!(secret_env_key("PATH=/usr/bin"), None);
        assert_eq!(secret_env_key("API_KEY="), None);
    }

    #[test]
    fn mode_bit_checks() {
        assert!(is_setuid_or_setgid("-rwsr-xr-x"));
        assert!(is_setuid_or_setgid("-rwxr-sr-x"));
        assert!(!is_setuid_or_setgid("-rwxr-xr-x"));
        assert!(is_world_writable_file("-rw-rw-rw-"));
        assert!(!is_world_writable_file("drwxrwxrwt"));
        assert!(!is_world_writable_file("-rw-r--r--"));
    }

    #[test]
    fn empty_result_only_flags_root_user() {
        // A default result has no USER set, so the only finding should be the
        // "runs as root" security check — and it must not panic on total_size 0.
        let r = DockerAnalyzeResult::default();
        let recs = build_recommendations(&r);
        assert!(recs.iter().all(|x| x.category == CATEGORY_SECURITY));
        assert!(recs.iter().any(|x| x.title == "Container runs as root"));
    }

    #[test]
    fn token_kind_classifies_known_tokens() {
        assert_eq!(
            token_kind("ghp_0123456789abcdefghijklmnopqrstuvwx"),
            Some("GitHub token")
        );
        assert_eq!(token_kind("xoxb-12345-abcde"), Some("Slack token"));
        assert_eq!(
            token_kind("eyJhbGciOiJI.eyJzdWIiOiIxMjM0.SflKxwRJSMeKKF2QT4"),
            Some("JWT")
        );
        // Lowercase sha256 checksum must NOT be flagged (no uppercase letters).
        assert_eq!(
            token_kind("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
            None
        );
        // Mixed-case high-entropy base64-ish secret is flagged.
        assert_eq!(
            token_kind("aB3xK9pQ2rL7mN4tV6wY8zC1dE5fG0hJ4kM7nP2qR9sT"),
            Some("high-entropy token")
        );
        assert_eq!(token_kind("/usr/local/bin"), None);
    }

    #[test]
    fn dockerfile_linter_flags_antipatterns() {
        let df = "RUN apt-get install -y curl\n\
                  ADD https://example.com/app.tar.gz /tmp/\n\
                  RUN pip install flask\n\
                  RUN chown -R app:app /srv";
        let issues = lint_dockerfile(df);
        assert!(issues.iter().any(|i| i.contains("apt install without")));
        assert!(issues.iter().any(|i| i.contains("ADD with URL/tarball")));
        assert!(issues.iter().any(|i| i.contains("--no-cache-dir")));
        assert!(issues.iter().any(|i| i.contains("recursive chown/chmod")));
        // A clean instruction sequence yields nothing.
        assert!(lint_dockerfile("CMD [\"/app\"]\nEXPOSE 8080").is_empty());
        // The base-image rootfs bootstrap must NOT be flagged (not user-controlled).
        assert!(
            lint_dockerfile("ADD alpine-minirootfs-3.23.4-aarch64.tar.gz / # buildkit").is_empty()
        );
        assert!(lint_dockerfile("ADD file:abc123 in /").is_empty());
    }

    #[test]
    fn net_reclaimable_deduplicates_overlapping_bytes() {
        // `__pycache__/x.pyc` matches BOTH the dev-artifact and build-only
        // rules, so each card counts 1000 bytes — but the net roll-up must
        // count those bytes only once.
        let leaf = FileTreeItem {
            name: "x.pyc".into(),
            size: 1000,
            ..Default::default()
        };
        let dir = FileTreeItem {
            name: "__pycache__".into(),
            size: 1000,
            children: vec![leaf],
            ..Default::default()
        };
        let r = DockerAnalyzeResult {
            file_tree_list: vec![vec![dir]],
            ..Default::default()
        };
        let recs = build_recommendations(&r);
        let net = recs
            .iter()
            .find(|x| x.title.starts_with("Net reclaimable"))
            .expect("net card present");
        let sum_cards: u64 = recs
            .iter()
            .filter(|x| {
                (x.category == CATEGORY_SIZE || x.category == CATEGORY_NECESSITY)
                    && x.est_saved_bytes > 0
                    && !x.title.starts_with("Net reclaimable")
            })
            .map(|x| x.est_saved_bytes)
            .sum();
        assert_eq!(net.est_saved_bytes, 1000);
        assert!(
            sum_cards > net.est_saved_bytes,
            "net must de-duplicate overlap"
        );
    }

    #[test]
    fn junk_and_world_readable_helpers() {
        assert!(is_editor_os_junk("src/.DS_Store"));
        assert!(is_editor_os_junk(".idea/workspace.xml"));
        assert!(is_editor_os_junk("a/b/.foo.swp"));
        assert!(!is_editor_os_junk("usr/bin/app"));
        assert!(is_world_readable("-rw-r--r--"));
        assert!(!is_world_readable("-rw-------"));
    }
}
