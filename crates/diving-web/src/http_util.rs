use axum::http::{header::HeaderName, HeaderMap, HeaderValue};
use ring::digest::{digest, SHA256};
use std::str::FromStr;

use crate::error::{HTTPError, HTTPResult};

/// Hash a secret token with SHA-256 so fixed-length digest comparison never
/// leaks token length or shared prefixes.
pub(crate) fn token_fingerprint(token: &str) -> Vec<u8> {
    digest(&SHA256, token.as_bytes()).as_ref().to_vec()
}

/// Constant-time comparison of equal-length digests.
pub(crate) fn digests_equal(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn insert_header(
    headers: &mut HeaderMap<HeaderValue>,
    name: &str,
    value: &str,
) -> HTTPResult<()> {
    let header_name = HeaderName::from_str(name)
        .map_err(|err| HTTPError::new_with_category(&err.to_string(), "invalidHeaderName"))?;
    let header_value = HeaderValue::from_str(value)
        .map_err(|err| HTTPError::new_with_category(&err.to_string(), "invalidHeaderValue"))?;
    headers.insert(header_name, header_value);
    Ok(())
}

pub fn set_header_if_not_exist(
    headers: &mut HeaderMap<HeaderValue>,
    name: &str,
    value: &str,
) -> HTTPResult<()> {
    if headers.get(name).is_some() {
        return Ok(());
    }
    insert_header(headers, name, value)
}

pub fn set_no_cache_if_not_exist(headers: &mut HeaderMap<HeaderValue>) {
    let _ = set_header_if_not_exist(headers, "Cache-Control", "no-cache");
}
