//! Process-wide [`reqwest::Client`] singleton.
//!
//! Building a `Client` re-establishes TLS context, rebuilds the DNS
//! resolver, and discards any warm connection pool. We do that once at
//! first use and share the result across every outbound HTTP call —
//! registry calls (`src/image/docker.rs`), the AI endpoint (`src/ai.rs`),
//! and the WeCom webhook (`src/wecom.rs`).
//!
//! Concurrency: `reqwest::Client` is internally `Arc<Inner>` and is
//! `Send + Sync`. The connection pool is partitioned per-host, so
//! cross-domain requests still run fully in parallel (each host has its
//! own idle pool, requests to different hosts never share a connection).
//!
//! Per-request timeouts: this Client has no default request timeout. Call
//! sites set their own via `RequestBuilder::timeout(...)` because the
//! sensible cap differs by endpoint (manifest ≈ 30 min for big multi-arch
//! transfers, auth/HEAD ≈ 5 min, AI inference ≈ 3 min, webhook ≈ 30 s).

use once_cell::sync::OnceCell;
use reqwest::Client;
use std::time::Duration;

/// Returns the shared, lazily-initialized client. The first caller pays
/// the construction cost; every subsequent caller gets the same instance.
pub fn get_http_client() -> &'static Client {
    static HTTP_CLIENT: OnceCell<Client> = OnceCell::new();
    HTTP_CLIENT.get_or_init(|| {
        Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(32)
            .build()
            .expect("failed to build shared reqwest client")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn singleton_returns_same_instance() {
        let a = get_http_client();
        let b = get_http_client();
        // `Client` doesn't implement PartialEq, but it's `Arc<Inner>`
        // internally — same-instance singletons share the same pointer.
        assert!(std::ptr::eq(a, b));
    }
}
