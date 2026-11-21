//! Path-based sensitive-file and bloat heuristics used during analysis.
//!
//! These never open file contents — only path / name patterns. Content-level
//! secret scanning is intentionally out of scope.

use crate::error::{Error, Result};
use glob::{MatchOptions, Pattern};

const GLOB_OPTS: MatchOptions = MatchOptions {
    case_sensitive: false,
    require_literal_separator: false,
    require_literal_leading_dot: false,
};

fn glob_matches(pattern: &Pattern, path: &str) -> bool {
    if pattern.matches_with(path, GLOB_OPTS) {
        return true;
    }
    // Also match against the filename alone so `*.pem` hits `a/b/c.pem`.
    let filename = path.rsplit('/').next().unwrap_or(path);
    pattern.matches_with(filename, GLOB_OPTS)
}

/// A custom sensitive-file glob pattern and the risk label reported on match.
#[derive(Debug, Clone)]
pub struct SensitiveRule {
    pub pattern: Pattern,
    pub reason: String,
}

/// Custom sensitive-file additions and suppressions applied on top of the
/// built-in path rules.
#[derive(Debug, Clone, Default)]
pub struct SensitivePatterns {
    patterns: Vec<SensitiveRule>,
    ignores: Vec<Pattern>,
}

impl SensitivePatterns {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a glob pattern flagging matching paths as sensitive.
    pub fn add_pattern(&mut self, pattern: &str, reason: impl Into<String>) -> Result<&mut Self> {
        let compiled = Pattern::new(pattern.trim()).map_err(|e| Error::InvalidConfig {
            message: format!("invalid sensitive pattern `{pattern}`: {e}"),
        })?;
        self.patterns.push(SensitiveRule {
            pattern: compiled,
            reason: reason.into(),
        });
        Ok(self)
    }

    /// Add a glob pattern suppressing built-in or custom sensitive-file matches.
    pub fn add_ignore(&mut self, pattern: &str) -> Result<&mut Self> {
        let compiled = Pattern::new(pattern.trim()).map_err(|e| Error::InvalidConfig {
            message: format!("invalid sensitive ignore pattern `{pattern}`: {e}"),
        })?;
        self.ignores.push(compiled);
        Ok(self)
    }

    /// Parse rules from text using the `~/.diving/sensitive-files` line format:
    /// - `<glob-pattern>` — flag with reason `"Custom sensitive file"`
    /// - `<glob-pattern> | <reason>` — flag with a custom reason
    /// - `!<glob-pattern>` — suppress built-in or custom matches
    ///
    /// Blank lines, `#` comments, and malformed patterns are skipped.
    pub fn parse_rules(content: &str) -> Self {
        let mut patterns = vec![];
        let mut ignores = vec![];
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix('!') {
                if let Ok(pat) = Pattern::new(rest.trim()) {
                    ignores.push(pat);
                }
            } else {
                let (pat_str, reason) = if let Some(pos) = line.find('|') {
                    (line[..pos].trim_end(), line[pos + 1..].trim().to_string())
                } else {
                    (line, "Custom sensitive file".to_string())
                };
                if let Ok(pat) = Pattern::new(pat_str) {
                    patterns.push(SensitiveRule {
                        pattern: pat,
                        reason,
                    });
                }
            }
        }
        Self { patterns, ignores }
    }

    /// Return the reason string if `path` matches a custom pattern and is not
    /// suppressed by an ignore rule.
    pub fn check(&self, path: &str) -> Option<&str> {
        for p in &self.patterns {
            if glob_matches(&p.pattern, path) {
                if self.is_ignored(path) {
                    return None;
                }
                return Some(&p.reason);
            }
        }
        None
    }

    /// Return true if `path` is suppressed by an ignore rule.
    pub fn is_ignored(&self, path: &str) -> bool {
        self.ignores.iter().any(|ig| glob_matches(ig, path))
    }

    /// Evaluate both built-in rules and custom additions/suppressions for `path`.
    pub(crate) fn evaluate_path(&self, path: &str) -> Option<String> {
        if let Some(reason) = is_sensitive_file(path) {
            if self.is_ignored(path) {
                None
            } else {
                Some(reason.to_string())
            }
        } else {
            self.check(path).map(str::to_string)
        }
    }
}

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
    const LANG_CACHE_FRAGS: &[&str] = &[
        ".cache/pip/",
        ".cache/uv/",
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
    path.starts_with("usr/local/bundle/cache/")
        || path.starts_with("usr/local/cargo/registry/")
        || path.starts_with("opt/conda/pkgs/")
}

/// Development / build artifacts that are usually dead weight in runtime images.
pub(crate) fn is_dev_artifact(path: &str) -> bool {
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
pub(crate) fn is_sensitive_file(path: &str) -> Option<&'static str> {
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

/// Maximum file size scanned for inline secrets during the single-pass tar scan (1 MiB).
pub(crate) const MAX_CONTENT_SECRET_SCAN_BYTES: u64 = 1024 * 1024;

/// Fast pre-filter deciding whether a tar entry is worth reading for content-level
/// secret scanning (small regular text/config/script files outside system binary dirs).
pub(crate) fn should_scan_content(path: &str, size: u64) -> bool {
    if size == 0 || size > MAX_CONTENT_SECRET_SCAN_BYTES {
        return false;
    }
    let clean = path.trim_start_matches("./").trim_start_matches('/');
    let pl = clean.to_ascii_lowercase();
    if is_public_ca_store(&pl, pl.rsplit('/').next().unwrap_or(&pl)) {
        return false;
    }
    // Skip OS/runtime bulk directories where high-volume static data lives.
    for skip in [
        "usr/share/",
        "usr/include/",
        "usr/lib/",
        "usr/lib64/",
        "lib/",
        "lib64/",
        "var/lib/dpkg/",
        "var/lib/apt/",
        "var/lib/rpm/",
        "var/cache/",
        "node_modules/",
    ] {
        if pl.starts_with(skip) {
            return false;
        }
    }
    // Skip obvious binary / media / archive extensions.
    let file = pl.rsplit('/').next().unwrap_or(&pl);
    if let Some((_, ext)) = file.rsplit_once('.') {
        if matches!(
            ext,
            "so" | "a"
                | "o"
                | "pyc"
                | "pyo"
                | "whl"
                | "jar"
                | "class"
                | "wasm"
                | "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "webp"
                | "ico"
                | "svg"
                | "woff"
                | "woff2"
                | "ttf"
                | "otf"
                | "eot"
                | "mp3"
                | "mp4"
                | "pdf"
                | "zip"
                | "gz"
                | "bz2"
                | "xz"
                | "zst"
                | "tar"
                | "db"
                | "sqlite"
                | "bin"
                | "exe"
                | "dll"
                | "dylib"
        ) {
            return false;
        }
    }
    true
}

/// Zero-regex content-level secret detector for small text files (<= 1 MiB).
pub(crate) fn scan_content_secrets(path: &str, bytes: &[u8]) -> Option<&'static str> {
    if bytes.is_empty() || bytes.len() > MAX_CONTENT_SECRET_SCAN_BYTES as usize {
        return None;
    }
    // Reject binary files (NUL byte in prefix).
    let sniff = &bytes[..bytes.len().min(4096)];
    if sniff.contains(&0) {
        return None;
    }
    let text = std::str::from_utf8(bytes).ok()?;

    // 1. PEM private key headers
    if text.contains("-----BEGIN ") && text.contains(" PRIVATE KEY-----") {
        return Some("Embedded private key (content)");
    }

    // 2. High-confidence provider token prefixes
    if contains_aws_access_key(text) {
        return Some("AWS Access Key ID (content)");
    }
    for (prefix, min_tail, reason) in [
        ("ghp_", 30, "GitHub personal access token (content)"),
        ("gho_", 30, "GitHub OAuth token (content)"),
        ("ghu_", 30, "GitHub user-to-server token (content)"),
        ("ghs_", 30, "GitHub server token (content)"),
        ("github_pat_", 20, "GitHub fine-grained token (content)"),
        ("xoxb-", 20, "Slack bot token (content)"),
        ("xoxp-", 20, "Slack user token (content)"),
        ("sk_live_", 16, "Stripe live secret key (content)"),
        ("AIza", 35, "Google API key (content)"),
    ] {
        if contains_prefixed_token(text, prefix, min_tail) {
            return Some(reason);
        }
    }

    // 3. Hardcoded secret assignments in .env / config files
    let file = path
        .trim_start_matches("./")
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase();
    let is_config_file = file.starts_with(".env")
        || file.ends_with(".env")
        || file.ends_with(".yml")
        || file.ends_with(".yaml")
        || file.ends_with(".toml")
        || file.ends_with(".ini")
        || file.ends_with(".properties")
        || file.ends_with(".conf");
    if is_config_file && has_hardcoded_secret_assignment(text) {
        return Some("Hardcoded secret assignment (content)");
    }

    None
}

fn contains_aws_access_key(text: &str) -> bool {
    for prefix in ["AKIA", "ASIA"] {
        let mut rest = text;
        while let Some(idx) = rest.find(prefix) {
            let after = &rest[idx + prefix.len()..];
            let candidate = after.as_bytes();
            if candidate.len() >= 16
                && candidate[..16]
                    .iter()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            {
                let next_ok = candidate.get(16).is_none_or(|b| !b.is_ascii_alphanumeric());
                let prev_ok = idx == 0 || !rest.as_bytes()[idx - 1].is_ascii_alphanumeric();
                if prev_ok && next_ok {
                    return true;
                }
            }
            rest = &rest[idx + prefix.len()..];
        }
    }
    false
}

fn contains_prefixed_token(text: &str, prefix: &str, min_tail: usize) -> bool {
    let mut rest = text;
    while let Some(idx) = rest.find(prefix) {
        let prev_ok = idx == 0 || !rest.as_bytes()[idx - 1].is_ascii_alphanumeric();
        let after = &rest[idx + prefix.len()..];
        let tail_len = after
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_' || *b == b'-')
            .count();
        if prev_ok && tail_len >= min_tail {
            return true;
        }
        rest = &rest[idx + prefix.len()..];
    }
    false
}

fn has_hardcoded_secret_assignment(text: &str) -> bool {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }
        let Some((raw_key, raw_val)) = trimmed.split_once('=').or_else(|| trimmed.split_once(':'))
        else {
            continue;
        };
        let key = raw_key
            .trim()
            .trim_start_matches("export ")
            .trim_matches(['"', '\''])
            .to_ascii_uppercase();
        let sensitive_key = key.ends_with("_SECRET")
            || key.ends_with("_PASSWORD")
            || key.ends_with("_API_KEY")
            || key.ends_with("_TOKEN")
            || key == "DATABASE_URL"
            || key == "AWS_SECRET_ACCESS_KEY";
        if !sensitive_key {
            continue;
        }
        let val = raw_val
            .split('#')
            .next()
            .unwrap_or(raw_val)
            .trim()
            .trim_matches(['"', '\'', ',']);
        if val.len() < 12 {
            continue;
        }
        let val_lower = val.to_ascii_lowercase();
        let is_placeholder = val.starts_with('$')
            || val.starts_with("{{")
            || val.starts_with('<')
            || val_lower.contains("changeme")
            || val_lower.contains("example")
            || val_lower.contains("placeholder")
            || val_lower.contains("your_")
            || val_lower.contains("xxx");
        if !is_placeholder {
            return true;
        }
    }
    false
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
    fn custom_sensitive_patterns_support_additions_and_ignores() {
        let rules = SensitivePatterns::parse_rules(
            "# comment\n\n**/*.vault-token | Vault token\n!**/.env.example\n!**/certs/app.pem\n",
        );
        assert_eq!(
            rules.evaluate_path("app/prod.vault-token").as_deref(),
            Some("Vault token")
        );
        assert_eq!(rules.evaluate_path("app/.env.example"), None);
        assert_eq!(rules.evaluate_path("app/certs/app.pem"), None);
        assert_eq!(
            rules.evaluate_path("app/secret.pem").as_deref(),
            Some("Private key / certificate")
        );
    }

    #[test]
    fn content_secret_scanner_detects_keys_and_ignores_placeholders() {
        assert!(scan_content_secrets(
            "app/config.txt",
            b"-----BEGIN RSA PRIVATE KEY-----\nMIIE...\n-----END RSA PRIVATE KEY-----"
        )
        .is_some());
        assert!(scan_content_secrets("app/app.py", b"AWS_ID = \"AKIAIOSFODNN7EXAMPLE\"").is_some());
        assert!(
            scan_content_secrets("app/.env", b"API_SECRET=super_secret_prod_value_998877")
                .is_some()
        );
        assert_eq!(
            scan_content_secrets("app/.env", b"API_SECRET=${MY_SECRET_FROM_VAULT}"),
            None
        );
        assert_eq!(
            scan_content_secrets("app/.env", b"API_SECRET=your_example_placeholder"),
            None
        );
        assert_eq!(
            scan_content_secrets("app/bin", b"\x7fELF\0\0AKIAIOSFODNN7EXAMPLE"),
            None
        );
    }

    #[test]
    fn pkg_cache_covers_os_and_language_ecosystems() {
        assert!(is_pkg_cache("var/cache/apt/archives/foo.deb"));
        assert!(is_pkg_cache("var/lib/apt/lists/foo"));
        assert!(is_pkg_cache("var/cache/apk/index"));
        assert!(is_pkg_cache("var/cache/dnf/x"));
        assert!(is_pkg_cache("var/cache/zypp/packages/foo"));
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
        assert!(!is_pkg_cache("usr/bin/python3"));
        assert!(!is_pkg_cache("app/main.py"));
        assert!(!is_pkg_cache("usr/lib/x86_64-linux-gnu/libc.so.6"));
        assert!(!is_pkg_cache("app/my.cache/pipx/foo"));
        assert!(!is_pkg_cache(
            "root/.cache/pypoetry/virtualenvs/app-py3.12/x"
        ));
        assert!(!is_pkg_cache("usr/bin/apt"));
    }

    #[test]
    fn dev_artifact_covers_framework_caches() {
        assert!(is_dev_artifact("app/.git/HEAD"));
        assert!(is_dev_artifact("srv/__pycache__/foo.cpython-311.pyc"));
        assert!(is_dev_artifact("usr/src/app/node_modules/.cache/babel/foo"));
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
        assert!(!is_dev_artifact("web/.next/server/pages/index.js"));
        assert!(!is_dev_artifact("usr/lib/foo.so"));
    }

    #[test]
    fn path_frag_matches_only_at_segment_boundaries() {
        assert!(has_path_frag(".cache/pip/x", ".cache/pip/"));
        assert!(has_path_frag("home/app/.cache/pip/x", ".cache/pip/"));
        assert!(!has_path_frag("app/my.cache/pip/x", ".cache/pip/"));
        assert!(has_path_frag("etc/ssl/certs/ca.crt", "etc/ssl/certs/"));
        assert!(has_path_frag(
            "usr/local/etc/ssl/certs/ca.crt",
            "etc/ssl/certs/"
        ));
        assert!(!has_path_frag("fetc/ssl/certs/ca.crt", "etc/ssl/certs/"));
        assert!(!has_path_frag("", "x"));
    }
}
