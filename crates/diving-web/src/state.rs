use crate::http_util::token_fingerprint;
use diving_core::{
    parse_image_info, Analyzer, GateConfig, REGISTRY_LOCAL_DOCKER, REGISTRY_LOCAL_PODMAN,
};
use lru::LruCache;
use reqwest::Client;
use serde::Serialize;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Mutex as StdMutex;

const LATEST_IMAGES_CAP: usize = 20;

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    pub enabled: bool,
    pub token_required: bool,
}

/// Shared application state injected into web API, MCP, and webhook routers.
pub struct WebState {
    pub analyzer: Analyzer,
    pub http_client: Client,
    pub config_dir: PathBuf,
    pub gate_config: GateConfig,
    pub max_download_file_size: u64,
    pub mcp_status: McpStatus,
    pub mcp_allowed_hosts: Vec<String>,
    pub api_token_fingerprint: Option<Vec<u8>>,
    latest_images: StdMutex<LruCache<String, ()>>,
}

impl WebState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        analyzer: Analyzer,
        http_client: Client,
        config_dir: PathBuf,
        gate_config: GateConfig,
        max_download_file_size: u64,
        mcp_status: McpStatus,
        mcp_allowed_hosts: Vec<String>,
    ) -> Self {
        Self {
            analyzer,
            http_client,
            config_dir,
            gate_config,
            max_download_file_size,
            mcp_status,
            mcp_allowed_hosts,
            api_token_fingerprint: None,
            latest_images: StdMutex::new(LruCache::new(
                NonZeroUsize::new(LATEST_IMAGES_CAP).expect("non-zero cache capacity"),
            )),
        }
    }

    pub fn with_api_token(mut self, token: Option<&str>) -> Self {
        self.api_token_fingerprint = token
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(token_fingerprint);
        self
    }

    pub fn add_latest_image(&self, image: &str) {
        if let Ok(mut cache) = self.latest_images.lock() {
            cache.put(image.to_string(), ());
        }
    }

    pub fn latest_images(&self) -> Vec<String> {
        self.latest_images
            .lock()
            .map(|cache| cache.iter().map(|(k, _)| k.clone()).collect())
            .unwrap_or_default()
    }
}

/// Attach `?arch=<arch>` to `image` when not targeting single-image local daemons.
pub(crate) fn with_arch(image: &str, arch: Option<&str>) -> String {
    let image = image.trim();
    let arch = arch.map(str::trim).unwrap_or_default();
    let registry = parse_image_info(image).registry;
    let one_image = registry == REGISTRY_LOCAL_DOCKER || registry == REGISTRY_LOCAL_PODMAN;
    if arch.is_empty() || one_image || image.contains("?arch=") {
        return image.to_string();
    }
    format!("{image}?arch={arch}")
}
