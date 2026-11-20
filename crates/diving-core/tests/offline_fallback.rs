//! End-to-end test of the "registry unavailable → last known analysis"
//! fallback, against an in-process mock registry using `diving_core::Analyzer`.

mod common;

use axum::extract::{Path, State};
use axum::http::{header, HeaderName, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use chrono::DateTime;
use common::tar_bytes;
use diving_core::{
    sha256_hex, to_markdown, AnalyzeOptions, Analyzer, CacheConfig, DockerAnalyzeResult, ImageInfo,
    Lang,
};
use flate2::write::GzEncoder;
use flate2::Compression;
use std::collections::HashMap;
use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Runtime;

const MANIFEST_TYPE: &str = "application/vnd.docker.distribution.manifest.v2+json";

struct Image {
    manifest: Vec<u8>,
    manifest_digest: String,
    blobs: HashMap<String, Vec<u8>>,
}

fn digest_of(data: &[u8]) -> String {
    format!("sha256:{}", sha256_hex(data))
}

fn build_image() -> Image {
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(&tar_bytes(&[
        ("usr/bin/app", &[0x7f; 4096]),
        ("etc/app.conf", b"mode: prod"),
    ]))
    .unwrap();
    let layer = gz.finish().unwrap();
    let config = br#"{
        "architecture": "amd64",
        "created": "2024-06-01T00:00:00Z",
        "history": [{"created": "2024-06-01T00:00:00Z", "created_by": "/bin/sh -c #(nop) COPY . /"}],
        "os": "linux",
        "rootfs": {"type": "layers", "diff_ids": ["sha256:aaa"]},
        "config": {"User": "", "Env": ["PATH=/usr/bin"], "Entrypoint": ["/usr/bin/app"], "Cmd": []}
    }"#
    .to_vec();
    let manifest = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 2,
        "mediaType": MANIFEST_TYPE,
        "config": {
            "mediaType": "application/vnd.docker.container.image.v1+json",
            "size": config.len(),
            "digest": digest_of(&config),
        },
        "layers": [{
            "mediaType": "application/vnd.docker.image.rootfs.diff.tar.gzip",
            "size": layer.len(),
            "digest": digest_of(&layer),
        }],
    }))
    .unwrap();
    Image {
        manifest_digest: digest_of(&manifest),
        manifest,
        blobs: HashMap::from([(digest_of(&config), config), (digest_of(&layer), layer)]),
    }
}

async fn manifest(State(image): State<Arc<Image>>) -> Response {
    (
        [
            (header::CONTENT_TYPE, MANIFEST_TYPE.to_string()),
            (
                HeaderName::from_static("docker-content-digest"),
                image.manifest_digest.clone(),
            ),
        ],
        image.manifest.clone(),
    )
        .into_response()
}

async fn blob(State(image): State<Arc<Image>>, Path(digest): Path<String>) -> Response {
    match image.blobs.get(&digest) {
        Some(data) => data.clone().into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

struct MockRegistry {
    addr: SocketAddr,
    runtime: Option<Runtime>,
}

impl MockRegistry {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let router = Router::new()
            .route("/v2/team/app/manifests/{reference}", get(manifest))
            .route("/v2/team/app/blobs/{digest}", get(blob))
            .with_state(Arc::new(build_image()));
        let runtime = Runtime::new().unwrap();
        runtime.spawn(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            addr,
            runtime: Some(runtime),
        }
    }

    async fn stop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
        for _ in 0..100 {
            if TcpStream::connect(self.addr).is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("mock registry still accepts connections");
    }

    fn image(&self, tag: &str) -> ImageInfo {
        ImageInfo {
            registry: format!("http://{}/v2", self.addr),
            user: "team".to_string(),
            name: "app".to_string(),
            tag: tag.to_string(),
            arch: "amd64".to_string(),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn falls_back_to_last_known_analysis_when_registry_is_down() {
    let scratch = tempfile::tempdir().unwrap();
    let analyzer = Analyzer::builder()
        .cache(CacheConfig::persistent(
            scratch.path().join("layers"),
            scratch.path().join("analysis"),
        ))
        .build()
        .unwrap();

    let mut registry = MockRegistry::start();

    let fresh = analyzer
        .analyze_info(
            registry.image("latest"),
            AnalyzeOptions::default()
                .lang(Lang::En)
                .fallback_to_last_known(true),
        )
        .await
        .expect("analysis with the registry up");
    assert_eq!(fresh.stale_as_of, None);
    assert_eq!(fresh.layers.len(), 1);
    assert!(fresh.recommendations.iter().any(|r| r.id == "runasroot"));

    registry.stop().await;

    let strict = analyzer
        .analyze_info(
            registry.image("latest"),
            AnalyzeOptions::default()
                .lang(Lang::En)
                .fallback_to_last_known(false),
        )
        .await;
    assert!(
        strict.is_err(),
        "strict analysis must fail when the registry is down"
    );

    let stale = analyzer
        .analyze_info(
            registry.image("latest"),
            AnalyzeOptions::default()
                .lang(Lang::Zh)
                .fallback_to_last_known(true),
        )
        .await
        .expect("fallback to the last known analysis");
    let as_of = stale
        .stale_as_of
        .clone()
        .expect("fallback result is marked");
    assert!(DateTime::parse_from_rfc3339(&as_of).is_ok(), "{as_of}");
    assert_eq!(stale.layers, fresh.layers);
    assert_eq!(stale.file_tree_list, fresh.file_tree_list);
    let root = |lang_result: &DockerAnalyzeResult| {
        lang_result
            .recommendations
            .iter()
            .find(|r| r.id == "runasroot")
            .map(|r| r.title.clone())
    };
    assert_ne!(
        root(&stale),
        root(&fresh),
        "recommendations follow the requested language"
    );
    assert!(to_markdown(&stale, false, Lang::En).contains("Registry unavailable"));
    assert!(!to_markdown(&fresh, false, Lang::En).contains("Registry unavailable"));

    let unknown = analyzer
        .analyze_info(
            registry.image("v2"),
            AnalyzeOptions::default()
                .lang(Lang::En)
                .fallback_to_last_known(true),
        )
        .await;
    assert!(unknown.is_err(), "no cached analysis for an unseen tag");
}
