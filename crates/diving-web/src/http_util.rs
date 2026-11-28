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

/// Extract the token from a case-insensitive `Bearer <token>` header value.
pub(crate) fn extract_bearer_token(header_val: &str) -> Option<&str> {
    let trimmed = header_val.trim();
    let (scheme, rest) = trimmed.split_once(char::is_whitespace)?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = rest.trim();
    (!token.is_empty()).then_some(token)
}

/// Whether `header_val` starts with a `Bearer` scheme prefix (any ASCII casing)
/// followed by whitespace (or is bare `"Bearer"`).
pub(crate) fn has_bearer_scheme(header_val: &str) -> bool {
    let trimmed = header_val.trim();
    trimmed.eq_ignore_ascii_case("bearer")
        || trimmed
            .split_once(char::is_whitespace)
            .is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_bearer_token_is_case_insensitive_and_rejects_empty() {
        assert_eq!(extract_bearer_token("Bearer secret"), Some("secret"));
        assert_eq!(extract_bearer_token("bearer secret"), Some("secret"));
        assert_eq!(extract_bearer_token("BEARER  secret  "), Some("secret"));
        assert_eq!(extract_bearer_token("Bearer "), None);
        assert_eq!(extract_bearer_token("Bearer"), None);
        assert_eq!(extract_bearer_token("Basic abc"), None);
        assert!(has_bearer_scheme("bearer "));
        assert!(has_bearer_scheme("Bearer"));
        assert!(!has_bearer_scheme("raw-secret"));
    }
}
