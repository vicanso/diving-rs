use super::elf::{RuntimeCompat, UnstrippedBinary};
use super::oci_image::{DuplicateGroup, FileTreeItem, ImageFileSummary, ImageLayer, Op};
use crate::recommend::Recommendation;
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::HashMap;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BigModifiedFileInfo {
    pub path: String,
    pub size: u64,
    pub digest: String,
    pub mode: String,
    pub uid: u64,
    pub gid: u64,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SensitiveFileInfo {
    pub path: String,
    pub size: u64,
    pub layer_index: usize,
    pub reason: String,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerAnalyzeResult {
    pub name: String,
    pub arch: String,
    pub os: String,
    pub user: String,
    pub envs: Vec<String>,
    pub labels: Vec<String>,
    pub dockerfile: String,
    pub base_os: String,
    pub supported_archs: Vec<String>,
    pub layers: Vec<ImageLayer>,
    pub size: u64,
    pub total_size: u64,
    pub file_tree_list: Vec<Vec<FileTreeItem>>,
    pub file_summary_list: Vec<ImageFileSummary>,
    pub big_modified_file_list: Vec<BigModifiedFileInfo>,
    pub sensitive_files: Vec<SensitiveFileInfo>,
    pub tags: Vec<String>,
    pub recommendations: Vec<Recommendation>,
    #[serde(default)]
    pub duplicate_groups: Vec<DuplicateGroup>,
    #[serde(default)]
    pub runtime_compat: RuntimeCompat,
    #[serde(default)]
    pub unstripped_binaries: Vec<UnstrippedBinary>,
    #[serde(default)]
    pub has_healthcheck: bool,
    #[serde(default)]
    pub exposed_ports: Vec<String>,
    #[serde(default)]
    pub entrypoint: Vec<String>,
    #[serde(default)]
    pub cmd: Vec<String>,
    #[serde(default)]
    pub working_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale_as_of: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize)]
pub struct ImageFileWastedSummary {
    pub path: String,
    pub total_size: u64,
    pub count: u32,
    pub changes: Vec<WasteChange>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize)]
pub struct WasteChange {
    pub written_in: Option<usize>,
    pub layer: usize,
    pub removed: bool,
}

#[derive(Default, Debug, Clone, Serialize)]
pub struct DockerAnalyzeSummary {
    pub wasted_list: Vec<ImageFileWastedSummary>,
    pub wasted_size: u64,
    pub wasted_percent: f64,
    pub score: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalyzeReport<'a> {
    #[serde(flatten)]
    pub result: &'a DockerAnalyzeResult,
    pub efficiency_score: u64,
    pub wasted_size: u64,
    pub wasted_percent: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePreview {
    pub name: String,
    pub arch: String,
    pub os: String,
    pub layers: Vec<ImageLayer>,
    pub size: u64,
    pub supported_archs: Vec<String>,
}

impl DockerAnalyzeResult {
    /// Wrap `self` with derived efficiency numbers for JSON serialization.
    pub fn report<'a>(&'a self, summary: &DockerAnalyzeSummary) -> AnalyzeReport<'a> {
        AnalyzeReport {
            result: self,
            efficiency_score: summary.score,
            wasted_size: summary.wasted_size,
            wasted_percent: summary.wasted_percent,
        }
    }

    /// Return a copy with `file_tree_list` cleared, avoiding cloning huge
    /// per-layer file trees for slim API/dashboard responses.
    pub fn without_file_trees(&self) -> Self {
        Self {
            name: self.name.clone(),
            arch: self.arch.clone(),
            os: self.os.clone(),
            user: self.user.clone(),
            envs: self.envs.clone(),
            labels: self.labels.clone(),
            dockerfile: self.dockerfile.clone(),
            base_os: self.base_os.clone(),
            supported_archs: self.supported_archs.clone(),
            layers: self.layers.clone(),
            size: self.size,
            total_size: self.total_size,
            file_tree_list: vec![],
            file_summary_list: self.file_summary_list.clone(),
            big_modified_file_list: self.big_modified_file_list.clone(),
            sensitive_files: self.sensitive_files.clone(),
            tags: self.tags.clone(),
            recommendations: self.recommendations.clone(),
            duplicate_groups: self.duplicate_groups.clone(),
            runtime_compat: self.runtime_compat.clone(),
            unstripped_binaries: self.unstripped_binaries.clone(),
            has_healthcheck: self.has_healthcheck,
            exposed_ports: self.exposed_ports.clone(),
            entrypoint: self.entrypoint.clone(),
            cmd: self.cmd.clone(),
            working_dir: self.working_dir.clone(),
            stale_as_of: self.stale_as_of.clone(),
        }
    }

    pub fn summary(&self) -> DockerAnalyzeSummary {
        let mut wasted_list: Vec<ImageFileWastedSummary> = vec![];
        let mut path_index: HashMap<&str, usize> = HashMap::new();
        let mut wasted_size = 0;
        for file in self.file_summary_list.iter() {
            let info = &file.info;
            wasted_size += info.size;
            let change = WasteChange {
                written_in: file.written_in,
                layer: file.layer_index,
                removed: file.op == Op::Removed,
            };
            if let Some(&i) = path_index.get(info.path.as_str()) {
                wasted_list[i].count += 1;
                wasted_list[i].total_size += info.size;
                wasted_list[i].changes.push(change);
            } else {
                path_index.insert(&info.path, wasted_list.len());
                wasted_list.push(ImageFileWastedSummary {
                    path: info.path.clone(),
                    count: 1,
                    total_size: info.size,
                    changes: vec![change],
                });
            }
        }
        wasted_list.sort_by_key(|b| Reverse(b.total_size));

        let (score, wasted_percent) = match wasted_size
            .checked_mul(100)
            .and_then(|n| n.checked_div(self.total_size))
        {
            None => (100u64, 0.0f64),
            Some(wasted_pct) => {
                let mut score = 100 - wasted_pct;
                if wasted_size != 0 {
                    score = score.saturating_sub(1);
                }
                (score, (wasted_size as f64) / (self.total_size as f64))
            }
        };
        DockerAnalyzeSummary {
            wasted_list,
            wasted_size,
            wasted_percent,
            score,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_handles_zero_total_size() {
        let result = DockerAnalyzeResult {
            total_size: 0,
            ..Default::default()
        };
        let summary = result.summary();
        assert_eq!(summary.score, 100);
        assert_eq!(summary.wasted_percent, 0.0);
        assert_eq!(summary.wasted_size, 0);
    }

    #[test]
    fn without_file_trees_drops_trees_only() {
        let result = DockerAnalyzeResult {
            name: "app:1".into(),
            total_size: 42,
            file_tree_list: vec![vec![FileTreeItem {
                name: "a".into(),
                ..Default::default()
            }]],
            ..Default::default()
        };
        let slim = result.without_file_trees();
        assert_eq!(slim.name, "app:1");
        assert_eq!(slim.total_size, 42);
        assert!(slim.file_tree_list.is_empty());
    }
}
