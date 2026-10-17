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
use diving::diff::{diff_images, to_diff_markdown};
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

// ---- OCI image layouts ------------------------------------------------

use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::{json, Value};
use std::io::Write;

fn sha256(data: &[u8]) -> String {
    format!("sha256:{}", diving::store::sha256_hex(data))
}

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(data).unwrap();
    gz.finish().unwrap()
}

/// Content-addressed blobs of an OCI layout, plus helpers to add them.
#[derive(Default)]
struct Blobs(Vec<(String, Vec<u8>)>);

impl Blobs {
    fn add(&mut self, data: Vec<u8>) -> (String, usize) {
        let digest = sha256(&data);
        let size = data.len();
        let path = format!("blobs/{}", digest.replacen(':', "/", 1));
        self.0.push((path, data));
        (digest, size)
    }

    fn add_json(&mut self, value: &Value) -> (String, usize) {
        self.add(serde_json::to_vec(value).unwrap())
    }

    /// One image: a gzip layer holding `files`, its config and manifest.
    /// Returns the manifest descriptor.
    fn image(&mut self, arch: &str, files: &[(&str, &[u8])]) -> Value {
        let (layer, layer_size) = self.add(gzip(&common::tar_bytes(files)));
        let (config, config_size) = self.add_json(&json!({
            "architecture": arch,
            "os": "linux",
            "created": "2024-06-01T00:00:00Z",
            "history": [{"created": "2024-06-01T00:00:00Z", "created_by": "/bin/sh -c #(nop) COPY . /"}],
            "rootfs": {"type": "layers", "diff_ids": ["sha256:aaa"]},
            "config": {"User": "app", "Env": [], "Entrypoint": ["/app/run"], "Cmd": []}
        }));
        let (manifest, manifest_size) = self.add_json(&json!({
            "schemaVersion": 2,
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "config": {"mediaType": "application/vnd.oci.image.config.v1+json", "digest": config, "size": config_size},
            "layers": [{"mediaType": "application/vnd.oci.image.layer.v1.tar+gzip", "digest": layer, "size": layer_size}]
        }));
        json!({
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "digest": manifest,
            "size": manifest_size,
            "platform": {"architecture": arch, "os": "linux"}
        })
    }

    /// The layout's top-level files plus every blob.
    fn layout(mut self, index: Value) -> Vec<(String, Vec<u8>)> {
        self.0.push((
            "oci-layout".into(),
            br#"{"imageLayoutVersion":"1.0.0"}"#.to_vec(),
        ));
        self.0
            .push(("index.json".into(), serde_json::to_vec(&index).unwrap()));
        self.0
    }
}

fn file_paths(result: &diving::image::DockerAnalyzeResult) -> Vec<String> {
    diving::image::final_filesystem(&result.file_tree_list)
        .into_keys()
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn analyze_oci_archive_with_gzip_layers() {
    common::use_default_config();
    let mut blobs = Blobs::default();
    // Single-image layouts often omit the platform in index.json.
    let mut manifest = blobs.image(
        "amd64",
        &[
            ("app/run", b"#!/bin/sh\nexec /app/bin"),
            ("app/bin", &[0x7f; 4096]),
        ],
    );
    manifest.as_object_mut().unwrap().remove("platform");
    let files = blobs.layout(json!({"schemaVersion": 2, "manifests": [manifest]}));
    let entries: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(p, d)| (p.as_str(), d.as_slice()))
        .collect();

    let tmp = tempfile::Builder::new().suffix(".tar").tempfile().unwrap();
    std::fs::write(tmp.path(), common::tar_bytes(&entries)).unwrap();
    let image = format!("file://{}", tmp.path().display());
    let result = analyze_docker_image(parse_image_info(&image), Lang::En, true, false, None)
        .await
        .expect("analyze OCI archive");

    assert_eq!(result.arch, "amd64");
    assert_eq!(result.layers.len(), 1);
    assert!(result.layers[0].media_type.ends_with("+gzip"));
    assert_eq!(file_paths(&result), ["app/bin", "app/run"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn analyze_oci_layout_directory_picks_the_requested_platform() {
    common::use_default_config();
    let mut blobs = Blobs::default();
    let amd64 = blobs.image("amd64", &[("app/amd64-only", b"x86")]);
    let arm64 = blobs.image("arm64", &[("app/arm64-only", b"arm")]);
    // buildkit adds attestation manifests with an unknown platform.
    let (attestation, size) = blobs.add_json(&json!({"schemaVersion": 2}));
    let (nested, nested_size) = blobs.add_json(&json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.index.v1+json",
        "manifests": [
            {"mediaType": "application/vnd.oci.image.manifest.v1+json", "digest": attestation, "size": size,
             "platform": {"architecture": "unknown", "os": "unknown"}},
            amd64,
            arm64,
        ]
    }));
    // Multi-platform export: index.json → image index → manifests.
    let files = blobs.layout(json!({
        "schemaVersion": 2,
        "manifests": [{"mediaType": "application/vnd.oci.image.index.v1+json", "digest": nested, "size": nested_size}]
    }));
    let dir = tempfile::tempdir().unwrap();
    for (path, data) in &files {
        let target = dir.path().join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, data).unwrap();
    }

    for (arch, only) in [("arm64", "app/arm64-only"), ("amd64", "app/amd64-only")] {
        let image = format!("file://{}?arch={arch}", dir.path().display());
        let result = analyze_docker_image(parse_image_info(&image), Lang::En, true, false, None)
            .await
            .unwrap_or_else(|e| panic!("analyze OCI layout for {arch}: {e}"));
        assert_eq!(result.arch, arch);
        assert_eq!(file_paths(&result), [only]);
    }
}

async fn analyze_tar(data: Vec<u8>) -> diving::image::DockerAnalyzeResult {
    let tmp = tempfile::Builder::new().suffix(".tar").tempfile().unwrap();
    std::fs::write(tmp.path(), data).unwrap();
    let image = format!("file://{}", tmp.path().display());
    analyze_docker_image(parse_image_info(&image), Lang::En, true, false, None)
        .await
        .expect("analyze fixture")
}

/// Two releases sharing their first layer; docker-save names layers by path
/// (`layer2/layer.tar` in both), so only the diff ids tell them apart.
#[tokio::test(flavor = "multi_thread")]
async fn diff_two_local_releases() {
    use_default_config();
    let base = analyze_tar(build_fixture_tar()).await;
    let target = analyze_tar(common::build_fixture_v2_tar()).await;
    assert_eq!(base.layers[2].digest, target.layers[2].digest);
    let diff = diff_images(&base, &target);

    assert_eq!(diff.shared_layers, 1);
    assert_eq!(diff.base_layers.len(), 1);
    assert_eq!(diff.target_layers[0].layer, 3);
    assert_eq!(
        diff.size_delta,
        target.total_size as i64 - base.total_size as i64
    );
    let paths = |set: &diving::diff::ChangeSet| -> Vec<(String, i64)> {
        set.files
            .iter()
            .map(|f| (f.path.clone(), f.delta()))
            .collect()
    };
    // `usr/bin/old` is no longer whited out, so it is back.
    assert_eq!(
        paths(&diff.added),
        [
            ("data/extra.bin".to_string(), 300_000),
            ("usr/bin/old".to_string(), 13)
        ]
    );
    assert_eq!(
        paths(&diff.changed),
        [
            ("data/model.bin".to_string(), 1_000_000),
            ("etc/config.yml".to_string(), 1)
        ]
    );
    assert_eq!(diff.removed.count, 0);
    assert_eq!(diff.dirs[0].path, "data");
    assert_eq!(diff.dirs[0].delta(), 1_300_000);

    let md = to_diff_markdown(&diff, Lang::En, None);
    assert!(md.contains("`data/extra.bin`"), "{md}");
}
