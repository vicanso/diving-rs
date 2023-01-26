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
pub struct LayerInfos {
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
    pub layers: Vec<LayerInfos>,
}
