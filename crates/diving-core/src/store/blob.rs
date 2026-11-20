use super::CacheStore;
use crate::error::{Error, Result};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use nanoid::nanoid;
use ring::digest::{digest as ring_digest, Context, SHA256};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use tokio::fs;
use tracing::{info, warn};

/// Reject path traversal and other unsafe blob identifiers before they are
/// joined onto the layer cache directory. Accepts normal OCI digests
/// (`sha256:` + hex) and a conservative set of other content-addressable
/// forms used by local/tar layouts.
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
            Component::CurDir => {}
            _ => return false,
        }
    }
    normals == 1
}

/// Resolve the on-disk path for `digest` inside `layer_dir`.
///
/// Unsafe digests are remapped to a content-addressed placeholder under the
/// cache root so a caller can never escape via `../`.
pub(crate) fn blob_path_in(layer_dir: &Path, digest: &str) -> PathBuf {
    let name = if is_safe_blob_id(digest) {
        digest.to_string()
    } else {
        let hash = blake3::hash(digest.as_bytes());
        format!("_unsafe_{}", hash.to_hex())
    };
    layer_dir.join(name)
}

/// sha256 hex of an in-memory buffer.
pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(ring_digest(&SHA256, data).as_ref())
}

/// sha256 hex of a file on disk, streamed in 64 KB chunks（同步阻塞 I/O；
/// 大文件调用方应放在 spawn_blocking 中运行）.
pub fn sha256_hex_of_file(path: &Path) -> std::io::Result<String> {
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

/// Sibling temp path used for atomic blob/file writes. Kept in the same
/// directory so the final `rename` never crosses a filesystem boundary;
/// TTL sweeps also collect orphaned temp files.
pub fn tmp_sibling_path(file: &Path) -> PathBuf {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "blob".to_string());
    file.with_file_name(format!("{name}.tmp-{}", nanoid!(8)))
}

/// Save a blob atomically via a same-directory temp file + rename.
pub async fn save_blob_to_file(store: &CacheStore, digest: &str, data: &Bytes) -> Result<()> {
    let file = store.blob_path(digest);
    let tmp = tmp_sibling_path(&file);
    if let Err(source) = fs::write(&tmp, data).await {
        let _ = fs::remove_file(&tmp).await;
        return Err(Error::Io { source });
    }
    if let Err(source) = fs::rename(&tmp, &file).await {
        let _ = fs::remove_file(&tmp).await;
        return Err(Error::Io { source });
    }
    Ok(())
}

/// Sweep expired regular files in `dir` whose access/modification timestamp is
/// at or before `expired_ts`. Uses `read_dir` rather than globbing so paths
/// containing `[`, `]`, or `*` work safely.
pub(crate) async fn sweep_expired_dir(dir: &Path, expired_ts: i64) -> Result<usize> {
    let mut entries = fs::read_dir(dir)
        .await
        .map_err(|source| Error::Io { source })?;
    let mut removed = 0usize;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|source| Error::Io { source })?
    {
        let path = entry.path();
        let meta = match fs::metadata(&path).await {
            Ok(m) => m,
            Err(e) => {
                warn!(err = e.to_string(), file = %path.display(), "failed to stat cache file");
                continue;
            }
        };
        if !meta.is_file() {
            continue;
        }
        let Ok(time) = meta.accessed().or_else(|_| meta.modified()) else {
            continue;
        };
        let t: DateTime<Utc> = DateTime::from(time);
        if t.timestamp() > expired_ts {
            continue;
        }
        match fs::remove_file(&path).await {
            Ok(()) => removed += 1,
            Err(e) => {
                warn!(err = e.to_string(), file = %path.display(), "failed to clear cache file")
            }
        }
    }
    Ok(removed)
}

/// Remove layer blobs older than `store.layer_ttl()`.
pub async fn clear_blob_files(store: &CacheStore) -> Result<usize> {
    let expired = Utc::now().timestamp() - store.layer_ttl().as_secs() as i64;
    sweep_expired_dir(store.layer_dir(), expired).await
}

/// Evict least-recently-accessed layer blobs until total directory bytes fit
/// within `store.max_layer_cache_size()`. Returns `(evicted_count, freed_bytes)`.
pub async fn enforce_layer_cache_limit(store: &CacheStore) -> Result<(usize, u64)> {
    let Some(max) = store.max_layer_cache_size() else {
        return Ok((0, 0));
    };
    let mut entries = fs::read_dir(store.layer_dir())
        .await
        .map_err(|source| Error::Io { source })?;
    let mut files: Vec<(PathBuf, u64, i64)> = vec![];
    let mut total: u64 = 0;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|source| Error::Io { source })?
    {
        let path = entry.path();
        let Ok(meta) = fs::metadata(&path).await else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let size = meta.len();
        let atime = meta
            .accessed()
            .or_else(|_| meta.modified())
            .map(|t| DateTime::<Utc>::from(t).timestamp())
            .unwrap_or(0);
        total += size;
        files.push((path, size, atime));
    }
    if total <= max {
        return Ok((0, 0));
    }
    files.sort_by_key(|&(_, _, atime)| atime);
    let mut evicted = 0usize;
    let mut freed = 0u64;
    for (path, size, _) in files {
        if total <= max {
            break;
        }
        match fs::remove_file(&path).await {
            Ok(()) => {
                total = total.saturating_sub(size);
                evicted += 1;
                freed += size;
                info!(file = %path.display(), size, "evicted blob to enforce cache size limit");
            }
            Err(e) => warn!(err = e.to_string(), "failed to evict blob"),
        }
    }
    Ok((evicted, freed))
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
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join("sample.bin");
        std::fs::write(&tmp, b"abc").unwrap();
        assert_eq!(
            sha256_hex_of_file(&tmp).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
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
    fn blob_path_never_escapes_cache_root() {
        let root = Path::new("/cache/layers");
        let safe = blob_path_in(
            root,
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        );
        let unsafe_path = blob_path_in(root, "../../etc/passwd");
        assert_eq!(safe.parent(), unsafe_path.parent());
        assert!(unsafe_path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("_unsafe_"));
    }
}
