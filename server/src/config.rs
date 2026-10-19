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
            listen_address: "0.0.0.0:8443".to_string(),
            api_keys: vec![],
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let config_path = if crate::termux::is_termux() {
            let data = crate::termux::data_dir();
            let termux_path = format!("{}/config.toml", data);
            if std::path::Path::new(&termux_path).exists() {
                termux_path
            } else {
                "config.toml".to_string()
            }
        } else {
            "config.toml".to_string()
        };

        let config_str = std::fs::read_to_string(&config_path)
            .unwrap_or_else(|_| panic!("Failed to read {}", config_path));
        let config: Config = toml::from_str(&config_str)
            .unwrap_or_else(|_| panic!("Failed to parse {}", config_path));
        assert_eq!(
            config.settings.encryption_key.len(),
            32,
            "encryption_key must be exactly 32 bytes (AES-256)"
        );

        if crate::termux::is_termux() {
            crate::termux::warn_low_ports(&config.settings.tcp_address, "TCP");
            crate::termux::warn_low_ports(&config.settings.admin_address, "Admin");
            if config.agentic.enabled {
                crate::termux::warn_low_ports(&config.agentic.listen_address, "Agentic API");
            }
            if config.p2p.enabled {
                crate::termux::warn_low_ports(&config.p2p.gossip_address, "P2P gossip");
            }
        }

        config
    }
}
