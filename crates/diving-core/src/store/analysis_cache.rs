//! Caches the per-image `DockerAnalyzeResult` JSON so that repeated analyses
//! of the same image (same registry manifest digest + same platform) skip
//! layer decompression and file-tree construction entirely.

use super::blob::{sha256_hex, sweep_expired_dir, tmp_sibling_path};
use super::CacheStore;
use crate::error::Result;
use crate::image::DockerAnalyzeResult;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::{fs, task};
use tracing::warn;

const SCHEMA_VERSION: u32 = 6;

#[derive(Debug, Serialize, Deserialize)]
struct AnalysisCacheEntry {
    schema_version: u32,
    diving_version: String,
    digest: String,
    arch: String,
    cached_at: i64,
    result: DockerAnalyzeResult,
}

/// Serialize-only view for writes — borrows the result instead of cloning
/// the (potentially tens of MB) file trees.
#[derive(Serialize)]
struct AnalysisCacheEntryRef<'a> {
    schema_version: u32,
    diving_version: &'a str,
    digest: &'a str,
    arch: &'a str,
    cached_at: i64,
    result: &'a DockerAnalyzeResult,
}

fn safe_segment(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '/' | ':' | '\\' | ' ' => '_',
            _ => c,
        })
        .collect()
}

fn cache_file_path_in(analysis_dir: &Path, digest: &str, arch: &str) -> PathBuf {
    let name = if arch.is_empty() {
        format!("{}.json", safe_segment(digest))
    } else {
        format!("{}_{}.json", safe_segment(digest), safe_segment(arch))
    };
    analysis_dir.join(name)
}

const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

async fn read_entry(store: &CacheStore, digest: &str, arch: &str) -> Option<AnalysisCacheEntry> {
    let path = cache_file_path_in(store.analysis_dir(), digest, arch);
    let entry: AnalysisCacheEntry = task::spawn_blocking(move || {
        let file = File::open(&path).ok()?;
        let mut reader = BufReader::new(file);
        let is_zstd = reader
            .fill_buf()
            .ok()
            .is_some_and(|buf| buf.len() >= 4 && buf[..4] == ZSTD_MAGIC);
        let parsed = if is_zstd {
            let dec = zstd::stream::Decoder::new(reader).ok()?;
            serde_json::from_reader(dec)
        } else {
            serde_json::from_reader(reader)
        };
        match parsed {
            Ok(e) => Some(e),
            Err(e) => {
                warn!(
                    err = e.to_string(),
                    "failed to parse analysis cache; ignoring"
                );
                None
            }
        }
    })
    .await
    .ok()??;
    (entry.schema_version == SCHEMA_VERSION).then_some(entry)
}

/// Try to read a cached analysis result. Any failure yields `None`.
pub async fn read_analysis(
    store: &CacheStore,
    digest: &str,
    arch: &str,
) -> Option<DockerAnalyzeResult> {
    read_entry(store, digest, arch)
        .await
        .map(|entry| entry.result)
}

fn last_known_path(store: &CacheStore, image_ref: &str, arch: &str) -> PathBuf {
    let arch_part = if arch.is_empty() {
        "default".to_string()
    } else {
        safe_segment(arch)
    };
    store.analysis_dir().join(format!(
        "ref_{}_{}.txt",
        sha256_hex(image_ref.as_bytes()),
        arch_part
    ))
}

/// Record that `image_ref` currently resolves to manifest `digest`.
pub async fn remember_digest(store: &CacheStore, image_ref: &str, arch: &str, digest: &str) {
    let path = last_known_path(store, image_ref, arch);
    let tmp = tmp_sibling_path(&path);
    if let Err(e) = fs::write(&tmp, digest).await {
        let _ = fs::remove_file(&tmp).await;
        warn!(err = e.to_string(), "failed to record last known digest");
        return;
    }
    if let Err(e) = fs::rename(&tmp, &path).await {
        let _ = fs::remove_file(&tmp).await;
        warn!(err = e.to_string(), "failed to record last known digest");
    }
}

/// The analysis cached for the digest `image_ref` last resolved to, with
/// the Unix time it was made.
pub async fn read_last_known(
    store: &CacheStore,
    image_ref: &str,
    arch: &str,
) -> Option<(DockerAnalyzeResult, i64)> {
    let digest = fs::read_to_string(last_known_path(store, image_ref, arch))
        .await
        .ok()?;
    let digest = digest.trim();
    let entry = match read_entry(store, digest, arch).await {
        Some(entry) => entry,
        None => read_entry(store, digest, "").await?,
    };
    Some((entry.result, entry.cached_at))
}

/// Best-effort atomic write of the analysis cache entry.
pub async fn write_analysis(
    store: &CacheStore,
    digest: &str,
    arch: &str,
    result: Arc<DockerAnalyzeResult>,
) {
    let path = cache_file_path_in(store.analysis_dir(), digest, arch);
    let digest = digest.to_string();
    let arch = arch.to_string();
    let join = task::spawn_blocking(move || {
        let entry = AnalysisCacheEntryRef {
            schema_version: SCHEMA_VERSION,
            diving_version: env!("CARGO_PKG_VERSION"),
            digest: &digest,
            arch: &arch,
            cached_at: Utc::now().timestamp(),
            result: result.as_ref(),
        };
        let tmp = tmp_sibling_path(&path);
        let outcome = (|| -> std::io::Result<()> {
            let file = File::create(&tmp)?;
            let mut encoder = zstd::stream::Encoder::new(BufWriter::new(file), 3)?;
            serde_json::to_writer(&mut encoder, &entry)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            let mut writer = encoder.finish()?;
            writer.flush()?;
            std::fs::rename(&tmp, &path)?;
            Ok(())
        })();
        if let Err(e) = outcome {
            let _ = std::fs::remove_file(&tmp);
            warn!(err = e.to_string(), "failed to write analysis cache");
        }
    })
    .await;
    if let Err(e) = join {
        warn!(err = e.to_string(), "analysis cache task failed");
    }
}

/// Sweep expired analysis cache entries using `store.layer_ttl()`.
pub async fn clear_analysis_files(store: &CacheStore) -> Result<usize> {
    let expired = Utc::now().timestamp() - store.layer_ttl().as_secs() as i64;
    sweep_expired_dir(store.analysis_dir(), expired).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_segment_rewrites_forbidden_chars() {
        assert_eq!(safe_segment("sha256:abcdef"), "sha256_abcdef");
        assert_eq!(safe_segment("arm/v7"), "arm_v7");
        assert_eq!(safe_segment("a:b/c\\d e"), "a_b_c_d_e");
        assert_eq!(safe_segment("amd64"), "amd64");
    }

    #[test]
    fn cache_file_path_leaves_the_platform_out_when_arch_empty() {
        let root = Path::new("/tmp/analysis");
        let p1 = cache_file_path_in(root, "sha256:abc", "");
        let p2 = cache_file_path_in(root, "sha256:abc", "amd64");
        assert!(p1.to_string_lossy().ends_with("/sha256_abc.json"));
        assert!(p2.to_string_lossy().ends_with("/sha256_abc_amd64.json"));
    }
}
