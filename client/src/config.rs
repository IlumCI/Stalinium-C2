use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub server_address: String,
    pub encryption_key: String,
}

impl Config {
    pub fn load() -> Self {
        let config_str = std::fs::read_to_string("config.toml").expect("Failed to read config file");
        toml::from_str(&config_str).expect("Failed to parse config file")
    }
}
