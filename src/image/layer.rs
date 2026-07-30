use crate::error::HTTPError;
use flate2::read::MultiGzDecoder as GzipDecoder;
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Take};
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
    #[snafu(display("File {} is too large: {} bytes (limit {})", file, size, limit))]
    TooLarge { file: String, size: u64, limit: u64 },
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
        let mut is_opaque = None;
        if let Some(filename) = Path::new(&path).file_name() {
            let name = filename.to_string_lossy();
            // OCI opaque whiteout: hides *all* prior contents of the parent dir.
            // Spec name is exactly `.wh..wh..opq` (see image-spec layer.md).
            if name == ".wh..wh..opq" {
                let parent = Path::new(&path)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                // Normalize empty parent (opaque root) to "".
                path = parent.trim_end_matches('/').to_string();
                is_whiteout = Some(true);
                is_opaque = Some(true);
            } else if let Some(stripped) = name.strip_prefix(".wh.") {
                // 只改写最后一个路径段：`path.replace(name, …)` 会把路径里
                // 所有同名子串一并替换（目录本身叫 `.wh.x` 时父目录会被改错），
                // `name.replace(".wh.", "")` 也会误删文件名中间的标记
                // （`.wh.a.wh.b` 应还原为 `a.wh.b`）。
                let parent = Path::new(&path)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                path = if parent.is_empty() {
                    stripped.to_string()
                } else {
                    format!("{parent}/{stripped}")
                };
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
            is_opaque,
        });
    }
    Ok(files)
}

/// Whether `path` is under directory `dir` (or is `dir` itself).
/// Empty `dir` means the image root — every path is under it.
pub fn path_under_dir(path: &str, dir: &str) -> bool {
    if dir.is_empty() {
        return true;
    }
    path == dir || path.starts_with(&format!("{dir}/"))
}

/// One-pass index of a plain (uncompressed) image tar: file path →
/// (data offset, size). `docker save` / `file://` analysis used to rescan
/// the whole tar once per lookup — O(layers × tar size); building this
/// index once turns every later read into an open + seek + bounded read.
#[derive(Debug)]
pub struct TarIndex {
    tar_path: String,
    entries: HashMap<String, (u64, u64)>,
}

impl TarIndex {
    /// Scan the tar once and record every entry's data offset + size.
    /// 同步阻塞 I/O；调用方负责在 spawn_blocking 中运行。
    pub fn build(tar: &str) -> Result<TarIndex> {
        let file = File::open(tar).context(TarSnafu {})?;
        let mut archive = Archive::new(BufReader::new(file));
        let mut entries = HashMap::new();
        for entry in archive.entries().context(TarSnafu {})? {
            let entry = entry.context(TarSnafu {})?;
            let path = entry
                .path()
                .context(TarSnafu {})?
                .to_string_lossy()
                .to_string();
            entries.insert(path, (entry.raw_file_position(), entry.size()));
        }
        Ok(TarIndex {
            tar_path: tar.to_string(),
            entries,
        })
    }

    /// Size of an entry, or `None` when the tar has no such path.
    pub fn size_of(&self, filename: &str) -> Option<u64> {
        self.entries.get(filename).map(|&(_, size)| size)
    }

    /// Open a bounded reader positioned at the entry's data. Lets callers
    /// stream-decompress a layer without buffering it in memory.
    /// 同步阻塞 I/O；调用方负责在 spawn_blocking 中运行。
    pub fn open_reader(&self, filename: &str) -> Result<(Take<File>, u64)> {
        let &(offset, size) = self.entries.get(filename).ok_or(Error::NotFound {})?;
        let mut file = File::open(&self.tar_path).context(TarSnafu {})?;
        file.seek(SeekFrom::Start(offset)).context(ReadSnafu {})?;
        Ok((file.take(size), size))
    }

    /// Read an entry's full content（小文件用；大层请走 `open_reader` 流式）.
    pub fn read(&self, filename: &str) -> Result<Vec<u8>> {
        let (mut reader, size) = self.open_reader(filename)?;
        let mut content = Vec::with_capacity(size as usize);
        reader.read_to_end(&mut content).context(ReadSnafu {})?;
        Ok(content)
    }
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

// 从layer数据中读取指定文件内容（流式解压，只读取目标文件）。
// tar 里的路径可能带 `./` 前缀也可能不带，单趟内两种拼写都匹配——
// 调用方无需为另一种拼写把整层再解压一遍。
// `max_bytes`：目标文件超过上限直接报 `TooLarge`，在读之前拒绝，
// 避免把超大文件整个缓冲进内存。
pub fn get_file_content_from_layer<R: Read>(
    reader: R,
    media_type: &str,
    filename: &str,
    max_bytes: u64,
) -> Result<Vec<u8>> {
    let want = filename.trim_start_matches("./");
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
                if name.trim_start_matches("./") == want {
                    let size = entry.size();
                    if size > max_bytes {
                        return Err(Error::TooLarge {
                            file: want.to_string(),
                            size,
                            limit: max_bytes,
                        });
                    }
                    let mut content = Vec::with_capacity(size as usize);
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

    #[test]
    fn path_under_dir_matches_prefix() {
        assert!(path_under_dir("usr/bin/app", "usr"));
        assert!(path_under_dir("usr", "usr"));
        assert!(!path_under_dir("usrbin", "usr"));
        assert!(path_under_dir("anything", ""));
    }

    fn append(builder: &mut tar::Builder<Vec<u8>>, path: &str, data: &[u8]) {
        let mut h = tar::Header::new_gnu();
        h.set_path(path).unwrap();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_uid(0);
        h.set_gid(0);
        h.set_cksum();
        builder.append(&h, data).unwrap();
    }

    #[test]
    fn tar_index_reads_entries_after_one_scan() {
        use tar::Builder;

        let tar_path =
            std::env::temp_dir().join(format!("diving-tarindex-test-{}.tar", std::process::id()));
        let mut builder = Builder::new(Vec::new());
        append(&mut builder, "manifest.json", b"[]");
        append(&mut builder, "abc/layer.tar", b"layer-bytes-here");
        let data = builder.into_inner().unwrap();
        std::fs::write(&tar_path, &data).unwrap();

        let index = TarIndex::build(tar_path.to_str().unwrap()).unwrap();
        assert_eq!(index.size_of("manifest.json"), Some(2));
        assert_eq!(index.size_of("abc/layer.tar"), Some(16));
        assert_eq!(index.size_of("missing"), None);
        assert_eq!(index.read("manifest.json").unwrap(), b"[]");
        assert_eq!(index.read("abc/layer.tar").unwrap(), b"layer-bytes-here");
        assert!(matches!(index.read("missing"), Err(Error::NotFound {})));

        let (mut reader, size) = index.open_reader("abc/layer.tar").unwrap();
        assert_eq!(size, 16);
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"layer-bytes-here");

        let _ = std::fs::remove_file(&tar_path);
    }

    #[test]
    fn layer_file_lookup_matches_dot_slash_spellings() {
        use std::io::Cursor;
        use tar::Builder;

        let mut builder = Builder::new(Vec::new());
        append(&mut builder, "./etc/hostname", b"web-1");
        append(&mut builder, "usr/bin/app", b"ELF!");
        let data = builder.into_inner().unwrap();

        // tar 存 `./` 前缀，查询不带前缀也要命中；反之亦然。
        let content =
            get_file_content_from_layer(Cursor::new(&data), "tar", "etc/hostname", u64::MAX)
                .unwrap();
        assert_eq!(content, b"web-1");
        let content =
            get_file_content_from_layer(Cursor::new(&data), "tar", "./usr/bin/app", u64::MAX)
                .unwrap();
        assert_eq!(content, b"ELF!");
        assert!(
            get_file_content_from_layer(Cursor::new(&data), "tar", "missing", u64::MAX).is_err()
        );
        // 超过大小上限的文件在读之前就被拒绝
        assert!(matches!(
            get_file_content_from_layer(Cursor::new(&data), "tar", "etc/hostname", 3),
            Err(Error::TooLarge {
                size: 5,
                limit: 3,
                ..
            })
        ));
    }

    #[test]
    fn whiteout_rename_only_touches_last_segment() {
        use std::io::Cursor;
        use tar::Builder;

        let mut builder = Builder::new(Vec::new());
        // 目录本身叫 `.wh.f`，其下的 whiteout 不应改写父目录名
        append(&mut builder, "x/.wh.f/.wh.f", &[]);
        // 文件名中间也含 `.wh.` 标记，只剥前缀
        append(&mut builder, "app/.wh.a.wh.b", &[]);
        let data = builder.into_inner().unwrap();

        let mut archive = tar::Archive::new(Cursor::new(data));
        let files = collect_tar_entries(&mut archive).unwrap();

        let nested = files.iter().find(|f| f.path == "x/.wh.f/f").unwrap();
        assert_eq!(nested.is_whiteout, Some(true));
        let interior = files.iter().find(|f| f.path == "app/a.wh.b").unwrap();
        assert_eq!(interior.is_whiteout, Some(true));
    }

    #[test]
    fn collect_tar_entries_parses_whiteout_and_opaque() {
        use std::io::Cursor;
        use tar::Builder;

        let mut builder = Builder::new(Vec::new());
        append(&mut builder, "app/config.json", b"{}");
        append(&mut builder, "app/.wh.secret", &[]);
        append(&mut builder, "cache/.wh..wh..opq", &[]);
        let data = builder.into_inner().unwrap();

        let mut archive = tar::Archive::new(Cursor::new(data));
        let files = collect_tar_entries(&mut archive).unwrap();
        assert_eq!(files.len(), 3);

        let normal = files.iter().find(|f| f.path == "app/config.json").unwrap();
        assert!(normal.is_whiteout.is_none());
        assert!(normal.is_opaque.is_none());

        let wh = files.iter().find(|f| f.path == "app/secret").unwrap();
        assert_eq!(wh.is_whiteout, Some(true));
        assert!(wh.is_opaque.is_none());

        let opq = files.iter().find(|f| f.path == "cache").unwrap();
        assert_eq!(opq.is_whiteout, Some(true));
        assert_eq!(opq.is_opaque, Some(true));
    }
}
