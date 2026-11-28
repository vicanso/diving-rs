use bytesize::ByteSize;
use config::{Config, File, FileFormat};
use diving_core::{
    is_private_or_loopback_host, normalize_registry_host_pattern, registry_host_matches, Analyzer,
    CacheConfig, Error as CoreError, GateConfig, RegistryRule, SensitivePatterns,
    SeverityThreshold,
};
use home::home_dir;
use humantime::Duration as HumanDuration;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::to_string as quote_yaml_scalar;
use std::collections::BTreeMap;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::id as process_id;
use std::time::Duration;
use std::{env, fs};
use tracing::warn;

use crate::env_util::expand_env_vars;
use crate::logging::install_crypto_provider;

const DEFAULT_LAYER_TTL: Duration = Duration::from_secs(90 * 24 * 3600);
const DEFAULT_MEMORY_TTL: Duration = Duration::from_secs(60);
const DEFAULT_MAX_DOWNLOAD_FILE_SIZE: u64 = 100 * 1024 * 1024;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DivingConfig {
    pub layer_path: Option<String>,
    pub layer_ttl: Option<String>,
    pub analysis_path: Option<String>,
    pub threads: Option<usize>,
    pub worker_threads: Option<usize>,
    pub layer_concurrency: Option<usize>,
    pub lowest_efficiency: Option<f64>,
    pub highest_wasted_bytes: Option<ByteSize>,
    pub highest_user_wasted_percent: Option<f64>,
    pub highest_image_size: Option<ByteSize>,
    pub highest_size_increase: Option<ByteSize>,
    pub fail_on_severity: Option<SeverityThreshold>,
    pub ignore_recommendations: Option<Vec<String>>,
    pub cleanup_interval_hours: Option<u64>,
    pub registry_allowlist: Option<Vec<String>>,
    pub max_download_file_size: Option<ByteSize>,
    pub max_layer_cache_size: Option<ByteSize>,
    pub analysis_memory_ttl: Option<String>,
    pub max_concurrent_analyses: Option<usize>,
    pub mcp_allowed_hosts: Option<Vec<String>>,
    pub public_url: Option<String>,
    pub webhook_token: Option<String>,
    pub webhook_allow_public: Option<bool>,
    pub wecom_webhook: Option<String>,
    pub feishu_webhook: Option<String>,
    pub feishu_secret: Option<String>,
    pub dingtalk_webhook: Option<String>,
    pub dingtalk_secret: Option<String>,
    pub slack_webhook: Option<String>,
    pub notify_webhook: Option<String>,
    pub notify_webhook_secret: Option<String>,
    pub registries: Option<Vec<RegistryRule>>,
}

impl DivingConfig {
    pub fn worker_threads(&self) -> usize {
        self.worker_threads
            .or(self.threads)
            .unwrap_or_else(num_cpus::get)
            .max(1)
    }

    pub fn layer_concurrency(&self) -> Option<usize> {
        self.layer_concurrency.or(self.threads).map(|v| v.max(1))
    }

    pub fn layer_ttl(&self) -> Duration {
        let Some(raw) = self.layer_ttl.as_deref() else {
            return DEFAULT_LAYER_TTL;
        };
        raw.parse::<HumanDuration>()
            .map(Into::into)
            .unwrap_or(DEFAULT_LAYER_TTL)
    }

    pub fn cleanup_interval(&self) -> Duration {
        let hours = self.cleanup_interval_hours.unwrap_or(1).max(1);
        Duration::from_secs(hours * 3600)
    }

    pub fn max_layer_cache_size(&self) -> Option<u64> {
        self.max_layer_cache_size.map(|v| v.0)
    }

    pub fn max_download_file_size(&self) -> u64 {
        self.max_download_file_size
            .map(|v| v.0)
            .unwrap_or(DEFAULT_MAX_DOWNLOAD_FILE_SIZE)
    }

    pub fn analysis_memory_ttl(&self) -> Duration {
        let Some(value) = self.analysis_memory_ttl.as_deref() else {
            return DEFAULT_MEMORY_TTL;
        };
        match value.parse::<HumanDuration>() {
            Ok(ttl) => ttl.into(),
            Err(err) => {
                warn!(
                    value,
                    err = err.to_string(),
                    "invalid analysis_memory_ttl, using 1m"
                );
                DEFAULT_MEMORY_TTL
            }
        }
    }

    pub fn max_concurrent_analyses(&self) -> Option<usize> {
        self.max_concurrent_analyses.filter(|limit| *limit > 0)
    }

    pub fn registry_allowlist(&self) -> Vec<String> {
        normalize_string_list(self.registry_allowlist.as_deref())
    }

    pub fn mcp_allowed_hosts(&self) -> Vec<String> {
        normalize_string_list(self.mcp_allowed_hosts.as_deref())
    }

    pub fn ignore_recommendations(&self) -> Vec<String> {
        normalize_string_list(self.ignore_recommendations.as_deref())
    }

    pub fn webhook_allow_public(&self) -> bool {
        self.webhook_allow_public.unwrap_or(false)
    }

    pub fn gate_config(&self) -> GateConfig {
        let defaults = GateConfig::default();
        GateConfig {
            lowest_efficiency: self.lowest_efficiency.unwrap_or(defaults.lowest_efficiency),
            highest_wasted_bytes: self
                .highest_wasted_bytes
                .map(|v| v.0)
                .unwrap_or(defaults.highest_wasted_bytes),
            highest_user_wasted_percent: self
                .highest_user_wasted_percent
                .unwrap_or(defaults.highest_user_wasted_percent),
            highest_image_size: self.highest_image_size.map(|v| v.0),
            highest_size_increase: self.highest_size_increase.map(|v| v.0),
            fail_on_severity: self.fail_on_severity,
            ignore_recommendations: self.ignore_recommendations(),
        }
    }
}

fn normalize_string_list(raw: Option<&[String]>) -> Vec<String> {
    raw.unwrap_or_default()
        .iter()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Loaded YAML configuration plus resolved directories under `~/.diving`.
#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub config: DivingConfig,
    pub config_dir: PathBuf,
    pub layer_dir: PathBuf,
    pub analysis_dir: PathBuf,
}

fn custom_config_file(flag: Option<PathBuf>, env_val: Option<String>) -> Option<PathBuf> {
    flag.or_else(|| env_val.filter(|v| !v.trim().is_empty()).map(PathBuf::from))
}

fn expand_tilde(raw: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed == "~" {
        return home
            .map(Path::to_path_buf)
            .ok_or_else(|| "failed to determine home directory".to_string());
    }
    if let Some(rest) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
    {
        let h = home.ok_or_else(|| "failed to determine home directory".to_string())?;
        return Ok(h.join(rest));
    }
    Ok(PathBuf::from(trimmed))
}

fn resolve_cache_path(
    raw: Option<&str>,
    default_name: &str,
    config_dir: &Path,
    home: Option<&Path>,
) -> Result<PathBuf, String> {
    match raw {
        Some(val) if !val.trim().is_empty() => {
            let expanded = expand_tilde(val, home)?;
            if expanded.is_absolute() {
                Ok(expanded)
            } else {
                Ok(config_dir.join(expanded))
            }
        }
        _ => Ok(config_dir.join(default_name)),
    }
}

fn create_dir(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| {
        format!(
            "failed to create {} directory: check permissions ({e})",
            dir.display()
        )
    })
}

/// Load `config.yml` (`explicit_file` > `$DIVING_CONFIG` > `~/.diving/config.yml`).
pub fn load_config(explicit_file: Option<&str>) -> Result<LoadedConfig, String> {
    load_config_with_home(explicit_file, env::var("DIVING_CONFIG").ok(), home_dir())
}

fn load_config_with_home(
    explicit_file: Option<&str>,
    div_cfg_env: Option<String>,
    home: Option<PathBuf>,
) -> Result<LoadedConfig, String> {
    let custom = custom_config_file(explicit_file.map(PathBuf::from), div_cfg_env);
    let default_config_dir = home.as_deref().map(|h| h.join(".diving"));

    let (config_file, config_dir, need_default_dir) = match custom {
        Some(raw_path) => {
            let path = expand_tilde(&raw_path.to_string_lossy(), home.as_deref())?;
            if !path.is_file() {
                return Err(format!("config file not found: {}", path.display()));
            }
            let dir = default_config_dir.clone().unwrap_or_else(|| {
                path.parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf()
            });
            (path, dir, false)
        }
        None => {
            let dir = default_config_dir
                .clone()
                .ok_or_else(|| "failed to determine home directory".to_string())?;
            create_dir(&dir)?;
            let path = dir.join("config.yml");
            if !path.exists() {
                fs::File::create(&path).map_err(|e| {
                    format!(
                        "failed to create {}: check directory permissions ({e})",
                        path.display()
                    )
                })?;
            }
            (path, dir, true)
        }
    };

    let config = Config::builder()
        .add_source(File::from(config_file.as_path()).format(FileFormat::Yaml))
        .build()
        .map_err(|e| format!("failed to read {}: {e}", config_file.display()))?
        .try_deserialize::<DivingConfig>()
        .map_err(|e| format!("{} contains invalid fields: {e}", config_file.display()))?;

    let layer_dir = resolve_cache_path(
        config.layer_path.as_deref(),
        "layers",
        &config_dir,
        home.as_deref(),
    )?;
    let analysis_dir = resolve_cache_path(
        config.analysis_path.as_deref(),
        "analysis",
        &config_dir,
        home.as_deref(),
    )?;

    if !need_default_dir {
        if let Some(ref default_dir) = default_config_dir {
            if layer_dir.starts_with(default_dir) || analysis_dir.starts_with(default_dir) {
                create_dir(default_dir)?;
            }
        }
    }

    Ok(LoadedConfig {
        config,
        config_dir,
        layer_dir,
        analysis_dir,
    })
}

/// Loads `config.yml` directly inside an isolated `config_dir` (never falling back to `$HOME/.diving`).
pub fn load_config_in_dir(config_dir: &Path) -> Result<LoadedConfig, String> {
    let config_file = config_dir.join("config.yml");
    let config = if config_file.is_file() {
        Config::builder()
            .add_source(File::from(config_file.as_path()).format(FileFormat::Yaml))
            .build()
            .map_err(|e| format!("failed to read {}: {e}", config_file.display()))?
            .try_deserialize::<DivingConfig>()
            .map_err(|e| format!("{} contains invalid fields: {e}", config_file.display()))?
    } else {
        DivingConfig::default()
    };
    let layer_dir = resolve_cache_path(config.layer_path.as_deref(), "layers", config_dir, None)?;
    let analysis_dir = resolve_cache_path(
        config.analysis_path.as_deref(),
        "analysis",
        config_dir,
        None,
    )?;
    Ok(LoadedConfig {
        config,
        config_dir: config_dir.to_path_buf(),
        layer_dir,
        analysis_dir,
    })
}

/// Load custom sensitive-file patterns from `<config_dir>/sensitive-files`.
pub fn load_user_sensitive_patterns(config_dir: &Path) -> SensitivePatterns {
    let path = config_dir.join("sensitive-files");
    match fs::read_to_string(&path) {
        Ok(content) => SensitivePatterns::parse_rules(&content),
        Err(_) => SensitivePatterns::default(),
    }
}

#[derive(Default, Deserialize)]
struct RegistriesDoc {
    #[serde(default)]
    registries: Vec<RegistryRule>,
}

/// Path to the user-managed private registry overrides file (`<config_dir>/registries.yml`).
pub fn registries_file_path(config_dir: &Path) -> PathBuf {
    config_dir.join("registries.yml")
}

/// Loads user-managed registry rules from `<config_dir>/registries.yml`.
pub fn load_user_registries(config_dir: &Path) -> Result<Vec<RegistryRule>, String> {
    let path = registries_file_path(config_dir);
    let raw = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(format!("failed to read {}: {err}", path.display())),
    };
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }
    let doc = Config::builder()
        .add_source(File::from_str(&raw, FileFormat::Yaml))
        .build()
        .map_err(|e| format!("failed to parse {}: {e}", path.display()))?
        .try_deserialize::<RegistriesDoc>()
        .map_err(|e| format!("{} contains invalid registry entries: {e}", path.display()))?;
    Ok(doc.registries)
}

fn push_yaml_kv(out: &mut String, indent: &str, key: &str, val: &str) -> Result<(), String> {
    let quoted = quote_yaml_scalar(val).map_err(|e| format!("serialize yaml scalar fail: {e}"))?;
    out.push_str(indent);
    out.push_str(key);
    out.push_str(": ");
    out.push_str(&quoted);
    out.push('\n');
    Ok(())
}

fn format_registries_yaml(rules: &[RegistryRule]) -> Result<String, String> {
    let mut out = String::from("# Managed by Diving Desktop — file mode 0600\n");
    let valid_rules: Vec<RegistryRule> = rules
        .iter()
        .cloned()
        .filter_map(|mut r| {
            r.host = normalize_registry_host_pattern(&r.host);
            (!r.host.is_empty()).then_some(r)
        })
        .collect();
    if valid_rules.is_empty() {
        out.push_str("registries: []\n");
        return Ok(out);
    }
    out.push_str("registries:\n");
    for rule in valid_rules {
        let host_quoted =
            quote_yaml_scalar(&rule.host).map_err(|e| format!("serialize host fail: {e}"))?;
        out.push_str("  - host: ");
        out.push_str(&host_quoted);
        out.push('\n');
        if let Some(u) = rule.username.as_deref().filter(|s| !s.is_empty()) {
            push_yaml_kv(&mut out, "    ", "username", u)?;
        }
        if let Some(p) = rule.password.as_deref().filter(|s| !s.is_empty()) {
            push_yaml_kv(&mut out, "    ", "password", p)?;
        }
        if let Some(t) = rule.token.as_deref().filter(|s| !s.is_empty()) {
            push_yaml_kv(&mut out, "    ", "token", t)?;
        }
        if !rule.headers.is_empty() {
            let mut non_empty = rule
                .headers
                .iter()
                .filter(|(k, _)| !k.trim().is_empty())
                .peekable();
            if non_empty.peek().is_some() {
                out.push_str("    headers:\n");
                for (k, v) in non_empty {
                    let kq = quote_yaml_scalar(k.trim())
                        .map_err(|e| format!("serialize header name fail: {e}"))?;
                    let vq = quote_yaml_scalar(v)
                        .map_err(|e| format!("serialize header value fail: {e}"))?;
                    out.push_str("      ");
                    out.push_str(&kq);
                    out.push_str(": ");
                    out.push_str(&vq);
                    out.push('\n');
                }
            }
        }
        if rule.insecure_http {
            out.push_str("    insecure_http: true\n");
        }
        if rule.insecure_skip_tls_verify {
            out.push_str("    insecure_skip_tls_verify: true\n");
        }
        if let Some(ca) = rule
            .ca_cert
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            push_yaml_kv(&mut out, "    ", "ca_cert", ca)?;
        }
    }
    Ok(out)
}

/// Atomically writes `<config_dir>/registries.yml` with `0600` permissions on Unix.
pub fn save_user_registries(config_dir: &Path, rules: &[RegistryRule]) -> Result<(), String> {
    create_dir(config_dir)?;
    let yaml = format_registries_yaml(rules)?;
    let dest = registries_file_path(config_dir);
    let tmp = config_dir.join(format!(".registries.yml.tmp-{}", process_id()));

    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    opts.mode(0o600);

    let mut file = opts
        .open(&tmp)
        .map_err(|e| format!("failed to write {}: {e}", dest.display()))?;
    if let Err(e) = file.write_all(yaml.as_bytes()).and_then(|()| file.flush()) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("failed to write {}: {e}", dest.display()));
    }
    drop(file);
    #[cfg(unix)]
    if let Err(e) = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600)) {
        let _ = fs::remove_file(&tmp);
        return Err(format!(
            "failed to set permissions on {}: {e}",
            dest.display()
        ));
    }
    fs::rename(&tmp, &dest).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("failed to replace {}: {e}", dest.display())
    })
}

/// Merges base `config.yml` rules with `registries.yml` overrides (normalized host key wins).
pub fn merge_registry_rules(
    base: &[RegistryRule],
    overrides: &[RegistryRule],
) -> Vec<RegistryRule> {
    let mut out = Vec::with_capacity(base.len() + overrides.len());
    let mut index_by_host = BTreeMap::<String, usize>::new();

    for rule in base {
        let mut norm = rule.clone();
        norm.host = normalize_registry_host_pattern(&norm.host);
        if norm.host.is_empty() {
            continue;
        }
        if let Some(&idx) = index_by_host.get(&norm.host) {
            out[idx] = norm;
        } else {
            index_by_host.insert(norm.host.clone(), out.len());
            out.push(norm);
        }
    }
    for rule in overrides {
        let mut norm = rule.clone();
        norm.host = normalize_registry_host_pattern(&norm.host);
        if norm.host.is_empty() {
            continue;
        }
        if let Some(&idx) = index_by_host.get(&norm.host) {
            out[idx] = norm;
        } else {
            index_by_host.insert(norm.host.clone(), out.len());
            out.push(norm);
        }
    }
    out
}

fn expand_registry_rule(
    mut rule: RegistryRule,
    config_dir: &Path,
    home: Option<&Path>,
) -> Result<RegistryRule, String> {
    rule.host = normalize_registry_host_pattern(&rule.host);
    rule.username = rule.username.map(|v| expand_env_vars(&v));
    rule.password = rule.password.map(|v| expand_env_vars(&v));
    rule.token = rule.token.map(|v| expand_env_vars(&v));
    rule.headers = rule
        .headers
        .into_iter()
        .map(|(k, v)| (k.trim().to_string(), expand_env_vars(&v)))
        .filter(|(k, _)| !k.is_empty())
        .collect();
    rule.ca_cert = rule
        .ca_cert
        .as_deref()
        .map(expand_env_vars)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(|s| -> Result<String, String> {
            let expanded = expand_tilde(&s, home)?;
            let resolved = if expanded.is_absolute() {
                expanded
            } else {
                config_dir.join(expanded)
            };
            Ok(resolved.to_string_lossy().into_owned())
        })
        .transpose()?;
    Ok(rule)
}

/// Loads, merges (`config.yml` + `registries.yml`), and expands env/CA paths for runtime use.
pub fn resolve_runtime_registries(loaded: &LoadedConfig) -> Result<Vec<RegistryRule>, String> {
    let base = loaded.config.registries.as_deref().unwrap_or(&[]);
    let overrides = load_user_registries(&loaded.config_dir)?;
    let merged = merge_registry_rules(base, &overrides);
    let home = home_dir();
    merged
        .into_iter()
        .filter(|r| !r.host.trim().is_empty())
        .map(|rule| expand_registry_rule(rule, &loaded.config_dir, home.as_deref()))
        .collect()
}

/// Build the shared `reqwest::Client` used by both `Analyzer` and AI/notifications.
pub fn build_http_client() -> Result<Client, String> {
    install_crypto_provider();
    Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(32)
        .build()
        .map_err(|e| format!("failed to build shared reqwest client: {e}"))
}

/// Build a `reqwest::Client` for `diving-web` registry fetches.
/// When `block_private_redirects` is true, HTTP redirects to non-HTTPS or
/// private/loopback/link-local hosts are refused unless explicitly permitted by `registries`.
pub fn build_web_http_client(block_private_redirects: bool) -> Result<Client, String> {
    build_web_http_client_with_rules(block_private_redirects, &[])
}

/// Build a `reqwest::Client` for `diving-web` honoring configured private [`RegistryRule`] entries.
pub fn build_web_http_client_for(loaded: &LoadedConfig) -> Result<Client, String> {
    let block = loaded.config.registry_allowlist().is_empty();
    let rules = resolve_runtime_registries(loaded)?;
    build_web_http_client_with_rules(block, &rules)
}

fn build_web_http_client_with_rules(
    block_private_redirects: bool,
    rules: &[RegistryRule],
) -> Result<Client, String> {
    install_crypto_provider();
    let mut builder = Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(32);
    if block_private_redirects {
        let allowed_rules: Vec<RegistryRule> = rules.to_vec();
        builder = builder.redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error("too many redirects");
            }
            let url = attempt.url();
            let host_only = url.host_str().unwrap_or_default();
            let host_port = match url.port() {
                Some(port) => format!("{host_only}:{port}"),
                None => host_only.to_string(),
            };
            let explicit_rule = allowed_rules
                .iter()
                .filter(|r| registry_host_matches(&r.host, &host_port))
                .max_by_key(|r| (!r.host.starts_with("*."), r.host.len()));
            if url.scheme() != "https" {
                let allow_http =
                    url.scheme() == "http" && explicit_rule.is_some_and(|rule| rule.insecure_http);
                if !allow_http {
                    return attempt.error("redirect to non-https scheme blocked");
                }
            }
            if is_private_or_loopback_host(host_only) && explicit_rule.is_none() {
                let blocked_host = host_only.to_owned();
                return attempt.error(format!("redirect to private host blocked: {blocked_host}"));
            }
            attempt.follow()
        }));
    }
    builder
        .build()
        .map_err(|e| format!("failed to build web reqwest client: {e}"))
}

fn cache_config_from(loaded: &LoadedConfig) -> CacheConfig {
    CacheConfig::persistent(&loaded.layer_dir, &loaded.analysis_dir)
        .layer_ttl(loaded.config.layer_ttl())
        .max_layer_cache_size(loaded.config.max_layer_cache_size())
}

fn build_local_analyzer(
    loaded: &LoadedConfig,
    http_client: Client,
    singleflight: bool,
    memory_cache_ttl: Duration,
) -> diving_core::Result<Analyzer> {
    let registries = resolve_runtime_registries(loaded)
        .map_err(|message| CoreError::InvalidConfig { message })?;
    let mut builder = Analyzer::builder()
        .cache(cache_config_from(loaded))
        .http_client(http_client)
        .allow_local_sources(true)
        .use_docker_config(true)
        .registries(registries)
        .singleflight(singleflight)
        .memory_cache_ttl(memory_cache_ttl)
        .sensitive_patterns(load_user_sensitive_patterns(&loaded.config_dir));
    if let Some(concurrency) = loaded.config.layer_concurrency() {
        builder = builder.layer_concurrency(concurrency);
    }
    builder.build()
}

/// Construct an [`Analyzer`] configured for CLI/TUI/CI runs.
pub fn build_cli_analyzer(
    loaded: &LoadedConfig,
    http_client: Client,
) -> diving_core::Result<Analyzer> {
    build_local_analyzer(loaded, http_client, false, Duration::ZERO)
}

/// Construct an [`Analyzer`] configured for the native desktop app (`Diving Desktop`).
pub fn build_desktop_analyzer(
    loaded: &LoadedConfig,
    http_client: Client,
) -> diving_core::Result<Analyzer> {
    build_local_analyzer(
        loaded,
        http_client,
        true,
        loaded.config.analysis_memory_ttl(),
    )
}

const DEFAULT_WEB_MAX_CONCURRENT_ANALYSES: usize = 8;

/// Construct an [`Analyzer`] configured for Web/MCP/Webhook runs.
pub fn build_web_analyzer(
    loaded: &LoadedConfig,
    http_client: Client,
) -> diving_core::Result<Analyzer> {
    let allowlist = loaded.config.registry_allowlist();
    let registries = resolve_runtime_registries(loaded)
        .map_err(|message| CoreError::InvalidConfig { message })?;
    let mut builder = Analyzer::builder()
        .cache(cache_config_from(loaded))
        .http_client(http_client)
        .use_docker_config(true)
        .registries(registries)
        .singleflight(true)
        .memory_cache_ttl(loaded.config.analysis_memory_ttl())
        .sensitive_patterns(load_user_sensitive_patterns(&loaded.config_dir));

    if allowlist.is_empty() {
        builder = builder
            .allow_local_sources(true)
            .block_private_registries(true);
    } else {
        builder = builder.registry_allowlist(allowlist);
    }
    let max_concurrent = loaded
        .config
        .max_concurrent_analyses()
        .unwrap_or(DEFAULT_WEB_MAX_CONCURRENT_ANALYSES);
    builder = builder.max_concurrent_analyses(max_concurrent);
    if let Some(concurrency) = loaded.config.layer_concurrency() {
        builder = builder.layer_concurrency(concurrency);
    }
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::ConfigError;

    fn parse(yaml: &str) -> Result<DivingConfig, ConfigError> {
        Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()?
            .try_deserialize()
    }

    #[test]
    fn custom_config_file_prefers_flag_over_env() {
        let flag = || Some(PathBuf::from("ci/diving.yml"));
        let env_val = || Some("/etc/diving.yml".to_string());
        assert_eq!(custom_config_file(flag(), env_val()), flag());
        assert_eq!(
            custom_config_file(None, env_val()),
            Some(PathBuf::from("/etc/diving.yml"))
        );
        assert_eq!(custom_config_file(None, None), None);
        assert_eq!(custom_config_file(None, Some("  ".to_string())), None);
    }

    #[test]
    fn fail_on_severity_accepts_known_levels_only() {
        assert_eq!(parse("layer_ttl: 30d").unwrap().fail_on_severity, None);
        assert_eq!(
            parse("fail_on_severity: high").unwrap().fail_on_severity,
            Some(SeverityThreshold::High)
        );
        assert_eq!(
            parse("fail_on_severity: info").unwrap().fail_on_severity,
            Some(SeverityThreshold::Info)
        );
        assert_eq!(
            parse("ignore_recommendations: [runasroot, setuid]")
                .unwrap()
                .ignore_recommendations,
            Some(vec!["runasroot".to_string(), "setuid".to_string()])
        );
        assert_eq!(
            parse("max_concurrent_analyses: 2")
                .unwrap()
                .max_concurrent_analyses,
            Some(2)
        );
        let err = parse("fail_on_severity: hgih").unwrap_err().to_string();
        assert!(err.contains("hgih"), "{err}");
    }

    #[test]
    fn webhook_and_notify_fields_deserialize() {
        let yaml = r#"
public_url: https://diving.example.com
webhook_token: topsecret
webhook_allow_public: true
wecom_webhook: wecom-key
feishu_webhook: https://open.feishu.cn/open-apis/bot/v2/hook/abc
feishu_secret: fs-sec
dingtalk_webhook: dt-token
dingtalk_secret: SEC123
slack_webhook: https://hooks.slack.com/services/T/B/X
notify_webhook: https://hooks.example.com/diving
notify_webhook_secret: hook-sec
"#;
        let cfg = parse(yaml).unwrap();
        assert_eq!(
            cfg.public_url.as_deref(),
            Some("https://diving.example.com")
        );
        assert_eq!(cfg.webhook_token.as_deref(), Some("topsecret"));
        assert_eq!(cfg.webhook_allow_public, Some(true));
        assert_eq!(cfg.wecom_webhook.as_deref(), Some("wecom-key"));
        assert_eq!(
            cfg.feishu_webhook.as_deref(),
            Some("https://open.feishu.cn/open-apis/bot/v2/hook/abc")
        );
        assert_eq!(cfg.feishu_secret.as_deref(), Some("fs-sec"));
        assert_eq!(cfg.dingtalk_webhook.as_deref(), Some("dt-token"));
        assert_eq!(cfg.dingtalk_secret.as_deref(), Some("SEC123"));
        assert_eq!(
            cfg.slack_webhook.as_deref(),
            Some("https://hooks.slack.com/services/T/B/X")
        );
        assert_eq!(
            cfg.notify_webhook.as_deref(),
            Some("https://hooks.example.com/diving")
        );
        assert_eq!(cfg.notify_webhook_secret.as_deref(), Some("hook-sec"));
    }

    #[test]
    fn missing_explicit_config_returns_err_instead_of_panic() {
        let err = load_config(Some("/nonexistent/diving-config-12345.yml")).unwrap_err();
        assert!(err.contains("config file not found"), "{err}");
    }

    #[test]
    fn build_desktop_analyzer_succeeds_with_temp_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = LoadedConfig {
            config: DivingConfig::default(),
            config_dir: dir.path().to_path_buf(),
            layer_dir: dir.path().join("layers"),
            analysis_dir: dir.path().join("analysis"),
        };
        let client = build_http_client().unwrap();
        let analyzer = build_desktop_analyzer(&loaded, client);
        if let Err(err) = analyzer {
            panic!("desktop analyzer build failed: {err}");
        }
    }

    #[test]
    fn tilde_cache_paths_expand_and_external_config_skips_home_diving_dir() {
        use tempfile::tempdir;

        let fake_home = tempdir().unwrap();
        let ext_dir = tempdir().unwrap();

        let tilde_cfg = ext_dir.path().join("tilde.yml");
        fs::write(
            &tilde_cfg,
            "layer_path: ~/my-layers\nanalysis_path: ~/my-analysis\n",
        )
        .unwrap();
        let loaded = load_config_with_home(
            Some(tilde_cfg.to_str().unwrap()),
            None,
            Some(fake_home.path().to_path_buf()),
        )
        .unwrap();
        assert_eq!(loaded.layer_dir, fake_home.path().join("my-layers"));
        assert_eq!(loaded.analysis_dir, fake_home.path().join("my-analysis"));
        assert!(
            !fake_home.path().join(".diving").exists(),
            "custom config with non-default paths must not create ~/.diving"
        );

        let ext_layers = ext_dir.path().join("layers");
        let ext_analysis = ext_dir.path().join("analysis");
        let ext_cfg = ext_dir.path().join("external.yml");
        fs::write(
            &ext_cfg,
            format!(
                "layer_path: {}\nanalysis_path: {}\n",
                ext_layers.display(),
                ext_analysis.display()
            ),
        )
        .unwrap();
        let loaded_ext = load_config_with_home(
            Some(ext_cfg.to_str().unwrap()),
            None,
            Some(fake_home.path().to_path_buf()),
        )
        .unwrap();
        assert_eq!(loaded_ext.layer_dir, ext_layers);
        assert_eq!(loaded_ext.analysis_dir, ext_analysis);
        assert!(!fake_home.path().join(".diving").exists());

        // When default config (~/.diving/config.yml) is used, ~/.diving IS created.
        let loaded_default =
            load_config_with_home(None, None, Some(fake_home.path().to_path_buf())).unwrap();
        assert_eq!(loaded_default.config_dir, fake_home.path().join(".diving"));
        assert!(fake_home.path().join(".diving").exists());
    }

    #[test]
    fn save_and_load_user_registries_preserves_harbor_dollar_headers_and_0600_mode() {
        let dir = tempfile::tempdir().unwrap();
        let rule = RegistryRule {
            host: "https://Harbor.Corp.Internal/v2/".to_owned(),
            username: Some("robot$prod+ci".to_owned()),
            password: Some("${REG_PASS:-p@ss\"quote}".to_owned()),
            token: None,
            headers: BTreeMap::from([("X-Gateway-Token".to_owned(), "tok:123".to_owned())]),
            insecure_http: true,
            insecure_skip_tls_verify: true,
            ca_cert: Some("certs/root.pem".to_owned()),
        };

        save_user_registries(dir.path(), &[rule]).unwrap();

        #[cfg(unix)]
        {
            let mode = fs::metadata(registries_file_path(dir.path()))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }

        let loaded = load_user_registries(dir.path()).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].host, "harbor.corp.internal");
        assert_eq!(loaded[0].username.as_deref(), Some("robot$prod+ci"));
        assert_eq!(
            loaded[0].password.as_deref(),
            Some("${REG_PASS:-p@ss\"quote}")
        );
        assert_eq!(
            loaded[0].headers.get("X-Gateway-Token").map(String::as_str),
            Some("tok:123")
        );
        assert!(loaded[0].insecure_http);
        assert!(loaded[0].insecure_skip_tls_verify);
        assert_eq!(loaded[0].ca_cert.as_deref(), Some("certs/root.pem"));
    }

    #[test]
    fn load_config_in_dir_merges_overrides_and_expands_env_and_relative_ca_cert() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("config.yml"),
            r#"registries:
  - host: harbor.corp.internal
    username: robot$from-config
    token: old-config-token
  - host: "*.corp.internal"
    username: robot$wildcard
    headers:
      X-Org: "${NONEXISTENT_DIVING_TEST_VAR:-default-org}"
"#,
        )
        .unwrap();

        let override_harbor = RegistryRule {
            host: "harbor.corp.internal".to_owned(),
            username: Some("robot$prod+ci".to_owned()),
            password: None,
            token: Some("override-token".to_owned()),
            headers: BTreeMap::from([("X-Custom".to_owned(), "from-ui".to_owned())]),
            insecure_http: false,
            insecure_skip_tls_verify: false,
            ca_cert: Some("certs/ca.pem".to_owned()),
        };
        let added_ip = RegistryRule {
            host: "10.0.0.5:5000".to_owned(),
            username: None,
            password: None,
            token: None,
            headers: BTreeMap::new(),
            insecure_http: true,
            insecure_skip_tls_verify: false,
            ca_cert: None,
        };
        save_user_registries(dir.path(), &[override_harbor, added_ip]).unwrap();

        let loaded = load_config_in_dir(dir.path()).unwrap();
        let runtime = resolve_runtime_registries(&loaded).unwrap();
        assert_eq!(runtime.len(), 3);

        assert_eq!(runtime[0].host, "harbor.corp.internal");
        assert_eq!(runtime[0].username.as_deref(), Some("robot$prod+ci"));
        assert_eq!(runtime[0].token.as_deref(), Some("override-token"));
        assert_eq!(
            runtime[0].headers.get("X-Custom").map(String::as_str),
            Some("from-ui")
        );
        let expected_ca = dir
            .path()
            .join("certs/ca.pem")
            .to_string_lossy()
            .into_owned();
        assert_eq!(runtime[0].ca_cert.as_deref(), Some(expected_ca.as_str()));

        assert_eq!(runtime[1].host, "*.corp.internal");
        assert_eq!(runtime[1].username.as_deref(), Some("robot$wildcard"));
        assert_eq!(
            runtime[1].headers.get("X-Org").map(String::as_str),
            Some("default-org")
        );

        assert_eq!(runtime[2].host, "10.0.0.5:5000");
        assert!(runtime[2].insecure_http);
    }
}
