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

/// Package-manager cache paths that should not ship in production images.
pub fn is_pkg_cache(path: &str) -> bool {
    path.starts_with("var/cache/apt/")
        || path.starts_with("var/lib/apt/lists/")
        || path.starts_with("var/cache/apk/")
        || path.starts_with("var/cache/yum/")
        || path.starts_with("var/cache/dnf/")
        || path.starts_with("var/cache/pacman/")
}

/// Development / build artifacts that are usually dead weight in runtime images.
pub fn is_dev_artifact(path: &str) -> bool {
    let p = path;
    p.starts_with("node_modules/.cache/")
        || p.contains("/node_modules/.cache/")
        || p.starts_with(".git/")
        || p.contains("/.git/")
        || p.starts_with("target/debug/")
        || p.contains("/target/debug/")
        || p.starts_with("__pycache__/")
        || p.contains("/__pycache__/")
        || p.starts_with(".gradle/")
        || p.contains("/.gradle/")
        || p.starts_with(".m2/")
        || p.contains("/.m2/")
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
    fn pkg_cache_and_dev_artifact_helpers() {
        assert!(is_pkg_cache("var/cache/apt/archives/foo.deb"));
        assert!(is_dev_artifact("app/node_modules/.cache/x"));
        assert!(!is_pkg_cache("usr/bin/apt"));
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
