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

use super::blob::tmp_sibling_path;
use crate::config::{get_analysis_path, must_load_config};
use crate::image::DockerAnalyzeResult;

/// Bump when the on-disk payload format changes incompatibly with prior
/// releases. Mismatched entries are treated as cache misses and the
/// cleanup sweep will eventually remove them.
///
/// v2: added `DockerAnalyzeResult::duplicate_groups` (cross-layer dup
/// detection). v1 caches lack the field and would silently surface zero
/// duplicates; bump forces a one-time re-analysis.
/// v3: added `DockerAnalyzeResult::runtime_compat` (ELF / glibc compat
/// probe of the entrypoint). v2 caches lack the field and would silently
/// suppress the new mismatch card; bump forces a one-time re-analysis.
/// v4: extended the runtime probe to unwrap shell-script wrappers
/// (`ENTRYPOINT ["/entrypoint.sh"]` + `CMD ["app"]` patterns) and to
/// fall back to a basename search across layers. v3 caches captured an
/// empty `runtime_compat` for those images; bump forces a re-analysis
/// so the new code actually gets to run.
const SCHEMA_VERSION: u32 = 4;

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

/// Serialize-only view for writes — borrows the result instead of cloning
/// the (potentially tens of MB) file trees. Field names must stay in sync
/// with [`AnalysisCacheEntry`] so reads keep round-tripping.
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
/// never blocked by a cache write failure. Recommendations are written
/// as-is: they are language-specific and always rebuilt on read (see the
/// analysis-cache hit path in docker.rs), and stripping them used to cost
/// a full clone of the result just to blank one small field.
pub async fn write_analysis(digest: &str, arch: &str, result: &DockerAnalyzeResult) {
    let path = cache_file_path(digest, arch);
    let entry = AnalysisCacheEntryRef {
        schema_version: SCHEMA_VERSION,
        diving_version: env!("CARGO_PKG_VERSION"),
        digest,
        arch,
        cached_at: Utc::now().timestamp(),
        result,
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
    // 与 blob 写入一致：同目录临时文件 + 原子 rename，读方不会读到半截 JSON。
    let tmp = tmp_sibling_path(&path);
    if let Err(e) = fs::write(&tmp, &data).await {
        warn!(err = e.to_string(), "failed to write analysis cache");
        return;
    }
    if let Err(e) = fs::rename(&tmp, &path).await {
        let _ = fs::remove_file(&tmp).await;
        warn!(
            err = e.to_string(),
            "failed to move analysis cache into place"
        );
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

    // `*`（而非 `*.json`）：崩溃残留的 `.json.tmp-*` 临时文件也一并按 TTL 回收
    let value = format!("{path}/*");
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
