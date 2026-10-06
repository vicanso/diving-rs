//! Shared test fixtures.

use diving::config::set_config_file;

/// Point diving at the checked-in empty config instead of the developer's
/// `~/.diving/config.yml`. Call it first in every test: only the first call
/// in a process counts, and it must come before anything reads the config.
pub fn use_default_config() {
    set_config_file(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/config.yml"
    ));
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
