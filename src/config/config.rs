use home::home_dir;
use once_cell::sync::OnceCell;
use std::{fs, path::PathBuf};

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
        let layer_path = config_path.join("layers");
        fs::create_dir_all(layer_path.clone()).unwrap();
        layer_path
    })
}
