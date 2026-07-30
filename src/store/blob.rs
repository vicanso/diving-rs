use bytes::Bytes;
use chrono::{DateTime, Utc};
use glob::glob;
use nanoid::nanoid;
use ring::digest::{digest as ring_digest, Context, SHA256};
use snafu::{ResultExt, Snafu};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;
use tokio::fs;
use tracing::{info, warn};

use crate::config::{get_layer_path, get_max_layer_cache_size, must_load_config};
use crate::error::HTTPError;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Write file {} fail: {}", file, source))]
    Write {
        source: std::io::Error,
        file: String,
    },
    #[snafu(display("Read file {} fail: {}", file, source))]
    Read {
        source: std::io::Error,
        file: String,
    },
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
    #[snafu(display("Invalid blob digest: {}", digest))]
    InvalidDigest { digest: String },
}

impl From<Error> for HTTPError {
    fn from(err: Error) -> Self {
        // 对于部分error单独转换
        HTTPError::new_with_category(&err.to_string(), "blob")
    }
}

/// Reject path traversal and other unsafe blob identifiers before they are
/// joined onto the layer cache directory. Accepts normal OCI digests
/// (`sha256:` + hex) and a conservative set of other content-addressable
/// forms used by local/tar layouts.
///
/// Rules:
/// - non-empty, length ≤ 256
/// - no absolute path, no `..` components, no multi-segment paths
/// - only `[A-Za-z0-9._:+-]` characters (covers `sha256:…`)
pub fn is_safe_blob_id(digest: &str) -> bool {
    if digest.is_empty() || digest.len() > 256 {
        return false;
    }
    let path = Path::new(digest);
    if path.is_absolute() {
        return false;
    }
    let mut normals = 0usize;
    for c in path.components() {
        match c {
            Component::Normal(s) => {
                normals += 1;
                let s = s.to_string_lossy();
                if !s.chars().all(|ch| {
                    ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '+' | ':')
                }) {
                    return false;
                }
            }
            // `.` is useless but not dangerous; everything else (ParentDir,
            // RootDir, Prefix) is rejected.
            Component::CurDir => {}
            _ => return false,
        }
    }
    // Exactly one normal component — digests are flat filenames, never dirs.
    normals == 1
}

// 返回blob缓存文件路径（不检查文件是否存在）
//
// Unsafe digests are remapped to a content-addressed placeholder under the
// cache dir so a caller can never escape `layer_path` via `../`. Prefer
// rejecting at the API boundary with [`is_safe_blob_id`] when the value is
// user-controlled.
pub fn get_blob_path(digest: &str) -> PathBuf {
    let name = if is_safe_blob_id(digest) {
        digest.to_string()
    } else {
        // Stable, path-safe fallback so save/load of a bad id stay consistent
        // without ever joining `../…` onto the cache root.
        let hash = blake3::hash(digest.as_bytes());
        format!("_unsafe_{}", hash.to_hex())
    };
    get_layer_path().join(name)
}

/// sha256 hex of an in-memory buffer.
pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(ring_digest(&SHA256, data).as_ref())
}

/// sha256 hex of a file on disk, streamed in 64 KB chunks（同步阻塞 I/O；
/// 大文件调用方应放在 spawn_blocking 中运行）.
pub fn sha256_hex_of_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut ctx = Context::new(&SHA256);
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        ctx.update(&buf[..n]);
    }
    Ok(hex::encode(ctx.finish().as_ref()))
}

/// Sibling temp path used for atomic blob writes. Kept in the same
/// directory so the final `rename` never crosses a filesystem boundary;
/// the TTL sweep's `<layer_path>/*` glob also collects orphaned temps.
pub fn tmp_sibling_path(file: &Path) -> PathBuf {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "blob".to_string());
    file.with_file_name(format!("{name}.tmp-{}", nanoid!(8)))
}

// 将blob数据保存至文件。先写同目录临时文件再原子 rename，读方（含并发的
// 另一个分析任务/进程）不会读到写了一半的 blob。
pub async fn save_blob_to_file(digest: &str, data: &Bytes) -> Result<()> {
    let file = get_blob_path(digest);
    let tmp = tmp_sibling_path(&file);
    fs::write(&tmp, data).await.context(WriteSnafu {
        file: tmp.to_string_lossy(),
    })?;
    if let Err(source) = fs::rename(&tmp, &file).await {
        let _ = fs::remove_file(&tmp).await;
        return Err(Error::IO {
            source,
            file: file.to_string_lossy().to_string(),
        });
    }
    Ok(())
}

// 从文件中读取blob数据
pub async fn get_blob_from_file(digest: &str) -> Result<Vec<u8>> {
    let file = get_blob_path(digest);
    fs::read(file.clone()).await.context(ReadSnafu {
        file: file.to_string_lossy(),
    })
}

async fn clear_blob(file: PathBuf, expired: i64) -> Result<()> {
    let meta = fs::metadata(file.clone()).await.context(IOSnafu {
        file: file.to_string_lossy(),
    })?;
    // 优先用访问时间，再取修改时间
    let time = meta.accessed().or(meta.modified()).context(IOSnafu {
        file: file.to_string_lossy(),
    })?;

    // 未过期
    let t: DateTime<Utc> = DateTime::from(time);
    if t.timestamp() > expired {
        return Ok(());
    }
    fs::remove_file(file.clone()).await.context(IOSnafu {
        file: file.to_string_lossy(),
    })?;
    Ok(())
}

// 启动时清除较早下载的blob
pub async fn clear_blob_files() -> Result<()> {
    let path = get_layer_path().to_str().unwrap_or_default();
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

    let value = path.to_string() + "/*";
    for entry in (glob(value.as_str()).context(PatternSnafu {
        path: value.to_string(),
    })?)
    .flatten()
    {
        if let Err(e) = clear_blob(entry, expired).await {
            warn!(err = e.to_string(), "failed to clear blob file");
        }
    }
    Ok(())
}

/// 按配置强制 layer 缓存目录总大小上限：超出时按访问时间从旧到新淘汰，
/// 直到总量回到限额内。未配置 `max_layer_cache_size` 时是 no-op。
/// 与 TTL 清扫一起在定时任务 / CLI 启动时运行。
pub async fn enforce_layer_cache_limit() -> Result<()> {
    let Some(max) = get_max_layer_cache_size() else {
        return Ok(());
    };
    let dir = get_layer_path().to_str().unwrap_or_default();
    if dir.is_empty() {
        return Ok(());
    }
    let pattern = format!("{dir}/*");
    let mut files: Vec<(PathBuf, u64, i64)> = vec![];
    let mut total: u64 = 0;
    for entry in (glob(pattern.as_str()).context(PatternSnafu {
        path: pattern.clone(),
    })?)
    .flatten()
    {
        let Ok(meta) = fs::metadata(&entry).await else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let size = meta.len();
        let atime = meta
            .accessed()
            .or(meta.modified())
            .map(|t| DateTime::<Utc>::from(t).timestamp())
            .unwrap_or(0);
        total += size;
        files.push((entry, size, atime));
    }
    if total <= max {
        return Ok(());
    }
    // 最久未访问的先淘汰
    files.sort_by_key(|&(_, _, atime)| atime);
    for (path, size, _) in files {
        if total <= max {
            break;
        }
        match fs::remove_file(&path).await {
            Ok(()) => {
                total = total.saturating_sub(size);
                info!(file = %path.display(), size, "evicted blob to enforce cache size limit");
            }
            Err(e) => warn!(err = e.to_string(), "failed to evict blob"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_oci_sha256_digest() {
        let d = format!("sha256:{}", "a".repeat(64));
        assert!(is_safe_blob_id(&d));
    }

    #[test]
    fn rejects_path_traversal() {
        assert!(!is_safe_blob_id("../etc/passwd"));
        assert!(!is_safe_blob_id("..\\windows"));
        assert!(!is_safe_blob_id("/etc/passwd"));
        assert!(!is_safe_blob_id("foo/bar"));
        assert!(!is_safe_blob_id("sha256:abc/../x"));
    }

    #[test]
    fn rejects_empty_and_oversized() {
        assert!(!is_safe_blob_id(""));
        assert!(!is_safe_blob_id(&"a".repeat(257)));
    }

    #[test]
    fn sha256_helpers_match_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let tmp = std::env::temp_dir().join(format!("diving-sha-test-{}", nanoid!(8)));
        std::fs::write(&tmp, b"abc").unwrap();
        assert_eq!(
            sha256_hex_of_file(&tmp).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn tmp_sibling_stays_in_same_dir() {
        let file = Path::new("/cache/layers/sha256:abc");
        let tmp = tmp_sibling_path(file);
        assert_eq!(tmp.parent(), file.parent());
        assert!(tmp
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("sha256:abc.tmp-"));
    }

    #[test]
    fn get_blob_path_never_escapes_cache_root() {
        let safe = get_blob_path(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        let unsafe_path = get_blob_path("../../etc/passwd");
        // Both must be single-file children of the same parent (layer cache).
        assert_eq!(safe.parent(), unsafe_path.parent());
        assert!(unsafe_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("_unsafe_"));
    }
}
