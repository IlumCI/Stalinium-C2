use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub settings: Settings,
    pub backup_servers: BackupServers,
    #[serde(default)]
    pub p2p: P2pConfig,
    #[serde(default)]
    pub agentic: AgenticConfig,
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

#[derive(Deserialize)]
pub struct P2pConfig {
    pub enabled: bool,
    pub gossip_address: String,
    pub max_peers: usize,
    pub bootstrap_peers: Vec<String>,
}

impl Default for P2pConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            gossip_address: "0.0.0.0:9090".to_string(),
            max_peers: 50,
            bootstrap_peers: vec![],
        }
    }
}

#[derive(Deserialize)]
pub struct AgenticConfig {
    pub enabled: bool,
    pub listen_address: String,
    pub api_keys: Vec<String>,
}

impl Default for AgenticConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            listen_address: "0.0.0.0:443".to_string(),
            api_keys: vec![],
        }
    }
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
