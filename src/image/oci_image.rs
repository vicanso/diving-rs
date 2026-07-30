use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::BufReader;

use super::layer::hash_files_from_layer;
use crate::store::get_blob_path;

pub static MEDIA_TYPE_IMAGE_INDEX: &str = "application/vnd.oci.image.index.v1+json";

pub static MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST: &str =
    "application/vnd.docker.distribution.manifest.v2+json";
pub static MEDIA_TYPE_MANIFEST_LIST: &str =
    "application/vnd.docker.distribution.manifest.list.v2+json";

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageFileInfo {
    // 文件目录
    pub path: String,
    // 文件链接
    pub link: String,
    // 文件大小
    pub size: u64,
    // unix mode
    pub mode: String,
    pub uid: u64,
    pub gid: u64,
    // 该文件是否对应删除（OCI whiteout `.wh.<name>`）
    pub is_whiteout: Option<bool>,
    /// Directory opaque whiteout (`.wh..wh..opq`). When set, `path` is the
    /// directory that becomes opaque — every prior entry under it is hidden.
    /// Older analysis-cache entries lack this field and default to `None`.
    #[serde(default)]
    pub is_opaque: Option<bool>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageLayer {
    // 创建时间
    pub created: String,
    pub digest: String,
    // 创建该层的命令
    pub cmd: String,
    // layer的大小
    pub size: u64,
    // 类型
    pub media_type: String,
    // layer解压之后的文件大小
    pub unpack_size: u64,
    // 该层是否为空（无文件操作）
    pub empty: bool,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageFileSummary {
    // 所在层
    pub layer_index: usize,
    // 操作
    pub op: Op,
    // 文件信息
    pub info: ImageFileInfo,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageIndex {
    // 类型
    pub media_type: String,
    // 版本
    pub schema_version: i64,
    // 镜像的manifest
    pub manifests: Vec<ImageIndexManifest>,
}

impl ImageIndex {
    // 返回匹配manifest，如果无则返回第一个；index 无任何 manifest 时返回
    // None（release 构建 panic=abort，索引越界会直接杀掉 web 服务进程）
    pub fn guess_manifest(&self, arch: &str) -> Option<ImageIndexManifest> {
        let os = "linux";
        let mut os_match_manifests = vec![];
        let mut architecture = arch.to_string();
        if architecture.is_empty() {
            architecture = "amd64".to_string();
            let arch = std::env::consts::ARCH;
            if arch.contains("arm") || arch.contains("aarch64") {
                architecture = "arm64".to_string()
            }
        }
        for item in &self.manifests {
            if item.platform.os != os {
                continue;
            }
            if item.platform.architecture == architecture {
                return Some(item.clone());
            }
            os_match_manifests.push(item)
        }
        // 如果有匹配os的，则返回对应os的
        if let Some(&first) = os_match_manifests.first() {
            return Some(first.clone());
        }
        self.manifests.first().cloned()
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageIndexManifest {
    // 类型
    pub media_type: String,
    // 内容对应的digest
    pub digest: String,
    // 大小
    pub size: i64,
    // 平台
    pub platform: ImageIndexPlatform,
    pub annotations: Option<ImageIndexAnnotations>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageIndexPlatform {
    // 架构
    pub architecture: String,
    pub os: String,
    pub variant: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageIndexAnnotations {
    #[serde(rename = "vnd.docker.reference.digest")]
    pub vnd_docker_reference_digest: Option<String>,
    #[serde(rename = "vnd.docker.reference.type")]
    pub vnd_docker_reference_type: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageManifest {
    pub media_type: String,
    pub schema_version: i64,
    pub config: ImageManifestConfig,
    // 文件分层信息
    pub layers: Vec<ImageManifestLayer>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageManifestConfig {
    pub media_type: String,
    pub digest: String,
    pub size: i64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageManifestLayer {
    pub media_type: String,
    pub digest: String,
    pub size: u64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageExtraInfo {
    #[serde(rename = "User")]
    pub user: Option<String>,
    #[serde(rename = "Env")]
    pub env: Option<Vec<String>>,
    #[serde(rename = "Labels")]
    pub labels: Option<HashMap<String, String>>,
    // Container entrypoint as defined in the image config. The effective
    // command at runtime is `Entrypoint` concatenated with `Cmd`; we use
    // these to locate the binary whose ELF dependencies determine runtime
    // libc compatibility with the base OS.
    #[serde(rename = "Entrypoint")]
    pub entrypoint: Option<Vec<String>>,
    #[serde(rename = "Cmd")]
    pub cmd: Option<Vec<String>>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageConfig {
    // 架构
    pub architecture: String,
    // 创建时间
    pub created: String,
    // 历史记录
    pub history: Vec<ImageHistory>,
    pub os: String,
    pub rootfs: ImageRootfs,
    // 镜像信息(还有其它更多字段未读取)
    pub config: Option<ImageExtraInfo>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageHistory {
    pub created: String,
    #[serde(rename = "created_by")]
    pub created_by: Option<String>,
    #[serde(rename = "empty_layer")]
    pub empty_layer: Option<bool>,
    pub comment: Option<String>,
}
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageRootfs {
    #[serde(rename = "type")]
    pub type_field: String,
    #[serde(rename = "diff_ids")]
    pub diff_ids: Vec<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize_repr, Deserialize_repr)]
#[repr(u8)]
pub enum Op {
    #[default]
    None,
    Removed,
    Modified,
    Added,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTreeItem {
    // 文件或目录名称
    pub name: String,
    // 链接
    pub link: String,
    // 文件大小
    pub size: u64,
    // unix mode
    pub mode: String,
    pub uid: u64,
    pub gid: u64,
    // 操作：删除、更新等
    pub op: Op,
    // 子文件
    pub children: Vec<FileTreeItem>,
}

// 添加文件至文件树
fn add_file(items: &mut Vec<FileTreeItem>, name_list: &[&str], item: FileTreeItem) {
    if name_list.is_empty() {
        items.push(item);
        return;
    }
    let name = name_list[0];
    // tar 条目几乎总是按目录聚簇排列，目标目录大概率就是最后一个子节点；
    // 先查最后一个再退回线性扫描，避免在有数千个兄弟目录的层
    // （usr/share/doc、node_modules 等）上退化成 O(n²)。
    let found_index = match items.last() {
        Some(last) if last.name == name => Some(items.len() - 1),
        _ => items.iter().position(|dir| dir.name == name),
    };
    let index = match found_index {
        Some(i) => {
            items[i].size += item.size;
            i
        }
        None => {
            let op = if item.op == Op::Modified {
                Op::Modified
            } else {
                Op::None
            };
            items.push(FileTreeItem {
                name: name.to_string(),
                size: item.size,
                op,
                ..Default::default()
            });
            items.len() - 1
        }
    };
    add_file(&mut items[index].children, &name_list[1..], item);
}

pub fn convert_files_to_file_tree(
    files: &[ImageFileInfo],
    modified_paths: &HashSet<String>,
) -> Vec<FileTreeItem> {
    let mut file_tree: Vec<FileTreeItem> = vec![];
    let mut arr: Vec<&str> = Vec::with_capacity(8);
    for file in files.iter() {
        arr.clear();
        arr.extend(file.path.split('/'));
        if arr.is_empty() {
            continue;
        }
        let mut op = Op::None;
        if file.is_whiteout.is_some() {
            op = Op::Removed;
        } else if modified_paths.contains(file.path.as_str()) {
            op = Op::Modified;
        }

        let size = arr.len();
        add_file(
            &mut file_tree,
            &arr[0..size - 1],
            FileTreeItem {
                // 已保证不会为空
                name: arr[size - 1].to_string(),
                link: file.link.clone(),
                size: file.size,
                mode: file.mode.clone(),
                uid: file.uid,
                gid: file.gid,
                op,
                ..Default::default()
            },
        )
    }
    file_tree
}

// ---- Cross-layer duplicate file detection ---------------------------

/// One occurrence of a file that is duplicated across layers.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicatePath {
    pub layer_index: usize,
    pub path: String,
}

/// A set of two-or-more byte-identical files spread across two-or-more
/// distinct layers. `total_wasted = (count - 1) * size` — every copy
/// beyond the first is redundant on disk.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    /// blake3 hex of the file contents — the source of truth confirming
    /// these really are byte-identical, not just same name + same size.
    pub hash: String,
    /// Size in bytes of each individual copy.
    pub size: u64,
    /// Total number of duplicates observed (≥ 2).
    pub count: usize,
    /// `(count - 1) * size` — bytes that could be reclaimed if only one
    /// copy were kept.
    pub total_wasted: u64,
    /// Per-copy locations. Bounded sample (`DUP_PATHS_SAMPLE`).
    pub paths: Vec<DuplicatePath>,
}

/// Files below this size are skipped. The verify cost dominates and the
/// false-positive rate (many small files happen to share name + size) is
/// too high to act on.
const MIN_DUP_FILE_SIZE: u64 = 64 * 1024;

/// Hard cap on candidate files hashed per analysis to keep verification
/// bounded on pathological images.
const MAX_DUP_CANDIDATES: usize = 500;

/// Sample size kept in each `DuplicateGroup` for the report.
const DUP_PATHS_SAMPLE: usize = 8;

#[derive(Debug, Clone)]
struct DupLeaf {
    layer_idx: usize,
    path: String,
    name: String,
    size: u64,
}

fn walk_for_dup(items: &[FileTreeItem], prefix: &str, layer_idx: usize, out: &mut Vec<DupLeaf>) {
    for item in items {
        let path = if prefix.is_empty() {
            item.name.clone()
        } else {
            format!("{}/{}", prefix, item.name)
        };
        if item.children.is_empty() {
            // Whiteouts are not real content; skip.
            if item.op == Op::Removed {
                continue;
            }
            if item.size < MIN_DUP_FILE_SIZE {
                continue;
            }
            out.push(DupLeaf {
                layer_idx,
                name: item.name.clone(),
                size: item.size,
                path,
            });
        } else {
            walk_for_dup(&item.children, &path, layer_idx, out);
        }
    }
}

/// Detect files that appear with identical content in two or more layers
/// (e.g. a multi-stage build that re-copies the entire `node_modules`,
/// `.so` files, or model weights from the builder stage).
///
/// Pipeline:
///   1. Collect leaves ≥ `MIN_DUP_FILE_SIZE`, skipping whiteouts.
///   2. Group by `(name, size)`; keep only groups spanning ≥ 2 layers.
///   3. Cap at `MAX_DUP_CANDIDATES` (largest first).
///   4. Per-layer batched blake3 verification — each layer's tar is
///      opened and walked exactly once.
///   5. Cluster by hash; keep clusters spanning ≥ 2 layers.
///
/// Returns an empty vec on no significant duplication, missing layer
/// blobs, or any I/O failure (best-effort — never bubbles an error).
pub fn detect_cross_layer_duplicates(
    layers: &[ImageLayer],
    file_tree_list: &[Vec<FileTreeItem>],
) -> Vec<DuplicateGroup> {
    let mut leaves: Vec<DupLeaf> = Vec::new();
    for (idx, tree) in file_tree_list.iter().enumerate() {
        walk_for_dup(tree, "", idx, &mut leaves);
    }
    if leaves.len() < 2 {
        return vec![];
    }

    // (name, size) initial filter — cheap and gets the false-positive
    // rate manageable before we read any blob bytes.
    let mut by_ns: HashMap<(String, u64), Vec<usize>> = HashMap::new();
    for (i, leaf) in leaves.iter().enumerate() {
        by_ns
            .entry((leaf.name.clone(), leaf.size))
            .or_default()
            .push(i);
    }
    let mut candidate_idxs: Vec<usize> = Vec::new();
    for idxs in by_ns.values() {
        if idxs.len() < 2 {
            continue;
        }
        let mut seen_layers = HashSet::new();
        for &i in idxs {
            seen_layers.insert(leaves[i].layer_idx);
        }
        if seen_layers.len() < 2 {
            continue;
        }
        candidate_idxs.extend(idxs);
    }
    if candidate_idxs.is_empty() {
        return vec![];
    }
    // Cap: hash the largest first so the worst offenders never get
    // dropped by the limit.
    candidate_idxs.sort_by_key(|&i| Reverse(leaves[i].size));
    candidate_idxs.truncate(MAX_DUP_CANDIDATES);

    // Batch hashing — one tar pass per layer.
    let mut by_layer: HashMap<usize, HashSet<String>> = HashMap::new();
    for &i in &candidate_idxs {
        by_layer
            .entry(leaves[i].layer_idx)
            .or_default()
            .insert(leaves[i].path.clone());
    }
    let mut hashes: HashMap<(usize, String), String> = HashMap::new();
    for (layer_idx, paths) in by_layer {
        let Some(layer) = layers.get(layer_idx) else {
            continue;
        };
        let blob = get_blob_path(&layer.digest);
        let Ok(file) = File::open(&blob) else {
            continue;
        };
        let reader = BufReader::new(file);
        if let Ok(map) = hash_files_from_layer(reader, &layer.media_type, &paths) {
            for (p, h) in map {
                hashes.insert((layer_idx, p), h);
            }
        }
    }

    // Cluster by hash; demand ≥ 2 copies AND ≥ 2 layers.
    let mut by_hash: HashMap<String, Vec<usize>> = HashMap::new();
    for &i in &candidate_idxs {
        if let Some(h) = hashes.get(&(leaves[i].layer_idx, leaves[i].path.clone())) {
            by_hash.entry(h.clone()).or_default().push(i);
        }
    }
    let mut groups: Vec<DuplicateGroup> = Vec::new();
    for (hash, idxs) in by_hash {
        if idxs.len() < 2 {
            continue;
        }
        let mut layer_set = HashSet::new();
        for &i in &idxs {
            layer_set.insert(leaves[i].layer_idx);
        }
        if layer_set.len() < 2 {
            continue;
        }
        let size = leaves[idxs[0]].size;
        let count = idxs.len();
        let total_wasted = ((count as u64).saturating_sub(1)) * size;
        let mut paths: Vec<DuplicatePath> = idxs
            .iter()
            .map(|&i| DuplicatePath {
                layer_index: leaves[i].layer_idx,
                path: leaves[i].path.clone(),
            })
            .collect();
        paths.sort_by(|a, b| {
            a.layer_index
                .cmp(&b.layer_index)
                .then_with(|| a.path.cmp(&b.path))
        });
        paths.truncate(DUP_PATHS_SAMPLE);
        groups.push(DuplicateGroup {
            hash,
            size,
            count,
            total_wasted,
            paths,
        });
    }
    // Largest waste first so the report's leading entry is the most
    // actionable.
    groups.sort_by_key(|g| Reverse(g.total_wasted));
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_files_to_file_tree_empty() {
        assert!(convert_files_to_file_tree(&[], &HashSet::new()).is_empty());
    }

    #[test]
    fn guess_manifest_handles_empty_index() {
        // 恶意/异常 registry 可能返回空 manifests；不能 panic（panic=abort
        // 会带走整个 web 进程），必须优雅返回 None。
        let index = ImageIndex::default();
        assert!(index.guess_manifest("amd64").is_none());
        assert!(index.guess_manifest("").is_none());
    }

    #[test]
    fn guess_manifest_prefers_arch_then_os_then_first() {
        let mk = |os: &str, arch: &str| ImageIndexManifest {
            platform: ImageIndexPlatform {
                os: os.to_string(),
                architecture: arch.to_string(),
                variant: None,
            },
            ..Default::default()
        };
        let index = ImageIndex {
            manifests: vec![
                mk("windows", "amd64"),
                mk("linux", "arm64"),
                mk("linux", "amd64"),
            ],
            ..Default::default()
        };
        // Exact arch match wins.
        let hit = index.guess_manifest("amd64").unwrap();
        assert_eq!(hit.platform.architecture, "amd64");
        assert_eq!(hit.platform.os, "linux");
        // Unknown arch falls back to the first linux manifest.
        let hit = index.guess_manifest("riscv64").unwrap();
        assert_eq!(hit.platform.os, "linux");
        assert_eq!(hit.platform.architecture, "arm64");
        // No linux at all falls back to the first manifest.
        let windows_only = ImageIndex {
            manifests: vec![mk("windows", "amd64")],
            ..Default::default()
        };
        assert_eq!(
            windows_only.guess_manifest("amd64").unwrap().platform.os,
            "windows"
        );
    }

    #[test]
    fn test_convert_files_to_file_tree_nested() {
        let files = vec![ImageFileInfo {
            path: "usr/local/bin/app".to_string(),
            size: 1024,
            ..Default::default()
        }];
        let tree = convert_files_to_file_tree(&files, &HashSet::new());
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].name, "usr");
        assert_eq!(tree[0].size, 1024);
        assert_eq!(tree[0].children[0].name, "local");
        assert_eq!(tree[0].children[0].children[0].name, "bin");
        let leaf = &tree[0].children[0].children[0].children[0];
        assert_eq!(leaf.name, "app");
        assert_eq!(leaf.size, 1024);
    }

    #[test]
    fn test_convert_files_sibling_dirs_accumulate_size() {
        let files = vec![
            ImageFileInfo {
                path: "usr/bin/a".to_string(),
                size: 100,
                ..Default::default()
            },
            ImageFileInfo {
                path: "usr/bin/b".to_string(),
                size: 200,
                ..Default::default()
            },
        ];
        let tree = convert_files_to_file_tree(&files, &HashSet::new());
        assert_eq!(tree[0].size, 300);
    }
}
