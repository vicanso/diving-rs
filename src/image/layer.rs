use bytes::Bytes;
use libflate::gzip::Decoder;
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};
use std::{io::Read, path::Path};

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
}

pub async fn get_files_from_layer(data: Vec<u8>, is_gzip: bool) -> Result<()> {
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
    let mut a = tar::Archive::new(&buf[..]);
    for file in a.entries().context(TarSnafu {})? {
        // Make sure there wasn't an I/O error
        let file = file.context(TarSnafu {})?;
        // if let Ok(path) = file.path() {

        let info = FileInfo {
            path: file
                .path()
                .context(TarSnafu {})?
                .to_string_lossy()
                .to_string(),
        };

        println!("{:?}", file.link_name());

        // Inspect metadata about the file
        // println!("{:?}", file.header());
        println!("{:?}", info);

        // files implement the Read trait
        // let mut s = String::new();
        // file.read_to_string(&mut s).unwrap();
        // println!("{}", s);
        // }
    }
    Ok(())
}
