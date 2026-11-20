use bytesize::ByteSize;
use config::{Config, File, FileFormat};
use diving_core::{Analyzer, CacheConfig, GateConfig, SensitivePatterns, SeverityThreshold};
use home::home_dir;
use humantime::Duration as HumanDuration;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{env, fs};
use tracing::warn;

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

fn ensure_config_dir() -> Result<PathBuf, String> {
    let dir = home_dir().ok_or_else(|| "failed to determine home directory".to_string())?;
    let config_dir = dir.join(".diving");
    fs::create_dir_all(&config_dir).map_err(|e| {
        format!(
            "failed to create {} directory: check permissions ({e})",
            config_dir.display()
        )
    })?;
    Ok(config_dir)
}

/// Load `config.yml` (`explicit_file` > `$DIVING_CONFIG` > `~/.diving/config.yml`).
pub fn load_config(explicit_file: Option<&str>) -> Result<LoadedConfig, String> {
    let config_dir = ensure_config_dir()?;
    let custom = custom_config_file(
        explicit_file.map(PathBuf::from),
        env::var("DIVING_CONFIG").ok(),
    );
    let config_file = match custom {
        Some(path) => {
            if !path.is_file() {
                return Err(format!("config file not found: {}", path.display()));
            }
            path
        }
        None => {
            let path = config_dir.join("config.yml");
            if !path.exists() {
                fs::File::create(&path).map_err(|e| {
                    format!(
                        "failed to create {}: check directory permissions ({e})",
                        path.display()
                    )
                })?;
            }
            path
        }
    };

    let config = Config::builder()
        .add_source(File::from(config_file.as_path()).format(FileFormat::Yaml))
        .build()
        .map_err(|e| format!("failed to read {}: {e}", config_file.display()))?
        .try_deserialize::<DivingConfig>()
        .map_err(|e| format!("{} contains invalid fields: {e}", config_file.display()))?;

    let layer_dir = config_dir.join(config.layer_path.as_deref().unwrap_or("layers"));
    let analysis_dir = config_dir.join(config.analysis_path.as_deref().unwrap_or("analysis"));

    Ok(LoadedConfig {
        config,
        config_dir,
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

/// Build the shared `reqwest::Client` used by both `Analyzer` and AI/notifications.
pub fn build_http_client() -> Result<Client, String> {
    install_crypto_provider();
    Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .pool_max_idle_per_host(32)
        .build()
        .map_err(|e| format!("failed to build shared reqwest client: {e}"))
}

fn cache_config_from(loaded: &LoadedConfig) -> CacheConfig {
    CacheConfig::persistent(&loaded.layer_dir, &loaded.analysis_dir)
        .layer_ttl(loaded.config.layer_ttl())
        .max_layer_cache_size(loaded.config.max_layer_cache_size())
}

/// Construct an [`Analyzer`] configured for CLI/TUI/CI runs.
pub fn build_cli_analyzer(
    loaded: &LoadedConfig,
    http_client: Client,
) -> diving_core::Result<Analyzer> {
    let mut builder = Analyzer::builder()
        .cache(cache_config_from(loaded))
        .http_client(http_client)
        .allow_local_sources(true)
        .use_docker_config(true)
        .singleflight(false)
        .memory_cache_ttl(Duration::ZERO)
        .sensitive_patterns(load_user_sensitive_patterns(&loaded.config_dir));
    if let Some(concurrency) = loaded.config.layer_concurrency() {
        builder = builder.layer_concurrency(concurrency);
    }
    builder.build()
}

/// Construct an [`Analyzer`] configured for Web/MCP/Webhook runs.
pub fn build_web_analyzer(
    loaded: &LoadedConfig,
    http_client: Client,
) -> diving_core::Result<Analyzer> {
    let allowlist = loaded.config.registry_allowlist();
    let mut builder = Analyzer::builder()
        .cache(cache_config_from(loaded))
        .http_client(http_client)
        .use_docker_config(true)
        .singleflight(true)
        .memory_cache_ttl(loaded.config.analysis_memory_ttl())
        .sensitive_patterns(load_user_sensitive_patterns(&loaded.config_dir));

    if allowlist.is_empty() {
        builder = builder.allow_local_sources(true);
    } else {
        builder = builder.registry_allowlist(allowlist);
    }
    if let Some(max) = loaded.config.max_concurrent_analyses() {
        builder = builder.max_concurrent_analyses(max);
    }
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
}
