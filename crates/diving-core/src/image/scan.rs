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

pub(crate) fn reconstruct_dockerfile(history: &[ImageHistory]) -> String {
    let mut lines: Vec<String> = Vec::with_capacity(history.len());
    for h in history {
        let raw = match h.created_by.as_deref() {
            None | Some("") => continue,
            Some(s) => s,
        };
        let raw = if raw.starts_with('|') {
            raw.find("/bin/sh -c ")
                .map(|pos| &raw[pos..])
                .unwrap_or(raw)
        } else {
            raw
        };
        let line = if let Some(rest) = raw.strip_prefix("/bin/sh -c #(nop) ") {
            rest.trim().to_string()
        } else if let Some(rest) = raw.strip_prefix("/bin/sh -c ") {
            format!("RUN {rest}")
        } else {
            raw.to_string()
        };
        lines.push(line);
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
                            .filter(|(p, _)| path_under_dir(p, dir))
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
    if run_user.is_empty() || run_user == "root" {
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
}
