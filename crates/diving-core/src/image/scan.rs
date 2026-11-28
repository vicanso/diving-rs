use super::layer::{get_os_release_from_layer, path_under_dir, ImageLayerInfo};
use super::model::{BigModifiedFileInfo, SensitiveFileInfo};
use super::oci_image::{
    convert_files_to_file_tree, FileTreeItem, ImageConfig, ImageFileSummary, ImageHistory,
    ImageLayer, ImageManifest, ImageManifestLayer, Op,
};
use crate::sensitive::{is_dev_artifact, is_pkg_cache, SensitivePatterns, MIN_PKG_CACHE_BYTES};
use crate::store::CacheStore;
use chrono::DateTime;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;

pub(crate) fn parse_os_release(content: &[u8], filename: &str) -> Option<String> {
    let text = std::str::from_utf8(content).ok()?.trim();
    match filename {
        "etc/alpine-release" => {
            return Some(format!("Alpine Linux {}", text.lines().next()?.trim()));
        }
        "etc/debian_version" => {
            return Some(format!("Debian {}", text.lines().next()?.trim()));
        }
        f if f.ends_with("redhat-release") => {
            return Some(text.lines().next()?.trim().to_string());
        }
        _ => {}
    }
    let mut pretty_name: Option<String> = None;
    let mut name: Option<String> = None;
    let mut version_id: Option<String> = None;
    let mut version: Option<String> = None;
    for line in text.lines() {
        if let Some((k, v)) = line.trim().split_once('=') {
            let v = v.trim_matches('"').trim_matches('\'').to_string();
            match k {
                "PRETTY_NAME" => pretty_name = Some(v),
                "NAME" => name = Some(v),
                "VERSION_ID" => version_id = Some(v),
                "VERSION" => version = Some(v),
                _ => {}
            }
        }
    }
    if let Some(pn) = pretty_name {
        return Some(pn);
    }
    match (name, version.or(version_id)) {
        (Some(n), Some(v)) => Some(format!("{} {}", n, v)),
        (Some(n), None) => Some(n),
        _ => None,
    }
}

pub(crate) fn detect_os_from_history(layers: &[ImageLayer]) -> Option<String> {
    const DISTROS: &[(&str, &str)] = &[
        ("trixie", "Debian 13 (trixie)"),
        ("bookworm", "Debian 12 (bookworm)"),
        ("bullseye", "Debian 11 (bullseye)"),
        ("buster", "Debian 10 (buster)"),
        ("noble", "Ubuntu 24.04 (Noble Numbat)"),
        ("jammy", "Ubuntu 22.04 (Jammy Jellyfish)"),
        ("focal", "Ubuntu 20.04 (Focal Fossa)"),
        ("bionic", "Ubuntu 18.04 (Bionic Beaver)"),
        ("alpine", "Alpine Linux"),
        ("ubuntu", "Ubuntu"),
        ("debian", "Debian"),
        ("centos", "CentOS"),
        ("fedora", "Fedora"),
        ("rhel", "Red Hat Enterprise Linux"),
        ("amazon", "Amazon Linux"),
        ("suse", "openSUSE"),
    ];
    for layer in layers.iter().take(3) {
        let cmd = layer.cmd.to_lowercase();
        for &(pattern, display) in DISTROS {
            if cmd.contains(pattern) {
                return Some(format!("{} (from history)", display));
            }
        }
    }
    None
}

pub(crate) fn extract_base_os_from_layers(info_list: &[ImageLayerInfo]) -> Option<String> {
    for info in info_list.iter().take(4) {
        if let Some((ref filename, ref content)) = info.os_release {
            if let Some(name) = parse_os_release(content, filename) {
                return Some(name);
            }
        }
    }
    None
}

pub(crate) fn extract_health_and_ports(config: &ImageConfig) -> (bool, Vec<String>) {
    let Some(extra) = config.config.as_ref() else {
        return (false, vec![]);
    };
    let has_healthcheck = extra.healthcheck.as_ref().is_some_and(|hc| {
        hc.get("Test")
            .and_then(|t| t.as_array())
            .is_some_and(|arr| !arr.is_empty() && arr[0].as_str() != Some("NONE"))
    });
    let mut ports: Vec<String> = extra
        .exposed_ports
        .as_ref()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    ports.sort();
    (has_healthcheck, ports)
}

pub(crate) fn probe_base_os(manifest: &ImageManifest, store: &CacheStore) -> String {
    for layer in manifest.layers.iter().take(3) {
        let path = store.blob_path(&layer.digest);
        if let Ok(file) = File::open(&path) {
            if let Some((filename, content)) =
                get_os_release_from_layer(BufReader::new(file), &layer.media_type)
            {
                if let Some(name) = parse_os_release(&content, filename) {
                    return name;
                }
            }
        }
    }
    String::new()
}

/// Normalizes a single OCI history `created_by` command into a Dockerfile line.
///
/// Strips trailing `# buildkit`, unwraps `|<N> ... /bin/{sh,bash,ash} -c`
/// (with or without a leading `RUN `), and preserves `#(nop)` directives.
pub(crate) fn clean_dockerfile_history_line(raw: &str) -> Option<String> {
    let mut trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(without_bk) = strip_buildkit_comment_suffix(trimmed) {
        trimmed = without_bk;
        if trimmed.is_empty() {
            return None;
        }
    }
    let after_run = strip_ci_prefix(trimmed, "RUN ")
        .map(str::trim_start)
        .unwrap_or(trimmed);
    let unwrapped = if starts_with_buildkit_pipe_prefix(after_run) {
        find_shell_c_segment(after_run).unwrap_or(trimmed)
    } else if has_shell_c_prefix(after_run) {
        after_run
    } else {
        trimmed
    };
    if let Some(body) = strip_shell_c_prefix(unwrapped) {
        if let Some(nop_rest) = strip_nop_directive(body) {
            let clean = nop_rest.trim();
            return (!clean.is_empty()).then(|| clean.to_owned());
        }
        let body = body.trim();
        return (!body.is_empty()).then(|| format!("RUN {body}"));
    }
    Some(trimmed.to_owned())
}

fn strip_ci_prefix<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    s.get(..prefix.len())
        .filter(|head| head.eq_ignore_ascii_case(prefix))
        .map(|_| &s[prefix.len()..])
}

fn strip_buildkit_comment_suffix(s: &str) -> Option<&str> {
    let trimmed = s.trim_end();
    let suffix = "# buildkit";
    if trimmed.len() < suffix.len() {
        return None;
    }
    let pos = trimmed.len() - suffix.len();
    if !trimmed.is_char_boundary(pos) || !trimmed[pos..].eq_ignore_ascii_case(suffix) {
        return None;
    }
    if pos == 0 || trimmed[..pos].ends_with(char::is_whitespace) {
        Some(trimmed[..pos].trim_end())
    } else {
        None
    }
}

fn starts_with_buildkit_pipe_prefix(s: &str) -> bool {
    let Some(rest) = s.strip_prefix('|') else {
        return false;
    };
    let digits = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
    digits > 0 && rest[digits..].starts_with(char::is_whitespace)
}

fn strip_shell_c_prefix(s: &str) -> Option<&str> {
    for prefix in ["/bin/sh -c ", "/bin/bash -c ", "/bin/ash -c "] {
        if let Some(rest) = s.strip_prefix(prefix) {
            return Some(rest);
        }
    }
    None
}

fn has_shell_c_prefix(s: &str) -> bool {
    strip_shell_c_prefix(s).is_some()
}

fn find_shell_c_segment(s: &str) -> Option<&str> {
    for marker in [" /bin/sh -c ", " /bin/bash -c ", " /bin/ash -c "] {
        if let Some(pos) = s.find(marker) {
            return Some(&s[pos + 1..]);
        }
    }
    None
}

fn strip_nop_directive(s: &str) -> Option<&str> {
    let trimmed = s.trim_start();
    trimmed.strip_prefix("#(nop)").map(|rest| rest.trim_start())
}

pub(crate) fn reconstruct_dockerfile(history: &[ImageHistory]) -> String {
    let mut lines: Vec<String> = Vec::with_capacity(history.len());
    for h in history {
        let Some(raw) = h.created_by.as_deref() else {
            continue;
        };
        if let Some(line) = clean_dockerfile_history_line(raw) {
            lines.push(line);
        }
    }
    lines.join("\n")
}

pub(crate) fn display_name(repo: &str, tag: &str) -> String {
    if tag.is_empty() {
        repo.to_string()
    } else if tag.contains(':') {
        format!("{repo}@{tag}")
    } else {
        format!("{repo}:{tag}")
    }
}

pub(crate) fn compression_label(media_type: &str) -> &'static str {
    if media_type.contains("zstd") {
        "zstd"
    } else if media_type.contains("gzip") {
        "gzip"
    } else {
        "tar"
    }
}

pub(crate) fn preview_layers(
    history: &[ImageHistory],
    manifest_layers: &[ImageManifestLayer],
) -> Vec<ImageLayer> {
    let mut manifest_layers = manifest_layers.iter();
    history
        .iter()
        .map(|history| {
            let empty = history.empty_layer.unwrap_or_default();
            let layer = if empty { None } else { manifest_layers.next() };
            ImageLayer {
                created: history.created.clone(),
                cmd: history.created_by.clone().unwrap_or_default(),
                empty,
                digest: layer.map(|l| l.digest.clone()).unwrap_or_default(),
                media_type: layer.map(|l| l.media_type.clone()).unwrap_or_default(),
                size: layer.map_or(0, |l| l.size),
                ..Default::default()
            }
        })
        .collect()
}

pub(crate) struct LayerScan {
    pub layers: Vec<ImageLayer>,
    pub file_tree_list: Vec<Vec<FileTreeItem>>,
    pub file_summary_list: Vec<ImageFileSummary>,
    pub big_modified_file_list: Vec<BigModifiedFileInfo>,
    pub sensitive_files: Vec<SensitiveFileInfo>,
    pub image_size: u64,
    pub image_total_size: u64,
    pub has_pkg_cache: bool,
    pub has_dev_artifacts: bool,
}

pub(crate) fn scan_layers(
    config: &ImageConfig,
    manifest_layers: &[ImageManifestLayer],
    info_list: &[ImageLayerInfo],
    sensitive_patterns: &SensitivePatterns,
) -> LayerScan {
    let mut image_created = 0;
    if let Some(value) = config.history.last() {
        if let Ok(value) = DateTime::parse_from_rfc3339(&value.created) {
            image_created = value.timestamp();
        }
    }
    let mut layers = vec![];
    let mut file_tree_list: Vec<Vec<FileTreeItem>> = vec![];
    let mut index = 0;
    let mut file_summary_list = vec![];
    let mut image_size = 0;
    let mut image_total_size = 0;
    let mut seen_files: HashMap<String, (u64, usize)> = HashMap::new();
    let mut modified_paths: HashSet<String> = HashSet::new();
    let mut big_modified_file_list = vec![];
    let mut sensitive_files: Vec<SensitiveFileInfo> = vec![];
    let mut sensitive_seen: HashSet<String> = HashSet::new();
    let mut pkg_cache_bytes = 0u64;
    let mut has_dev_artifacts = false;
    let empty_info = ImageLayerInfo::default();

    for (layer_index, history) in config.history.iter().enumerate() {
        let is_new = if let Ok(value) = DateTime::parse_from_rfc3339(&history.created) {
            image_created - value.timestamp() < 300
        } else {
            false
        };
        let empty = history.empty_layer.unwrap_or_default();
        let mut digest = String::new();
        let mut info = &empty_info;
        let mut media_type = String::new();
        let mut diff_id = String::new();
        let mut size = 0;
        let mut file_tree = vec![];

        if !empty {
            if let Some(value) = manifest_layers.get(index) {
                info = info_list.get(index).unwrap_or(&empty_info);
                size = value.size;
                digest = value.digest.clone();
                media_type = value.media_type.clone();
                diff_id = config
                    .rootfs
                    .diff_ids
                    .get(index)
                    .cloned()
                    .unwrap_or_default();

                for file in &info.files {
                    if file.is_opaque == Some(true) {
                        let dir = file.path.as_str();
                        let doomed: Vec<(String, (u64, usize))> = seen_files
                            .iter()
                            .filter(|(p, &(_, written_in))| {
                                written_in != layer_index && path_under_dir(p, dir)
                            })
                            .map(|(p, &seen)| (p.clone(), seen))
                            .collect();
                        for (path, (prev_size, written_in)) in doomed {
                            seen_files.remove(&path);
                            let mut file_info = file.clone();
                            file_info.path = path.clone();
                            file_info.size = prev_size;
                            file_info.is_opaque = None;
                            file_summary_list.push(ImageFileSummary {
                                layer_index,
                                op: Op::Removed,
                                info: file_info,
                                written_in: Some(written_in),
                            });
                            modified_paths.insert(path);
                        }
                        continue;
                    }
                    if layer_index != 0 {
                        if let Some(&(prev_size, written_in)) = seen_files.get(&file.path) {
                            let op;
                            let mut file_info = file.clone();
                            if file.is_whiteout.is_some() {
                                op = Op::Removed;
                                file_info.size = prev_size;
                            } else {
                                op = Op::Modified;
                            }
                            file_summary_list.push(ImageFileSummary {
                                layer_index,
                                op,
                                info: file_info,
                                written_in: Some(written_in),
                            });
                            modified_paths.insert(file.path.clone());
                        }
                    }
                    if file.is_whiteout.is_some() {
                        seen_files.remove(&file.path);
                    } else {
                        seen_files.insert(file.path.clone(), (file.size, layer_index));
                    }
                    if is_new && file.size >= 1_000_000 && file.link.is_empty() {
                        big_modified_file_list.push(BigModifiedFileInfo {
                            path: file.path.clone(),
                            size: file.size,
                            digest: digest.clone(),
                            mode: file.mode.clone(),
                            uid: file.uid,
                            gid: file.gid,
                        });
                    }
                    if is_pkg_cache(&file.path) {
                        pkg_cache_bytes += file.size;
                    }
                    if !has_dev_artifacts && is_dev_artifact(&file.path) {
                        has_dev_artifacts = true;
                    }
                    if file.is_whiteout.is_none() {
                        if let Some(reason) = sensitive_patterns.evaluate_path(&file.path) {
                            let dedup_key = if let Some(pos) = file
                                .path
                                .find("/.git/")
                                .map(|p| p + 1)
                                .or_else(|| file.path.starts_with(".git/").then_some(0))
                            {
                                format!("{}/.git/", &file.path[..pos])
                            } else {
                                file.path.clone()
                            };
                            if sensitive_seen.insert(dedup_key.clone()) {
                                let display_path = if dedup_key.ends_with("/.git/") {
                                    dedup_key
                                } else {
                                    file.path.clone()
                                };
                                sensitive_files.push(SensitiveFileInfo {
                                    path: display_path,
                                    size: file.size,
                                    layer_index,
                                    reason,
                                });
                            }
                        }
                    }
                }
                for (path, file_size, reason) in &info.content_secrets {
                    if !sensitive_patterns.is_ignored(path) && sensitive_seen.insert(path.clone()) {
                        sensitive_files.push(SensitiveFileInfo {
                            path: path.clone(),
                            size: *file_size,
                            layer_index,
                            reason: reason.clone(),
                        });
                    }
                }
                image_size += info.size;
                image_total_size += info.unpack_size;
                file_tree = convert_files_to_file_tree(&info.files, &modified_paths);
            }
            index += 1;
        }

        let created_by = history.created_by.clone().unwrap_or_default();
        layers.push(ImageLayer {
            created: history.created.clone(),
            cmd: created_by,
            empty,
            digest,
            media_type,
            unpack_size: info.unpack_size,
            size,
            diff_id,
        });
        file_tree_list.push(file_tree);
    }

    LayerScan {
        layers,
        file_tree_list,
        file_summary_list,
        big_modified_file_list,
        sensitive_files,
        image_size,
        image_total_size,
        has_pkg_cache: pkg_cache_bytes >= MIN_PKG_CACHE_BYTES,
        has_dev_artifacts,
    }
}

pub(crate) fn extract_image_meta(config: &ImageConfig) -> (String, Vec<String>, Vec<String>) {
    let mut run_user = String::new();
    let mut envs = vec![];
    let mut labels = vec![];
    if let Some(ref extra_info) = config.config {
        if let Some(ref value) = extra_info.user {
            run_user = value.to_string();
        }
        if let Some(ref value) = extra_info.env {
            envs = value.clone();
        }
        if let Some(ref value) = extra_info.labels {
            for (k, v) in value.iter() {
                labels.push(format!("{k}={v}"));
            }
        }
    }
    (run_user, envs, labels)
}

/// Extract container execution defaults (`ENTRYPOINT`, `CMD`, `WORKDIR`) from
/// the image config so cross-image diffs can detect configuration drift.
pub(crate) fn extract_image_exec_meta(config: &ImageConfig) -> (Vec<String>, Vec<String>, String) {
    let Some(ref extra_info) = config.config else {
        return (Vec::new(), Vec::new(), String::new());
    };
    (
        extra_info.entrypoint.clone().unwrap_or_default(),
        extra_info.cmd.clone().unwrap_or_default(),
        extra_info.working_dir.clone().unwrap_or_default(),
    )
}

/// Whether a `USER` config value resolves to UID 0 (`""`, `"root"`, `"0"`,
/// `"0:0"`, `"root:root"`, `"0:*"`, `"root:*"`, etc.).
pub(crate) fn is_root_user(user: &str) -> bool {
    let trimmed = user.trim();
    if trimmed.is_empty() {
        return true;
    }
    let name = trimmed.split(':').next().unwrap_or(trimmed).trim();
    name == "root" || name == "0"
}

pub(crate) fn build_risk_tags(scan: &LayerScan, run_user: &str) -> Vec<String> {
    let mut tags: Vec<String> = vec![];
    if scan.has_pkg_cache {
        tags.push("[Contains Package Manager Cache]".to_string());
    }
    if scan.has_dev_artifacts {
        tags.push("[Development Artifacts]".to_string());
    }
    if !scan.sensitive_files.is_empty() {
        tags.push("[Potential Secrets]".to_string());
    }
    if is_root_user(run_user) {
        tags.push("[Runs as Root]".to_string());
    }
    if scan.layers.len() > 30 {
        tags.push(format!("[High Layer Count: {}]", scan.layers.len()));
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_layers_skip_empty_history_entries() {
        let entry = |cmd: &str, empty: bool| ImageHistory {
            created_by: Some(cmd.to_string()),
            empty_layer: empty.then_some(true),
            ..Default::default()
        };
        let blob = |digest: &str, size: u64| ImageManifestLayer {
            digest: digest.to_string(),
            size,
            ..Default::default()
        };
        let layers = preview_layers(
            &[
                entry("ADD rootfs", false),
                entry("ENV A=1", true),
                entry("RUN make", false),
                entry("CMD [\"app\"]", true),
            ],
            &[blob("sha256:a", 10), blob("sha256:b", 20)],
        );
        let view: Vec<(&str, bool, &str, u64)> = layers
            .iter()
            .map(|l| (l.cmd.as_str(), l.empty, l.digest.as_str(), l.size))
            .collect();
        assert_eq!(
            view,
            [
                ("ADD rootfs", false, "sha256:a", 10),
                ("ENV A=1", true, "", 0),
                ("RUN make", false, "sha256:b", 20),
                ("CMD [\"app\"]", true, "", 0),
            ]
        );
    }

    #[test]
    fn same_layer_opaque_whiteout_only_evicts_prior_layers() {
        use super::super::oci_image::ImageRootfs;
        use super::super::ImageFileInfo;

        let config = ImageConfig {
            rootfs: ImageRootfs {
                diff_ids: vec!["sha256:l0".into(), "sha256:l1".into()],
                ..Default::default()
            },
            history: vec![
                ImageHistory {
                    created_by: Some("RUN base".into()),
                    ..Default::default()
                },
                ImageHistory {
                    created_by: Some("RUN replace cache".into()),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let manifest = vec![
            ImageManifestLayer {
                digest: "sha256:l0".into(),
                size: 100,
                ..Default::default()
            },
            ImageManifestLayer {
                digest: "sha256:l1".into(),
                size: 100,
                ..Default::default()
            },
        ];
        // Layer 0 writes cache/old; Layer 1 writes cache/new *and* then encounters cache/.wh..wh..opq
        // (possible when archive order places the opaque marker after new files in the same layer).
        let info_list = vec![
            ImageLayerInfo {
                files: vec![ImageFileInfo {
                    path: "cache/old".into(),
                    size: 500,
                    ..Default::default()
                }],
                ..Default::default()
            },
            ImageLayerInfo {
                files: vec![
                    ImageFileInfo {
                        path: "cache/new".into(),
                        size: 300,
                        ..Default::default()
                    },
                    ImageFileInfo {
                        path: "cache".into(),
                        is_whiteout: Some(true),
                        is_opaque: Some(true),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
        ];
        let scan = scan_layers(
            &config,
            &manifest,
            &info_list,
            &SensitivePatterns::default(),
        );
        assert_eq!(scan.file_summary_list.len(), 1);
        assert_eq!(scan.file_summary_list[0].info.path, "cache/old");
        assert_eq!(scan.file_summary_list[0].written_in, Some(0));
    }

    #[test]
    fn is_root_user_handles_numeric_and_group_forms() {
        for user in ["", "root", "0", "0:0", "root:root", "0:1000", "root:wheel"] {
            assert!(is_root_user(user), "{user:?} should be treated as root");
        }
        for user in ["1000", "1000:0", "nobody", "app:root"] {
            assert!(
                !is_root_user(user),
                "{user:?} should not be treated as root"
            );
        }
    }

    #[test]
    fn clean_dockerfile_history_line_strips_buildkit_run_pipe_and_nop() {
        assert_eq!(
            clean_dockerfile_history_line("RUN |2 A=1 B=2 /bin/sh -c echo hi # buildkit"),
            Some("RUN echo hi".to_string())
        );
        assert_eq!(
            clean_dockerfile_history_line("|1 FLAG=on /bin/bash -c apk add curl # buildkit"),
            Some("RUN apk add curl".to_string())
        );
        assert_eq!(
            clean_dockerfile_history_line("/bin/sh -c #(nop)  ENV APP_ENV=prod"),
            Some("ENV APP_ENV=prod".to_string())
        );
        assert_eq!(
            clean_dockerfile_history_line("/bin/sh -c apt-get update"),
            Some("RUN apt-get update".to_string())
        );
        assert_eq!(
            clean_dockerfile_history_line("ENV TOKEN=foo# buildkit"),
            Some("ENV TOKEN=foo# buildkit".to_string())
        );
        assert_eq!(clean_dockerfile_history_line("   "), None);
    }
}
