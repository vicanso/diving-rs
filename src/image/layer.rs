use bytes::Bytes;
use libflate::gzip::Decoder;
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};
use std::io::Read;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Gzip decode fail: {}", source))]
    GzipDecode { source: std::io::Error },
    #[snafu(display("Tar fail: {}", source))]
    Tar { source: std::io::Error },
}
pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileInfo {
    pub path: String,
    pub link: String,
    pub size: u64,
    pub mode: u32,
    pub uid: u64,
    pub gid: u64,
}

pub async fn get_files_from_layer(data: Vec<u8>, is_gzip: bool) -> Result<Vec<FileInfo>> {
    let mut buf = data;
    if is_gzip {
        let mut decoder = Decoder::new(&buf[..]).context(GzipDecodeSnafu {})?;
        let mut decode_data = vec![];
        let _ = decoder
            .read_to_end(&mut decode_data)
            .context(GzipDecodeSnafu {})?;
        let data = Bytes::copy_from_slice(&decode_data);
        buf = data.to_vec()
    }
    let mut files = vec![];
    let mut a = tar::Archive::new(&buf[..]);
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

        let info = FileInfo {
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
