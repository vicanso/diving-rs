use serde::{Deserialize, Serialize};
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub path: String,
    pub link: String,
    pub size: u64,
    pub mode: u32,
    pub uid: u64,
    pub gid: u64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub created: String,
    pub digest: String,
    pub cmd: String,
    pub size: i64,
    pub files: Vec<FileInfo>,
    pub empty: bool,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisResult {
    pub created: String,
    pub architecture: String,
    pub layers: Vec<Layer>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageIndex {
    pub media_type: String,
    pub schema_version: i64,
    pub manifests: Vec<Manifest>,
}

impl ImageIndex {
    pub fn get_config_digest(&self, arch: &str, os: &str) -> String {
        // TODO 如果判断manifests中哪一个是config
        for item in &self.manifests {
            if item.platform.architecture == arch &&
                item.platform.os == os {
                    return item.digest.clone()
                }
        }
        "".to_string()
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub media_type: String,
    pub digest: String,
    pub size: i64,
    pub platform: Platform,
    pub annotations: Option<Annotations>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Platform {
    pub architecture: String,
    pub os: String,
    pub variant: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Annotations {
    #[serde(rename = "vnd.docker.reference.digest")]
    pub vnd_docker_reference_digest: String,
    #[serde(rename = "vnd.docker.reference.type")]
    pub vnd_docker_reference_type: String,
}
