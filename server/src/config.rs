use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub settings: Settings,
    pub backup_servers: BackupServers,
}

#[derive(Deserialize)]
pub struct Settings {
    pub tcp_address: String,
    pub admin_address: String,
    pub encryption_key: String,
}

#[derive(Deserialize)]
pub struct BackupServers {
    pub primary: String,
    pub secondary: String,
}

impl Config {
    pub fn load() -> Self {
        let config_str = std::fs::read_to_string("config.toml")
            .expect("Failed to read config.toml");
        let config: Config = toml::from_str(&config_str)
            .expect("Failed to parse config.toml");
        assert_eq!(
            config.settings.encryption_key.len(),
            32,
            "encryption_key must be exactly 32 bytes (AES-256)"
        );
        config
    }
}
