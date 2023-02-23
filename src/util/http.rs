use axum::http::{header::HeaderName, HeaderMap, HeaderValue};
use std::str::FromStr;

use crate::error::{HTTPError, HTTPResult};

/// 插入HTTP头
pub fn insert_header(
    headers: &mut HeaderMap<HeaderValue>,
    name: &str,
    value: &str,
) -> HTTPResult<()> {
    // 如果失败则不设置
    let header_name = HeaderName::from_str(name)
        .map_err(|err| HTTPError::new_with_category(&err.to_string(), "invalidHeaderName"))?;
    let header_value = HeaderValue::from_str(value)
        .map_err(|err| HTTPError::new_with_category(&err.to_string(), "invalidHeaderValue"))?;
    headers.insert(header_name, header_value);
    Ok(())
}
/// HTTP头不存在时才设置
pub fn set_header_if_not_exist(
    headers: &mut HeaderMap<HeaderValue>,
    name: &str,
    value: &str,
) -> HTTPResult<()> {
    let current = headers.get(name);
    if current.is_some() {
        return Ok(());
    }
    insert_header(headers, name, value)
}

/// 如果未设置cache-control，则设置为no-cache
pub fn set_no_cache_if_not_exist(headers: &mut HeaderMap<HeaderValue>) {
    // 因为只会字符导致设置错误
    // 因此此处理不会出错
    let _ = set_header_if_not_exist(headers, "Cache-Control", "no-cache");
}
