use bytes::Bytes;
use libflate::gzip::Decoder;
use snafu::{ResultExt, Snafu};
use std::io::Read;
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
pub type Result<T, E = Error> = std::result::Result<T, E>;

// 解压gzip
fn gunzip(data: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = Decoder::new(data).context(GzipDecodeSnafu {})?;
    let mut decode_data = vec![];
    let _ = decoder
        .read_to_end(&mut decode_data)
        .context(GzipDecodeSnafu {})?;
    Ok(Bytes::copy_from_slice(&decode_data).to_vec())
}

// zstd解压
pub fn zstd_decode(data: &[u8]) -> Result<Vec<u8>> {
    let mut buf = vec![];
    zstd::stream::copy_decode(data, &mut buf).context(ZstdDecodeSnafu {})?;
    Ok(buf)
}

// 从分层数据中读取文件
pub async fn get_file_content_from_layer(
    data: &[u8],
    media_type: &str,
    filename: &str,
) -> Result<Vec<u8>> {
    let buf;
    let mut a = if media_type.contains("gzip") {
        buf = gunzip(data)?;
        Archive::new(&buf[..])
    } else if media_type.contains("zstd") {
        buf = zstd_decode(data)?;
        Archive::new(&buf[..])
    } else {
        Archive::new(data)
    };
    let mut content = vec![];
    for file in a.entries().context(TarSnafu {})? {
        let mut file = file.context(TarSnafu {})?;
        let name = file
            .path()
            .context(TarSnafu {})?
            .to_string_lossy()
            .to_string();
        if name == filename {
            file.read_to_end(&mut content).context(ReadSnafu {})?;
            break;
        }
    }
    if content.is_empty() {
        return Err(Error::NotFound {});
    }
    Ok(content)
}

// 从分层数据中读取所有文件信息
// "application/vnd.oci.image.layer.v1.tar+gzip",

pub async fn get_files_from_layer(data: &[u8], media_type: &str) -> Result<Vec<ImageFileInfo>> {
    let buf;
    // TODO 支持gzip zstd等
    let mut a = if media_type.contains("gzip") {
        buf = gunzip(data)?;
        Archive::new(&buf[..])
    } else if media_type.contains("zstd") {
        buf = zstd_decode(data)?;
        Archive::new(&buf[..])
    } else {
        Archive::new(data)
    };

    let mut files = vec![];
    for file in a.entries().context(TarSnafu {})? {
        let file = file.context(TarSnafu {})?;
        let header = file.header();
        // 不返回目录
        if header.entry_type().is_dir() {
            continue;
        }
        let mut link = "".to_string();

        if let Some(value) = file.link_name().context(TarSnafu {})? {
            link = value.to_string_lossy().to_string()
        }

        let info = ImageFileInfo {
            path: file
                .path()
                .context(TarSnafu {})?
                .to_string_lossy()
                .to_string(),
            link,
            size: file.size(),
            mode: header.mode().context(TarSnafu {})?,
            uid: header.uid().context(TarSnafu {})?,
            gid: header.gid().context(TarSnafu {})?,
        };
        files.push(info);
    }
    Ok(files)
}
