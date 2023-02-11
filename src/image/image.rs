use serde::{Deserialize, Serialize};

use super::layer::Op;

pub static MEDIA_TYPE_IMAGE_INDEX: &str = "application/vnd.oci.image.index.v1+json";

pub static MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST: &str =
    "application/vnd.docker.distribution.manifest.v2+json";

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
pub struct ImageLayer {
    // 创建时间
    pub created: String,
    pub digest: String,
    // 创建该层的命令
    pub cmd: String,
    // layer的大小
    pub size: u64,
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
pub struct ImageConfig {
    // 架构
    pub architecture: String,
    // 创建时间
    pub created: String,
    // 历史记录
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
