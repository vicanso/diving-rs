//! Runtime-compatibility analyzer.
//!
//! Inspects the image's entrypoint binary with [`goblin`] to determine:
//!   * which libc family it is linked against (glibc / musl / static),
//!   * the minimum glibc version it requires (parsed from `.gnu.version_r`
//!     entries — these are the `GLIBC_x.y` symbol versions that
//!     `objdump -T … | grep GLIBC_` would print),
//!   * and the list of `DT_NEEDED` shared libraries.
//!
//! That fingerprint is compared against the glibc version implied by the
//! detected base OS. A glibc binary on a musl host (or vice versa) is a
//! 100% breakage; a `GLIBC_2.x` requirement higher than the host's glibc
//! is a runtime `version GLIBC_2.x not found` failure. Both are surfaced
//! as recommendation cards via `RuntimeCompat::issue`.
//!
//! Every step is best-effort: a missing entrypoint, unreadable layer
//! blob, non-ELF binary, or parse failure yields a `RuntimeCompat` with
//! an empty `issue` (no card) — never an error to the caller.
//!
//! Limitations: only the binary directly referenced by `Entrypoint[0]` /
//! `Cmd[0]` is inspected; shell-wrapped entrypoints (`["sh", "-c", …]`)
//! and dlopen()-ed plugins are not followed. The OS → glibc map is a
//! best-effort static table covering mainstream distros, not an
//! exhaustive registry.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::io::BufReader;

use crate::store::get_blob_path;

use super::layer::get_file_content_from_layer;
use super::{FileTreeItem, ImageConfig, ImageLayer, Op};

/// Outcome of analyzing the image entrypoint for runtime-libc compatibility.
/// Serialized as part of `DockerAnalyzeResult`. Every field is "best
/// effort" — empty strings / empty vectors mean "couldn't determine".
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCompat {
    /// Path of the binary that was analyzed, as resolved from
    /// `Entrypoint[0]` / `Cmd[0]`. Empty when no entrypoint could be
    /// resolved (e.g. scratch image with no Entrypoint/Cmd).
    pub entrypoint: String,
    /// `"glibc"` | `"musl"` | `"static"` | `""` (unknown).
    pub libc: String,
    /// Highest `GLIBC_x.y` symbol version required by the binary, e.g.
    /// `"2.34"`. Empty for musl/static binaries or when no version
    /// requirement could be parsed.
    pub required_glibc: String,
    /// Capped sample of `DT_NEEDED` shared-library names.
    pub needed_libs: Vec<String>,
    /// glibc version provided by the detected base OS, e.g. `"2.36"`.
    /// Empty when the OS could not be mapped (musl-based distro,
    /// distroless without a discriminator, unknown OS).
    pub os_glibc: String,
    /// Short machine tag for downstream consumers (recommend.rs):
    ///   * `""` — no issue / not enough information
    ///   * `"glibc-too-old"` — binary needs a newer glibc than the OS provides
    ///   * `"glibc-on-musl"` — glibc-linked binary in a musl image (Alpine etc.)
    ///   * `"musl-on-glibc"` — musl-linked binary in a glibc image (rare; usually fine via interp)
    pub issue: String,
}

/// Max number of `DT_NEEDED` entries kept in the report. Prevents a
/// pathological binary with hundreds of dependencies from bloating the
/// JSON payload or AI prompt.
const NEEDED_LIBS_CAP: usize = 32;

/// Maximum bytes accepted from a candidate binary. The ELF header and
/// dynamic sections live near the start; capping the read keeps memory
/// bounded and the analysis fast. Real-world entrypoints rarely exceed
/// a few MB.
const MAX_BINARY_BYTES: usize = 64 * 1024 * 1024;

/// Default `PATH` used when the image config doesn't set one explicitly.
/// Matches the value Docker exports when no PATH is configured.
const DEFAULT_PATH: &str =
    "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// Top-level orchestrator. Returns `None` only when there is nothing
/// reportable at all (no entrypoint, no readable binary). The caller
/// stores the result on `DockerAnalyzeResult::runtime_compat`.
///
/// Pipeline:
///   1. Determine the primary candidate = first non-empty token of
///      `Entrypoint ++ Cmd`.
///   2. Probe it (locate in layers, read bytes).
///   3. If the bytes are ELF, use them.
///   4. If the bytes look like a shell script, parse one `exec` line and
///      probe its target. This handles the very common idiom
///      `ENTRYPOINT ["/entrypoint.sh"]` + `CMD ["app"]` where the script
///      ends with `exec "$@"` or `exec /usr/local/bin/app`.
///   5. As a last resort (script with no parseable `exec`), if `CMD[0]`
///      is distinct from the entrypoint, try it directly.
///
/// When unwrapping succeeds, the `entrypoint` field carries the resolved
/// binary path with the wrapper noted, e.g. `app (via /entrypoint.sh)`,
/// so the user can see which file was actually analyzed.
pub fn analyze_runtime_compat(
    config: &ImageConfig,
    layers: &[ImageLayer],
    file_tree_list: &[Vec<FileTreeItem>],
    base_os: &str,
) -> Option<RuntimeCompat> {
    let extra = config.config.as_ref()?;
    let entrypoint = extra.entrypoint.as_deref().unwrap_or(&[]);
    let cmd = extra.cmd.as_deref().unwrap_or(&[]);

    // Effective argv = Entrypoint ++ Cmd. First non-empty token is the
    // primary candidate.
    let first = entrypoint
        .iter()
        .chain(cmd.iter())
        .find(|s| !s.is_empty())
        .cloned()?;

    // `cmd_first` = what `"$@"` would expand to inside a wrapper script.
    // Only meaningful when an explicit Entrypoint is set; otherwise
    // Cmd[0] *is* the binary and there's nothing to forward to.
    let cmd_first = if entrypoint.iter().any(|s| !s.is_empty()) {
        cmd.iter().find(|s| !s.is_empty()).cloned()
    } else {
        None
    };

    let env_path = extra.env.as_ref().and_then(|envs| {
        envs.iter()
            .find_map(|e| e.strip_prefix("PATH=").map(|s| s.to_string()))
    });

    // First probe.
    let probe1 = probe_binary(&first, env_path.as_deref(), file_tree_list, layers)?;
    let Probe {
        path: orig_path,
        bytes: orig_bytes,
    } = probe1;

    // If we read bytes and they're ELF — use as-is. Otherwise attempt one
    // shell-script unwrap hop; if that fails, fall back to CMD[0] when
    // it's distinct from the entrypoint (the most common "wrapper script
    // I can't parse" recovery).
    let (final_path, final_bytes, wrapper) = match orig_bytes {
        Some(bytes) if goblin::elf::Elf::parse(&bytes).is_ok() => {
            (orig_path, Some(bytes), None)
        }
        Some(bytes) => {
            let candidate = parse_exec_target(&bytes, cmd_first.as_deref()).or_else(|| {
                cmd_first
                    .as_deref()
                    .filter(|c| *c != first.as_str())
                    .map(|s| s.to_string())
            });
            if let Some(target) = candidate {
                match probe_binary(&target, env_path.as_deref(), file_tree_list, layers) {
                    Some(Probe {
                        path: p2,
                        bytes: Some(b2),
                    }) => (p2, Some(b2), Some(orig_path)),
                    _ => (orig_path, Some(bytes), None),
                }
            } else {
                (orig_path, Some(bytes), None)
            }
        }
        None => (orig_path, None, None),
    };

    // Display path includes the wrapper, when applicable, so the report
    // makes clear which file was actually inspected vs. how we got there.
    let display_entry = match wrapper {
        Some(w) if w != final_path => format!("{final_path} (via {w})"),
        _ => final_path,
    };

    let Some(bytes) = final_bytes else {
        return Some(RuntimeCompat {
            entrypoint: display_entry,
            ..Default::default()
        });
    };

    let info = parse_elf(&bytes).unwrap_or_default();
    let os_glibc = glibc_version_for_os(base_os);
    let os_glibc_str = os_glibc
        .map(|(a, b)| format!("{a}.{b}"))
        .unwrap_or_default();
    let issue = classify_issue(
        &info.libc,
        info.required_glibc.as_deref(),
        base_os,
        os_glibc,
    );

    Some(RuntimeCompat {
        entrypoint: display_entry,
        libc: info.libc,
        required_glibc: info.required_glibc.unwrap_or_default(),
        needed_libs: info.needed_libs,
        os_glibc: os_glibc_str,
        issue,
    })
}

// ---- entrypoint resolution -----------------------------------------------

struct EntryHit {
    path: String,
    /// Index into `layers` where this file is added/modified most recently.
    layer_index: usize,
}

/// Probe result: resolved filesystem path within the image, plus the raw
/// bytes read from the layer blob (if reachable). The path is filled in
/// even when bytes are `None`, so the report can name what was attempted.
struct Probe {
    path: String,
    bytes: Option<Vec<u8>>,
}

/// Resolve `name` to a concrete file path in the layer trees (handling
/// absolute paths, `PATH` lookup for bare names, and one symlink hop)
/// AND read its bytes from the cached layer blob. Returns `None` when
/// the name resolves to nothing in any layer.
fn probe_binary(
    name: &str,
    env_path: Option<&str>,
    file_tree_list: &[Vec<FileTreeItem>],
    layers: &[ImageLayer],
) -> Option<Probe> {
    let mut candidates: Vec<String> = Vec::new();
    if name.starts_with('/') {
        candidates.push(name.trim_start_matches('/').to_string());
    } else {
        let path_env = env_path.unwrap_or(DEFAULT_PATH);
        for dir in path_env.split(':').filter(|s| !s.is_empty()) {
            let dir = dir.trim_start_matches('/');
            if dir.is_empty() {
                candidates.push(name.to_string());
            } else {
                candidates.push(format!("{dir}/{name}"));
            }
        }
    }
    for cand in &candidates {
        if let Some(hit) = locate_file_in_layers(cand, file_tree_list) {
            let bytes = read_binary_bytes(&hit.path, hit.layer_index, layers);
            return Some(Probe {
                path: hit.path,
                bytes,
            });
        }
    }
    // Last-resort: PATH didn't help, but the image may have installed the
    // binary at a non-standard location (`/app/foo`, `/diving`, etc.) and
    // the wrapper script knew where it was. Search every layer's tree for
    // a leaf whose basename matches `name` exactly. Conservative: only
    // accept basename-only matches (`name` must not contain `/`) and stop
    // on the first executable hit walking layers high → low.
    if !name.contains('/') {
        if let Some(hit) = find_by_basename(name, file_tree_list) {
            let bytes = read_binary_bytes(&hit.path, hit.layer_index, layers);
            return Some(Probe {
                path: hit.path,
                bytes,
            });
        }
    }
    None
}

/// Walk every layer tree looking for a leaf whose `name` matches the
/// supplied basename. Returns the most recent (highest layer index) hit,
/// resolving one symlink hop the same way `locate_file_in_layers` does.
/// Used as a fallback when PATH lookup misses (binary installed at a
/// non-standard location, image with no PATH env, etc.).
fn find_by_basename(
    basename: &str,
    file_tree_list: &[Vec<FileTreeItem>],
) -> Option<EntryHit> {
    for (idx, tree) in file_tree_list.iter().enumerate().rev() {
        if let Some(path) = scan_tree_for_basename(tree, basename, "") {
            // Re-route through locate_file_in_layers so symlink resolution
            // is handled uniformly (it does a `find_leaf` + one-hop link
            // follow, matching the PATH-lookup behavior).
            if let Some(hit) = locate_file_in_layers(&path, file_tree_list) {
                return Some(hit);
            }
            // Symlink chain too deep / target missing — accept the bare
            // hit anyway so we can at least report what we found.
            return Some(EntryHit {
                path,
                layer_index: idx,
            });
        }
    }
    None
}

fn scan_tree_for_basename(items: &[FileTreeItem], basename: &str, prefix: &str) -> Option<String> {
    for item in items {
        let path = if prefix.is_empty() {
            item.name.clone()
        } else {
            format!("{}/{}", prefix, item.name)
        };
        if item.children.is_empty() {
            if item.name == basename && item.op != Op::Removed {
                return Some(path);
            }
        } else if let Some(hit) = scan_tree_for_basename(&item.children, basename, &path) {
            return Some(hit);
        }
    }
    None
}

/// Best-effort extraction of the target of the first `exec` line in a
/// shell script. Returns `None` if the bytes aren't valid UTF-8 or no
/// `exec` line is found.
///
/// Handles:
///   * `exec /path/to/bin [args]`  → `"/path/to/bin"`
///   * `exec name [args]`          → `"name"` (caller does PATH lookup)
///   * `exec "$@"` / `exec $@`     → `cmd_first` (the CMD passed in)
///   * Surrounding `"…"` or `'…'`  → stripped
///
/// Does NOT follow forms like `exec env VAR=x foo` (returns `"env"` — the
/// PATH lookup will resolve to a glibc binary anyway, which is typically
/// the right answer for libc classification).
fn parse_exec_target(script: &[u8], cmd_first: Option<&str>) -> Option<String> {
    let text = std::str::from_utf8(script).ok()?;
    static EXEC_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"(?m)^\s*exec\s+(\S+)"#).expect("exec regex compiles")
    });
    for cap in EXEC_RE.captures_iter(text) {
        let raw = cap.get(1)?.as_str();
        let token = raw
            .trim_start_matches(['"', '\''])
            .trim_end_matches(['"', '\'']);
        if token.is_empty() {
            continue;
        }
        if token == "$@" {
            return cmd_first.map(|s| s.to_string());
        }
        return Some(token.to_string());
    }
    None
}

/// Walk `file_tree_list` from top to bottom looking for `path`. Returns
/// the highest-index layer where `path` is added/modified (and not later
/// removed). Follows one level of symlink — that's enough for the common
/// `/usr/local/bin/foo → ../share/foo/foo.real` case without risking an
/// unbounded chase.
fn locate_file_in_layers(
    path: &str,
    file_tree_list: &[Vec<FileTreeItem>],
) -> Option<EntryHit> {
    let (idx, link) = find_leaf(path, file_tree_list)?;
    if link.is_empty() {
        return Some(EntryHit {
            path: path.to_string(),
            layer_index: idx,
        });
    }
    // Resolve the symlink relative to the directory containing the link.
    let resolved = resolve_link(path, &link);
    let (idx2, link2) = find_leaf(&resolved, file_tree_list)?;
    if !link2.is_empty() {
        // One symlink hop is enough for now; give up rather than loop.
        return None;
    }
    Some(EntryHit {
        path: resolved,
        layer_index: idx2.max(idx),
    })
}

/// Walks the per-layer trees from highest index down. Returns `(layer_index,
/// link_target)` for the most recent non-removed leaf at `path`. `link_target`
/// is `""` for a regular file.
fn find_leaf(path: &str, file_tree_list: &[Vec<FileTreeItem>]) -> Option<(usize, String)> {
    let path = path.trim_start_matches('/');
    let segments: Vec<&str> = path.split('/').collect();
    for (idx, tree) in file_tree_list.iter().enumerate().rev() {
        if let Some(leaf) = walk_tree(tree, &segments) {
            if leaf.op == Op::Removed {
                return None;
            }
            return Some((idx, leaf.link.clone()));
        }
    }
    None
}

fn walk_tree<'a>(items: &'a [FileTreeItem], segments: &[&str]) -> Option<&'a FileTreeItem> {
    let (head, rest) = segments.split_first()?;
    let next = items.iter().find(|i| i.name == *head)?;
    if rest.is_empty() {
        Some(next)
    } else {
        walk_tree(&next.children, rest)
    }
}

/// Resolve `link` relative to the parent directory of `from`. Both
/// absolute (`/foo`) and relative (`../bar`) links are supported.
fn resolve_link(from: &str, link: &str) -> String {
    if link.starts_with('/') {
        return link.trim_start_matches('/').to_string();
    }
    let mut dir: Vec<&str> = from.trim_start_matches('/').split('/').collect();
    dir.pop();
    for part in link.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                dir.pop();
            }
            other => dir.push(other),
        }
    }
    dir.join("/")
}

// ---- binary fetch + ELF parse --------------------------------------------

/// Read the entrypoint binary's bytes from any layer at or below
/// `starting_layer`. Tries the exact path first, then the `./`-prefixed
/// variant (some tar producers emit one form, some the other). Returns
/// `None` if nothing matches.
fn read_binary_bytes(
    path: &str,
    starting_layer: usize,
    layers: &[ImageLayer],
) -> Option<Vec<u8>> {
    for layer_idx in (0..=starting_layer).rev() {
        if let Some(bytes) = read_from_layer(path, layer_idx, layers) {
            return Some(bytes);
        }
        let alt = format!("./{path}");
        if let Some(bytes) = read_from_layer(&alt, layer_idx, layers) {
            return Some(bytes);
        }
    }
    None
}

fn read_from_layer(path: &str, layer_idx: usize, layers: &[ImageLayer]) -> Option<Vec<u8>> {
    let layer = layers.get(layer_idx)?;
    if layer.digest.is_empty() {
        return None;
    }
    let blob = get_blob_path(&layer.digest);
    let file = std::fs::File::open(&blob).ok()?;
    let reader = BufReader::new(file);
    let bytes = get_file_content_from_layer(reader, &layer.media_type, path).ok()?;
    if bytes.is_empty() || bytes.len() > MAX_BINARY_BYTES {
        return None;
    }
    Some(bytes)
}

#[derive(Default)]
struct ParsedElf {
    libc: String,
    required_glibc: Option<String>,
    needed_libs: Vec<String>,
}

/// Parse ELF bytes and extract libc family + minimum glibc version.
/// Returns `None` only when goblin fails outright (not an ELF). All
/// other failures yield best-effort partial info.
fn parse_elf(bytes: &[u8]) -> Option<ParsedElf> {
    use goblin::elf::Elf;
    let elf = Elf::parse(bytes).ok()?;
    let needed: Vec<String> = elf.libraries.iter().map(|s| s.to_string()).collect();
    let libc = classify_libc(&needed);

    let required_glibc = if libc == "glibc" {
        extract_max_glibc(&elf)
    } else {
        None
    };

    let mut capped = needed;
    if capped.len() > NEEDED_LIBS_CAP {
        capped.truncate(NEEDED_LIBS_CAP);
    }

    Some(ParsedElf {
        libc,
        required_glibc,
        needed_libs: capped,
    })
}

/// Inspect the `DT_NEEDED` list and label the binary as glibc, musl, or
/// statically linked. `"glibc"` requires `libc.so.6`; `"musl"` is any
/// musl-distinguished soname (`libc.musl-*.so` or `ld-musl-*.so`).
fn classify_libc(needed: &[String]) -> String {
    let mut has_glibc = false;
    let mut has_musl = false;
    for lib in needed {
        if lib == "libc.so.6" {
            has_glibc = true;
        }
        if lib.starts_with("libc.musl-") || lib.starts_with("ld-musl-") {
            has_musl = true;
        }
    }
    match (has_glibc, has_musl) {
        (true, _) => "glibc".to_string(),
        (false, true) => "musl".to_string(),
        // No `DT_NEEDED` at all ⇒ statically linked.
        (false, false) if needed.is_empty() => "static".to_string(),
        _ => String::new(),
    }
}

/// Walk `.gnu.version_r` and return the largest `GLIBC_x.y` requirement
/// observed, formatted as `"x.y"`. `None` when no glibc version is found.
fn extract_max_glibc(elf: &goblin::elf::Elf) -> Option<String> {
    let verneed = elf.verneed.as_ref()?;
    let mut best: Option<(u32, u32)> = None;
    for need_file in verneed.iter() {
        for need_ver in need_file.iter() {
            let Some(name) = elf.dynstrtab.get_at(need_ver.vna_name) else {
                continue;
            };
            if let Some(v) = parse_glibc_version(name) {
                best = Some(best.map(|cur| max_tuple(cur, v)).unwrap_or(v));
            }
        }
    }
    best.map(|(a, b)| format!("{a}.{b}"))
}

fn max_tuple(a: (u32, u32), b: (u32, u32)) -> (u32, u32) {
    if a.0 > b.0 || (a.0 == b.0 && a.1 >= b.1) {
        a
    } else {
        b
    }
}

/// Accepts `GLIBC_2.34`, `GLIBC_2.34.1` (major+minor used). Returns
/// `None` for `GLIBC_PRIVATE` (a private versioning namespace that does
/// not imply a public glibc release).
fn parse_glibc_version(s: &str) -> Option<(u32, u32)> {
    let rest = s.strip_prefix("GLIBC_")?;
    if rest == "PRIVATE" {
        return None;
    }
    let mut parts = rest.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    Some((major, minor))
}

// ---- OS → glibc version mapping ------------------------------------------

/// Best-effort lookup of the glibc version shipped by the detected base
/// OS. Returns `None` for musl-based distros (where there is no glibc),
/// scratch / distroless without a discriminator, and unknown OSs.
///
/// Numbers are sourced from each distro's announced default glibc
/// version at release time. They are conservative — patch-level glibc
/// updates do not shift the supported `GLIBC_x.y` symbol set.
pub fn glibc_version_for_os(base_os: &str) -> Option<(u32, u32)> {
    let s = base_os.to_lowercase();
    if s.contains("alpine") || s.contains("musl") {
        return None;
    }
    // Debian — match codename first, then numeric version.
    if s.contains("debian") {
        if s.contains("trixie") || s.contains("debian 13") {
            return Some((2, 41));
        }
        if s.contains("bookworm") || s.contains("debian 12") {
            return Some((2, 36));
        }
        if s.contains("bullseye") || s.contains("debian 11") {
            return Some((2, 31));
        }
        if s.contains("buster") || s.contains("debian 10") {
            return Some((2, 28));
        }
    }
    // Ubuntu
    if s.contains("ubuntu") {
        if s.contains("24.04") || s.contains("noble") {
            return Some((2, 39));
        }
        if s.contains("22.04") || s.contains("jammy") {
            return Some((2, 35));
        }
        if s.contains("20.04") || s.contains("focal") {
            return Some((2, 31));
        }
        if s.contains("18.04") || s.contains("bionic") {
            return Some((2, 27));
        }
    }
    // RHEL / UBI / Rocky / AlmaLinux / CentOS Stream — match the major version.
    let rhel_like = s.contains("rhel")
        || s.contains("red hat")
        || s.contains("ubi")
        || s.contains("rocky")
        || s.contains("alma")
        || s.contains("centos");
    if rhel_like {
        if s.contains('9') {
            return Some((2, 34));
        }
        if s.contains('8') {
            return Some((2, 28));
        }
        if s.contains('7') {
            return Some((2, 17));
        }
    }
    // Amazon Linux
    if s.contains("amazon linux 2023") || s.contains("al2023") {
        return Some((2, 34));
    }
    if s.contains("amazon linux 2") || s.contains("amzn2") {
        return Some((2, 26));
    }
    None
}

/// Cross-check the binary's libc against the OS's libc. Returns a short
/// machine tag (see `RuntimeCompat::issue` doc) or `""`.
fn classify_issue(
    libc: &str,
    required_glibc: Option<&str>,
    base_os: &str,
    os_glibc: Option<(u32, u32)>,
) -> String {
    let os_lower = base_os.to_lowercase();
    let os_is_musl = os_lower.contains("alpine") || os_lower.contains("musl");

    match libc {
        "glibc" if os_is_musl => "glibc-on-musl".to_string(),
        "glibc" => {
            let (Some(req_raw), Some(os)) = (required_glibc, os_glibc) else {
                return String::new();
            };
            let Some(req) = parse_glibc_version(&format!("GLIBC_{req_raw}")) else {
                return String::new();
            };
            if req.0 > os.0 || (req.0 == os.0 && req.1 > os.1) {
                "glibc-too-old".to_string()
            } else {
                String::new()
            }
        }
        "musl" if !os_is_musl && os_glibc.is_some() => "musl-on-glibc".to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_glibc_versions() {
        assert_eq!(parse_glibc_version("GLIBC_2.34"), Some((2, 34)));
        assert_eq!(parse_glibc_version("GLIBC_2.34.1"), Some((2, 34)));
        assert_eq!(parse_glibc_version("GLIBC_PRIVATE"), None);
        assert_eq!(parse_glibc_version("FOO_2.34"), None);
        assert_eq!(parse_glibc_version("GLIBC_2"), None);
    }

    #[test]
    fn classifies_libc_from_needed_list() {
        assert_eq!(
            classify_libc(&["libc.so.6".into(), "libm.so.6".into()]),
            "glibc"
        );
        assert_eq!(classify_libc(&["libc.musl-x86_64.so.1".into()]), "musl");
        assert_eq!(classify_libc(&["ld-musl-aarch64.so.1".into()]), "musl");
        assert_eq!(classify_libc(&[]), "static");
        assert_eq!(classify_libc(&["libfoo.so".into()]), "");
    }

    #[test]
    fn os_map_covers_mainstream_distros() {
        assert_eq!(glibc_version_for_os("Debian 12 (bookworm)"), Some((2, 36)));
        assert_eq!(
            glibc_version_for_os("Debian GNU/Linux 13 (trixie)"),
            Some((2, 41))
        );
        assert_eq!(
            glibc_version_for_os("Ubuntu 24.04 LTS (Noble Numbat)"),
            Some((2, 39))
        );
        assert_eq!(glibc_version_for_os("Alpine Linux v3.20"), None);
        assert_eq!(glibc_version_for_os("Red Hat UBI 9"), Some((2, 34)));
        assert_eq!(glibc_version_for_os("Amazon Linux 2023"), Some((2, 34)));
        assert_eq!(glibc_version_for_os("Scratch / Distroless"), None);
    }

    #[test]
    fn issue_classification() {
        assert_eq!(
            classify_issue("glibc", Some("2.34"), "Alpine Linux v3.20", None),
            "glibc-on-musl"
        );
        assert_eq!(
            classify_issue("glibc", Some("2.34"), "Debian 12", Some((2, 36))),
            ""
        );
        assert_eq!(
            classify_issue("glibc", Some("2.39"), "Debian 12", Some((2, 36))),
            "glibc-too-old"
        );
        assert_eq!(
            classify_issue("musl", None, "Debian 12", Some((2, 36))),
            "musl-on-glibc"
        );
        assert_eq!(
            classify_issue("static", None, "Debian 12", Some((2, 36))),
            ""
        );
        assert_eq!(classify_issue("", None, "Debian 12", Some((2, 36))), "");
    }

    #[test]
    fn parse_exec_extracts_absolute_path() {
        let s = b"#!/bin/sh\nset -e\nexec /usr/local/bin/myapp --port 8080\n";
        assert_eq!(
            parse_exec_target(s, None),
            Some("/usr/local/bin/myapp".to_string())
        );
    }

    #[test]
    fn parse_exec_extracts_relative_name() {
        let s = b"#!/bin/sh\nexec myapp\n";
        assert_eq!(parse_exec_target(s, None), Some("myapp".to_string()));
    }

    #[test]
    fn parse_exec_dollar_at_falls_through_to_cmd() {
        let s = b"#!/bin/sh\nset -e\nexec \"$@\"\n";
        assert_eq!(
            parse_exec_target(s, Some("static-serve")),
            Some("static-serve".to_string())
        );
        // No CMD configured → caller has no fallback target.
        assert_eq!(parse_exec_target(s, None), None);
    }

    #[test]
    fn parse_exec_returns_none_when_no_exec_line() {
        let s = b"#!/bin/sh\necho hello\n/usr/bin/foo &\n";
        assert_eq!(parse_exec_target(s, Some("ignored")), None);
    }

    #[test]
    fn parse_exec_strips_surrounding_quotes() {
        let s = b"exec '/opt/app/bin'\n";
        assert_eq!(parse_exec_target(s, None), Some("/opt/app/bin".to_string()));
        let s = b"exec \"/opt/app/bin\" --flag\n";
        assert_eq!(parse_exec_target(s, None), Some("/opt/app/bin".to_string()));
    }

    #[test]
    fn parse_exec_rejects_non_utf8() {
        let s: &[u8] = &[0xff, 0xfe, 0xfd];
        assert_eq!(parse_exec_target(s, None), None);
    }

    #[test]
    fn parse_exec_first_match_wins() {
        // Some scripts have an early `exec 3<file` (redirection — not a
        // command replacement). The regex matches `3<file` and we return
        // it. That's a known cheap-and-cheerful limitation; the more
        // common pattern (single `exec <cmd>` at end of script) works.
        let s = b"exec 3<somefile\nexec /usr/local/bin/app\n";
        assert_eq!(parse_exec_target(s, None), Some("3<somefile".to_string()));
    }

    #[test]
    fn basename_scan_finds_file_at_nonstandard_path() {
        // Image layout: /diving (root-level binary, NOT in any standard
        // PATH dir). PATH lookup would miss it; basename search should
        // find it on the `static-serve` → "diving" example pattern.
        let tree = vec![FileTreeItem {
            name: "diving".to_string(),
            op: Op::None,
            ..Default::default()
        }];
        let file_tree_list = vec![tree];
        let hit = scan_tree_for_basename(&file_tree_list[0], "diving", "");
        assert_eq!(hit.as_deref(), Some("diving"));
    }

    #[test]
    fn basename_scan_descends_into_dirs() {
        // /app/bin/static-serve — non-PATH location, needs recursive scan.
        let leaf = FileTreeItem {
            name: "static-serve".to_string(),
            op: Op::None,
            ..Default::default()
        };
        let bin = FileTreeItem {
            name: "bin".to_string(),
            children: vec![leaf],
            ..Default::default()
        };
        let app = FileTreeItem {
            name: "app".to_string(),
            children: vec![bin],
            ..Default::default()
        };
        let hit = scan_tree_for_basename(&[app], "static-serve", "");
        assert_eq!(hit.as_deref(), Some("app/bin/static-serve"));
    }

    #[test]
    fn basename_scan_skips_whiteouts() {
        let tree = vec![FileTreeItem {
            name: "static-serve".to_string(),
            op: Op::Removed,
            ..Default::default()
        }];
        assert_eq!(scan_tree_for_basename(&tree, "static-serve", ""), None);
    }

    #[test]
    fn resolve_link_handles_relative_and_absolute() {
        assert_eq!(
            resolve_link("usr/bin/foo", "../share/foo/foo.real"),
            "usr/share/foo/foo.real"
        );
        assert_eq!(resolve_link("usr/bin/foo", "/opt/app/foo"), "opt/app/foo");
        assert_eq!(resolve_link("usr/bin/foo", "./bar"), "usr/bin/bar");
    }
}
