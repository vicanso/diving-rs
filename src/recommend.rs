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

use crate::i18n::{self, Lang};
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
    // OS-level package manager caches.
    if path.starts_with("var/cache/apt/")
        || path.starts_with("var/lib/apt/lists/")
        || path.starts_with("var/cache/apk/")
        || path.starts_with("var/cache/yum/")
        || path.starts_with("var/cache/dnf/")
        || path.starts_with("var/cache/pacman/")
        || path.starts_with("var/cache/zypp/")
    {
        return true;
    }
    // Language ecosystem caches. Match at path start or after a slash so the
    // fragments behave like real path segments and don't catch unrelated
    // directory names.
    const LANG_CACHE_FRAGS: &[&str] = &[
        ".cache/pip/",
        ".npm/_cacache/",
        ".cache/yarn/",
        ".yarn/cache/",
        ".cache/go-build/",
        "go/pkg/mod/cache/",
        ".cargo/registry/",
        ".cargo/git/db/",
        ".composer/cache/",
        ".gem/cache/",
    ];
    if LANG_CACHE_FRAGS
        .iter()
        .any(|f| path.starts_with(f) || path.contains(&format!("/{}", f)))
    {
        return true;
    }
    // Fixed container locations used by language toolchains.
    path.starts_with("usr/local/bundle/cache/")
        || path.starts_with("usr/local/cargo/registry/")
        || path.starts_with("opt/conda/pkgs/")
}

fn is_dev_artifact(path: &str) -> bool {
    // Each fragment is a real path segment we want to match either at the
    // path root or after a `/`. Framework caches (`.next/cache/`, etc.) must
    // be matched specifically so the framework's runtime files (`.next/server/...`)
    // are not flagged.
    const FRAGS: &[&str] = &[
        "node_modules/.cache/",
        ".git/",
        "target/debug/",
        "__pycache__/",
        ".gradle/",
        ".m2/",
        ".pytest_cache/",
        ".mypy_cache/",
        ".ruff_cache/",
        ".tox/",
        ".next/cache/",
        ".nuxt/cache/",
        ".angular/cache/",
        ".parcel-cache/",
        ".ipynb_checkpoints/",
        ".eslintcache",
    ];
    FRAGS
        .iter()
        .any(|f| path.starts_with(f) || path.contains(&format!("/{}", f)))
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
        || path.starts_with("bin/")
        || path.starts_with("usr/local/go/bin/");
    if !in_bin {
        return false;
    }
    const TOOLS: &[&str] = &[
        // C/C++ compilers
        "gcc",
        "g++",
        "cc",
        "c++",
        "clang",
        "clang++",
        // assembler / linker
        "as",
        "ld",
        "ld.gold",
        "ld.lld",
        // binutils
        "ar",
        "nm",
        "objcopy",
        "objdump",
        "ranlib",
        "strip",
        "readelf",
        "addr2line",
        // build orchestration
        "make",
        "cmake",
        "ninja",
        "meson",
        // autotools
        "autoconf",
        "autoreconf",
        "automake",
        "libtool",
        "libtoolize",
        "m4",
        "bison",
        "flex",
        "yacc",
        "pkg-config",
        // language toolchains rarely needed at runtime
        "rustc",
        "cargo",
        "go",
        "gofmt",
        "javac",
        "jar",
        "mvn",
        "python3-config",
        "python-config",
    ];
    TOOLS.contains(&f)
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
/// Returns the localized issue list plus a language-independent flag for
/// "apt install without cleanup" (used to pick the recommendation severity).
fn lint_dockerfile(lang: Lang, df: &str) -> (Vec<String>, bool) {
    let mut issues: Vec<String> = Vec::new();
    let mut has_apt_no_cleanup = false;
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
                issues.push(i18n::fill(i18n::tr(lang, "lint.add"), &[&snip]));
            }
        }
        if is_run
            && (lower.contains("apt-get install") || lower.contains("apt install"))
            && !lower.contains("rm -rf /var/lib/apt/lists")
        {
            has_apt_no_cleanup = true;
            issues.push(i18n::fill(i18n::tr(lang, "lint.apt"), &[&snip]));
        }
        if is_run && (lower.contains("apt-get upgrade") || lower.contains("dist-upgrade")) {
            issues.push(i18n::fill(i18n::tr(lang, "lint.upgrade"), &[&snip]));
        }
        if is_run && lower.contains("pip install") && !lower.contains("--no-cache-dir") {
            issues.push(i18n::fill(i18n::tr(lang, "lint.pip"), &[&snip]));
        }
        if is_run
            && (lower.contains("npm install")
                || lower.contains("npm i ")
                || lower.contains("yarn install"))
            && !lower.contains("cache clean")
            && !lower.contains("--production")
            && !lower.contains("npm ci")
        {
            issues.push(i18n::fill(i18n::tr(lang, "lint.npm"), &[&snip]));
        }
        if is_run && (lower.contains("chown -r") || lower.contains("chmod -r")) {
            issues.push(i18n::fill(i18n::tr(lang, "lint.chown"), &[&snip]));
        }
    }
    if max_streak >= 3 {
        issues.push(i18n::fill(
            i18n::tr(lang, "lint.runstreak"),
            &[&max_streak.to_string()],
        ));
    }
    (issues, has_apt_no_cleanup)
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

fn more_note(lang: Lang, shown: usize, total: usize) -> String {
    if total > shown {
        i18n::fill(i18n::tr(lang, "frag.more"), &[&(total - shown).to_string()])
    } else {
        String::new()
    }
}

/// Produce all recommendations for an analysis result, ordered by severity
/// (high → info) so the most important items render first.
pub fn build_recommendations(result: &DockerAnalyzeResult, lang: Lang) -> Vec<Recommendation> {
    let mut out: Vec<Recommendation> = Vec::new();
    // Localization helpers: `t` = static catalog entry, `f` = entry with
    // `{0}`,`{1}`,… placeholders substituted.
    let t = |k: &str| -> String { i18n::tr(lang, k).to_string() };
    let f = |k: &str, args: &[&str]| -> String { i18n::fill(i18n::tr(lang, k), args) };

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
            title: t("rec.wasted.title"),
            detail: f(
                "rec.wasted.detail",
                &[
                    &bytesize::ByteSize(summary.wasted_size).to_string(),
                    &format!("{:.1}", summary.wasted_percent * 100.0),
                ],
            ),
            dockerfile_hint: t("rec.wasted.hint"),
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
                title: t("rec.pkgcache.title"),
                detail: f(
                    "rec.pkgcache.detail",
                    &[
                        &bytesize::ByteSize(size).to_string(),
                        &total.to_string(),
                        &more_note(lang, sample.len(), total),
                    ],
                ),
                dockerfile_hint: t("rec.pkgcache.hint"),
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
                title: t("rec.devart.title"),
                detail: f(
                    "rec.devart.detail",
                    &[
                        &bytesize::ByteSize(size).to_string(),
                        &total.to_string(),
                        &more_note(lang, sample.len(), total),
                    ],
                ),
                dockerfile_hint: t("rec.devart.hint"),
                est_saved_bytes: size,
                heuristic: false,
                paths: sample,
            });
        }
    }

    // Cross-layer duplicate files (multi-stage builders that re-copied
    // node_modules, .so files, model weights, etc. into the runtime stage).
    // Populated by `detect_cross_layer_duplicates` during analysis; empty
    // when `--no-verify-dup` was set (or no large duplicates exist).
    if !result.duplicate_groups.is_empty() {
        let total_wasted: u64 = result.duplicate_groups.iter().map(|g| g.total_wasted).sum();
        let group_count = result.duplicate_groups.len();
        let severity = if total_wasted >= 50_000_000 {
            SEVERITY_HIGH
        } else if total_wasted >= 5_000_000 {
            SEVERITY_MEDIUM
        } else {
            SEVERITY_LOW
        };
        // Sample paths from the worst-offending groups, formatted with
        // their layer index so the user can jump to the offending stage.
        let mut paths: Vec<String> = Vec::new();
        for g in result.duplicate_groups.iter().take(PATH_SAMPLE_LIMIT) {
            let extra = g.count.saturating_sub(1);
            let sample_path = g
                .paths
                .first()
                .map(|p| format!("L{} {}", p.layer_index + 1, p.path))
                .unwrap_or_default();
            paths.push(format!(
                "{} × {} ({} extra) — {}",
                sample_path,
                bytesize::ByteSize(g.size),
                extra,
                &g.hash[..g.hash.len().min(12)]
            ));
        }
        out.push(Recommendation {
            category: CATEGORY_SIZE.into(),
            severity: severity.into(),
            title: t("rec.crossdup.title"),
            detail: f(
                "rec.crossdup.detail",
                &[
                    &bytesize::ByteSize(total_wasted).to_string(),
                    &group_count.to_string(),
                ],
            ),
            dockerfile_hint: t("rec.crossdup.hint"),
            est_saved_bytes: total_wasted,
            heuristic: false,
            paths,
        });
    }

    // High layer count.
    if result.layers.len() > 30 {
        out.push(Recommendation {
            category: CATEGORY_SIZE.into(),
            severity: SEVERITY_LOW.into(),
            title: t("rec.layercount.title"),
            detail: f("rec.layercount.detail", &[&result.layers.len().to_string()]),
            dockerfile_hint: t("rec.layercount.hint"),
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
            title: t("rec.bigfiles.title"),
            detail: f(
                "rec.bigfiles.detail",
                &[
                    &bytesize::ByteSize(total_big).to_string(),
                    &result.big_modified_file_list.len().to_string(),
                ],
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
                title: t("rec.junk.title"),
                detail: f(
                    "rec.junk.detail",
                    &[
                        &bytesize::ByteSize(size).to_string(),
                        &total.to_string(),
                        &more_note(lang, sample.len(), total),
                    ],
                ),
                dockerfile_hint: t("rec.junk.hint"),
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
            let mut detail = f("rec.slimbase.detail", &[&result.base_os]);
            if let Some(cut) = find_base_layer_split(result) {
                let n = cut.min(result.layers.len());
                let base_sz: u64 = result.layers[..n].iter().map(|l| l.size).sum();
                detail.push_str(&f(
                    "rec.slimbase.extra",
                    &[&bytesize::ByteSize(base_sz).to_string(), &n.to_string()],
                ));
            }
            out.push(Recommendation {
                category: CATEGORY_SIZE.into(),
                severity: SEVERITY_LOW.into(),
                title: t("rec.slimbase.title"),
                detail,
                dockerfile_hint: t("rec.slimbase.hint"),
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
                title: t("rec.oversized.title"),
                detail: f(
                    "rec.oversized.detail",
                    &[
                        &big.len().to_string(),
                        &bytesize::ByteSize(threshold).to_string(),
                    ],
                ),
                dockerfile_hint: t("rec.oversized.hint"),
                est_saved_bytes: 0,
                heuristic: false,
                paths,
            });
        }
    }

    // Dockerfile anti-patterns (reconstructed from image history).
    {
        let (issues, has_apt_no_cleanup) = lint_dockerfile(lang, &result.dockerfile);
        if !issues.is_empty() {
            let severity = if has_apt_no_cleanup {
                SEVERITY_MEDIUM
            } else {
                SEVERITY_LOW
            };
            let (sample, total) = truncate_sample(issues);
            out.push(Recommendation {
                category: CATEGORY_SIZE.into(),
                severity: severity.into(),
                title: t("rec.dflint.title"),
                detail: f(
                    "rec.dflint.detail",
                    &[&total.to_string(), &more_note(lang, sample.len(), total)],
                ),
                dockerfile_hint: t("rec.dflint.hint"),
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
                title: t("rec.buildonly.title"),
                detail: f(
                    "rec.buildonly.detail",
                    &[
                        &bytesize::ByteSize(size).to_string(),
                        &total.to_string(),
                        &more_note(lang, sample.len(), total),
                    ],
                ),
                dockerfile_hint: t("rec.buildonly.hint"),
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
                title: t("rec.doclocale.title"),
                detail: f(
                    "rec.doclocale.detail",
                    &[
                        &bytesize::ByteSize(size).to_string(),
                        &total.to_string(),
                        &more_note(lang, sample.len(), total),
                    ],
                ),
                dockerfile_hint: t("rec.doclocale.hint"),
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
                title: t("rec.logtemp.title"),
                detail: f(
                    "rec.logtemp.detail",
                    &[
                        &bytesize::ByteSize(size).to_string(),
                        &total.to_string(),
                        &more_note(lang, sample.len(), total),
                    ],
                ),
                dockerfile_hint: t("rec.logtemp.hint"),
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
                title: t("rec.toolchain.title"),
                detail: f(
                    "rec.toolchain.detail",
                    &[
                        &bytesize::ByteSize(size).to_string(),
                        &total.to_string(),
                        &more_note(lang, sample.len(), total),
                    ],
                ),
                dockerfile_hint: t("rec.toolchain.hint"),
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
            title: t("rec.secfiles.title"),
            detail: f(
                "rec.secfiles.detail",
                &[&result.sensitive_files.len().to_string()],
            ),
            dockerfile_hint: t("rec.secfiles.hint"),
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
            let entword = if total == 1 {
                t("word.entry_sg")
            } else {
                t("word.entry_pl")
            };
            out.push(Recommendation {
                category: CATEGORY_SECURITY.into(),
                severity: SEVERITY_HIGH.into(),
                title: t("rec.secmeta.title"),
                detail: f("rec.secmeta.detail", &[&total.to_string(), &entword]),
                dockerfile_hint: t("rec.secmeta.hint"),
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
                title: t("rec.worldread.title"),
                detail: f("rec.worldread.detail", &[&total.to_string()]),
                dockerfile_hint: t("rec.worldread.hint"),
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
            title: t("rec.runasroot.title"),
            detail: t("rec.runasroot.detail"),
            dockerfile_hint: t("rec.runasroot.hint"),
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
            let binword = if total == 1 {
                t("word.binary_sg")
            } else {
                t("word.binary_pl")
            };
            out.push(Recommendation {
                category: CATEGORY_SECURITY.into(),
                severity: SEVERITY_MEDIUM.into(),
                title: t("rec.setuid.title"),
                detail: f("rec.setuid.detail", &[&total.to_string(), &binword]),
                dockerfile_hint: t("rec.setuid.hint"),
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
                title: t("rec.worldwrite.title"),
                detail: f("rec.worldwrite.detail", &[&total.to_string()]),
                dockerfile_hint: t("rec.worldwrite.hint"),
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
                    title: t("rec.netreclaim.title"),
                    detail: f(
                        "rec.netreclaim.detail",
                        &[&bytesize::ByteSize(net).to_string()],
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
        let recs = build_recommendations(&r, Lang::En);
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
        let (issues, has_apt) = lint_dockerfile(Lang::En, df);
        assert!(has_apt);
        assert!(issues.iter().any(|i| i.contains("apt install without")));
        assert!(issues.iter().any(|i| i.contains("ADD with URL/tarball")));
        assert!(issues.iter().any(|i| i.contains("--no-cache-dir")));
        assert!(issues.iter().any(|i| i.contains("recursive chown/chmod")));
        // A clean instruction sequence yields nothing.
        assert!(lint_dockerfile(Lang::En, "CMD [\"/app\"]\nEXPOSE 8080")
            .0
            .is_empty());
        // The base-image rootfs bootstrap must NOT be flagged (not user-controlled).
        assert!(lint_dockerfile(
            Lang::En,
            "ADD alpine-minirootfs-3.23.4-aarch64.tar.gz / # buildkit"
        )
        .0
        .is_empty());
        assert!(lint_dockerfile(Lang::En, "ADD file:abc123 in /")
            .0
            .is_empty());
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
        let recs = build_recommendations(&r, Lang::En);
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

    #[test]
    fn pkg_cache_covers_os_and_language_ecosystems() {
        // OS package managers (existing behaviour).
        assert!(is_pkg_cache("var/cache/apt/archives/foo.deb"));
        assert!(is_pkg_cache("var/lib/apt/lists/foo"));
        assert!(is_pkg_cache("var/cache/apk/index"));
        assert!(is_pkg_cache("var/cache/dnf/x"));
        assert!(is_pkg_cache("var/cache/zypp/packages/foo"));
        // Language ecosystem caches.
        assert!(is_pkg_cache("root/.cache/pip/wheels/abc.whl"));
        assert!(is_pkg_cache("home/app/.cache/pip/http/00/aa"));
        assert!(is_pkg_cache("root/.npm/_cacache/index-v5/00/foo"));
        assert!(is_pkg_cache("root/.cache/yarn/v6/npm-foo"));
        assert!(is_pkg_cache("usr/local/share/.cache/yarn/v6/x"));
        assert!(is_pkg_cache("app/.yarn/cache/foo.zip"));
        assert!(is_pkg_cache("root/.cache/go-build/00/aa"));
        assert!(is_pkg_cache("go/pkg/mod/cache/download/sumdb/x"));
        assert!(is_pkg_cache("root/.cargo/registry/cache/index/foo.crate"));
        assert!(is_pkg_cache("usr/local/cargo/registry/cache/foo"));
        assert!(is_pkg_cache("root/.composer/cache/files/x"));
        assert!(is_pkg_cache("usr/local/bundle/cache/foo.gem"));
        assert!(is_pkg_cache("opt/conda/pkgs/numpy-1.26.0/x"));
        // Negatives — must NOT be flagged.
        assert!(!is_pkg_cache("usr/bin/python3"));
        assert!(!is_pkg_cache("app/main.py"));
        assert!(!is_pkg_cache("usr/lib/x86_64-linux-gnu/libc.so.6"));
        // A directory whose name only *contains* a cache name without the
        // path-segment boundary must not match.
        assert!(!is_pkg_cache("app/my.cache/pipx/foo"));
    }

    #[test]
    fn dev_artifact_covers_framework_caches() {
        // Existing behaviour.
        assert!(is_dev_artifact("app/.git/HEAD"));
        assert!(is_dev_artifact("srv/__pycache__/foo.cpython-311.pyc"));
        assert!(is_dev_artifact("usr/src/app/node_modules/.cache/babel/foo"));
        // New framework caches.
        assert!(is_dev_artifact("app/.pytest_cache/v/cache/lastfailed"));
        assert!(is_dev_artifact("app/.mypy_cache/3.11/foo.json"));
        assert!(is_dev_artifact("app/.ruff_cache/0.1.0/foo"));
        assert!(is_dev_artifact("app/.tox/py311/lib/x"));
        assert!(is_dev_artifact("web/.next/cache/webpack/foo.pack"));
        assert!(is_dev_artifact("web/.nuxt/cache/foo"));
        assert!(is_dev_artifact("web/.angular/cache/16.0.0/foo"));
        assert!(is_dev_artifact("web/.parcel-cache/foo"));
        assert!(is_dev_artifact("nb/.ipynb_checkpoints/foo.ipynb"));
        assert!(is_dev_artifact(".eslintcache"));
        // `.next/<not-cache>` is required by Next.js at runtime — must NOT flag.
        assert!(!is_dev_artifact("web/.next/server/pages/index.js"));
        assert!(!is_dev_artifact("usr/lib/foo.so"));
    }

    #[test]
    fn toolchain_binary_covers_binutils_autotools_and_langs() {
        // Existing.
        assert!(is_toolchain_binary("usr/bin/gcc"));
        assert!(is_toolchain_binary("usr/local/bin/clang"));
        // Binutils.
        assert!(is_toolchain_binary("usr/bin/ar"));
        assert!(is_toolchain_binary("usr/bin/nm"));
        assert!(is_toolchain_binary("usr/bin/strip"));
        assert!(is_toolchain_binary("usr/bin/objdump"));
        assert!(is_toolchain_binary("usr/bin/readelf"));
        // Autotools.
        assert!(is_toolchain_binary("usr/bin/autoconf"));
        assert!(is_toolchain_binary("usr/bin/automake"));
        assert!(is_toolchain_binary("usr/bin/libtool"));
        assert!(is_toolchain_binary("usr/bin/pkg-config"));
        assert!(is_toolchain_binary("usr/bin/m4"));
        assert!(is_toolchain_binary("usr/bin/bison"));
        // Build orchestration.
        assert!(is_toolchain_binary("usr/bin/ninja"));
        assert!(is_toolchain_binary("usr/bin/meson"));
        // Language toolchains.
        assert!(is_toolchain_binary("usr/local/go/bin/go"));
        assert!(is_toolchain_binary("usr/local/go/bin/gofmt"));
        assert!(is_toolchain_binary("usr/bin/javac"));
        assert!(is_toolchain_binary("usr/bin/mvn"));
        assert!(is_toolchain_binary("usr/local/bin/cargo"));
        assert!(is_toolchain_binary("usr/local/bin/rustc"));
        assert!(is_toolchain_binary("usr/bin/python3-config"));
        // Runtime tools must NOT be flagged (these legitimately ship in many
        // runtime images).
        assert!(!is_toolchain_binary("usr/bin/python3"));
        assert!(!is_toolchain_binary("usr/bin/node"));
        assert!(!is_toolchain_binary("usr/bin/curl"));
        assert!(!is_toolchain_binary("usr/bin/git"));
        // Outside known bin roots — never flag.
        assert!(!is_toolchain_binary("opt/myapp/gcc"));
        assert!(!is_toolchain_binary("home/dev/local/bin/gcc"));
    }
}
