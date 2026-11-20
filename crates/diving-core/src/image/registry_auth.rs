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
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
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
}
