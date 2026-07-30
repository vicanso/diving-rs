//! Resolve registry credentials for private pulls.
//!
//! Priority (first hit wins):
//! 1. Explicit credentials (CLI `--username` / `--password` / `--password-stdin`,
//!    or `REGISTRY_USERNAME` / `REGISTRY_PASSWORD` env vars)
//! 2. `~/.docker/config.json` static `auths` entries (base64 `user:pass`)
//! 3. Docker credential helpers (`credsStore` / `credHelpers`) when present
//!
//! Never logs the password. Web mode only uses env / docker config — credentials
//! are intentionally not accepted as query parameters.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use home::home_dir;
use once_cell::sync::OnceCell;
use serde::Deserialize;
use std::collections::HashMap;
use std::process::{Command, Stdio};
use tracing::{debug, warn};

/// Username + password (or PAT) for a registry Basic-auth handshake.
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
/// `explicit` (CLI/env) takes precedence; otherwise consult docker config.
pub fn resolve_for_registry(
    registry_url: &str,
    explicit: Option<&RegistryCredentials>,
) -> Option<RegistryCredentials> {
    if let Some(c) = explicit {
        if !c.is_empty() {
            return Some(c.clone());
        }
    }
    let host = registry_host(registry_url)?;
    load_from_docker_config(&host)
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
    use std::io::{self, BufRead};
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
struct DockerConfig {
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

fn docker_config_path() -> Option<std::path::PathBuf> {
    // Honour DOCKER_CONFIG when set (same as the docker CLI).
    if let Ok(dir) = std::env::var("DOCKER_CONFIG") {
        let p = std::path::PathBuf::from(dir).join("config.json");
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

fn load_docker_config() -> Option<&'static DockerConfig> {
    static CFG: OnceCell<Option<DockerConfig>> = OnceCell::new();
    CFG.get_or_init(|| {
        let path = docker_config_path()?;
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
    })
    .as_ref()
}

fn load_from_docker_config(host: &str) -> Option<RegistryCredentials> {
    let cfg = load_docker_config()?;

    // 1. Named credential helper for this host
    if let Some(helper) = cfg.cred_helpers.get(host) {
        if let Some(c) = credential_helper_get(helper, host) {
            return Some(c);
        }
    }

    // 2. Static auths map — try common key spellings
    for key in auth_lookup_keys(host) {
        if let Some(entry) = cfg.auths.get(&key) {
            if let Some(c) = entry_to_credentials(entry) {
                debug!(host, key, "docker config auth hit");
                return Some(c);
            }
        }
    }

    // 3. Default credsStore (e.g. "desktop", "osxkeychain")
    if let Some(store) = cfg.creds_store.as_deref() {
        if let Some(c) = credential_helper_get(store, host) {
            return Some(c);
        }
        // Docker Hub is often stored under the v1 index URL in the helper.
        if host == "index.docker.io" || host == "registry-1.docker.io" {
            if let Some(c) = credential_helper_get(store, "https://index.docker.io/v1/") {
                return Some(c);
            }
        }
    }

    None
}

/// Keys to try when looking up `auths` for a host.
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
    // Docker Hub aliases used by the docker CLI.
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
    // identitytoken alone (e.g. some cloud helpers write it here) — use as password
    // with username empty or from the field.
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

/// Invoke `docker-credential-<name> get` with the server URL on stdin.
fn credential_helper_get(helper: &str, server: &str) -> Option<RegistryCredentials> {
    let bin = format!("docker-credential-{helper}");
    let mut child = Command::new(&bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    {
        use std::io::Write;
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
        // user:pass → dXNlcjpwYXNz
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
}
