//! Resolve registry credentials for private pulls.
//!
//! Priority (first hit wins):
//! 1. Explicit credentials supplied via `AnalyzeOptions::credentials` /
//!    `PreviewOptions::credentials` (or `resolve_explicit` for CLI/env)
//! 2. `~/.docker/config.json` (or custom path) static `auths` entries
//! 3. Docker credential helpers (`credsStore` / `credHelpers`) when enabled
//!
//! Never logs passwords.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use home::home_dir;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tracing::{debug, warn};

use crate::store::sha256_hex;

/// Per-registry authentication, custom header, and HTTP/TLS transport rule.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RegistryRule {
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    pub insecure_http: bool,
    pub insecure_skip_tls_verify: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ca_cert: Option<String>,
}

/// Normalizes a registry host or wildcard pattern (`https://Host.Example:5000/v2` -> `host.example:5000`).
pub fn normalize_registry_host_pattern(raw: &str) -> String {
    let trimmed = raw.trim();
    let without_scheme = if trimmed.len() >= 8 && trimmed[..8].eq_ignore_ascii_case("https://") {
        &trimmed[8..]
    } else if trimmed.len() >= 7 && trimmed[..7].eq_ignore_ascii_case("http://") {
        &trimmed[7..]
    } else {
        trimmed
    };
    without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .trim()
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

fn is_docker_hub_host(host: &str) -> bool {
    matches!(
        host,
        "index.docker.io" | "registry-1.docker.io" | "docker.io"
    )
}

fn strip_port_from_host(host: &str) -> &str {
    if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else if host.matches(':').count() == 1 {
        host.split(':').next().unwrap_or(host)
    } else {
        host
    }
}

/// Returns `true` if `pattern` (exact host[:port] or `*.domain[:port]`) matches `target_host`.
pub fn registry_host_matches(pattern: &str, target_host: &str) -> bool {
    match_host_specificity(pattern, target_host).is_some()
}

/// Scores how specifically `pattern` matches `target_host` (`(2, _)` exact, `(1, len)` wildcard).
pub(crate) fn match_host_specificity(pattern: &str, target_host: &str) -> Option<(u8, usize)> {
    let pat = normalize_registry_host_pattern(pattern);
    let target = normalize_registry_host_pattern(target_host);
    if pat.is_empty() || target.is_empty() {
        return None;
    }
    if pat == target || (is_docker_hub_host(&pat) && is_docker_hub_host(&target)) {
        return Some((2, usize::MAX));
    }
    let suffix = pat.strip_prefix('*')?;
    if !suffix.starts_with('.') || suffix.len() <= 1 {
        return None;
    }
    if suffix.contains(':') {
        (target.ends_with(suffix) && target.len() > suffix.len()).then_some((1, pat.len()))
    } else {
        let bare = strip_port_from_host(&target);
        (bare.ends_with(suffix) && bare.len() > suffix.len()).then_some((1, pat.len()))
    }
}

/// Indexed, normalized registry rule matcher preserving original order indices.
#[derive(Debug, Clone, Default)]
pub(crate) struct RegistryRuleSet {
    rules: Vec<RegistryRule>,
}

impl RegistryRuleSet {
    pub(crate) fn new(rules: impl IntoIterator<Item = RegistryRule>) -> Self {
        let normalized = rules
            .into_iter()
            .filter_map(|mut rule| {
                rule.host = normalize_registry_host_pattern(&rule.host);
                (!rule.host.is_empty()).then_some(rule)
            })
            .collect();
        Self { rules: normalized }
    }

    pub(crate) fn rules(&self) -> &[RegistryRule] {
        &self.rules
    }

    /// Returns the best matching rule index (exact match > longest wildcard suffix).
    pub(crate) fn match_index(&self, host: &str) -> Option<usize> {
        let mut best: Option<(usize, (u8, usize))> = None;
        for (idx, rule) in self.rules.iter().enumerate() {
            let Some(score) = match_host_specificity(&rule.host, host) else {
                continue;
            };
            if best.is_none_or(|(_, best_score)| score > best_score) {
                best = Some((idx, score));
            }
        }
        best.map(|(idx, _)| idx)
    }

    /// Returns the best matching `(index, &RegistryRule)` for `host`.
    pub(crate) fn match_host(&self, host: &str) -> Option<(usize, &RegistryRule)> {
        self.match_index(host).map(|idx| (idx, &self.rules[idx]))
    }

    /// Whether an explicitly configured rule permits targeting `(scheme, host)` past default SSRF blocks.
    pub(crate) fn allows_private_target(&self, scheme: &str, host: &str) -> bool {
        let Some((_, rule)) = self.match_host(host) else {
            return false;
        };
        scheme.eq_ignore_ascii_case("https")
            || (scheme.eq_ignore_ascii_case("http") && rule.insecure_http)
    }

    /// Computes a stable hash of the matched rule's auth and connection properties for cache isolation.
    pub(crate) fn auth_fingerprint(&self, host: &str) -> Option<String> {
        let (_, rule) = self.match_host(host)?;
        let mut buf = String::new();
        buf.push_str(&rule.host);
        buf.push('\0');
        if let Some(u) = rule.username.as_deref() {
            buf.push_str(u);
        }
        buf.push('\0');
        if let Some(p) = rule.password.as_deref() {
            buf.push_str(p);
        }
        buf.push('\0');
        if let Some(t) = rule.token.as_deref() {
            buf.push_str(t);
        }
        buf.push('\0');
        for (k, v) in &rule.headers {
            buf.push_str(&k.to_ascii_lowercase());
            buf.push('=');
            buf.push_str(v);
            buf.push('\0');
        }
        buf.push(if rule.insecure_http { '1' } else { '0' });
        buf.push(if rule.insecure_skip_tls_verify {
            '1'
        } else {
            '0'
        });
        if let Some(ca) = rule.ca_cert.as_deref() {
            buf.push('\0');
            buf.push_str(ca);
        }
        Some(sha256_hex(buf.as_bytes()))
    }
}

/// Username + password (or PAT) for a registry Basic/Bearer handshake.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegistryCredentials {
    pub username: String,
    pub password: String,
}

impl RegistryCredentials {
    pub fn is_empty(&self) -> bool {
        self.username.is_empty()
    }

    /// `Authorization: Basic …` header value (without the `Basic ` prefix).
    pub fn basic_token(&self) -> String {
        B64.encode(format!("{}:{}", self.username, self.password))
    }
}

/// Build credentials from explicit CLI / env values.
///
/// - `username` / `password` win when provided
/// - Falls back to `REGISTRY_USERNAME` / `REGISTRY_PASSWORD`
/// - `password_stdin`: when true, password is read from stdin (one line)
pub fn resolve_explicit(
    username: Option<&str>,
    password: Option<&str>,
    password_stdin: bool,
) -> Option<RegistryCredentials> {
    let username = first_non_empty(username, "REGISTRY_USERNAME")?;
    let password = if password_stdin {
        read_password_stdin().unwrap_or_default()
    } else {
        first_non_empty(password, "REGISTRY_PASSWORD").unwrap_or_default()
    };
    Some(RegistryCredentials { username, password })
}

/// Resolve credentials for a registry base URL such as
/// `https://ghcr.io/v2` or `https://index.docker.io/v2`.
///
/// `explicit` takes precedence; otherwise consults `docker_cfg` if enabled.
pub(crate) fn resolve_for_registry(
    registry_url: &str,
    explicit: Option<&RegistryCredentials>,
    docker_cfg: Option<&DockerConfig>,
) -> Option<RegistryCredentials> {
    if let Some(c) = explicit {
        if !c.is_empty() {
            return Some(c.clone());
        }
    }
    let cfg = docker_cfg?;
    let host = registry_host(registry_url)?;
    load_from_docker_config(cfg, &host)
}

/// Extract host from a diving registry URL (`https://host[:port]/v2`).
pub fn registry_host(registry_url: &str) -> Option<String> {
    let s = registry_url
        .strip_prefix("https://")
        .or_else(|| registry_url.strip_prefix("http://"))
        .unwrap_or(registry_url);
    let host = s.split('/').next().unwrap_or("").trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn first_non_empty(explicit: Option<&str>, env_key: &str) -> Option<String> {
    if let Some(v) = explicit {
        let v = v.trim();
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    if let Ok(v) = std::env::var(env_key) {
        let v = v.trim();
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    None
}

fn read_password_stdin() -> Option<String> {
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line).ok()?;
    let line = line.trim_end_matches(['\r', '\n']).to_string();
    if line.is_empty() {
        None
    } else {
        Some(line)
    }
}

// ---- docker config.json ---------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DockerConfig {
    #[serde(default)]
    auths: HashMap<String, DockerAuthEntry>,
    #[serde(default)]
    creds_store: Option<String>,
    #[serde(default)]
    cred_helpers: HashMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
struct DockerAuthEntry {
    #[serde(default)]
    auth: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    identitytoken: Option<String>,
}

fn default_docker_config_path() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("DOCKER_CONFIG") {
        let p = PathBuf::from(dir).join("config.json");
        if p.exists() {
            return Some(p);
        }
    }
    let home = home_dir()?;
    let p = home.join(".docker").join("config.json");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}

pub(crate) fn load_docker_config(custom_path: Option<&Path>) -> Option<DockerConfig> {
    let path = match custom_path {
        Some(p) if p.exists() => p.to_path_buf(),
        Some(_) => return None,
        None => default_docker_config_path()?,
    };
    let data = std::fs::read_to_string(&path).ok()?;
    match serde_json::from_str::<DockerConfig>(&data) {
        Ok(c) => Some(c),
        Err(e) => {
            warn!(
                err = e.to_string(),
                path = %path.display(),
                "failed to parse docker config.json"
            );
            None
        }
    }
}

fn load_from_docker_config(cfg: &DockerConfig, host: &str) -> Option<RegistryCredentials> {
    if let Some(helper) = cfg.cred_helpers.get(host) {
        if let Some(c) = credential_helper_get(helper, host) {
            return Some(c);
        }
    }

    for key in auth_lookup_keys(host) {
        if let Some(entry) = cfg.auths.get(&key) {
            if let Some(c) = entry_to_credentials(entry) {
                debug!(host, key, "docker config auth hit");
                return Some(c);
            }
        }
    }

    if let Some(store) = cfg.creds_store.as_deref() {
        if let Some(c) = credential_helper_get(store, host) {
            return Some(c);
        }
        if host == "index.docker.io" || host == "registry-1.docker.io" {
            if let Some(c) = credential_helper_get(store, "https://index.docker.io/v1/") {
                return Some(c);
            }
        }
    }

    None
}

fn auth_lookup_keys(host: &str) -> Vec<String> {
    let mut keys = vec![
        host.to_string(),
        format!("https://{host}"),
        format!("https://{host}/"),
        format!("http://{host}"),
        format!("http://{host}/"),
        format!("https://{host}/v1/"),
        format!("https://{host}/v2/"),
        format!("http://{host}/v1/"),
        format!("http://{host}/v2/"),
    ];
    if host == "index.docker.io" || host == "registry-1.docker.io" {
        keys.push("https://index.docker.io/v1/".into());
        keys.push("https://index.docker.io/v1".into());
        keys.push("https://registry-1.docker.io/v2/".into());
        keys.push("docker.io".into());
        keys.push("https://docker.io".into());
    }
    keys
}

fn entry_to_credentials(entry: &DockerAuthEntry) -> Option<RegistryCredentials> {
    if let (Some(user), Some(pass)) = (
        entry.username.as_deref().filter(|s| !s.is_empty()),
        entry
            .password
            .as_deref()
            .or(entry.identitytoken.as_deref())
            .filter(|s| !s.is_empty()),
    ) {
        return Some(RegistryCredentials {
            username: user.to_string(),
            password: pass.to_string(),
        });
    }
    let auth = entry.auth.as_deref()?.trim();
    if auth.is_empty() {
        return None;
    }
    let raw = B64.decode(auth).ok()?;
    let s = String::from_utf8(raw).ok()?;
    let (user, pass) = s.split_once(':')?;
    if user.is_empty() {
        return None;
    }
    Some(RegistryCredentials {
        username: user.to_string(),
        password: pass.to_string(),
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct HelperPayload {
    username: String,
    secret: String,
}

fn credential_helper_get(helper: &str, server: &str) -> Option<RegistryCredentials> {
    let bin = format!("docker-credential-{helper}");
    let mut child = Command::new(&bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    {
        let mut stdin = child.stdin.take()?;
        let _ = writeln!(stdin, "{server}");
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        debug!(
            helper = bin.as_str(),
            server,
            status = ?output.status,
            "credential helper returned non-zero"
        );
        return None;
    }
    let payload: HelperPayload = serde_json::from_slice(&output.stdout).ok()?;
    if payload.username.is_empty() {
        return None;
    }
    Some(RegistryCredentials {
        username: payload.username,
        password: payload.secret,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_host_strips_scheme_and_v2() {
        assert_eq!(
            registry_host("https://ghcr.io/v2").as_deref(),
            Some("ghcr.io")
        );
        assert_eq!(
            registry_host("https://localhost:5000/v2").as_deref(),
            Some("localhost:5000")
        );
        assert_eq!(
            registry_host("https://index.docker.io/v2").as_deref(),
            Some("index.docker.io")
        );
    }

    #[test]
    fn entry_decodes_base64_auth() {
        let entry = DockerAuthEntry {
            auth: Some(B64.encode("user:s3cret")),
            ..Default::default()
        };
        let c = entry_to_credentials(&entry).expect("decodes");
        assert_eq!(c.username, "user");
        assert_eq!(c.password, "s3cret");
    }

    #[test]
    fn entry_prefers_username_password_fields() {
        let entry = DockerAuthEntry {
            username: Some("alice".into()),
            password: Some("pw".into()),
            auth: Some(B64.encode("ignored:x")),
            ..Default::default()
        };
        let c = entry_to_credentials(&entry).expect("fields");
        assert_eq!(c.username, "alice");
        assert_eq!(c.password, "pw");
    }

    #[test]
    fn basic_token_roundtrip_shape() {
        let c = RegistryCredentials {
            username: "u".into(),
            password: "p".into(),
        };
        let raw = B64.decode(c.basic_token()).unwrap();
        assert_eq!(String::from_utf8(raw).unwrap(), "u:p");
    }

    #[test]
    fn auth_lookup_keys_include_docker_hub_aliases() {
        let keys = auth_lookup_keys("index.docker.io");
        assert!(keys.iter().any(|k| k.contains("index.docker.io/v1")));
    }

    #[test]
    fn resolve_explicit_requires_username() {
        assert!(resolve_explicit(None, Some("pw"), false).is_none());
        let c = resolve_explicit(Some("u"), Some("pw"), false).unwrap();
        assert_eq!(c.username, "u");
        assert_eq!(c.password, "pw");
    }

    #[test]
    fn resolve_for_registry_skips_docker_config_when_disabled() {
        let cfg = DockerConfig {
            auths: HashMap::from([(
                "ghcr.io".to_string(),
                DockerAuthEntry {
                    username: Some("alice".into()),
                    password: Some("tok".into()),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        };
        assert!(resolve_for_registry("https://ghcr.io/v2", None, None).is_none());
        let hit = resolve_for_registry("https://ghcr.io/v2", None, Some(&cfg)).unwrap();
        assert_eq!(hit.username, "alice");
    }

    #[test]
    fn registry_rule_set_prefers_exact_then_longest_wildcard_and_docker_aliases() {
        let rules = RegistryRuleSet::new([
            RegistryRule {
                host: "*.corp.internal".into(),
                insecure_http: false,
                ..Default::default()
            },
            RegistryRule {
                host: "https://*.sub.corp.internal/v2".into(),
                insecure_http: true,
                ..Default::default()
            },
            RegistryRule {
                host: "reg.sub.corp.internal:5000".into(),
                insecure_http: true,
                token: Some("exact-tok".into()),
                ..Default::default()
            },
            RegistryRule {
                host: "docker.io".into(),
                username: Some("hub-user".into()),
                ..Default::default()
            },
        ]);

        assert_eq!(
            rules
                .match_host("reg.sub.corp.internal:5000")
                .and_then(|(_, r)| r.token.as_deref()),
            Some("exact-tok")
        );
        assert_eq!(
            rules
                .match_host("ci.sub.corp.internal:8443")
                .map(|(_, r)| r.host.as_str()),
            Some("*.sub.corp.internal")
        );
        assert_eq!(
            rules
                .match_host("artifactory.corp.internal")
                .map(|(_, r)| r.host.as_str()),
            Some("*.corp.internal")
        );
        assert_eq!(
            rules
                .match_host("index.docker.io")
                .and_then(|(_, r)| r.username.as_deref()),
            Some("hub-user")
        );
        assert!(rules.allows_private_target("http", "ci.sub.corp.internal:8443"));
        assert!(!rules.allows_private_target("http", "artifactory.corp.internal"));
        assert!(rules.allows_private_target("https", "artifactory.corp.internal"));
    }
}
