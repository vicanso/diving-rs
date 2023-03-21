use bytes::Bytes;
use chrono::{DateTime, Utc};
use glob::glob;
use snafu::{ResultExt, Snafu};
use std::{path::PathBuf, time::Duration};
use tokio::fs;

use crate::config::{get_layer_path, must_load_config};
use crate::error::HTTPError;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Write file {} fail: {}", file, source))]
    Write {
        source: std::io::Error,
        file: String,
    },
    #[snafu(display("Read file {} fail: {}", file, source))]
    Read {
        source: std::io::Error,
        file: String,
    },
    #[snafu(display("Glob {} fail: {}", path, source))]
    Pattern {
        source: glob::PatternError,
        path: String,
    },
    #[snafu(display("IO {} fail: {}", file, source))]
    IO {
        source: std::io::Error,
        file: String,
    },
}

impl From<Error> for HTTPError {
    fn from(err: Error) -> Self {
        // 对于部分error单独转换
        HTTPError::new_with_category(&err.to_string(), "blob")
    }
}

// 将blob数据保存至文件
pub async fn save_blob_to_file(digest: &str, data: &Bytes) -> Result<()> {
    let file = get_layer_path().join(digest);
    fs::write(file.clone(), data).await.context(WriteSnafu {
        file: file.to_string_lossy(),
    })
}

// 从文件中读取blob数据
pub async fn get_blob_from_file(digest: &str) -> Result<Vec<u8>> {
    let file = get_layer_path().join(digest);
    fs::read(file.clone()).await.context(ReadSnafu {
        file: file.to_string_lossy(),
    })
}

async fn clear_blob(file: PathBuf, expired: i64) -> Result<()> {
    let meta = fs::metadata(file.clone()).await.context(IOSnafu {
        file: file.to_string_lossy(),
    })?;
    // 优先用访问时间，再取修改时间
    let time = meta.accessed().or(meta.modified()).context(IOSnafu {
        file: file.to_string_lossy(),
    })?;

    // 未过期
    let t: DateTime<Utc> = DateTime::from(time);
    if t.timestamp() > expired {
        return Ok(());
    }
    fs::remove_file(file.clone()).await.context(IOSnafu {
        file: file.to_string_lossy(),
    })?;
    Ok(())
}

// 启动时清除较早下载的blob
pub async fn clear_blob_files() -> Result<()> {
    let path = get_layer_path().to_str();
    if path.is_none() {
        return Ok(());
    }
    let layer_ttl = must_load_config()
        .layer_ttl
        .clone()
        .unwrap_or_else(|| "90d".to_string());
    let ttl = layer_ttl
        .parse::<humantime::Duration>()
        .unwrap_or_else(|_| Duration::from_secs(90 * 24 * 3600).into());

    let expired = Utc::now().timestamp() - ttl.as_secs() as i64;

    // 已判断不为空
    let value = path.unwrap().to_string() + "/*";
    for entry in (glob(value.as_str()).context(PatternSnafu {
        path: value.to_string(),
    })?)
    .flatten()
    {
        // 清除失败忽略
        let _ = clear_blob(entry, expired).await;
    }
    Ok(())
}
