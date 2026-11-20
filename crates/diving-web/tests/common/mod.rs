//! Shared test fixtures.
//!
//! Each integration test is its own crate and uses only some of these.
#![allow(dead_code)]

use diving_app::config::build_http_client;
use diving_core::{Analyzer, CacheConfig, GateConfig};
use diving_web::state::{McpStatus, WebState};
use std::sync::Arc;

pub fn test_web_state(mcp_status: McpStatus) -> Arc<WebState> {
    let http_client = build_http_client().unwrap();
    let analyzer = Analyzer::builder()
        .cache(CacheConfig::ephemeral())
        .http_client(http_client.clone())
        .allow_local_file(true)
        .singleflight(true)
        .build()
        .unwrap();
    Arc::new(WebState::new(
        analyzer,
        http_client,
        std::env::temp_dir(),
        GateConfig::default(),
        100 * 1024 * 1024,
        mcp_status,
        vec![],
    ))
}

pub fn tar_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
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
pub fn build_fixture_tar() -> Vec<u8> {
    let big = vec![0u8; 1_500_000];
    let layer2 = tar_bytes(&[
        ("etc/config.yml", b"a: 22"),
        ("usr/bin/.wh.old", b""),
        ("data/model.bin", &big),
    ]);
    fixture_tar(&layer2, "sha256:bbb")
}

/// The next release of [`build_fixture_tar`]: the same first layer, a
/// rebuilt second one — a bigger model, a new file, a longer config, and
/// `usr/bin/old` left in place.
pub fn build_fixture_v2_tar() -> Vec<u8> {
    let big = vec![0u8; 2_500_000];
    let extra = vec![2u8; 300_000];
    let layer2 = tar_bytes(&[
        ("etc/config.yml", b"a: 333"),
        ("data/model.bin", &big),
        ("data/extra.bin", &extra),
    ]);
    fixture_tar(&layer2, "sha256:ccc")
}

fn fixture_tar(layer2: &[u8], layer2_diff_id: &str) -> Vec<u8> {
    // Above MIN_PKG_CACHE_BYTES, so it counts as a real package cache.
    let apt_cache = vec![1u8; 1_200_000];
    let layer1 = tar_bytes(&[
        ("usr/bin/app", &[0x7f; 2048]),
        ("usr/bin/old", b"legacy-binary"),
        ("etc/config.yml", b"a: 1"),
        ("var/cache/apt/archives/x.deb", &apt_cache),
        ("app/secret.pem", b"-----BEGIN PRIVATE KEY-----"),
    ]);
    let config = r#"{
        "architecture": "amd64",
        "created": "2024-06-01T00:00:00Z",
        "history": [
            {"created": "2024-01-01T00:00:00Z", "created_by": "/bin/sh -c apt-get install -y curl"},
            {"created": "2024-06-01T00:00:00Z", "created_by": "/bin/sh -c #(nop)  ENV APP_MODE=prod", "empty_layer": true},
            {"created": "2024-06-01T00:00:00Z", "created_by": "/bin/sh -c #(nop) COPY dir:abc in /data"}
        ],
        "os": "linux",
        "rootfs": {"type": "layers", "diff_ids": ["sha256:aaa", "LAYER2_DIFF_ID"]},
        "config": {
            "User": "",
            "Env": ["PATH=/usr/local/bin:/usr/bin", "APP_MODE=prod"],
            "Entrypoint": ["/usr/bin/app"],
            "Cmd": []
        }
    }"#
    .replace("LAYER2_DIFF_ID", layer2_diff_id);
    let manifest = br#"[{
        "Config": "config.json",
        "RepoTags": ["fixture:latest"],
        "Layers": ["layer1/layer.tar", "layer2/layer.tar"]
    }]"#;
    tar_bytes(&[
        ("manifest.json", manifest),
        ("config.json", config.as_bytes()),
        ("layer1/layer.tar", &layer1),
        ("layer2/layer.tar", layer2),
    ])
}
