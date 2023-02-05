use bytes::Bytes;
use libflate::gzip::Decoder;
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};
use std::{io::Read, path::Path};
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

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageLayerInfo {
    // 原始大小
    pub original_size: u64,
    // 解压后的大小
    pub size: u64,
    // 文件列表
    pub files: Vec<ImageFileInfo>,
}
pub async fn get_files_from_layer(data: &[u8], media_type: &str) -> Result<ImageLayerInfo> {
    let buf;
    let original_size = data.len() as u64;
    let mut size = original_size;
    // TODO 支持gzip zstd等
    let mut a = if media_type.contains("gzip") {
        buf = gunzip(data)?;
        size = buf.len() as u64;
        Archive::new(&buf[..])
    } else if media_type.contains("zstd") {
        buf = zstd_decode(data)?;
        size = buf.len() as u64;
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
        let mut path = file
            .path()
            .context(TarSnafu {})?
            .to_string_lossy()
            .to_string();
        let mut is_whiteout = None;
        // 为了实现这样的删除操作，AuFS 会在可读写层创建一个 whiteout 文件，把只读层里的文件“遮挡”起来。
        // .wh.
        // usr/local/bin/.wh.static
        if let Some(filename) = Path::new(&path).file_name() {
            let name = filename.to_string_lossy();
            let prefix = ".wh.";
            if name.starts_with(prefix) {
                path = path.replace(name.to_string().as_str(), &name.replace(prefix, ""));
                is_whiteout = Some(true);
            }
        }

        let info = ImageFileInfo {
            path,
            link,
            size: file.size(),
            mode: header.mode().context(TarSnafu {})?,
            uid: header.uid().context(TarSnafu {})?,
            gid: header.gid().context(TarSnafu {})?,
            is_whiteout,
        };
        files.push(info);
    }
    Ok(ImageLayerInfo {
        files,
        original_size,
        size,
    })
}
