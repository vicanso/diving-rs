use crate::recommend::{SEVERITY_HIGH, SEVERITY_INFO, SEVERITY_LOW, SEVERITY_MEDIUM};
use bytesize::ByteSize;
use config::{Config, File, FileFormat};
use glob::Pattern;
use home::home_dir;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::{env, fs, path::PathBuf};

/// `fail_on_severity` 的取值。用枚举而不是字符串：写错的值在启动时就让
/// 配置反序列化失败，而不是悄悄把卡口关掉。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SeverityThreshold {
    High,
    Medium,
    Low,
    Info,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DivingConfig {
    pub layer_path: Option<String>,
    pub layer_ttl: Option<String>,
    // Analysis-result cache directory (default: ~/.diving/analysis). Shares
    // `layer_ttl` for cleanup.
    pub analysis_path: Option<String>,
    /// Legacy single knob: when set (and the specific keys below are not),
    /// used for both the Tokio worker pool and per-image layer concurrency.
    pub threads: Option<usize>,
    /// Tokio multi-thread runtime worker count. Defaults to `threads` or
    /// `num_cpus`. Prefer this over `threads` for web servers that want more
    /// async capacity without oversubscribing CPU-bound layer work.
    pub worker_threads: Option<usize>,
    /// Max concurrent layer download/decompress tasks per image analysis.
    /// Defaults to `threads` or `min(layer_count, 2 × CPUs)`.
    pub layer_concurrency: Option<usize>,
    pub lowest_efficiency: Option<f64>,
    pub highest_wasted_bytes: Option<ByteSize>,
    pub highest_user_wasted_percent: Option<f64>,
    /// CI 严重度卡口：存在该级别及以上的优化建议时 CI 失败（如 `high`
    /// 拦截镜像内的密钥文件）。未配置（默认）则建议只打印、不影响结果。
    pub fail_on_severity: Option<SeverityThreshold>,
    // Interval between layer cache cleanup runs, in hours (default: 1)
    pub cleanup_interval_hours: Option<u64>,
    /// Web 模式 `/api/analyze` 的 registry 白名单（host 形式，如
    /// `index.docker.io`、`ghcr.io`、`registry.example.com:5000`）。
    /// 非空时仅放行列表内的 registry，`file://` / `docker://` 需显式加入
    /// `local-file` / `local-docker`；为空（默认）不限制。只约束 web
    /// API，命令行不受影响。
    pub registry_allowlist: Option<Vec<String>>,
    /// `/api/file` 单文件下载大小上限（默认 100MB）。层里的超大文件全量
    /// 读进内存再响应，会把 web 服务内存打爆。
    pub max_download_file_size: Option<ByteSize>,
    /// layer blob 缓存目录的总大小上限；超出时按访问时间从旧到新淘汰。
    /// 未配置（默认）则只按 TTL 清理。
    pub max_layer_cache_size: Option<ByteSize>,
    /// Web 模式 `/mcp` 额外放行的 `Host` 头（如 `diving.example.com`、
    /// `10.0.0.5:7001`）。默认只接受 loopback，防 DNS rebinding；配置
    /// `*` 关闭校验。设置了 `--mcp-token` 时不做 Host 校验（token 已足够）。
    pub mcp_allowed_hosts: Option<Vec<String>>,
}

/// `--config` 指定的配置文件。未设置时依次回退到 `$DIVING_CONFIG`、
/// `~/.diving/config.yml`。
static CONFIG_FILE: OnceCell<PathBuf> = OnceCell::new();

/// 指定配置文件路径（`--config`）。必须在第一次读取配置之前调用，
/// 之后的调用不生效。只改变 `config.yml` 的位置，`sensitive-files`、
/// `ai_history/` 与默认缓存目录仍在 `~/.diving/` 下。
pub fn set_config_file(path: &str) {
    let _ = CONFIG_FILE.set(PathBuf::from(path));
}

/// 用户指定的配置文件：`--config` 优先于 `$DIVING_CONFIG`；都没有则为
/// `None`（使用默认路径）。
fn custom_config_file(flag: Option<PathBuf>, env: Option<String>) -> Option<PathBuf> {
    flag.or_else(|| env.filter(|v| !v.trim().is_empty()).map(PathBuf::from))
}

pub fn must_load_config() -> &'static DivingConfig {
    static DIVING_CONFIG: OnceCell<DivingConfig> = OnceCell::new();
    DIVING_CONFIG.get_or_init(|| {
        let custom = custom_config_file(CONFIG_FILE.get().cloned(), env::var("DIVING_CONFIG").ok());
        let config_file = match custom {
            // 指定的文件必须存在：路径写错时若悄悄回退到默认值，CI 卡口
            // 会被无声地放宽。
            Some(path) => {
                if !path.is_file() {
                    panic!("config file not found: {}", path.display());
                }
                path
            }
            None => {
                let path = get_config_path().join("config.yml");
                if !path.exists() {
                    fs::File::create(&path).expect(
                        "failed to create ~/.diving/config.yml: check directory permissions",
                    );
                }
                path
            }
        };
        // 显式指定 YAML：自定义路径不一定带 .yml 扩展名。
        Config::builder()
            .add_source(File::from(config_file.as_path()).format(FileFormat::Yaml))
            .build()
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", config_file.display()))
            .try_deserialize::<DivingConfig>()
            .unwrap_or_else(|e| panic!("{} contains invalid fields: {e}", config_file.display()))
    })
}

// 获取或初始化配置目录
pub fn get_config_path() -> &'static PathBuf {
    static CONFIG_PATH: OnceCell<PathBuf> = OnceCell::new();
    CONFIG_PATH.get_or_init(|| {
        let dir = home_dir().expect("failed to determine home directory");
        let config_path = dir.join(".diving");
        fs::create_dir_all(&config_path)
            .expect("failed to create ~/.diving directory: check permissions");
        config_path
    })
}

// 获取或初始化layer目录
pub fn get_layer_path() -> &'static PathBuf {
    static LAYER_PATH: OnceCell<PathBuf> = OnceCell::new();
    LAYER_PATH.get_or_init(|| {
        let config_path = get_config_path();
        let config = must_load_config();
        let file = config
            .layer_path
            .clone()
            .unwrap_or_else(|| "layers".to_string());
        let layer_path = config_path.join(file);
        fs::create_dir_all(&layer_path)
            .expect("failed to create layer cache directory: check permissions");
        layer_path
    })
}

// 获取或初始化分析结果缓存目录
pub fn get_analysis_path() -> &'static PathBuf {
    static ANALYSIS_PATH: OnceCell<PathBuf> = OnceCell::new();
    ANALYSIS_PATH.get_or_init(|| {
        let config_path = get_config_path();
        let config = must_load_config();
        let file = config
            .analysis_path
            .clone()
            .unwrap_or_else(|| "analysis".to_string());
        let path = config_path.join(file);
        fs::create_dir_all(&path)
            .expect("failed to create analysis cache directory: check permissions");
        path
    })
}

const GLOB_OPTS: glob::MatchOptions = glob::MatchOptions {
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

pub struct UserSensitivePattern {
    pub pattern: Pattern,
    pub reason: String,
}

pub struct UserSensitiveConfig {
    /// Extra patterns to flag as sensitive (in addition to built-in rules).
    pub patterns: Vec<UserSensitivePattern>,
    /// Patterns that override / silence any match (built-in or custom).
    /// Entries come from lines prefixed with `!` in the config file.
    pub ignores: Vec<Pattern>,
}

impl UserSensitiveConfig {
    /// Return the reason string if `path` is considered sensitive, or `None`
    /// if it should be ignored or does not match any custom pattern.
    pub fn check(&self, path: &str) -> Option<&str> {
        for p in &self.patterns {
            if glob_matches(&p.pattern, path) {
                // Check ignore list before reporting
                if self.ignores.iter().any(|ig| glob_matches(ig, path)) {
                    return None;
                }
                return Some(&p.reason);
            }
        }
        None
    }

    /// Return true if `path` is explicitly ignored by the user config.
    pub fn is_ignored(&self, path: &str) -> bool {
        self.ignores.iter().any(|ig| glob_matches(ig, path))
    }
}

/// Load user-defined sensitive file patterns from `~/.diving/sensitive-files`.
///
/// File format (one entry per line):
///   <glob-pattern>              — add as a sensitive pattern
///   <glob-pattern> | <reason>  — add with a custom reason label
///   !<glob-pattern>             — ignore / suppress this path (built-in or custom)
/// Lines starting with `#` and blank lines are skipped.
pub fn load_user_sensitive_patterns() -> &'static UserSensitiveConfig {
    static CFG: OnceCell<UserSensitiveConfig> = OnceCell::new();
    CFG.get_or_init(|| {
        let path = get_config_path().join("sensitive-files");
        let mut patterns = vec![];
        let mut ignores = vec![];
        if let Ok(content) = fs::read_to_string(&path) {
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
                        patterns.push(UserSensitivePattern {
                            pattern: pat,
                            reason,
                        });
                    }
                }
            }
        }
        UserSensitiveConfig { patterns, ignores }
    })
}

pub fn get_lowest_efficiency() -> f64 {
    let config = must_load_config();
    if let Some(lowest_efficiency) = config.lowest_efficiency {
        return lowest_efficiency;
    }
    0.95
}

pub fn get_highest_wasted_bytes() -> u64 {
    let config = must_load_config();
    if let Some(highest_wasted_bytes) = config.highest_wasted_bytes {
        return highest_wasted_bytes.0;
    }
    20 * 1024 * 1024
}

pub fn get_highest_user_wasted_percent() -> f64 {
    let config = must_load_config();
    if let Some(highest_user_wasted_percent) = config.highest_user_wasted_percent {
        return highest_user_wasted_percent;
    }
    // Matches README default (10%).
    0.1
}

/// CI 严重度卡口的阈值（`recommend::SEVERITY_*` 之一）；`None` 表示不检查。
pub fn get_fail_on_severity() -> Option<&'static str> {
    must_load_config()
        .fail_on_severity
        .map(|threshold| match threshold {
            SeverityThreshold::High => SEVERITY_HIGH,
            SeverityThreshold::Medium => SEVERITY_MEDIUM,
            SeverityThreshold::Low => SEVERITY_LOW,
            SeverityThreshold::Info => SEVERITY_INFO,
        })
}

/// Web 模式的 registry 白名单，条目已归一化为小写、去掉空白。
/// 空切片表示不限制。
pub fn get_registry_allowlist() -> &'static [String] {
    static LIST: OnceCell<Vec<String>> = OnceCell::new();
    LIST.get_or_init(|| {
        must_load_config()
            .registry_allowlist
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    })
}

/// `/api/file` 单文件下载大小上限（字节）。
pub fn get_max_download_file_size() -> u64 {
    must_load_config()
        .max_download_file_size
        .map(|v| v.0)
        .unwrap_or(100 * 1024 * 1024)
}

/// `/mcp` 额外放行的 `Host` 列表，条目已归一化为小写、去掉空白。
pub fn get_mcp_allowed_hosts() -> &'static [String] {
    static LIST: OnceCell<Vec<String>> = OnceCell::new();
    LIST.get_or_init(|| {
        must_load_config()
            .mcp_allowed_hosts
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect()
    })
}

/// layer 缓存目录总大小上限（字节）；`None` 表示不设上限。
pub fn get_max_layer_cache_size() -> Option<u64> {
    must_load_config().max_layer_cache_size.map(|v| v.0)
}

/// Tokio runtime worker threads.
///
/// Resolution: `worker_threads` → legacy `threads` → `num_cpus`.
pub fn get_worker_threads() -> usize {
    let config = must_load_config();
    config
        .worker_threads
        .or(config.threads)
        .unwrap_or_else(num_cpus::get)
        .max(1)
}

/// Per-image layer download/decompress concurrency cap.
///
/// Resolution: `layer_concurrency` → legacy `threads` →
/// `min(layer_count, 2 × CPUs)`, always at least 1.
pub fn get_layer_concurrency(layer_count: usize) -> usize {
    let config = must_load_config();
    config
        .layer_concurrency
        .or(config.threads)
        .unwrap_or_else(|| layer_count.min(num_cpus::get() * 2))
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Result<DivingConfig, config::ConfigError> {
        Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()?
            .try_deserialize()
    }

    #[test]
    fn custom_config_file_prefers_flag_over_env() {
        let flag = || Some(PathBuf::from("ci/diving.yml"));
        let env = || Some("/etc/diving.yml".to_string());
        assert_eq!(custom_config_file(flag(), env()), flag());
        assert_eq!(
            custom_config_file(None, env()),
            Some(PathBuf::from("/etc/diving.yml"))
        );
        // Unset / blank env falls through to the default path.
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
        // A typo must fail loudly rather than silently disable the gate.
        let err = parse("fail_on_severity: hgih").unwrap_err().to_string();
        assert!(err.contains("hgih"), "{err}");
    }
}
