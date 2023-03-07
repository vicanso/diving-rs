use axum::response::{IntoResponse, Response};
use hex::encode;
use http::header;
use http::StatusCode;
use rust_embed::{EmbeddedFile, RustEmbed};

#[derive(RustEmbed)]
#[folder = "dist/"]
struct Asset;

impl IntoResponse for ServeFile {
    fn into_response(self) -> Response {
        if let Some(file) = self.file {
            let str = &encode(file.metadata.sha256_hash())[0..8];
            let e_tag = format!("{:x}-{str}", file.data.len());
            let guess = mime_guess::from_path(&self.filename);
            (
                [
                    // content type
                    (header::CONTENT_TYPE, guess.first_or_octet_stream().as_ref()),
                    // 为啥不设置Last-Modified
                    // https://developer.mozilla.org/en-US/docs/Web/HTTP/Caching#heuristic_caching
                    // e tag
                    (header::ETAG, e_tag.as_str()),
                    // max age
                    (
                        header::CACHE_CONTROL,
                        format!("public, max-age={}", self.max_age.unwrap_or_default()).as_str(),
                    ),
                ],
                file.data,
            )
                .into_response()
        } else {
            // 404
            StatusCode::NOT_FOUND.into_response()
        }
    }
}

/// A file embedded into the binary
pub struct ServeFile {
    pub file: Option<EmbeddedFile>,
    pub filename: String,
    pub max_age: Option<u32>,
}

// 获取资源文件
pub fn get_asset(filename: &str) -> ServeFile {
    let file = Asset::get(filename);
    ServeFile {
        file,
        filename: filename.to_string(),
        max_age: None,
    }
}
