//! End-to-end test of the analyze pipeline over a `file://` docker-save tar.
//!
//! Builds a two-layer fixture image in a temp file and runs the full
//! `analyze_docker_image` flow — manifest parsing, tar indexing, per-layer
//! file scanning, whiteout/modify diffing, sensitive-file detection,
//! Dockerfile reconstruction, and recommendations — without any network.
//!
//! `verify_dup` is false so the analysis cache under `~/.diving/analysis`
//! is neither read nor written by this test.

mod common;

use common::{build_fixture_tar, use_default_config};
use diving::i18n::Lang;
use diving::image::{analyze_docker_image, parse_image_info, FileTreeItem, Op};

fn find_leaf<'a>(items: &'a [FileTreeItem], path: &str) -> Option<&'a FileTreeItem> {
    let (head, rest) = match path.split_once('/') {
        Some((h, r)) => (h, Some(r)),
        None => (path, None),
    };
    let node = items.iter().find(|i| i.name == head)?;
    match rest {
        Some(r) => find_leaf(&node.children, r),
        None => Some(node),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn analyze_local_tar_end_to_end() {
    use_default_config();
    let tmp = tempfile::Builder::new()
        .suffix(".tar")
        .tempfile()
        .expect("create temp tar");
    std::fs::write(tmp.path(), build_fixture_tar()).expect("write fixture");

    let image = format!("file://{}", tmp.path().display());
    let info = parse_image_info(&image);
    let result = analyze_docker_image(info, Lang::En, true, false, None)
        .await
        .expect("analyze fixture image");

    // Basic identity from the image config.
    assert_eq!(result.arch, "amd64");
    assert_eq!(result.os, "linux");
    assert_eq!(result.envs.len(), 2);

    // History → layer mapping: 3 history entries, middle one empty.
    assert_eq!(result.layers.len(), 3);
    assert!(!result.layers[0].empty);
    assert!(result.layers[1].empty);
    assert!(!result.layers[2].empty);
    assert_eq!(result.layers[0].digest, "layer1/layer.tar");
    assert!(result.size > 0);
    assert!(result.total_size > 0);

    // File trees: layer1 tree has the base files, empty layer has none.
    let tree0 = &result.file_tree_list[0];
    let app = find_leaf(tree0, "usr/bin/app").expect("usr/bin/app in layer1 tree");
    assert_eq!(app.size, 2048);
    assert!(result.file_tree_list[1].is_empty());
    let model = find_leaf(&result.file_tree_list[2], "data/model.bin").expect("model in layer3");
    assert_eq!(model.size, 1_500_000);

    // Whiteout → Removed with the previous layer's size; edit → Modified.
    let removed = result
        .file_summary_list
        .iter()
        .find(|s| s.info.path == "usr/bin/old")
        .expect("whiteout summary entry");
    assert_eq!(removed.op, Op::Removed);
    assert_eq!(removed.layer_index, 2);
    assert_eq!(removed.info.size, "legacy-binary".len() as u64);
    let modified = result
        .file_summary_list
        .iter()
        .find(|s| s.info.path == "etc/config.yml")
        .expect("modified summary entry");
    assert_eq!(modified.op, Op::Modified);

    // Sensitive-file detection.
    assert!(result
        .sensitive_files
        .iter()
        .any(|s| s.path == "app/secret.pem"));
    assert!(result.tags.iter().any(|t| t.contains("Potential Secrets")));
    // apt cache in layer1 triggers the pkg-cache tag.
    assert!(result
        .tags
        .iter()
        .any(|t| t.contains("Package Manager Cache")));

    // Big files only from layers created near the image timestamp: the
    // 2024-01-01 base layer is old, so only layer2's model.bin qualifies.
    assert!(result
        .big_modified_file_list
        .iter()
        .any(|f| f.path == "data/model.bin"));
    assert!(!result
        .big_modified_file_list
        .iter()
        .any(|f| f.path.starts_with("usr/")));

    // Dockerfile reconstruction from history.
    assert!(result.dockerfile.contains("RUN apt-get install -y curl"));
    assert!(result.dockerfile.contains("ENV APP_MODE=prod"));

    // Every recommendation carries a stable, unique rule id — the handle
    // `ignore_recommendations` uses.
    let mut ids: Vec<&str> = result
        .recommendations
        .iter()
        .map(|r| r.id.as_str())
        .collect();
    assert!(ids.iter().all(|id| !id.is_empty()), "{ids:?}");
    assert!(ids.contains(&"secfiles"), "{ids:?}");
    ids.sort_unstable();
    let total = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), total, "duplicate rule ids");

    // The serialized report adds the efficiency numbers that CI scripts and
    // dashboards need but `DockerAnalyzeResult` itself does not store.
    let summary = result.summary();
    let json = serde_json::to_value(result.report(&summary)).unwrap();
    assert_eq!(json["efficiencyScore"], summary.score);
    assert_eq!(json["wastedSize"], summary.wasted_size);
    assert!(json["wastedPercent"].is_number());
    assert_eq!(json["arch"], "amd64");
    assert!(json["fileTreeList"].is_array());

    // Derived recommendations exist (pkg cache + secret file at minimum).
    assert!(result.recommendations.iter().any(|r| r.category == "size"));
    assert!(result
        .recommendations
        .iter()
        .any(|r| r.category == "security"));

    // Efficiency summary: the modified + removed files count as waste.
    let summary = result.summary();
    assert!(summary.wasted_size > 0);
    assert!(summary.score < 100);
}
