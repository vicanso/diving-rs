use snafu::Snafu;

/// Crate-wide result type.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Unified error type returned by `diving-core` operations.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum Error {
    #[snafu(display("IO fail: {source}"))]
    Io { source: std::io::Error },

    #[snafu(display("Request {url} fail: {source}"))]
    Request { source: reqwest::Error, url: String },

    #[snafu(display("Parse {url} json fail: {source}"))]
    ResponseJson { source: reqwest::Error, url: String },

    #[snafu(display("Request {url} code: {code} fail: {message}"))]
    Registry {
        status: u16,
        code: String,
        message: String,
        url: String,
    },

    #[snafu(display("Registry {url} unavailable: {message}"))]
    RegistryUnavailable { url: String, message: String },

    #[snafu(display("Serde json {context} fail: {source}"))]
    Json {
        source: serde_json::Error,
        context: String,
    },

    #[snafu(display("File not found"))]
    FileNotFound,

    #[snafu(display("File {file} is too large: {size} bytes (limit {limit})"))]
    FileTooLarge { file: String, size: u64, limit: u64 },

    #[snafu(display("Invalid blob digest: {digest}"))]
    InvalidDigest { digest: String },

    #[snafu(display("Decompress fail: {message}"))]
    Decompress { message: String },

    #[snafu(display("Tar fail: {source}"))]
    Tar { source: std::io::Error },

    #[snafu(display("registry {registry} is not allowed"))]
    DisallowedRegistry { registry: String },

    #[snafu(display("local image source {source} is disabled"))]
    LocalSourceDisabled {
        #[snafu(source(false))]
        source: String,
    },

    #[snafu(display("Invalid configuration: {message}"))]
    InvalidConfig { message: String },

    #[snafu(display("{message}"))]
    PreviewUnsupported { message: String },

    #[snafu(display("{message}"))]
    Other { message: String },
}

impl Error {
    /// Whether the registry is unavailable right now — unreachable (DNS,
    /// connect, TLS, timeout), overloaded (5xx) or rate limiting us (429) —
    /// as opposed to having answered that the request itself is wrong
    /// (401/403/404…). Only the former justifies serving a cached analysis.
    pub fn is_registry_unavailable(&self) -> bool {
        match self {
            Error::Request { source, .. } => {
                source.is_connect() || source.is_timeout() || source.is_request()
            }
            Error::Registry { status, .. } => *status == 429 || (500..600).contains(status),
            Error::RegistryUnavailable { .. } => true,
            _ => false,
        }
    }

    /// Whether the registry rejected credentials or token auth (HTTP 401).
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, Error::Registry { status: 401, .. })
    }

    /// Whether the requested layer entry or registry manifest was not found.
    pub fn is_not_found(&self) -> bool {
        matches!(
            self,
            Error::FileNotFound | Error::Registry { status: 404, .. }
        )
    }

    /// Whether access was rejected by policy (`registry_allowlist` / disabled
    /// local source) or by the registry (HTTP 403).
    pub fn is_forbidden(&self) -> bool {
        matches!(
            self,
            Error::DisallowedRegistry { .. }
                | Error::LocalSourceDisabled { .. }
                | Error::Registry { status: 403, .. }
        )
    }

    /// Clone an owned error for shared singleflight waiters while preserving
    /// error semantics (`is_registry_unavailable`, `is_unauthorized`,
    /// `is_not_found`, `is_forbidden`).
    pub(crate) fn clone_for_waiter(&self) -> Self {
        match self {
            Error::Io { source } => Error::Io {
                source: std::io::Error::new(source.kind(), source.to_string()),
            },
            Error::Request { url, .. } => {
                if self.is_registry_unavailable() {
                    Error::RegistryUnavailable {
                        url: url.clone(),
                        message: self.to_string(),
                    }
                } else {
                    Error::Other {
                        message: self.to_string(),
                    }
                }
            }
            Error::ResponseJson { .. } => Error::Other {
                message: self.to_string(),
            },
            Error::Registry {
                status,
                code,
                message,
                url,
            } => Error::Registry {
                status: *status,
                code: code.clone(),
                message: message.clone(),
                url: url.clone(),
            },
            Error::RegistryUnavailable { url, message } => Error::RegistryUnavailable {
                url: url.clone(),
                message: message.clone(),
            },
            Error::Json { .. } => Error::Other {
                message: self.to_string(),
            },
            Error::FileNotFound => Error::FileNotFound,
            Error::FileTooLarge { file, size, limit } => Error::FileTooLarge {
                file: file.clone(),
                size: *size,
                limit: *limit,
            },
            Error::InvalidDigest { digest } => Error::InvalidDigest {
                digest: digest.clone(),
            },
            Error::Decompress { message } => Error::Decompress {
                message: message.clone(),
            },
            Error::Tar { source } => Error::Tar {
                source: std::io::Error::new(source.kind(), source.to_string()),
            },
            Error::DisallowedRegistry { registry } => Error::DisallowedRegistry {
                registry: registry.clone(),
            },
            Error::LocalSourceDisabled { source } => Error::LocalSourceDisabled {
                source: source.clone(),
            },
            Error::InvalidConfig { message } => Error::InvalidConfig {
                message: message.clone(),
            },
            Error::PreviewUnsupported { message } => Error::PreviewUnsupported {
                message: message.clone(),
            },
            Error::Other { message } => Error::Other {
                message: message.clone(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_registry_outages_qualify_for_cached_fallback() {
        let status = |status: u16| Error::Registry {
            message: String::new(),
            code: String::new(),
            url: String::new(),
            status,
        };
        for code in [429, 500, 502, 503, 504] {
            assert!(status(code).is_registry_unavailable(), "{code}");
        }
        for code in [400, 401, 403, 404] {
            assert!(!status(code).is_registry_unavailable(), "{code}");
        }
        let other = Error::Other {
            message: "image index contains no manifests".to_string(),
        };
        assert!(!other.is_registry_unavailable());
        assert!(Error::RegistryUnavailable {
            url: "https://example.com".into(),
            message: "timeout".into(),
        }
        .is_registry_unavailable());
    }

    #[test]
    fn error_predicates_classify_status_and_policy_errors() {
        assert!(Error::Registry {
            status: 401,
            code: "UNAUTHORIZED".into(),
            message: "expired".into(),
            url: "u".into(),
        }
        .is_unauthorized());
        assert!(Error::Registry {
            status: 404,
            code: "NAME_UNKNOWN".into(),
            message: "missing".into(),
            url: "u".into(),
        }
        .is_not_found());
        assert!(Error::FileNotFound.is_not_found());
        assert!(Error::DisallowedRegistry {
            registry: "evil.io".into()
        }
        .is_forbidden());
        assert!(Error::LocalSourceDisabled {
            source: "local-file".into()
        }
        .is_forbidden());
    }
}
