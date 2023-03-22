use axum::response::{IntoResponse, Response};
use hex::encode;
use http::header;
use http::StatusCode;
use rust_embed::{EmbeddedFile, RustEmbed};
use std::convert::From;
use substring::Substring;

#[derive(RustEmbed)]
#[folder = "dist/"]
struct Assets;

#[derive(Default)]
pub struct StaticFile {
    content_type: String,
    entity_tag: String,
    max_age: u32,
    s_max_age: Option<u32>,
    file: Option<EmbeddedFile>,
}

// TODO 后续使用EmbeddedFile获取mime
impl From<(String, Option<EmbeddedFile>)> for StaticFile {
    fn from(data: (String, Option<EmbeddedFile>)) -> Self {
        if let Some(value) = data.1 {
            let str = if let Some(last_modified) = value.metadata.last_modified() {
                let value = last_modified as u32;
                encode(value.to_be_bytes())
            } else {
                encode(value.metadata.sha256_hash())
                    .substring(0, 8)
                    .to_string()
            };
            // 长度+hash
            let entity_tag = format!("\"{:x}-{str}\"", value.data.len());
            // 因为html对于网页是入口，避免缓存后更新不及时
            // 因此设置为0
            // 其它js,css会添加版本号，因此无影响
            let max_age = if data.0.ends_with(".html") {
                0
            } else {
                365 * 24 * 3600
            };

            // 缓存服务器的有效期设置为较短的值
            let server_max_age = 600;
            let s_max_age = if max_age > server_max_age {
                Some(server_max_age)
            } else {
                None
            };

            StaticFile {
                max_age,
                content_type: mime_guess::from_path(data.0)
                    .first_or_octet_stream()
                    .to_string(),
                entity_tag,
                s_max_age,
                file: Some(value),
            }
        } else {
            StaticFile::default()
        }
    }
}

impl IntoResponse for StaticFile {
    fn into_response(self) -> Response {
        if let Some(file) = self.file {
            let mut max_age = format!("public, max-age={}", self.max_age);
            if let Some(s_max_age) = self.s_max_age {
                max_age = format!("{max_age}, s-maxage={s_max_age}");
            }
            // 静态文件压缩由前置缓存服务器处理
            (
                [
                    // content type
                    (header::CONTENT_TYPE, self.content_type.as_str()),
                    // 为啥不设置Last-Modified
                    // https://developer.mozilla.org/en-US/docs/Web/HTTP/Caching#heuristic_caching
                    // e tag
                    (header::ETAG, self.entity_tag.as_str()),
                    // max age
                    (header::CACHE_CONTROL, max_age.as_str()),
                ],
                file.data,
            )
                .into_response()
        } else {
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

// 获取资源文件
fn get_asset(file_path: &str) -> Option<EmbeddedFile> {
    Assets::get(file_path)
}

// 获取静态资源文件
pub fn get_static_file(file_path: &str) -> StaticFile {
    let file = get_asset(file_path);
    (file_path.to_string(), file).into()
}
