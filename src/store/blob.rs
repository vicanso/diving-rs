use bytes::Bytes;
use tokio::{fs, io};

use crate::config::get_layer_path;

// 将blob数据保存至文件
pub async fn save_blob_to_file(digest: &str, data: &Bytes) -> io::Result<()> {
    let file = get_layer_path().join(digest);
    fs::write(file, data).await
}

// 从文件中读取blob数据
pub async fn get_blob_from_file(digest: &str) -> Result<Vec<u8>, io::Error> {
    let file = get_layer_path().join(digest);
    fs::read(file).await
}

// TODO 启动时清除较早下载的blob
