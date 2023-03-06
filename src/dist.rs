use crate::util::set_header_if_not_exist;
use axum::body::Full;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use rust_embed::{EmbeddedFile, RustEmbed};

#[derive(RustEmbed)]
#[folder = "dist/"]
struct Asset;

impl IntoResponse for ServeFile {
    fn into_response(self) -> Response {
        if let Some(file) = self.file {
            let mut res = Full::from(file.data).into_response();
            let guess = mime_guess::from_path(&self.filename);
            let headers = res.headers_mut();
            // 忽略出错
            let _ = set_header_if_not_exist(
                headers,
                "Content-Type",
                guess.first_or_octet_stream().as_ref(),
            );
            if let Some(max_age) = self.max_age {
                let _ = set_header_if_not_exist(
                    headers,
                    "Cache-Control",
                    &format!("public, max-age={max_age}"),
                );
            }

            res
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
