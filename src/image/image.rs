use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::layer::ImageLayerInfo;

pub static MEDIA_TYPE_IMAGE_INDEX: &str = "application/vnd.oci.image.index.v1+json";

pub static MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST: &str =
    "application/vnd.docker.distribution.manifest.v2+json";

pub static CATEGORY_REMOVED: &str = "removed";

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
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageLayer {
    pub created: String,
    pub digest: String,
    pub cmd: String,
    pub size: u64,
    pub info: ImageLayerInfo,
    pub empty: bool,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageAnalysisResult {
    pub name: String,
    pub created: String,
    pub architecture: String,
    pub layers: Vec<ImageLayer>,
    pub size: u64,
    pub total_size: u64,
    pub layer_file_summary_list: Vec<ImageFileSummary>,
    pub layer_file_wasted_summary_list: Vec<ImageFileWastedSummary>,
}

impl ImageAnalysisResult {
    pub fn get_category(&self, path: &str, layer_index: usize) -> Option<String> {
        for item in self.layer_file_summary_list.iter() {
            if item.layer_index == layer_index && item.info.path == path {
                return Some(item.category.clone());
            }
        }
        None
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageFileSummary {
    pub layer_index: usize,
    pub category: String,
    pub info: ImageFileInfo,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageFileWastedSummary {
    pub path: String,
    pub total_size: u64,
    pub count: u32,
}

impl ImageAnalysisResult {
    pub fn auto_fill(&mut self) {
        self.size = self.get_image_size();
        self.total_size = self.get_image_total_size();
        let (layer_file_summary_list, layer_file_wasted_summary_list) =
            self.get_layer_file_summary();
        self.layer_file_summary_list = layer_file_summary_list;
        self.layer_file_wasted_summary_list = layer_file_wasted_summary_list;
    }
    // 获取镜像压缩保存的汇总大小
    fn get_image_size(&self) -> u64 {
        self.layers.iter().map(|item| item.size).sum()
    }
    // 获取镜像解压后所有文件的汇总大小
    fn get_image_total_size(&self) -> u64 {
        self.layers.iter().map(|item| item.info.size).sum()
    }
    // 汇总layer的文件信息
    fn get_layer_file_summary(&self) -> (Vec<ImageFileSummary>, Vec<ImageFileWastedSummary>) {
        let mut exists_files = HashMap::new();
        let mut summary_list = vec![];
        let mut wasted_list: Vec<ImageFileWastedSummary> = vec![];
        // TODO 反转查询layer，则可获取后面删除文件
        // 对应的前置文件的大小
        for (index, layer) in self.layers.iter().enumerate() {
            for file in &layer.info.files {
                let key = &file.path;
                if index == 0 || !exists_files.contains_key(key) {
                    // 新增
                    exists_files.insert(key, file.clone());
                    continue;
                }
                let mut category = "modified".to_string();
                if let Some(is_whiteout) = file.is_whiteout {
                    if is_whiteout {
                        category = CATEGORY_REMOVED.to_string();
                    }
                }
                let mut info = file.clone();
                // 以前已存在，本次覆盖
                // 文件大小使用已存在文件大小
                if let Some(exists_file) = exists_files.get(key) {
                    info.size = exists_file.size;
                }
                let mut found = false;
                for wasted in wasted_list.iter_mut() {
                    if wasted.path == info.path {
                        found = true;
                        wasted.count += 1;
                        wasted.total_size += info.size;
                    }
                }
                if !found {
                    wasted_list.push(ImageFileWastedSummary {
                        path: info.path.clone(),
                        count: 1,
                        total_size: info.size,
                    })
                }
                summary_list.push(ImageFileSummary {
                    layer_index: index,
                    category,
                    info,
                });
            }
        }
        wasted_list.sort_by(|a, b| b.total_size.cmp(&a.total_size));
        (summary_list, wasted_list)
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageIndex {
    pub media_type: String,
    pub schema_version: i64,
    pub manifests: Vec<ImageIndexManifest>,
}

impl ImageIndex {
    // 返回匹配manifest，如果无则返回第一个
    pub fn guess_manifest(&self) -> ImageIndexManifest {
        let os = "linux";
        let mut os_match_manifests = vec![];
        let mut architecture = "amd64";
        let arch = std::env::consts::ARCH;
        if arch.contains("arm") || arch.contains("aarch64") {
            architecture = "arm64"
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
    pub vnd_docker_reference_digest: String,
    #[serde(rename = "vnd.docker.reference.type")]
    pub vnd_docker_reference_type: String,
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
pub struct ImageConfig {
    pub architecture: String,
    pub created: String,
    pub history: Vec<ImageHistory>,
    pub os: String,
    pub rootfs: ImageRootfs,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageHistory {
    pub created: String,
    #[serde(rename = "created_by")]
    pub created_by: String,
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
