use axum::{
    http::{header, Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    BoxError, Json,
};
use diving_core::Error as CoreError;
use http::HeaderValue;
use serde::{Deserialize, Serialize};
use tracing::error;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HTTPError {
    pub message: String,
    pub category: String,
    pub code: String,
    pub status: u16,
}

pub type HTTPResult<T> = Result<T, HTTPError>;

impl Default for HTTPError {
    fn default() -> Self {
        HTTPError {
            message: String::new(),
            category: String::new(),
            status: 400,
            code: String::new(),
        }
    }
}

impl HTTPError {
    pub fn new(message: &str) -> Self {
        Self {
            message: message.to_string(),
            ..Default::default()
        }
    }

    pub fn new_with_category(message: &str, category: &str) -> Self {
        Self {
            message: message.to_string(),
            category: category.to_string(),
            ..Default::default()
        }
    }

    pub fn new_with_category_status(message: &str, category: &str, status: u16) -> Self {
        Self {
            message: message.to_string(),
            category: category.to_string(),
            status,
            ..Default::default()
        }
    }
}

impl From<CoreError> for HTTPError {
    fn from(err: CoreError) -> Self {
        let msg = err.to_string();
        match err {
            CoreError::DisallowedRegistry { .. } | CoreError::LocalSourceDisabled { .. } => {
                HTTPError::new_with_category_status(&msg, "registry", 403)
            }
            CoreError::InvalidDigest { .. } => {
                HTTPError::new_with_category_status(&msg, "blob", 400)
            }
            CoreError::FileNotFound => HTTPError::new_with_category_status(&msg, "layer", 404),
            CoreError::FileTooLarge { .. } => {
                HTTPError::new_with_category_status(&msg, "layer", 400)
            }
            _ => HTTPError::new_with_category(&msg, "docker"),
        }
    }
}

impl IntoResponse for HTTPError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::BAD_REQUEST);
        let mut res = Json(self).into_response();
        res.headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        (status, res).into_response()
    }
}

pub async fn handle_error(method: Method, uri: Uri, err: BoxError) -> HTTPError {
    error!("method:{}, uri:{}, error:{}", method, uri, err.to_string());
    if err.is::<tower::timeout::error::Elapsed>() {
        return HTTPError::new_with_category_status("Request took too long", "timeout", 408);
    }
    HTTPError::new(&err.to_string())
}
