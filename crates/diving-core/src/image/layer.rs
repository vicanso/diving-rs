use super::ImageFileInfo;
use crate::error::{Error, IoSnafu, Result, TarSnafu};
use flate2::read::MultiGzDecoder as GzipDecoder;
use serde::{Deserialize, Serialize};
use snafu::ResultExt;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Take};
use std::path::Path;
use tar::{Archive, EntryType};

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

/// Reject tar entries whose paths could escape the conceptual layer root.
fn is_safe_tar_path(path: &str) -> bool {
    let p = path.trim_start_matches("./");
    if p.starts_with('/') || p.starts_with('\\') {
        return false;
    }
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

fn file_type_bits(entry_type: EntryType) -> u32 {
    match entry_type {
        EntryType::Symlink => 0o120000,
        EntryType::Directory => 0o040000,
        EntryType::Char => 0o020000,
        EntryType::Block => 0o060000,
        EntryType::Fifo => 0o010000,
        _ => 0o100000,
    }
}

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
        if !is_safe_tar_path(&path) {
            continue;
        }
        let mut is_whiteout = None;
        let mut is_opaque = None;
        if let Some(filename) = Path::new(&path).file_name() {
            let name = filename.to_string_lossy();
            if name == ".wh..wh..opq" {
                let parent = Path::new(&path)
                    .parent()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_default();
                path = parent.trim_end_matches('/').to_string();
                is_whiteout = Some(true);
                is_opaque = Some(true);
            } else if let Some(stripped) = name.strip_prefix(".wh.") {
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
        let mode =
            (header.mode().context(TarSnafu {})? & 0o7777) | file_type_bits(header.entry_type());
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
pub fn path_under_dir(path: &str, dir: &str) -> bool {
    if dir.is_empty() {
        return true;
    }
    path == dir || path.starts_with(&format!("{dir}/"))
}

/// One-pass index of a local image archive: entry path → (data offset, size).
#[derive(Debug)]
pub struct TarIndex {
    tar_path: String,
    is_dir: bool,
    entries: HashMap<String, (u64, u64)>,
}

fn index_directory(
    root: &Path,
    dir: &Path,
    entries: &mut HashMap<String, (u64, u64)>,
) -> Result<()> {
    for entry in std::fs::read_dir(dir).context(IoSnafu {})? {
        let entry = entry.context(IoSnafu {})?;
        let path = entry.path();
        let meta = entry.metadata().context(IoSnafu {})?;
        if meta.is_dir() {
            index_directory(root, &path, entries)?;
        } else if meta.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            entries.insert(rel, (0, meta.len()));
        }
    }
    Ok(())
}

impl TarIndex {
    /// Scan the archive once and record every entry's data offset + size.
    pub fn build(tar: &str) -> Result<TarIndex> {
        let root = Path::new(tar);
        if root.is_dir() {
            let mut entries = HashMap::new();
            index_directory(root, root, &mut entries)?;
            return Ok(TarIndex {
                tar_path: tar.to_string(),
                is_dir: true,
                entries,
            });
        }
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
            is_dir: false,
            entries,
        })
    }

    pub fn size_of(&self, filename: &str) -> Option<u64> {
        self.entries.get(filename).map(|&(_, size)| size)
    }

    pub fn open_reader(&self, filename: &str) -> Result<(Take<File>, u64)> {
        let &(offset, size) = self.entries.get(filename).ok_or(Error::FileNotFound)?;
        let path = if self.is_dir {
            Path::new(&self.tar_path).join(filename)
        } else {
            Path::new(&self.tar_path).to_path_buf()
        };
        let mut file = File::open(path).context(TarSnafu {})?;
        file.seek(SeekFrom::Start(offset)).context(IoSnafu {})?;
        Ok((file.take(size), size))
    }

    pub fn read(&self, filename: &str) -> Result<Vec<u8>> {
        let (mut reader, size) = self.open_reader(filename)?;
        let mut content = Vec::with_capacity(size as usize);
        reader.read_to_end(&mut content).context(IoSnafu {})?;
        Ok(content)
    }
}

/// Scan a layer archive once and return the content of the first OS-release file found.
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

fn decode_layer<'a, R: Read + 'a>(reader: R, media_type: &str) -> Result<Box<dyn Read + 'a>> {
    Ok(if media_type.contains("gzip") {
        Box::new(GzipDecoder::new(reader))
    } else if media_type.contains("zstd") {
        Box::new(
            zstd::Decoder::new(reader).map_err(|source| Error::Decompress {
                message: format!("Zstd decode fail: {source}"),
            })?,
        )
    } else {
        Box::new(reader)
    })
}

/// One pass over a layer, handing each file in `targets` (spelled without `./`) to `on_file`.
pub fn read_files_from_layer<R: Read>(
    reader: R,
    media_type: &str,
    targets: &HashSet<String>,
    max_bytes: u64,
    mut on_file: impl FnMut(&str, &[u8]),
) -> Result<()> {
    if targets.is_empty() {
        return Ok(());
    }
    let mut archive = Archive::new(decode_layer(reader, media_type)?);
    let mut seen = 0;
    for entry in archive.entries().context(TarSnafu {})? {
        let mut entry = entry.context(TarSnafu {})?;
        let raw = entry
            .path()
            .context(TarSnafu {})?
            .to_string_lossy()
            .into_owned();
        let path = raw.trim_start_matches("./");
        if !targets.contains(path) {
            continue;
        }
        seen += 1;
        if entry.size() <= max_bytes {
            let mut content = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut content).context(IoSnafu {})?;
            on_file(path, &content);
        }
        if seen >= targets.len() {
            break;
        }
    }
    Ok(())
}

fn with_layer_entry<'a, R: Read + 'a, T>(
    reader: R,
    media_type: &str,
    filename: &str,
    on_match: impl FnOnce(&mut dyn Read, u64) -> Result<T>,
) -> Result<T> {
    let want = filename.trim_start_matches("./");
    let mut archive = Archive::new(decode_layer(reader, media_type)?);
    for entry in archive.entries().context(TarSnafu {})? {
        let mut entry = entry.context(TarSnafu {})?;
        let name = entry
            .path()
            .context(TarSnafu {})?
            .to_string_lossy()
            .to_string();
        if name.trim_start_matches("./") == want {
            let size = entry.size();
            return on_match(&mut entry, size);
        }
    }
    Err(Error::FileNotFound)
}

pub fn get_file_content_from_layer<R: Read>(
    reader: R,
    media_type: &str,
    filename: &str,
    max_bytes: u64,
) -> Result<Vec<u8>> {
    with_layer_entry(reader, media_type, filename, |entry, size| {
        if size > max_bytes {
            return Err(Error::FileTooLarge {
                file: filename.trim_start_matches("./").to_string(),
                size,
                limit: max_bytes,
            });
        }
        let mut content = Vec::with_capacity(size as usize);
        entry.read_to_end(&mut content).context(IoSnafu {})?;
        Ok(content)
    })
}

pub fn get_file_head_from_layer<R: Read>(
    reader: R,
    media_type: &str,
    filename: &str,
    limit: u64,
) -> Result<(Vec<u8>, u64)> {
    with_layer_entry(reader, media_type, filename, |entry, size| {
        let mut head = Vec::with_capacity(size.min(limit) as usize);
        entry
            .take(limit)
            .read_to_end(&mut head)
            .context(IoSnafu {})?;
        Ok((head, size))
    })
}

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
                    hasher.update_reader(&mut entry).context(IoSnafu {})?;
                    out.insert(k, hasher.finalize().to_hex().to_string());
                    if out.len() >= targets.len() {
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
        let dec = zstd::Decoder::new(reader).map_err(|source| Error::Decompress {
            message: format!("Zstd decode fail: {source}"),
        })?;
        walk!(dec)
    } else {
        walk!(reader)
    }
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageLayerInfo {
    pub size: u64,
    pub unpack_size: u64,
    pub files: Vec<ImageFileInfo>,
}

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
        let dec = zstd::Decoder::new(reader).map_err(|source| Error::Decompress {
            message: format!("Zstd decode fail: {source}"),
        })?;
        parse_layer!(dec)
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
    fn modes_carry_the_file_type_character() {
        use std::io::Cursor;
        use tar::{Builder, Header};

        let mut builder = Builder::new(Vec::new());
        append(&mut builder, "etc/app.conf", b"a: 1");
        let mut suid = Header::new_gnu();
        suid.set_path("usr/bin/su").unwrap();
        suid.set_size(2);
        suid.set_mode(0o4755);
        suid.set_uid(0);
        suid.set_gid(0);
        suid.set_cksum();
        builder.append(&suid, &b"su"[..]).unwrap();
        let mut link = Header::new_gnu();
        link.set_entry_type(EntryType::Symlink);
        link.set_path("bin/sh").unwrap();
        link.set_link_name("/bin/busybox").unwrap();
        link.set_size(0);
        link.set_mode(0o777);
        link.set_uid(0);
        link.set_gid(0);
        link.set_cksum();
        builder.append(&link, &b""[..]).unwrap();
        let data = builder.into_inner().unwrap();

        let info = get_files_from_layer(Cursor::new(data), "application/tar", 0).unwrap();
        let modes: Vec<(&str, &str)> = info
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.mode.as_str()))
            .collect();
        assert_eq!(
            modes,
            [
                ("etc/app.conf", "-rw-r--r--"),
                ("usr/bin/su", "-rwsr-xr-x"),
                ("bin/sh", "lrwxrwxrwx"),
            ]
        );
    }

    #[test]
    fn tar_index_reads_entries_after_one_scan() {
        use tar::Builder;

        let dir = tempfile::tempdir().unwrap();
        let tar_path = dir.path().join("sample.tar");
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
        assert!(matches!(index.read("missing"), Err(Error::FileNotFound)));

        let (mut reader, size) = index.open_reader("abc/layer.tar").unwrap();
        assert_eq!(size, 16);
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"layer-bytes-here");
    }

    #[test]
    fn layer_file_lookup_matches_dot_slash_spellings() {
        use std::io::Cursor;
        use tar::Builder;

        let mut builder = Builder::new(Vec::new());
        append(&mut builder, "./etc/hostname", b"web-1");
        append(&mut builder, "usr/bin/app", b"ELF!");
        let data = builder.into_inner().unwrap();

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
        let (head, size) =
            get_file_head_from_layer(Cursor::new(&data), "tar", "etc/hostname", 3).unwrap();
        assert_eq!((head.as_slice(), size), (&b"web"[..], 5));
        assert!(matches!(
            get_file_content_from_layer(Cursor::new(&data), "tar", "etc/hostname", 3),
            Err(Error::FileTooLarge {
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
        append(&mut builder, "x/.wh.f/.wh.f", &[]);
        append(&mut builder, "app/.wh.a.wh.b", &[]);
        let data = builder.into_inner().unwrap();

        let mut archive = Archive::new(Cursor::new(data));
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

        let mut archive = Archive::new(Cursor::new(data));
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
