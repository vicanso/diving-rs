use crate::error::HTTPError;
use flate2::read::MultiGzDecoder as GzipDecoder;
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;
use tar::Archive;

use super::ImageFileInfo;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("File not found"))]
    NotFound,
    #[snafu(display("Read fail: {}", source))]
    Read { source: std::io::Error },
    #[snafu(display("Zstd decode fail: {}", source))]
    ZstdDecode { source: std::io::Error },
    #[snafu(display("Tar fail: {}", source))]
    Tar { source: std::io::Error },
}

impl From<Error> for HTTPError {
    fn from(err: Error) -> Self {
        HTTPError::new_with_category(&err.to_string(), "layer")
    }
}
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Wraps any `Read` and counts the total bytes read, letting us measure the
/// decompressed size without buffering the full output.
struct CountingReader<R: Read> {
    inner: R,
    count: u64,
}

impl<R: Read> CountingReader<R> {
    fn new(inner: R) -> Self {
        Self { inner, count: 0 }
    }
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count += n as u64;
        Ok(n)
    }
}

/// Reject tar entries whose paths could escape the conceptual layer root
/// (absolute paths or any component equal to `..`). diving never extracts
/// these to disk, but unsafe entries would still pollute the in-memory
/// file tree and the report, so they are filtered at ingestion.
fn is_safe_tar_path(path: &str) -> bool {
    let p = path.trim_start_matches("./");
    if p.starts_with('/') || p.starts_with('\\') {
        return false;
    }
    // Windows-style drive letter, e.g. `C:\...`.
    let bytes = p.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return false;
    }
    for component in p.split(['/', '\\']) {
        if component == ".." {
            return false;
        }
    }
    true
}

/// Parse every tar entry header from `archive`, collecting file metadata.
/// File content is never read — the tar crate reads and discards it when
/// advancing to the next entry.
fn collect_tar_entries<R: Read>(archive: &mut Archive<R>) -> Result<Vec<ImageFileInfo>> {
    let mut files = vec![];
    for entry in archive.entries().context(TarSnafu {})? {
        let file = entry.context(TarSnafu {})?;
        let header = file.header();
        if header.entry_type().is_dir() {
            continue;
        }
        let mut link = "".to_string();
        if let Some(value) = file.link_name().context(TarSnafu {})? {
            link = value.to_string_lossy().to_string();
        }
        let mut path = file
            .path()
            .context(TarSnafu {})?
            .to_string_lossy()
            .to_string();
        // Defense: silently skip malicious paths so they never enter the
        // file tree, the recommendations, or the AI payload.
        if !is_safe_tar_path(&path) {
            continue;
        }
        let mut is_whiteout = None;
        if let Some(filename) = Path::new(&path).file_name() {
            let name = filename.to_string_lossy();
            let prefix = ".wh.";
            if name.starts_with(prefix) {
                path = path.replace(name.to_string().as_str(), &name.replace(prefix, ""));
                is_whiteout = Some(true);
            }
        }
        let mode = header.mode().context(TarSnafu {})?;
        files.push(ImageFileInfo {
            path,
            link,
            size: file.size(),
            mode: unix_mode::to_string(mode),
            uid: header.uid().context(TarSnafu {})?,
            gid: header.gid().context(TarSnafu {})?,
            is_whiteout,
        });
    }
    Ok(files)
}

// 从tar中读取文件信息
pub async fn get_file_size_from_tar(tar: &str, filename: &str) -> Result<u64> {
    let file = File::open(tar).context(TarSnafu {})?;
    let mut a = Archive::new(file);
    for file in a.entries().context(TarSnafu {})? {
        let file = file.context(TarSnafu {})?;
        let name = file
            .path()
            .context(TarSnafu {})?
            .to_string_lossy()
            .to_string();
        if name == filename {
            return Ok(file.size());
        }
    }
    Ok(0)
}

// 从tar中读取文件内容
pub async fn get_file_content_from_tar(tar: &str, filename: &str) -> Result<Vec<u8>> {
    let file = File::open(tar).context(TarSnafu {})?;
    let mut a = Archive::new(file);
    for file in a.entries().context(TarSnafu {})? {
        let mut file = file.context(TarSnafu {})?;
        let name = file
            .path()
            .context(TarSnafu {})?
            .to_string_lossy()
            .to_string();
        if name == filename {
            let mut content = Vec::with_capacity(file.size() as usize);
            file.read_to_end(&mut content).context(ReadSnafu {})?;
            return Ok(content);
        }
    }
    Err(Error::NotFound {})
}

/// Scan a layer archive once and return the content of the first OS-release file found.
/// Returns `(matched_path, content)` or `None` if none of the candidates exist.
pub fn get_os_release_from_layer<R: Read>(
    reader: R,
    media_type: &str,
) -> Option<(&'static str, Vec<u8>)> {
    const CANDIDATES: &[&str] = &[
        "etc/os-release",
        "usr/lib/os-release",
        "etc/alpine-release",
        "etc/debian_version",
        "etc/redhat-release",
    ];

    macro_rules! scan {
        ($rdr:expr) => {{
            let mut archive = Archive::new($rdr);
            let entries = archive.entries().ok()?;
            for entry in entries {
                let mut entry = entry.ok()?;
                let raw = entry.path().ok()?;
                let name = raw.to_string_lossy();
                // Normalize leading "./"
                let name = name.trim_start_matches("./");
                if let Some(&hit) = CANDIDATES.iter().find(|&&c| c == name) {
                    let mut buf = Vec::new();
                    entry.read_to_end(&mut buf).ok()?;
                    return Some((hit, buf));
                }
            }
            None
        }};
    }

    if media_type.contains("gzip") {
        scan!(GzipDecoder::new(reader))
    } else if media_type.contains("zstd") {
        scan!(zstd::Decoder::new(reader).ok()?)
    } else {
        scan!(reader)
    }
}

// 从layer数据中读取指定文件内容（流式解压，只读取目标文件）
pub fn get_file_content_from_layer<R: Read>(
    reader: R,
    media_type: &str,
    filename: &str,
) -> Result<Vec<u8>> {
    macro_rules! find_file {
        ($reader:expr) => {{
            let mut archive = Archive::new($reader);
            for entry in archive.entries().context(TarSnafu {})? {
                let mut entry = entry.context(TarSnafu {})?;
                let name = entry
                    .path()
                    .context(TarSnafu {})?
                    .to_string_lossy()
                    .to_string();
                if name == filename {
                    let mut content = Vec::with_capacity(entry.size() as usize);
                    entry.read_to_end(&mut content).context(ReadSnafu {})?;
                    return Ok(content);
                }
            }
            Err(Error::NotFound {})
        }};
    }

    if media_type.contains("gzip") {
        find_file!(GzipDecoder::new(reader))
    } else if media_type.contains("zstd") {
        find_file!(zstd::Decoder::new(reader).context(ZstdDecodeSnafu {})?)
    } else {
        find_file!(reader)
    }
}

/// Hash a known set of files from a layer in a single tar pass.
///
/// Walks the (decompressed) archive entries, and for each entry whose
/// path is in `targets`, streams its bytes through blake3. Returns a
/// map keyed by the caller's spelling of the path (so the result can be
/// joined back to the candidate list without re-normalizing).
///
/// Tars store paths in two common ways — bare (`foo/bar.so`) or with a
/// leading `./` (`./foo/bar.so`). Both forms are matched against
/// `targets`, so the caller can pass either.
pub fn hash_files_from_layer<R: Read>(
    reader: R,
    media_type: &str,
    targets: &HashSet<String>,
) -> Result<HashMap<String, String>> {
    if targets.is_empty() {
        return Ok(HashMap::new());
    }
    macro_rules! walk {
        ($reader:expr) => {{
            let mut out: HashMap<String, String> = HashMap::new();
            let mut archive = Archive::new($reader);
            for entry in archive.entries().context(TarSnafu {})? {
                let mut entry = entry.context(TarSnafu {})?;
                let raw = entry
                    .path()
                    .context(TarSnafu {})?
                    .to_string_lossy()
                    .into_owned();
                let normalized = raw.trim_start_matches("./");
                let key = if targets.contains(&raw) {
                    Some(raw.clone())
                } else if targets.contains(normalized) {
                    Some(normalized.to_string())
                } else {
                    None
                };
                if let Some(k) = key {
                    let mut hasher = blake3::Hasher::new();
                    hasher.update_reader(&mut entry).context(ReadSnafu {})?;
                    out.insert(k, hasher.finalize().to_hex().to_string());
                    if out.len() >= targets.len() {
                        // All wanted files located; stop early.
                        break;
                    }
                }
            }
            Ok(out)
        }};
    }
    if media_type.contains("gzip") {
        walk!(GzipDecoder::new(reader))
    } else if media_type.contains("zstd") {
        walk!(zstd::Decoder::new(reader).context(ZstdDecodeSnafu {})?)
    } else {
        walk!(reader)
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageLayerInfo {
    // 原始（压缩）大小
    pub size: u64,
    // 解压后的大小
    pub unpack_size: u64,
    // 文件列表
    pub files: Vec<ImageFileInfo>,
}

// 从layer数据中读取所有文件信息
// 使用流式解压 + tar header-only 读取，不在内存中缓冲解压内容
pub fn get_files_from_layer<R: Read>(
    reader: R,
    media_type: &str,
    compressed_size: u64,
) -> Result<ImageLayerInfo> {
    macro_rules! parse_layer {
        ($reader:expr) => {{
            let mut counting = CountingReader::new($reader);
            let files = collect_tar_entries(&mut Archive::new(&mut counting))?;
            (files, counting.count)
        }};
    }

    let (files, unpack_size) = if media_type.contains("gzip") {
        parse_layer!(GzipDecoder::new(reader))
    } else if media_type.contains("zstd") {
        parse_layer!(zstd::Decoder::new(reader).context(ZstdDecodeSnafu {})?)
    } else {
        parse_layer!(reader)
    };

    Ok(ImageLayerInfo {
        files,
        size: compressed_size,
        unpack_size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_tar_paths_accept_normal_entries() {
        assert!(is_safe_tar_path("usr/bin/app"));
        assert!(is_safe_tar_path("./etc/os-release"));
        assert!(is_safe_tar_path("var/lib/dpkg/status"));
        assert!(is_safe_tar_path("a/b/c/d.txt"));
    }

    #[test]
    fn safe_tar_paths_reject_traversal_and_absolute() {
        assert!(!is_safe_tar_path("/etc/passwd"));
        assert!(!is_safe_tar_path("\\windows\\system32\\cmd.exe"));
        assert!(!is_safe_tar_path("../etc/shadow"));
        assert!(!is_safe_tar_path("a/../../etc/shadow"));
        assert!(!is_safe_tar_path("foo/../bar"));
        assert!(!is_safe_tar_path("C:\\windows\\system32"));
        assert!(!is_safe_tar_path("./../etc/shadow"));
    }
}
