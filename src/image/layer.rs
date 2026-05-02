use crate::error::HTTPError;
use libflate::gzip::Decoder as GzipDecoder;
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};
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
    #[snafu(display("Gzip decode fail: {}", source))]
    GzipDecode { source: std::io::Error },
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
        find_file!(GzipDecoder::new(reader).context(GzipDecodeSnafu {})?)
    } else if media_type.contains("zstd") {
        find_file!(zstd::Decoder::new(reader).context(ZstdDecodeSnafu {})?)
    } else {
        find_file!(reader)
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
        parse_layer!(GzipDecoder::new(reader).context(GzipDecodeSnafu {})?)
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
