use bytesize::ByteSize;
use config::{Config, File};
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
}

pub fn must_load_config() -> &'static DivingConfig {
    static DIVING_CONFIG: OnceCell<DivingConfig> = OnceCell::new();
    DIVING_CONFIG.get_or_init(|| {
        let config_file = get_config_path().join("config.yml");
        if !config_file.exists() {
            fs::File::create(config_file.clone()).unwrap();
        }
        Config::builder()
            .add_source(File::from(config_file))
            .build()
            .unwrap()
            .try_deserialize::<DivingConfig>()
            .unwrap()
    })
}

// 获取或初始化配置目录
pub fn get_config_path() -> &'static PathBuf {
    static CONFIG_PATH: OnceCell<PathBuf> = OnceCell::new();
    CONFIG_PATH.get_or_init(|| {
        let dir = home_dir().unwrap();
        let config_path = dir.join(".diving");
        fs::create_dir_all(config_path.clone()).unwrap();
        config_path
    })
}

// 获取或初始化layer目录
pub fn get_layer_path() -> &'static PathBuf {
    // 读取配置，若未配置则使用默认
    static LAYER_PATH: OnceCell<PathBuf> = OnceCell::new();
    LAYER_PATH.get_or_init(|| {
        let config_path = get_config_path();
        let config = must_load_config();
        let file = config
            .layer_path
            .clone()
            .unwrap_or_else(|| "layers".to_string());
        let layer_path = config_path.join(file);
        fs::create_dir_all(layer_path.clone()).unwrap();
        layer_path
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
