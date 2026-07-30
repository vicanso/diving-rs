//! End-to-end test of the analyze pipeline over a `file://` docker-save tar.
//!
//! Builds a two-layer fixture image in a temp file and runs the full
//! `analyze_docker_image` flow — manifest parsing, tar indexing, per-layer
//! file scanning, whiteout/modify diffing, sensitive-file detection,
//! Dockerfile reconstruction, and recommendations — without any network.
//!
//! `verify_dup` is false so the analysis cache under `~/.diving/analysis`
//! is neither read nor written by this test.

use diving::i18n::Lang;
use diving::image::{analyze_docker_image, parse_image_info, FileTreeItem, Op};

fn tar_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, data) in files {
        let mut h = tar::Header::new_gnu();
        h.set_path(path).unwrap();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_uid(0);
        h.set_gid(0);
        h.set_cksum();
        builder.append(&h, *data).unwrap();
    }
    builder.into_inner().unwrap()
}

/// A docker-save-format image tar:
///   layer1 (2024-01-01): base files + apt cache + a secret
///   (empty history entry: ENV)
///   layer2 (2024-06-01): modifies etc/config.yml, whiteouts usr/bin/old,
///                        adds a 1.5MB model file
fn build_fixture_tar() -> Vec<u8> {
    let big = vec![0u8; 1_500_000];
    let layer1 = tar_bytes(&[
        ("usr/bin/app", &[0x7f; 2048]),
        ("usr/bin/old", b"legacy-binary"),
        ("etc/config.yml", b"a: 1"),
        ("var/cache/apt/archives/x.deb", &[1u8; 2048]),
        ("app/secret.pem", b"-----BEGIN PRIVATE KEY-----"),
    ]);
    let layer2 = tar_bytes(&[
        ("etc/config.yml", b"a: 22"),
        ("usr/bin/.wh.old", b""),
        ("data/model.bin", &big),
    ]);
    let config = br#"{
        "architecture": "amd64",
        "created": "2024-06-01T00:00:00Z",
        "history": [
            {"created": "2024-01-01T00:00:00Z", "created_by": "/bin/sh -c apt-get install -y curl"},
            {"created": "2024-06-01T00:00:00Z", "created_by": "/bin/sh -c #(nop)  ENV APP_MODE=prod", "empty_layer": true},
            {"created": "2024-06-01T00:00:00Z", "created_by": "/bin/sh -c #(nop) COPY dir:abc in /data"}
        ],
        "os": "linux",
        "rootfs": {"type": "layers", "diff_ids": ["sha256:aaa", "sha256:bbb"]},
        "config": {
            "User": "",
            "Env": ["PATH=/usr/local/bin:/usr/bin", "APP_MODE=prod"],
            "Entrypoint": ["/usr/bin/app"],
            "Cmd": []
        }
    }"#;
    let manifest = br#"[{
        "Config": "config.json",
        "RepoTags": ["fixture:latest"],
        "Layers": ["layer1/layer.tar", "layer2/layer.tar"]
    }]"#;
    tar_bytes(&[
        ("manifest.json", manifest),
        ("config.json", config),
        ("layer1/layer.tar", &layer1),
        ("layer2/layer.tar", &layer2),
    ])
}

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
