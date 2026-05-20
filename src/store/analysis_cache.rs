//! Caches the per-image `DockerAnalyzeResult` JSON so that repeated analyses
//! of the same image (same registry manifest digest + same requested arch)
//! skip layer decompression and file-tree construction entirely.
//!
//! Cache key: `<safe(digest)>_<safe(arch)>.json` under `~/.diving/analysis/`.
//! `safe(...)` rewrites `:` / `/` / `\` / space to `_` so the digest /
//! architecture (e.g. `sha256:abc...`, `arm/v7`) can be used as a filename
//! portably.
//!
//! Semantics: every operation is best-effort. A missing file, corrupt
//! JSON, schema mismatch, IO error, or full disk MUST NOT bubble out as
//! an error — the caller falls back to a full analysis. The only escape
//! hatch is `clear_analysis_files`, which mirrors `clear_blob_files` for
//! the cron sweep.

use chrono::{DateTime, Utc};
use glob::glob;
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};
use std::path::PathBuf;
use std::time::Duration;
use tokio::fs;
use tracing::warn;

use crate::config::{get_analysis_path, must_load_config};
use crate::image::DockerAnalyzeResult;

/// Bump when the on-disk payload format changes incompatibly with prior
/// releases. Mismatched entries are treated as cache misses and the
/// cleanup sweep will eventually remove them.
const SCHEMA_VERSION: u32 = 1;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Glob {} fail: {}", path, source))]
    Pattern {
        source: glob::PatternError,
        path: String,
    },
    #[snafu(display("IO {} fail: {}", file, source))]
    IO {
        source: std::io::Error,
        file: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct AnalysisCacheEntry {
    schema_version: u32,
    diving_version: String,
    digest: String,
    arch: String,
    cached_at: i64,
    result: DockerAnalyzeResult,
}

fn safe_segment(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '/' | ':' | '\\' | ' ' => '_',
            _ => c,
        })
        .collect()
}

fn cache_file_path(digest: &str, arch: &str) -> PathBuf {
    let arch_part = if arch.is_empty() {
        "default".to_string()
    } else {
        safe_segment(arch)
    };
    get_analysis_path().join(format!("{}_{}.json", safe_segment(digest), arch_part))
}

/// Try to read a cached analysis result. Any failure (missing, stale
/// schema, corrupt JSON, IO error) yields `None` so the caller falls
/// through to a full analysis.
pub async fn read_analysis(digest: &str, arch: &str) -> Option<DockerAnalyzeResult> {
    let path = cache_file_path(digest, arch);
    let data = fs::read(&path).await.ok()?;
    let entry: AnalysisCacheEntry = match serde_json::from_slice(&data) {
        Ok(e) => e,
        Err(e) => {
            warn!(
                err = e.to_string(),
                "failed to parse analysis cache; ignoring"
            );
            return None;
        }
    };
    if entry.schema_version != SCHEMA_VERSION {
        return None;
    }
    Some(entry.result)
}

/// Best-effort write — any error is logged and swallowed so analysis is
/// never blocked by a cache write failure. Recommendations are stripped
/// because they are language-specific and get rebuilt on read.
pub async fn write_analysis(digest: &str, arch: &str, result: &DockerAnalyzeResult) {
    let path = cache_file_path(digest, arch);
    let mut stripped = result.clone();
    stripped.recommendations = vec![];
    let entry = AnalysisCacheEntry {
        schema_version: SCHEMA_VERSION,
        diving_version: env!("CARGO_PKG_VERSION").to_string(),
        digest: digest.to_string(),
        arch: arch.to_string(),
        cached_at: Utc::now().timestamp(),
        result: stripped,
    };
    let data = match serde_json::to_vec(&entry) {
        Ok(d) => d,
        Err(e) => {
            warn!(
                err = e.to_string(),
                "failed to serialize analysis cache entry"
            );
            return;
        }
    };
    if let Err(e) = fs::write(&path, &data).await {
        warn!(err = e.to_string(), "failed to write analysis cache");
    }
}

async fn clear_one(file: PathBuf, expired: i64) -> Result<()> {
    let meta = fs::metadata(file.clone()).await.context(IOSnafu {
        file: file.to_string_lossy(),
    })?;
    // Prefer access time, fall back to modification time. Matches
    // `clear_blob` so the analysis cache decays with actual use.
    let time = meta.accessed().or(meta.modified()).context(IOSnafu {
        file: file.to_string_lossy(),
    })?;
    let t: DateTime<Utc> = DateTime::from(time);
    if t.timestamp() > expired {
        return Ok(());
    }
    fs::remove_file(file.clone()).await.context(IOSnafu {
        file: file.to_string_lossy(),
    })?;
    Ok(())
}

/// Sweep expired analysis cache entries. Shares the `layer_ttl` config
/// value with the blob sweep so users have one knob to tune cache lifetime.
pub async fn clear_analysis_files() -> Result<()> {
    let path = get_analysis_path().to_str().unwrap_or_default().to_string();
    if path.is_empty() {
        return Ok(());
    }
    let layer_ttl = must_load_config()
        .layer_ttl
        .clone()
        .unwrap_or_else(|| "90d".to_string());
    let ttl = layer_ttl
        .parse::<humantime::Duration>()
        .unwrap_or_else(|_| Duration::from_secs(90 * 24 * 3600).into());
    let expired = Utc::now().timestamp() - ttl.as_secs() as i64;

    let value = format!("{path}/*.json");
    for entry in (glob(value.as_str()).context(PatternSnafu {
        path: value.to_string(),
    })?)
    .flatten()
    {
        if let Err(e) = clear_one(entry, expired).await {
            warn!(err = e.to_string(), "failed to clear analysis cache file");
        }
    }
    Ok(())
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
    fn cache_file_path_uses_default_when_arch_empty() {
        let p1 = cache_file_path("sha256:abc", "");
        let p2 = cache_file_path("sha256:abc", "amd64");
        assert!(p1.to_string_lossy().ends_with("sha256_abc_default.json"));
        assert!(p2.to_string_lossy().ends_with("sha256_abc_amd64.json"));
    }
}
