use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct ConfigFile {
    settings: Config,
}

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub server_address: String,
    pub encryption_key: String,
}

impl Config {
    pub fn load() -> Self {
        let config_str = std::fs::read_to_string("config.toml").expect("Failed to read config file");
        let config_file: ConfigFile = toml::from_str(&config_str).expect("Failed to parse config file");
        config_file.settings
    }
}
