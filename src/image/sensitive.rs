//! Path-based sensitive-file and bloat heuristics used during analysis.
//!
//! These never open file contents — only path / name patterns. Content-level
//! secret scanning is intentionally out of scope.

/// True when `frag` appears in `path` at the start or immediately after a
/// `/` — i.e. `frag` behaves like a path-segment prefix. Equivalent to
/// `path.starts_with(frag) || path.contains(&format!("/{frag}"))` without
/// the per-call allocation: these checks run for every file of every layer,
/// so building a fresh `String` per fragment per file adds up to millions
/// of heap allocations on large images.
pub(crate) fn has_path_frag(path: &str, frag: &str) -> bool {
    let bytes = path.as_bytes();
    let mut start = 0;
    while let Some(pos) = path[start..].find(frag) {
        let abs = start + pos;
        if abs == 0 || bytes[abs - 1] == b'/' {
            return true;
        }
        start = abs + 1;
    }
    false
}

/// Package-manager cache smaller than this is not worth a finding: a lone
/// empty `var/cache/apt/archives/lock` would otherwise produce a
/// "0 B of package-manager cache" recommendation and a risk tag.
pub const MIN_PKG_CACHE_BYTES: u64 = 1024 * 1024;

/// Package-manager cache paths that should not ship in production images.
/// The one definition behind both the risk tag (per layer, during the scan)
/// and the recommendation (on the final filesystem), so they always agree.
pub(crate) fn is_pkg_cache(path: &str) -> bool {
    // OS-level package manager caches.
    if path.starts_with("var/cache/apt/")
        || path.starts_with("var/lib/apt/lists/")
        || path.starts_with("var/cache/apk/")
        || path.starts_with("var/cache/yum/")
        || path.starts_with("var/cache/dnf/")
        || path.starts_with("var/cache/pacman/")
        || path.starts_with("var/cache/zypp/")
    {
        return true;
    }
    // Language ecosystem caches. Match at path start or after a slash so the
    // fragments behave like real path segments and don't catch unrelated
    // directory names.
    const LANG_CACHE_FRAGS: &[&str] = &[
        ".cache/pip/",
        ".cache/uv/",
        // poetry's downloads; its `virtualenvs/` beside them are the app
        ".cache/pypoetry/cache/",
        ".cache/pypoetry/artifacts/",
        ".npm/_cacache/",
        ".cache/yarn/",
        ".yarn/cache/",
        ".cache/go-build/",
        "go/pkg/mod/cache/",
        ".cargo/registry/",
        ".cargo/git/db/",
        ".composer/cache/",
        ".gem/cache/",
    ];
    if LANG_CACHE_FRAGS.iter().any(|f| has_path_frag(path, f)) {
        return true;
    }
    // Fixed container locations used by language toolchains.
    path.starts_with("usr/local/bundle/cache/")
        || path.starts_with("usr/local/cargo/registry/")
        || path.starts_with("opt/conda/pkgs/")
}

/// Development / build artifacts that are usually dead weight in runtime images.
pub(crate) fn is_dev_artifact(path: &str) -> bool {
    // Each fragment is a real path segment we want to match either at the
    // path root or after a `/`. Framework caches (`.next/cache/`, etc.) must
    // be matched specifically so the framework's runtime files (`.next/server/...`)
    // are not flagged.
    const FRAGS: &[&str] = &[
        "node_modules/.cache/",
        ".git/",
        "target/debug/",
        "__pycache__/",
        ".gradle/",
        ".m2/",
        ".pytest_cache/",
        ".mypy_cache/",
        ".ruff_cache/",
        ".tox/",
        ".next/cache/",
        ".nuxt/cache/",
        ".angular/cache/",
        ".parcel-cache/",
        ".ipynb_checkpoints/",
        ".eslintcache",
    ];
    FRAGS.iter().any(|f| has_path_frag(path, f))
}

/// Public CA trust stores hold *public* certificates, not secrets, so they
/// must not trip the cert/key extension heuristics.
///
/// Deliberately scoped: private keys conventionally live in
/// `.../ssl/private/`, which is NOT excluded here and stays flagged.
fn is_public_ca_store(pl: &str, fl: &str) -> bool {
    const CA_DIRS: &[&str] = &[
        "etc/ssl/certs/",
        "etc/ssl1.1/certs/",
        "etc/pki/tls/certs/",
        "etc/pki/ca-trust/",
        "etc/ca-certificates/",
        "usr/share/ca-certificates/",
        "usr/local/share/ca-certificates/",
        "usr/lib/ssl/certs/",
    ];
    if CA_DIRS.iter().any(|d| has_path_frag(pl, d)) {
        return true;
    }
    const CA_BUNDLES: &[&str] = &[
        "ca-certificates.crt",
        "ca-bundle.crt",
        "ca-bundle.pem",
        "tls-ca-bundle.pem",
        "cacert.pem",
    ];
    if CA_BUNDLES.contains(&fl) {
        return true;
    }
    if fl == "cert.pem"
        && (pl.starts_with("etc/ssl")
            || pl.starts_with("etc/pki/tls")
            || pl.contains("/ssl/")
            || pl.contains("/ssl1.1/")
            || pl.contains("/tls/"))
    {
        return true;
    }
    false
}

/// Check whether a file path looks like a sensitive/secret file.
/// Returns a short description of the risk, or None if not sensitive.
pub fn is_sensitive_file(path: &str) -> Option<&'static str> {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let fl = filename.to_lowercase();
    let pl = path.to_lowercase();

    if fl == ".env" || fl.starts_with(".env.") || fl.ends_with(".env") {
        return Some(".env file");
    }
    if matches!(
        fl.as_str(),
        "id_rsa" | "id_dsa" | "id_ecdsa" | "id_ed25519" | "id_ecdsa_sk" | "id_ed25519_sk"
    ) {
        return Some("SSH private key");
    }
    if pl.contains("/.aws/credentials") {
        return Some("AWS credentials");
    }
    if is_public_ca_store(&pl, &fl) {
        return None;
    }
    if fl.ends_with(".pem")
        || fl.ends_with(".p12")
        || fl.ends_with(".pfx")
        || fl.ends_with(".jks")
        || fl.ends_with(".keystore")
    {
        return Some("Private key / certificate");
    }
    if fl.ends_with(".key") && !pl.contains("/node_modules/") {
        return Some("Private key / certificate");
    }
    if pl.ends_with(".docker/config.json") {
        return Some("Docker registry credentials");
    }
    if fl == ".netrc" || fl == ".git-credentials" {
        return Some("Git / network credentials");
    }
    if fl == "kubeconfig" || fl.ends_with(".kubeconfig") {
        return Some("Kubernetes config");
    }
    if fl.ends_with(".tfvars") || fl == "terraform.tfstate" {
        return Some("Terraform secrets");
    }
    if fl.ends_with("-key.json")
        || ((fl.starts_with("service_account") || fl.starts_with("service-account"))
            && fl.ends_with(".json"))
    {
        return Some("Service account key");
    }
    if fl == ".htpasswd" {
        return Some("Password file");
    }
    if pl.starts_with(".git/") || pl.contains("/.git/") {
        return Some(".git directory (SCM history)");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ca_bundles_are_not_flagged_as_secrets() {
        assert_eq!(is_sensitive_file("etc/ssl/cert.pem"), None);
        assert_eq!(is_sensitive_file("etc/ssl1.1/cert.pem"), None);
        assert_eq!(is_sensitive_file("etc/ssl/certs/ca-certificates.crt"), None);
        assert_eq!(is_sensitive_file("etc/pki/tls/certs/ca-bundle.crt"), None);
        assert_eq!(
            is_sensitive_file("usr/share/ca-certificates/mozilla/GlobalSign.crt"),
            None
        );
    }

    #[test]
    fn real_private_keys_still_flagged() {
        assert!(is_sensitive_file("etc/ssl/private/server.key").is_some());
        assert!(is_sensitive_file("app/config/id_rsa").is_some());
        assert!(is_sensitive_file("home/user/secret.pem").is_some());
        assert!(is_sensitive_file("opt/app/keystore.jks").is_some());
    }

    #[test]
    fn pkg_cache_covers_os_and_language_ecosystems() {
        // OS package managers (existing behaviour).
        assert!(is_pkg_cache("var/cache/apt/archives/foo.deb"));
        assert!(is_pkg_cache("var/lib/apt/lists/foo"));
        assert!(is_pkg_cache("var/cache/apk/index"));
        assert!(is_pkg_cache("var/cache/dnf/x"));
        assert!(is_pkg_cache("var/cache/zypp/packages/foo"));
        // Language ecosystem caches.
        assert!(is_pkg_cache("root/.cache/pip/wheels/abc.whl"));
        assert!(is_pkg_cache("home/app/.cache/pip/http/00/aa"));
        assert!(is_pkg_cache("root/.npm/_cacache/index-v5/00/foo"));
        assert!(is_pkg_cache("root/.cache/yarn/v6/npm-foo"));
        assert!(is_pkg_cache("usr/local/share/.cache/yarn/v6/x"));
        assert!(is_pkg_cache("app/.yarn/cache/foo.zip"));
        assert!(is_pkg_cache("root/.cache/go-build/00/aa"));
        assert!(is_pkg_cache("go/pkg/mod/cache/download/sumdb/x"));
        assert!(is_pkg_cache("root/.cargo/registry/cache/index/foo.crate"));
        assert!(is_pkg_cache("usr/local/cargo/registry/cache/foo"));
        assert!(is_pkg_cache("root/.composer/cache/files/x"));
        assert!(is_pkg_cache("usr/local/bundle/cache/foo.gem"));
        assert!(is_pkg_cache("opt/conda/pkgs/numpy-1.26.0/x"));
        assert!(is_pkg_cache("root/.cache/uv/archive-v0/abc/x.py"));
        assert!(is_pkg_cache("root/.cache/pypoetry/cache/repositories/x"));
        // Negatives — must NOT be flagged.
        assert!(!is_pkg_cache("usr/bin/python3"));
        assert!(!is_pkg_cache("app/main.py"));
        assert!(!is_pkg_cache("usr/lib/x86_64-linux-gnu/libc.so.6"));
        // A directory whose name only *contains* a cache name without the
        // path-segment boundary must not match.
        assert!(!is_pkg_cache("app/my.cache/pipx/foo"));
        // poetry installs the app's dependencies into these
        assert!(!is_pkg_cache(
            "root/.cache/pypoetry/virtualenvs/app-py3.12/x"
        ));
        assert!(!is_pkg_cache("usr/bin/apt"));
    }

    #[test]
    fn dev_artifact_covers_framework_caches() {
        // Existing behaviour.
        assert!(is_dev_artifact("app/.git/HEAD"));
        assert!(is_dev_artifact("srv/__pycache__/foo.cpython-311.pyc"));
        assert!(is_dev_artifact("usr/src/app/node_modules/.cache/babel/foo"));
        // New framework caches.
        assert!(is_dev_artifact("app/.pytest_cache/v/cache/lastfailed"));
        assert!(is_dev_artifact("app/.mypy_cache/3.11/foo.json"));
        assert!(is_dev_artifact("app/.ruff_cache/0.1.0/foo"));
        assert!(is_dev_artifact("app/.tox/py311/lib/x"));
        assert!(is_dev_artifact("web/.next/cache/webpack/foo.pack"));
        assert!(is_dev_artifact("web/.nuxt/cache/foo"));
        assert!(is_dev_artifact("web/.angular/cache/16.0.0/foo"));
        assert!(is_dev_artifact("web/.parcel-cache/foo"));
        assert!(is_dev_artifact("nb/.ipynb_checkpoints/foo.ipynb"));
        assert!(is_dev_artifact(".eslintcache"));
        // `.next/<not-cache>` is required by Next.js at runtime — must NOT flag.
        assert!(!is_dev_artifact("web/.next/server/pages/index.js"));
        assert!(!is_dev_artifact("usr/lib/foo.so"));
    }

    #[test]
    fn path_frag_matches_only_at_segment_boundaries() {
        // At path start.
        assert!(has_path_frag(".cache/pip/x", ".cache/pip/"));
        // After a slash, any depth.
        assert!(has_path_frag("home/app/.cache/pip/x", ".cache/pip/"));
        // Mid-segment must NOT match (`my.cache` is not `.cache`).
        assert!(!has_path_frag("app/my.cache/pip/x", ".cache/pip/"));
        // Same behaviour as the old `starts_with || contains("/"+frag)` pair.
        assert!(has_path_frag("etc/ssl/certs/ca.crt", "etc/ssl/certs/"));
        assert!(has_path_frag(
            "usr/local/etc/ssl/certs/ca.crt",
            "etc/ssl/certs/"
        ));
        assert!(!has_path_frag("fetc/ssl/certs/ca.crt", "etc/ssl/certs/"));
        assert!(!has_path_frag("", "x"));
    }
}
