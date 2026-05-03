use bytesize::ByteSize;
use config::{Config, File};
use glob::Pattern;
use home::home_dir;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DivingConfig {
    pub layer_path: Option<String>,
    pub layer_ttl: Option<String>,
    pub threads: Option<usize>,
    pub lowest_efficiency: Option<f64>,
    pub highest_wasted_bytes: Option<ByteSize>,
    pub highest_user_wasted_percent: Option<f64>,
    // Interval between layer cache cleanup runs, in hours (default: 1)
    pub cleanup_interval_hours: Option<u64>,
}

pub fn must_load_config() -> &'static DivingConfig {
    static DIVING_CONFIG: OnceCell<DivingConfig> = OnceCell::new();
    DIVING_CONFIG.get_or_init(|| {
        let config_file = get_config_path().join("config.yml");
        if !config_file.exists() {
            fs::File::create(&config_file)
                .expect("failed to create ~/.diving/config.yml: check directory permissions");
        }
        Config::builder()
            .add_source(File::from(config_file))
            .build()
            .expect("failed to build config")
            .try_deserialize::<DivingConfig>()
            .expect("config.yml contains invalid fields: check ~/.diving/config.yml")
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
    0.2
}
