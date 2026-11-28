//! End-to-end test of the analyze pipeline over a `file://` docker-save tar
//! and OCI layouts using `diving_core::Analyzer`.

mod common;

use common::{build_fixture_tar, build_fixture_v2_tar, tar_bytes};
use diving_core::{
    diff_images, final_filesystem, sha256_hex, to_diff_markdown, to_markdown, AnalyzeOptions,
    Analyzer, BaseLayerMode, BaseLayerSkipReason, CacheConfig, ChangeSet, DiffOptions,
    DockerAnalyzeResult, Error, FileTreeItem, Lang, Op, ParityStatus,
};
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::{json, Value};
use std::fs::write as write_bytes;
use std::io::Write;
use tar::{Builder as TarBuilder, Header as TarHeader};
use tempfile::Builder;

fn local_analyzer() -> Analyzer {
    Analyzer::builder()
        .allow_local_file(true)
        .build()
        .expect("build local analyzer")
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
    let tmp = Builder::new()
        .suffix(".tar")
        .tempfile()
        .expect("create temp tar");
    std::fs::write(tmp.path(), build_fixture_tar()).expect("write fixture");

    let image = format!("file://{}", tmp.path().display());
    let analyzer = local_analyzer();
    let result = analyzer
        .analyze(&image, AnalyzeOptions::default().verify_dup(false))
        .await
        .expect("analyze fixture image");

    assert_eq!(result.arch, "amd64");
    assert_eq!(result.os, "linux");
    assert_eq!(result.envs.len(), 2);

    assert_eq!(result.layers.len(), 3);
    assert!(!result.layers[0].empty);
    assert!(result.layers[1].empty);
    assert!(!result.layers[2].empty);
    assert!(result.layers[0].digest.starts_with("sha256:"));
    assert!(analyzer.has_layer_blob(&result.layers[0].digest));
    assert!(result.size > 0);
    assert!(result.total_size > 0);

    let tree0 = &result.file_tree_list[0];
    let app = find_leaf(tree0, "usr/bin/app").expect("usr/bin/app in layer1 tree");
    assert_eq!(app.size, 2048);
    assert!(result.file_tree_list[1].is_empty());
    let model = find_leaf(&result.file_tree_list[2], "data/model.bin").expect("model in layer3");
    assert_eq!(model.size, 1_500_000);

    let removed = result
        .file_summary_list
        .iter()
        .find(|s| s.info.path == "usr/bin/old")
        .expect("whiteout summary entry");
    assert_eq!(removed.op, Op::Removed);
    assert_eq!(removed.layer_index, 2);
    assert_eq!(removed.info.size, "legacy-binary".len() as u64);
    assert_eq!(removed.written_in, Some(0));
    let modified = result
        .file_summary_list
        .iter()
        .find(|s| s.info.path == "etc/config.yml")
        .expect("modified summary entry");
    assert_eq!(modified.op, Op::Modified);
    assert_eq!(modified.written_in, Some(0));

    let md = to_markdown(&result, false, Lang::En);
    assert!(
        md.contains("| `usr/bin/old` | 13 B | 1 | L1 → L3 deleted |"),
        "{md}"
    );

    assert!(result
        .sensitive_files
        .iter()
        .any(|s| s.path == "app/secret.pem"));
    assert!(result.tags.iter().any(|t| t.contains("Potential Secrets")));
    assert!(result
        .tags
        .iter()
        .any(|t| t.contains("Package Manager Cache")));

    assert!(result
        .big_modified_file_list
        .iter()
        .any(|f| f.path == "data/model.bin"));
    assert!(!result
        .big_modified_file_list
        .iter()
        .any(|f| f.path.starts_with("usr/")));

    assert!(result.dockerfile.contains("RUN apt-get install -y curl"));
    assert!(result.dockerfile.contains("ENV APP_MODE=prod"));

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

    let summary = result.summary();
    let json = serde_json::to_value(result.report(&summary)).unwrap();
    assert_eq!(json["efficiencyScore"], summary.score);
    assert_eq!(json["wastedSize"], summary.wasted_size);
    assert!(json["wastedPercent"].is_number());
    assert_eq!(json["arch"], "amd64");
    assert!(json["fileTreeList"].is_array());

    assert!(result.recommendations.iter().any(|r| r.category == "size"));
    assert!(result
        .recommendations
        .iter()
        .any(|r| r.category == "security"));

    assert!(summary.wasted_size > 0);
    assert!(summary.score < 100);

    let archive_path = tmp.path().to_str().expect("temp tar path");
    let (head, total) = analyzer
        .read_local_file_head(
            archive_path,
            &result.layers[0].digest,
            &result.layers[0].media_type,
            "etc/config.yml",
            64,
        )
        .expect("read local layer file head");
    assert_eq!(head, b"a: 1");
    assert_eq!(total, 4);
}

fn sha256(data: &[u8]) -> String {
    format!("sha256:{}", sha256_hex(data))
}

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(data).unwrap();
    gz.finish().unwrap()
}

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

    fn image(&mut self, arch: &str, files: &[(&str, &[u8])]) -> Value {
        let (layer, layer_size) = self.add(gzip(&tar_bytes(files)));
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

fn file_paths(result: &DockerAnalyzeResult) -> Vec<String> {
    final_filesystem(&result.file_tree_list)
        .into_keys()
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn analyze_oci_archive_with_gzip_layers() {
    let mut blobs = Blobs::default();
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

    let tmp = Builder::new().suffix(".tar").tempfile().unwrap();
    std::fs::write(tmp.path(), tar_bytes(&entries)).unwrap();
    let image = format!("file://{}", tmp.path().display());
    let result = local_analyzer()
        .analyze(&image, AnalyzeOptions::default().verify_dup(false))
        .await
        .expect("analyze OCI archive");

    assert_eq!(result.arch, "amd64");
    assert_eq!(result.layers.len(), 1);
    assert!(result.layers[0].media_type.ends_with("+gzip"));
    assert_eq!(file_paths(&result), ["app/bin", "app/run"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn analyze_oci_layout_directory_picks_the_requested_platform() {
    let mut blobs = Blobs::default();
    let amd64 = blobs.image("amd64", &[("app/amd64-only", b"x86")]);
    let arm64 = blobs.image("arm64", &[("app/arm64-only", b"arm")]);
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

    let analyzer = local_analyzer();
    for (arch, only) in [("arm64", "app/arm64-only"), ("amd64", "app/amd64-only")] {
        let image = format!("file://{}?arch={arch}", dir.path().display());
        let result = analyzer
            .analyze(&image, AnalyzeOptions::default().verify_dup(false))
            .await
            .unwrap_or_else(|e| panic!("analyze OCI layout for {arch}: {e}"));
        assert_eq!(result.arch, arch);
        assert_eq!(file_paths(&result), [only]);
    }
}

async fn analyze_tar(analyzer: &Analyzer, data: Vec<u8>) -> std::sync::Arc<DockerAnalyzeResult> {
    let tmp = Builder::new().suffix(".tar").tempfile().unwrap();
    std::fs::write(tmp.path(), data).unwrap();
    let image = format!("file://{}", tmp.path().display());
    analyzer
        .analyze(&image, AnalyzeOptions::default().verify_dup(false))
        .await
        .expect("analyze fixture")
}

#[tokio::test(flavor = "multi_thread")]
async fn diff_two_local_releases() {
    let analyzer = local_analyzer();
    let base = analyze_tar(&analyzer, build_fixture_tar()).await;
    let target = analyze_tar(&analyzer, build_fixture_v2_tar()).await;
    assert_eq!(base.layers[0].digest, target.layers[0].digest);
    assert_ne!(base.layers[2].digest, target.layers[2].digest);
    let diff = diff_images(&base, &target);

    assert_eq!(diff.shared_layers, 1);
    assert_eq!(diff.base_layers.len(), 1);
    assert_eq!(diff.target_layers[0].layer, 3);
    assert_eq!(
        diff.size_delta,
        target.total_size as i64 - base.total_size as i64
    );
    let paths = |set: &ChangeSet| -> Vec<(String, i64)> {
        set.files
            .iter()
            .map(|f| (f.path.clone(), f.delta()))
            .collect()
    };
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

fn multi_arch_fixture_tar(
    arch: &str,
    base_bytes: &[u8],
    app_bin: &[u8],
    config_json: &[u8],
    user: &str,
) -> Vec<u8> {
    let mut layer1_builder = TarBuilder::new(Vec::new());
    let mut h1 = TarHeader::new_gnu();
    h1.set_path("lib/libc.so.6").unwrap();
    h1.set_size(base_bytes.len() as u64);
    h1.set_mode(0o644);
    h1.set_uid(0);
    h1.set_gid(0);
    h1.set_cksum();
    layer1_builder.append(&h1, base_bytes).unwrap();
    let layer1 = layer1_builder.into_inner().unwrap();

    let mut layer2_builder = TarBuilder::new(Vec::new());
    for (path, data, mode) in [
        ("usr/bin/server", app_bin, 0o755),
        ("app/settings.json", config_json, 0o644),
    ] {
        let mut h = TarHeader::new_gnu();
        h.set_path(path).unwrap();
        h.set_size(data.len() as u64);
        h.set_mode(mode);
        h.set_uid(0);
        h.set_gid(0);
        h.set_cksum();
        layer2_builder.append(&h, data).unwrap();
    }
    let layer2 = layer2_builder.into_inner().unwrap();

    let config = format!(
        r#"{{
        "architecture": "{arch}",
        "created": "2024-06-01T00:00:00Z",
        "history": [
            {{"created": "2024-01-01T00:00:00Z", "created_by": "/bin/sh -c apt-get install -y libc6"}},
            {{"created": "2024-06-01T00:00:00Z", "created_by": "/bin/sh -c COPY server settings.json /"}}
        ],
        "os": "linux",
        "rootfs": {{"type": "layers", "diff_ids": ["sha256:base-{arch}", "sha256:app-{arch}"]}},
        "config": {{
            "User": "{user}",
            "Env": ["APP_ENV=prod"],
            "Entrypoint": ["/usr/bin/server"],
            "WorkingDir": "/app"
        }}
    }}"#
    );
    let manifest = br#"[{
        "Config": "config.json",
        "RepoTags": ["svc:latest"],
        "Layers": ["layer1/layer.tar", "layer2/layer.tar"]
    }]"#;
    tar_bytes(&[
        ("manifest.json", manifest),
        ("config.json", config.as_bytes()),
        ("layer1/layer.tar", &layer1),
        ("layer2/layer.tar", &layer2),
    ])
}

#[tokio::test(flavor = "multi_thread")]
async fn multi_arch_diff_skips_base_layers_and_hashes_equal_size_files() {
    let analyzer = local_analyzer();
    let amd_tar = Builder::new().suffix(".tar").tempfile().unwrap();
    let arm_tar = Builder::new().suffix(".tar").tempfile().unwrap();

    write_bytes(
        amd_tar.path(),
        multi_arch_fixture_tar(
            "amd64",
            &[1u8; 4096],
            &[0x7f; 1200],
            br#"{"mode":"prod","id":"A"}"#,
            "app",
        ),
    )
    .unwrap();
    // Same-size binary-only parity test first: arm64 base layer differs dramatically,
    // binary grows cleanly, settings.json has identical bytes.
    write_bytes(
        arm_tar.path(),
        multi_arch_fixture_tar(
            "arm64",
            &[2u8; 8192],
            &[0x7f; 2000],
            br#"{"mode":"prod","id":"A"}"#,
            "app",
        ),
    )
    .unwrap();

    let amd_ref = format!("file://{}", amd_tar.path().display());
    let arm_ref = format!("file://{}", arm_tar.path().display());
    let diff = analyzer
        .diff_with_options(
            &amd_ref,
            &arm_ref,
            AnalyzeOptions::default().verify_dup(false),
            DiffOptions::default(),
        )
        .await
        .unwrap();

    assert!(diff.base_layers_ignored);
    assert_eq!(
        diff.base_skip_reason,
        BaseLayerSkipReason::CrossArchCreatedGap
    );
    assert_eq!(
        (diff.base_skipped_layers, diff.target_skipped_layers),
        (1, 1)
    );
    // Base libc.so.6 (4096 -> 8192) was ignored; only usr/bin/server changed
    // (+800 raw bytes, crossing 1 extra 512-byte tar block in layer unpack size).
    assert_eq!(diff.upper_size_delta, 512);
    assert_eq!(diff.changed.count, 1);
    assert_eq!(diff.changed.files[0].path, "usr/bin/server");
    assert!(diff.changed.files[0].is_expected_binary_only());
    assert!(diff.content_verified);
    assert_eq!(diff.parity.status, ParityStatus::BinaryDiffsOnly);
    assert_eq!(diff.parity.expected_binary_changes, 1);
    assert_eq!(diff.parity.unexpected_file_changes, 0);

    // Same-size config content change (`"A"` -> `"B"`) + User config drift on arm64
    let arm_drift_tar = Builder::new().suffix(".tar").tempfile().unwrap();
    write_bytes(
        arm_drift_tar.path(),
        multi_arch_fixture_tar(
            "arm64",
            &[2u8; 8192],
            &[0x7f; 2000],
            br#"{"mode":"prod","id":"B"}"#,
            "root",
        ),
    )
    .unwrap();
    let drift_ref = format!("file://{}", arm_drift_tar.path().display());
    let drift = analyzer
        .diff_with_options(
            &amd_ref,
            &drift_ref,
            AnalyzeOptions::default().verify_dup(false),
            DiffOptions::default(),
        )
        .await
        .unwrap();

    assert!(drift.content_verified);
    assert_eq!(drift.parity.status, ParityStatus::ConfigOrFileDrift);
    assert_eq!(drift.parity.same_size_content_changes, 1);
    assert_eq!(drift.parity.config_changes, 1);
    assert!(drift
        .changed
        .files
        .iter()
        .any(|c| c.path == "app/settings.json" && c.content_changed));
    assert_eq!(drift.config_diff.user.as_ref().unwrap().target, "root");

    // Instant off-mode toggle without re-analyzing
    let base_res = analyze_tar(&analyzer, std::fs::read(amd_tar.path()).unwrap()).await;
    let target_res = analyze_tar(&analyzer, std::fs::read(arm_tar.path()).unwrap()).await;
    let full = analyzer
        .compute_diff(
            &amd_ref,
            &arm_ref,
            base_res,
            target_res,
            DiffOptions::default().base_layer_mode(BaseLayerMode::Off),
        )
        .await
        .unwrap();
    assert!(!full.base_layers_ignored);
    assert!(full.changed.files.iter().any(|c| c.path == "lib/libc.so.6"));
}

fn minimal_unstripped_elf64() -> Vec<u8> {
    const PROGBITS: u32 = 1;
    const SYMTAB: u32 = 2;
    const STRTAB: u32 = 3;
    let sections = [
        (".text", PROGBITS, 1_100_000u64),
        (".debug_info", PROGBITS, 300_000u64),
        (".symtab", SYMTAB, 4_096u64),
        (".strtab", STRTAB, 1_024u64),
        (".shstrtab", STRTAB, 0u64),
    ];
    let mut names = vec![0u8];
    let mut name_offsets = Vec::new();
    for (name, _, _) in sections {
        name_offsets.push(names.len() as u32);
        names.extend_from_slice(name.as_bytes());
        names.push(0);
    }
    let mut body = vec![0u8; 64];
    let mut headers = vec![(0u32, 0u32, 0u64, 0u64)];
    for (idx, (_, ty, size)) in sections[..sections.len() - 1].iter().enumerate() {
        let off = body.len() as u64;
        body.resize(body.len() + *size as usize, 0x90);
        headers.push((name_offsets[idx], *ty, off, *size));
    }
    let shstrtab_off = body.len() as u64;
    body.extend_from_slice(&names);
    headers.push((
        *name_offsets.last().unwrap(),
        STRTAB,
        shstrtab_off,
        names.len() as u64,
    ));
    let shoff = body.len() as u64;
    for (name, ty, off, size) in &headers {
        let mut sh = Vec::with_capacity(64);
        sh.extend_from_slice(&name.to_le_bytes());
        sh.extend_from_slice(&ty.to_le_bytes());
        sh.extend_from_slice(&[0; 16]);
        sh.extend_from_slice(&off.to_le_bytes());
        sh.extend_from_slice(&size.to_le_bytes());
        sh.extend_from_slice(&[0; 24]);
        body.extend_from_slice(&sh);
    }
    let hdr = &mut body[..64];
    hdr[..7].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1]);
    hdr[16..18].copy_from_slice(&2u16.to_le_bytes());
    hdr[18..20].copy_from_slice(&62u16.to_le_bytes());
    hdr[20..24].copy_from_slice(&1u32.to_le_bytes());
    hdr[40..48].copy_from_slice(&shoff.to_le_bytes());
    hdr[52..54].copy_from_slice(&64u16.to_le_bytes());
    hdr[58..60].copy_from_slice(&64u16.to_le_bytes());
    hdr[60..62].copy_from_slice(&(headers.len() as u16).to_le_bytes());
    hdr[62..64].copy_from_slice(&((headers.len() - 1) as u16).to_le_bytes());
    body
}

#[tokio::test(flavor = "multi_thread")]
async fn file_url_parity_blob_cache_and_fallback() {
    use std::fs::remove_file;
    use tempfile::tempdir;

    let cache_dir = tempdir().unwrap();
    let cfg = CacheConfig::persistent(
        cache_dir.path().join("layers"),
        cache_dir.path().join("analysis"),
    );
    let analyzer = Analyzer::builder()
        .cache(cfg)
        .allow_local_file(true)
        .build()
        .expect("build persistent local analyzer");

    let elf_bytes = minimal_unstripped_elf64();
    let dup_bytes = vec![0x5au8; 70_000];
    let l1_tar = {
        let mut b = TarBuilder::new(Vec::new());
        for (path, data, mode) in [
            ("./etc/app.conf", b"mode: initial\n".as_slice(), 0o644u32),
            ("./usr/share/dup.bin", dup_bytes.as_slice(), 0o644u32),
            ("usr/bin/myelf", elf_bytes.as_slice(), 0o755u32),
        ] {
            let mut h = TarHeader::new_gnu();
            h.set_path(path).unwrap();
            h.set_size(data.len() as u64);
            h.set_mode(mode);
            h.set_uid(0);
            h.set_gid(0);
            h.set_cksum();
            b.append(&h, data).unwrap();
        }
        b.into_inner().unwrap()
    };
    let l2_tar = tar_bytes(&[
        ("etc/.wh.app.conf", b""),
        ("opt/dup.bin", &dup_bytes),
        ("app/leak.env", b"AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\n"),
    ]);
    let config_json = json!({
        "architecture": "amd64",
        "os": "linux",
        "history": [
            {"created": "2024-06-01T00:00:00Z", "created_by": "COPY ./ /"},
            {"created": "2024-06-01T00:01:00Z", "created_by": "COPY ./app /app"}
        ],
        "rootfs": {"type": "layers", "diff_ids": [sha256(&l1_tar), sha256(&l2_tar)]},
        "config": {
            "User": "root:root",
            "Env": ["PATH=/usr/local/bin:/usr/bin"],
            "Entrypoint": ["/usr/bin/myelf"]
        }
    });
    let manifest_json = json!([{
        "Config": "config.json",
        "RepoTags": ["parity:latest"],
        "Layers": ["layer1/layer.tar", "layer2/layer.tar"]
    }]);
    let save_tar = tar_bytes(&[
        ("manifest.json", manifest_json.to_string().as_bytes()),
        ("config.json", config_json.to_string().as_bytes()),
        ("layer1/layer.tar", &l1_tar),
        ("layer2/layer.tar", &l2_tar),
    ]);
    let tar_file = Builder::new().suffix(".tar").tempfile().unwrap();
    write_bytes(tar_file.path(), &save_tar).unwrap();
    let image_ref = format!("file://{}", tar_file.path().display());

    let result = analyzer
        .analyze(&image_ref, AnalyzeOptions::default().verify_dup(true))
        .await
        .expect("analyze local tar with verify_dup=true");

    // 1. Unified Scratch / Distroless base OS fallback.
    assert_eq!(
        result.base_os,
        "Scratch / Distroless (no OS identifier found)"
    );
    // 2. Blobs cached for file:// layers -> duplicates, unstripped binaries, runtimeCompat populated.
    let l0_digest = &result.layers[0].digest;
    assert!(l0_digest.starts_with("sha256:"));
    assert!(analyzer.has_layer_blob(l0_digest));
    assert_eq!(result.duplicate_groups.len(), 1);
    let dup_paths: Vec<&str> = result.duplicate_groups[0]
        .paths
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    assert!(dup_paths.contains(&"usr/share/dup.bin"), "{dup_paths:?}");
    assert!(dup_paths.contains(&"opt/dup.bin"), "{dup_paths:?}");
    assert_eq!(result.unstripped_binaries.len(), 1);
    assert_eq!(result.unstripped_binaries[0].path, "usr/bin/myelf");
    assert_eq!(result.runtime_compat.libc, "static");
    // 3. Content secrets scanned in non-os-release layers and `./` normalized cleanly.
    assert!(
        result
            .sensitive_files
            .iter()
            .any(|s| s.path == "app/leak.env"),
        "{:?}",
        result.sensitive_files
    );
    assert!(result.recommendations.iter().any(|r| r.id == "runasroot"));
    assert!(!result
        .file_tree_list
        .iter()
        .flatten()
        .any(|node| node.name == "."));
    assert!(result
        .file_summary_list
        .iter()
        .any(|s| s.info.path == "etc/app.conf" && s.written_in == Some(0)));

    // 4. Cached blob read works; evicting the cached blob falls back to `manifest.json` `Layers`
    //    both for `sha256:<hex>` digests and raw `layer1/layer.tar` entry names.
    let archive_path = tar_file.path().to_str().unwrap();
    let (head, size) = analyzer
        .read_local_file_head(archive_path, l0_digest, "", "etc/app.conf", 8)
        .unwrap();
    assert_eq!((head.as_slice(), size), (&b"mode: in"[..], 14));

    remove_file(analyzer.layer_blob_path(l0_digest).unwrap()).unwrap();
    assert!(!analyzer.has_layer_blob(l0_digest));
    let full_after_evict = analyzer
        .read_local_file(archive_path, l0_digest, "", "etc/app.conf", 1024)
        .unwrap();
    assert_eq!(full_after_evict, b"mode: initial\n");
    let (head_after_evict, size_after_evict) = analyzer
        .read_local_file_head(archive_path, l0_digest, "", "etc/app.conf", 8)
        .unwrap();
    assert_eq!(
        (head_after_evict.as_slice(), size_after_evict),
        (&b"mode: in"[..], 14)
    );

    // A separate Analyzer with an empty cache also resolves `sha256:<hex>` from `docker save`
    // `manifest.json` and enforces `max_bytes` -> `Error::FileTooLarge`.
    let fresh_analyzer = local_analyzer();
    assert_eq!(
        fresh_analyzer
            .read_local_file(archive_path, l0_digest, "", "etc/app.conf", 1024)
            .unwrap(),
        b"mode: initial\n"
    );
    assert!(matches!(
        fresh_analyzer.read_local_file(archive_path, l0_digest, "", "etc/app.conf", 4),
        Err(Error::FileTooLarge {
            size: 14,
            limit: 4,
            ..
        })
    ));

    let (head_rel, _) = analyzer
        .read_local_file_head(archive_path, "layer1/layer.tar", "", "etc/app.conf", 14)
        .unwrap();
    assert_eq!(head_rel, b"mode: initial\n");

    // 5. OCI archive: evicting the cached blob falls back cleanly to `blobs/sha256/<hex>`.
    let oci_layer_raw = tar_bytes(&[("etc/conf.txt", b"oci-fallback-ok")]);
    let oci_layer = gzip(&oci_layer_raw);
    let oci_cfg = json!({
        "architecture": "amd64",
        "os": "linux",
        "history": [{"created": "2024-06-01T00:00:00Z", "created_by": "COPY . /"}],
        "rootfs": {"type": "layers", "diff_ids": [sha256(&oci_layer_raw)]}
    })
    .to_string()
    .into_bytes();
    let mut blobs = Blobs::default();
    let (oci_cfg_digest, oci_cfg_len) = blobs.add(oci_cfg);
    let (oci_layer_digest, oci_layer_len) = blobs.add(oci_layer);
    let oci_manifest = json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {
            "mediaType": "application/vnd.oci.image.config.v1+json",
            "digest": oci_cfg_digest,
            "size": oci_cfg_len,
        },
        "layers": [{
            "mediaType": "application/vnd.oci.image.layer.v1.tar+gzip",
            "digest": oci_layer_digest,
            "size": oci_layer_len,
        }]
    })
    .to_string()
    .into_bytes();
    let (oci_manifest_digest, oci_manifest_len) = blobs.add(oci_manifest);
    let oci_index = json!({
        "schemaVersion": 2,
        "manifests": [{
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "digest": oci_manifest_digest,
            "size": oci_manifest_len,
            "platform": {"os": "linux", "architecture": "amd64"}
        }]
    })
    .to_string()
    .into_bytes();
    let mut oci_entries = vec![
        (
            "oci-layout",
            br#"{"imageLayoutVersion":"1.0.0"}"#.as_slice(),
        ),
        ("index.json", oci_index.as_slice()),
    ];
    for (p, d) in &blobs.0 {
        oci_entries.push((p.as_str(), d.as_slice()));
    }
    let oci_file = Builder::new().suffix(".tar").tempfile().unwrap();
    write_bytes(oci_file.path(), tar_bytes(&oci_entries)).unwrap();

    let oci_res = analyzer
        .analyze(
            &format!("file://{}", oci_file.path().display()),
            AnalyzeOptions::default(),
        )
        .await
        .unwrap();
    let digest = &oci_res.layers[0].digest;
    let media = &oci_res.layers[0].media_type;
    assert!(analyzer.has_layer_blob(digest));
    remove_file(analyzer.layer_blob_path(digest).unwrap()).unwrap();
    assert!(!analyzer.has_layer_blob(digest));
    let oci_path = oci_file.path().to_str().unwrap();
    let full_bytes = analyzer
        .read_local_file(oci_path, digest, media, "etc/conf.txt", 1024)
        .unwrap();
    assert_eq!(full_bytes, b"oci-fallback-ok");
    let (head_bytes, total) = analyzer
        .read_local_file_head(oci_path, digest, media, "etc/conf.txt", 3)
        .unwrap();
    assert_eq!((head_bytes.as_slice(), total), (&b"oci"[..], 15));
}
