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

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Op {
    #[default]
    None,
    Remove,
    Modified,
    Added,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTreeItem {
    pub name: String,
    pub link: String,
    pub size: u64,
    pub mode: String,
    pub uid: u64,
    pub gid: u64,
    pub op: Op,
    pub children: Vec<FileTreeItem>,
}

fn add_file(items: &mut Vec<FileTreeItem>, name_list: Vec<&str>, item: FileTreeItem) {
    // 文件
    if name_list.is_empty() {
        items.push(item);
        return;
    }
    // 目录
    let name = name_list[0];
    let mut found_index = -1;
    // 是否已存在此目录
    for (index, dir) in items.iter_mut().enumerate() {
        if dir.name == name {
            dir.size += item.size;
            found_index = index as i64;
        }
    }
    // 不存在则插入
    if found_index < 0 {
        found_index = items.len() as i64;
        items.push(FileTreeItem {
            name: name.to_string(),
            size: item.size,
            // TODO 其它属性
            ..Default::default()
        });
    }
    if let Some(file_tree_item) = items.get_mut(found_index as usize) {
        // 子目录
        add_file(&mut file_tree_item.children, name_list[1..].to_vec(), item);
    }
}

pub fn convert_files_to_file_tree(files: &[ImageFileInfo]) -> Vec<FileTreeItem> {
    let mut file_tree: Vec<FileTreeItem> = vec![];
    for file in files.iter() {
        let arr: Vec<&str> = file.path.split('/').collect();
        if arr.is_empty() {
            continue;
        }
        let size = arr.len();
        add_file(
            &mut file_tree,
            arr[0..size - 1].to_vec(),
            FileTreeItem {
                // 已保证不会为空
                name: arr[size - 1].to_string(),
                link: file.link.clone(),
                size: file.size,
                mode: file.mode.clone(),
                uid: file.uid,
                gid: file.gid,
                // TODO 其它属性
                ..Default::default()
            },
        )
    }
    file_tree
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageLayerInfo {
    // 原始大小
    pub size: u64,
    // 解压后的大小
    pub unpack_size: u64,
    // 文件列表
    pub files: Vec<ImageFileInfo>,
}

impl ImageLayerInfo {
    pub fn to_file_tree(&self) -> Vec<FileTreeItem> {
        convert_files_to_file_tree(&self.files)
    }
}

// 从分层数据中读取所有文件信息
// "application/vnd.oci.image.layer.v1.tar+gzip",
pub async fn get_files_from_layer(data: &[u8], media_type: &str) -> Result<ImageLayerInfo> {
    let buf;
    let size = data.len() as u64;
    let mut unpack_size = size;
    // TODO 支持gzip zstd等
    let mut a = if media_type.contains("gzip") {
        buf = gunzip(data)?;
        unpack_size = buf.len() as u64;
        Archive::new(&buf[..])
    } else if media_type.contains("zstd") {
        buf = zstd_decode(data)?;
        unpack_size = buf.len() as u64;
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
        let mode = header.mode().context(TarSnafu {})?;
        let info = ImageFileInfo {
            path,
            link,
            size: file.size(),
            mode: unix_mode::to_string(mode),
            uid: header.uid().context(TarSnafu {})?,
            gid: header.gid().context(TarSnafu {})?,
            is_whiteout,
        };
        files.push(info);
    }
    Ok(ImageLayerInfo {
        files,
        unpack_size,
        size,
    })
}
