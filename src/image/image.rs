use serde::{Deserialize, Serialize};

pub static MEDIA_TYPE_IMAGE_INDEX: &str = "application/vnd.oci.image.index.v1+json";

pub static MEDIA_TYPE_DOCKER_SCHEMA2_MANIFEST: &str =
    "application/vnd.docker.distribution.manifest.v2+json";

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageFileInfo {
    pub path: String,
    pub link: String,
    pub size: u64,
    pub mode: u32,
    pub uid: u64,
    pub gid: u64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageLayer {
    pub created: String,
    pub digest: String,
    pub cmd: String,
    pub size: u64,
    pub files: Vec<ImageFileInfo>,
    pub empty: bool,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageAnalysisResult {
    pub created: String,
    pub architecture: String,
    pub layers: Vec<ImageLayer>,
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
    pub fn guess_manifest(&self, arch: &str, os: &str) -> ImageIndexManifest {
        for item in &self.manifests {
            if item.platform.architecture == arch && item.platform.os == os {
                return item.clone();
            }
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
