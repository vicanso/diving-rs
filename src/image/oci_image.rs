use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use std::collections::HashMap;

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
    // 该文件是否对应删除
    pub is_whiteout: Option<bool>,
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
    // 返回匹配manifest，如果无则返回第一个
    pub fn guess_manifest(&self, arch: &str) -> ImageIndexManifest {
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
                return item.clone();
            }
            os_match_manifests.push(item)
        }
        // 如果有匹配os的，则返回对应os的
        if !os_match_manifests.is_empty() {
            return os_match_manifests[0].clone();
        }
        self.manifests[0].clone()
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

// 从文件树中查找文件
pub fn find_file_tree_item(items: &[FileTreeItem], path_list: &[&str]) -> Option<FileTreeItem> {
    if path_list.is_empty() {
        return None;
    }
    let is_last = path_list.len() == 1;
    let path = path_list[0];
    for item in items.iter() {
        if item.name == path {
            if is_last {
                return Some(item.clone());
            }
            return find_file_tree_item(&item.children, &path_list[1..]);
        }
    }
    None
}

// 添加文件至文件树
fn add_file(items: &mut Vec<FileTreeItem>, name_list: &[&str], item: FileTreeItem) {
    if name_list.is_empty() {
        items.push(item);
        return;
    }
    let name = name_list[0];
    let mut found_index = -1i64;
    for (index, dir) in items.iter_mut().enumerate() {
        if dir.name == name {
            dir.size += item.size;
            found_index = index as i64;
        }
    }
    if found_index < 0 {
        found_index = items.len() as i64;
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
    }
    if let Some(file_tree_item) = items.get_mut(found_index as usize) {
        add_file(&mut file_tree_item.children, &name_list[1..], item);
    }
}

pub fn convert_files_to_file_tree(
    files: &[ImageFileInfo],
    file_summary_list: &[ImageFileSummary],
) -> Vec<FileTreeItem> {
    let mut file_tree: Vec<FileTreeItem> = vec![];
    for file in files.iter() {
        let arr: Vec<&str> = file.path.split('/').collect();
        if arr.is_empty() {
            continue;
        }
        let mut op = Op::None;
        if file.is_whiteout.is_some() {
            op = Op::Removed;
        } else if file_summary_list
            .iter()
            .any(|item| item.info.path == file.path)
        {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_files_to_file_tree_empty() {
        assert!(convert_files_to_file_tree(&[], &[]).is_empty());
    }

    #[test]
    fn test_convert_files_to_file_tree_nested() {
        let files = vec![ImageFileInfo {
            path: "usr/local/bin/app".to_string(),
            size: 1024,
            ..Default::default()
        }];
        let tree = convert_files_to_file_tree(&files, &[]);
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
        let tree = convert_files_to_file_tree(&files, &[]);
        assert_eq!(tree[0].size, 300);
    }

    #[test]
    fn test_find_file_tree_item_hit() {
        let files = vec![ImageFileInfo {
            path: "usr/bin/app".to_string(),
            size: 512,
            ..Default::default()
        }];
        let tree = convert_files_to_file_tree(&files, &[]);
        let found = find_file_tree_item(&tree, &["usr", "bin", "app"]);
        assert!(found.is_some());
        assert_eq!(found.unwrap().size, 512);
    }

    #[test]
    fn test_find_file_tree_item_miss() {
        let files = vec![ImageFileInfo {
            path: "usr/bin/app".to_string(),
            size: 512,
            ..Default::default()
        }];
        let tree = convert_files_to_file_tree(&files, &[]);
        assert!(find_file_tree_item(&tree, &["usr", "bin", "missing"]).is_none());
        assert!(find_file_tree_item(&tree, &[]).is_none());
    }
}
