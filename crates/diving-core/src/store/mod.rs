mod analysis_cache;
mod blob;
mod recent_cache;

use crate::error::{Error, Result};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;
use tracing::warn;

pub use analysis_cache::{
    clear_analysis_files, read_analysis, read_last_known, remember_digest, write_analysis,
};
pub use blob::{
    clear_blob_files, enforce_layer_cache_limit, is_safe_blob_id, save_blob_to_file, sha256_hex,
    sha256_hex_of_file, tmp_sibling_path,
};
pub use recent_cache::RecentCache;

/// Default TTL for both layer blob and analysis caches (90 days).
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(90 * 24 * 3600);

/// Where `diving-core` stores layer blobs and cached analysis JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheMode {
    /// Isolated temporary directory created at build time and removed when the
    /// `Analyzer` drops. Leaves `~/.diving` untouched.
    Ephemeral,
    /// Persistent caller-managed directories for layer blobs and analysis JSON.
    Persistent {
        layer_dir: PathBuf,
        analysis_dir: PathBuf,
    },
}

/// Cache configuration for [`crate::Analyzer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheConfig {
    pub mode: CacheMode,
    pub layer_ttl: Duration,
    pub max_layer_cache_size: Option<u64>,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self::ephemeral()
    }
}

impl CacheConfig {
    /// Isolated ephemeral cache backed by a temporary directory.
    pub fn ephemeral() -> Self {
        Self {
            mode: CacheMode::Ephemeral,
            layer_ttl: DEFAULT_CACHE_TTL,
            max_layer_cache_size: None,
        }
    }

    /// Persistent on-disk cache using explicit `layer_dir` and `analysis_dir`.
    pub fn persistent(layer_dir: impl Into<PathBuf>, analysis_dir: impl Into<PathBuf>) -> Self {
        Self {
            mode: CacheMode::Persistent {
                layer_dir: layer_dir.into(),
                analysis_dir: analysis_dir.into(),
            },
            layer_ttl: DEFAULT_CACHE_TTL,
            max_layer_cache_size: None,
        }
    }

    /// TTL applied to both cached layer blobs and analysis JSON entries.
    pub fn layer_ttl(mut self, ttl: Duration) -> Self {
        self.layer_ttl = ttl;
        self
    }

    /// Optional byte limit on the layer blob cache directory.
    pub fn max_layer_cache_size(mut self, max_bytes: Option<u64>) -> Self {
        self.max_layer_cache_size = max_bytes;
        self
    }
}

/// Summary of entries removed during [`crate::Analyzer::cleanup_caches`].
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CleanupStats {
    pub removed_blobs: usize,
    pub removed_analyses: usize,
    pub evicted_blobs: usize,
    pub freed_bytes: u64,
}

/// Instance-scoped disk cache manager (replaces prior global `~/.diving` state).
#[derive(Debug)]
pub(crate) struct CacheStore {
    layer_dir: PathBuf,
    analysis_dir: PathBuf,
    layer_ttl: Duration,
    max_layer_cache_size: Option<u64>,
    _temp_dir: Option<TempDir>,
}

impl CacheStore {
    pub(crate) fn from_config(cfg: CacheConfig) -> Result<Self> {
        let (layer_dir, analysis_dir, temp_dir) = match cfg.mode {
            CacheMode::Ephemeral => {
                let dir = TempDir::new().map_err(|source| Error::Io { source })?;
                let layers = dir.path().join("layers");
                let analysis = dir.path().join("analysis");
                fs::create_dir_all(&layers).map_err(|source| Error::Io { source })?;
                fs::create_dir_all(&analysis).map_err(|source| Error::Io { source })?;
                (layers, analysis, Some(dir))
            }
            CacheMode::Persistent {
                layer_dir,
                analysis_dir,
            } => {
                fs::create_dir_all(&layer_dir).map_err(|source| Error::Io { source })?;
                fs::create_dir_all(&analysis_dir).map_err(|source| Error::Io { source })?;
                (layer_dir, analysis_dir, None)
            }
        };

        Ok(Self {
            layer_dir,
            analysis_dir,
            layer_ttl: cfg.layer_ttl,
            max_layer_cache_size: cfg.max_layer_cache_size,
            _temp_dir: temp_dir,
        })
    }

    pub(crate) fn layer_dir(&self) -> &PathBuf {
        &self.layer_dir
    }

    pub(crate) fn analysis_dir(&self) -> &PathBuf {
        &self.analysis_dir
    }

    pub(crate) fn layer_ttl(&self) -> Duration {
        self.layer_ttl
    }

    pub(crate) fn max_layer_cache_size(&self) -> Option<u64> {
        self.max_layer_cache_size
    }

    pub(crate) fn blob_path(&self, digest: &str) -> PathBuf {
        blob::blob_path_in(&self.layer_dir, digest)
    }

    pub(crate) async fn cleanup(&self) -> Result<CleanupStats> {
        let removed_blobs = clear_blob_files(self).await?;
        let removed_analyses = match clear_analysis_files(self).await {
            Ok(n) => n,
            Err(err) => {
                warn!(err = err.to_string(), "clear analysis cache fail");
                0
            }
        };
        let (evicted_blobs, freed_bytes) = match enforce_layer_cache_limit(self).await {
            Ok(stats) => stats,
            Err(err) => {
                warn!(err = err.to_string(), "enforce cache size limit fail");
                (0, 0)
            }
        };
        Ok(CleanupStats {
            removed_blobs,
            removed_analyses,
            evicted_blobs,
            freed_bytes,
        })
    }
}
