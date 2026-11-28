use super::image_ref::{host_arch, platform_matches};
use super::layer::normalize_tar_path;
use serde::{Deserialize, Deserializer, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};

pub static MEDIA_TYPE_IMAGE_INDEX: &str = "application/vnd.oci.image.index.v1+json";
pub static MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST: &str =
    "application/vnd.docker.distribution.manifest.v2+json";
pub static MEDIA_TYPE_MANIFEST_LIST: &str =
    "application/vnd.docker.distribution.manifest.list.v2+json";
pub static MEDIA_TYPE_OCI_MANIFEST: &str = "application/vnd.oci.image.manifest.v1+json";

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageFileInfo {
    pub path: String,
    pub link: String,
    pub size: u64,
    pub mode: String,
    pub uid: u64,
    pub gid: u64,
    pub is_whiteout: Option<bool>,
    #[serde(default)]
    pub is_opaque: Option<bool>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageLayer {
    pub created: String,
    pub digest: String,
    pub cmd: String,
    pub size: u64,
    pub media_type: String,
    pub unpack_size: u64,
    pub empty: bool,
    #[serde(default)]
    pub diff_id: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageFileSummary {
    pub layer_index: usize,
    pub op: Op,
    pub info: ImageFileInfo,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub written_in: Option<usize>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageIndex {
    pub media_type: String,
    pub schema_version: i64,
    pub manifests: Vec<ImageIndexManifest>,
}

impl ImageIndex {
    pub fn guess_manifest(&self, arch: &str) -> Option<ImageIndexManifest> {
        let os = "linux";
        let mut os_match_manifests = vec![];
        let architecture = if arch.is_empty() { host_arch() } else { arch };
        for item in &self.manifests {
            if item.platform.os != os {
                continue;
            }
            if platform_matches(
                architecture,
                &item.platform.architecture,
                item.platform.variant.as_deref(),
            ) {
                return Some(item.clone());
            }
            os_match_manifests.push(item)
        }
        if let Some(&first) = os_match_manifests.first() {
            return Some(first.clone());
        }
        self.manifests.first().cloned()
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageIndexManifest {
    pub media_type: String,
    pub digest: String,
    pub size: i64,
    pub platform: ImageIndexPlatform,
    pub annotations: Option<ImageIndexAnnotations>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageIndexPlatform {
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
    #[serde(rename = "Entrypoint")]
    pub entrypoint: Option<Vec<String>>,
    #[serde(rename = "Cmd")]
    pub cmd: Option<Vec<String>>,
    #[serde(rename = "ExposedPorts")]
    pub exposed_ports: Option<HashMap<String, serde_json::Value>>,
    #[serde(rename = "Healthcheck")]
    pub healthcheck: Option<serde_json::Value>,
    #[serde(rename = "WorkingDir")]
    pub working_dir: Option<String>,
    #[serde(rename = "Volumes")]
    pub volumes: Option<HashMap<String, serde_json::Value>>,
    #[serde(rename = "StopSignal")]
    pub stop_signal: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageConfig {
    pub architecture: String,
    #[serde(default, deserialize_with = "null_as_empty")]
    pub created: String,
    #[serde(default)]
    pub history: Vec<ImageHistory>,
    pub os: String,
    pub rootfs: ImageRootfs,
    pub config: Option<ImageExtraInfo>,
}

fn null_as_empty<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageHistory {
    #[serde(default, deserialize_with = "null_as_empty")]
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
    pub name: String,
    pub link: String,
    pub size: u64,
    pub mode: String,
    pub uid: u64,
    pub gid: u64,
    pub op: Op,
    pub children: Vec<FileTreeItem>,
}

struct TreeNode<'a> {
    item: FileTreeItem,
    children: Vec<TreeNode<'a>>,
    child_index: HashMap<&'a str, usize>,
}

impl<'a> TreeNode<'a> {
    fn into_file_tree_item(self) -> FileTreeItem {
        let mut item = self.item;
        item.children = self
            .children
            .into_iter()
            .map(TreeNode::into_file_tree_item)
            .collect();
        item
    }
}

fn insert_tree_node<'a>(
    nodes: &mut Vec<TreeNode<'a>>,
    index: &mut HashMap<&'a str, usize>,
    segments: &[&'a str],
    leaf: FileTreeItem,
) {
    let Some((&name, rest)) = segments.split_first() else {
        return;
    };
    if rest.is_empty() {
        let i = nodes.len();
        index.entry(name).or_insert(i);
        nodes.push(TreeNode {
            item: leaf,
            children: Vec::new(),
            child_index: HashMap::new(),
        });
        return;
    }
    let idx = if let Some(&i) = index.get(name) {
        nodes[i].item.size += leaf.size;
        i
    } else {
        let op = if leaf.op == Op::Modified {
            Op::Modified
        } else {
            Op::None
        };
        let i = nodes.len();
        index.insert(name, i);
        nodes.push(TreeNode {
            item: FileTreeItem {
                name: name.to_string(),
                size: leaf.size,
                op,
                ..Default::default()
            },
            children: Vec::new(),
            child_index: HashMap::new(),
        });
        i
    };
    let child = &mut nodes[idx];
    insert_tree_node(&mut child.children, &mut child.child_index, rest, leaf);
}

/// One file as it ends up in the running container: the topmost copy.
#[derive(Debug, Clone, PartialEq)]
pub struct FinalFile {
    pub size: u64,
    /// 0-based index of the layer that wrote this copy.
    pub layer: usize,
    pub mode: String,
    pub link: String,
}

/// The filesystem a container sees, from the per-layer file trees.
pub fn final_filesystem(file_tree_list: &[Vec<FileTreeItem>]) -> BTreeMap<String, FinalFile> {
    let mut files: BTreeMap<String, FinalFile> = BTreeMap::new();
    for (layer, tree) in file_tree_list.iter().enumerate() {
        let mut removed: Vec<String> = Vec::new();
        let mut added: Vec<(String, &FileTreeItem)> = Vec::new();
        collect_layer_entries(tree, "", &mut removed, &mut added);
        for path in removed {
            remove_subtree(&mut files, &path);
        }
        for (path, item) in added {
            files.insert(
                path,
                FinalFile {
                    size: item.size,
                    layer,
                    mode: item.mode.clone(),
                    link: item.link.clone(),
                },
            );
        }
    }
    files
}

fn remove_subtree(files: &mut BTreeMap<String, FinalFile>, path: &str) {
    files.remove(path);
    let below = format!("{path}/");
    let doomed: Vec<String> = files
        .range(below.clone()..)
        .take_while(|(p, _)| p.starts_with(&below))
        .map(|(p, _)| p.clone())
        .collect();
    for p in doomed {
        files.remove(&p);
    }
}

fn collect_layer_entries<'a>(
    items: &'a [FileTreeItem],
    parent: &str,
    removed: &mut Vec<String>,
    added: &mut Vec<(String, &'a FileTreeItem)>,
) {
    for item in items {
        let path = match (parent, item.name.as_str()) {
            ("", ".") => String::new(),
            ("", name) => name.to_string(),
            (parent, name) => format!("{parent}/{name}"),
        };
        if item.op == Op::Removed && !path.is_empty() {
            removed.push(path.clone());
        }
        if !item.children.is_empty() {
            collect_layer_entries(&item.children, &path, removed, added);
        } else if path.is_empty() {
        } else if item.op != Op::Removed {
            added.push((path, item));
        }
    }
}

pub fn convert_files_to_file_tree(
    files: &[ImageFileInfo],
    modified_paths: &HashSet<String>,
) -> Vec<FileTreeItem> {
    let mut nodes: Vec<TreeNode<'_>> = vec![];
    let mut root_index: HashMap<&str, usize> = HashMap::new();
    let mut arr: Vec<&str> = Vec::with_capacity(8);
    for file in files.iter() {
        let clean = normalize_tar_path(&file.path);
        arr.clear();
        arr.extend(clean.split('/').filter(|s| !s.is_empty() && *s != "."));
        if arr.is_empty() {
            continue;
        }
        let mut op = Op::None;
        if file.is_whiteout.is_some() {
            op = Op::Removed;
        } else if modified_paths.contains(clean) || modified_paths.contains(file.path.as_str()) {
            op = Op::Modified;
        }

        let leaf = FileTreeItem {
            name: arr[arr.len() - 1].to_string(),
            link: file.link.clone(),
            size: file.size,
            mode: file.mode.clone(),
            uid: file.uid,
            gid: file.gid,
            op,
            ..Default::default()
        };
        insert_tree_node(&mut nodes, &mut root_index, &arr, leaf);
    }
    nodes
        .into_iter()
        .map(TreeNode::into_file_tree_item)
        .collect()
}

// ---- Cross-layer duplicate file detection ---------------------------

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicatePath {
    pub layer_index: usize,
    pub path: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    pub hash: String,
    pub size: u64,
    pub count: usize,
    pub total_wasted: u64,
    pub paths: Vec<DuplicatePath>,
}

const MIN_DUP_FILE_SIZE: u64 = 64 * 1024;
const MAX_DUP_CANDIDATES: usize = 500;
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
            if item.op == Op::Removed || item.size < MIN_DUP_FILE_SIZE {
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

pub(crate) struct DupCandidatePlan {
    leaves: Vec<DupLeaf>,
    candidate_idxs: Vec<usize>,
    pub(crate) by_layer: HashMap<usize, HashSet<String>>,
}

impl DupCandidatePlan {
    pub(crate) fn finish(self, hashes: &HashMap<(usize, String), String>) -> Vec<DuplicateGroup> {
        let mut by_hash: HashMap<String, Vec<usize>> = HashMap::new();
        for &i in &self.candidate_idxs {
            if let Some(h) = hashes.get(&(self.leaves[i].layer_idx, self.leaves[i].path.clone())) {
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
                layer_set.insert(self.leaves[i].layer_idx);
            }
            if layer_set.len() < 2 {
                continue;
            }
            let size = self.leaves[idxs[0]].size;
            let count = idxs.len();
            let total_wasted = ((count as u64).saturating_sub(1)) * size;
            let mut paths: Vec<DuplicatePath> = idxs
                .iter()
                .map(|&i| DuplicatePath {
                    layer_index: self.leaves[i].layer_idx,
                    path: self.leaves[i].path.clone(),
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
        groups.sort_by_key(|g| Reverse(g.total_wasted));
        groups
    }
}

pub(crate) fn plan_cross_layer_duplicates(
    file_tree_list: &[Vec<FileTreeItem>],
) -> Option<DupCandidatePlan> {
    let mut leaves: Vec<DupLeaf> = Vec::new();
    for (idx, tree) in file_tree_list.iter().enumerate() {
        walk_for_dup(tree, "", idx, &mut leaves);
    }
    if leaves.len() < 2 {
        return None;
    }

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
        return None;
    }
    candidate_idxs.sort_by_key(|&i| Reverse(leaves[i].size));
    candidate_idxs.truncate(MAX_DUP_CANDIDATES);

    let mut by_layer: HashMap<usize, HashSet<String>> = HashMap::new();
    for &i in &candidate_idxs {
        by_layer
            .entry(leaves[i].layer_idx)
            .or_default()
            .insert(leaves[i].path.clone());
    }

    Some(DupCandidatePlan {
        leaves,
        candidate_idxs,
        by_layer,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf(name: &str, size: u64, op: Op) -> FileTreeItem {
        FileTreeItem {
            name: name.to_string(),
            size,
            op,
            ..Default::default()
        }
    }

    fn node(name: &str, children: Vec<FileTreeItem>) -> FileTreeItem {
        FileTreeItem {
            name: name.to_string(),
            children,
            ..Default::default()
        }
    }

    #[test]
    fn final_filesystem_applies_layers_in_order() {
        let layers = vec![
            vec![
                node(
                    "app",
                    vec![leaf("old.bin", 10, Op::None), leaf("keep.txt", 1, Op::None)],
                ),
                node(
                    "cache",
                    vec![leaf("a", 5, Op::None), leaf("b", 5, Op::None)],
                ),
                leaf("config.yml", 4, Op::None),
            ],
            vec![],
            vec![
                node("app", vec![leaf("old.bin", 0, Op::Removed)]),
                leaf("cache", 0, Op::Removed),
                node("cache", vec![leaf("c", 7, Op::None)]),
                leaf("config.yml", 9, Op::Modified),
            ],
        ];
        let fs = final_filesystem(&layers);
        let view: Vec<(&str, u64, usize)> = fs
            .iter()
            .map(|(p, f)| (p.as_str(), f.size, f.layer))
            .collect();
        assert_eq!(
            view,
            [
                ("app/keep.txt", 1, 0),
                ("cache/c", 7, 2),
                ("config.yml", 9, 2),
            ]
        );
    }

    #[test]
    fn final_filesystem_opaque_marker_before_new_files() {
        let entry = |path: &str, size: u64, whiteout: bool| ImageFileInfo {
            path: path.to_string(),
            size,
            is_whiteout: whiteout.then_some(true),
            is_opaque: whiteout.then_some(true),
            ..Default::default()
        };
        let lower = convert_files_to_file_tree(
            &[entry("cache/a", 5, false), entry("cache/b", 5, false)],
            &HashSet::new(),
        );
        let upper = convert_files_to_file_tree(
            &[entry("cache", 0, true), entry("cache/c", 7, false)],
            &HashSet::new(),
        );
        assert_eq!(upper.len(), 1, "files are filed under the marker");
        let fs = final_filesystem(&[lower, upper]);
        assert_eq!(fs.keys().collect::<Vec<_>>(), ["cache/c"]);
    }

    #[test]
    fn final_filesystem_strips_a_dot_root() {
        let layers = vec![vec![node(
            ".",
            vec![node("etc", vec![leaf("hosts", 3, Op::None)])],
        )]];
        assert_eq!(
            final_filesystem(&layers).keys().collect::<Vec<_>>(),
            ["etc/hosts"]
        );
    }

    #[test]
    fn test_convert_files_to_file_tree_empty() {
        assert!(convert_files_to_file_tree(&[], &HashSet::new()).is_empty());
    }

    #[test]
    fn guess_manifest_handles_empty_index() {
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
        let hit = index.guess_manifest("amd64").unwrap();
        assert_eq!(hit.platform.architecture, "amd64");
        assert_eq!(hit.platform.os, "linux");
        let arm = |variant: &str| ImageIndexManifest {
            platform: ImageIndexPlatform {
                os: "linux".to_string(),
                architecture: "arm".to_string(),
                variant: Some(variant.to_string()),
            },
            ..Default::default()
        };
        let index_arm = ImageIndex {
            manifests: vec![mk("linux", "amd64"), arm("v6"), arm("v7")],
            ..Default::default()
        };
        let pick = |arch: &str| index_arm.guess_manifest(arch).unwrap().platform.variant;
        assert_eq!(pick("arm/v7").as_deref(), Some("v7"));
        assert_eq!(pick("arm/v6").as_deref(), Some("v6"));
        assert_eq!(pick("arm").as_deref(), Some("v6"));
        let hit = index.guess_manifest("riscv64").unwrap();
        assert_eq!(hit.platform.os, "linux");
        assert_eq!(hit.platform.architecture, "arm64");
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

    #[test]
    fn image_history_accepts_missing_or_null_created() {
        use serde_json::from_str;

        let missing: ImageHistory = from_str(r#"{"created_by":"RUN true"}"#).unwrap();
        assert_eq!(missing.created, "");
        assert_eq!(missing.created_by.as_deref(), Some("RUN true"));

        let null_val: ImageHistory = from_str(r#"{"created":null,"empty_layer":true}"#).unwrap();
        assert_eq!(null_val.created, "");
        assert_eq!(null_val.empty_layer, Some(true));
    }

    #[test]
    fn convert_files_strips_dot_slash_and_ignores_root_opaque() {
        let files = vec![
            ImageFileInfo {
                path: "./".to_string(),
                ..Default::default()
            },
            ImageFileInfo {
                path: "".to_string(),
                is_whiteout: Some(true),
                is_opaque: Some(true),
                ..Default::default()
            },
            ImageFileInfo {
                path: "./etc/app.conf".to_string(),
                size: 42,
                ..Default::default()
            },
        ];
        let tree = convert_files_to_file_tree(&files, &HashSet::new());
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].name, "etc");
        assert_eq!(tree[0].children[0].name, "app.conf");
        assert_eq!(tree[0].children[0].size, 42);
    }
}
