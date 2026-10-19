use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
struct ConfigFile {
    settings: Config,
    #[serde(default)]
    p2p: P2pConfig,
    #[serde(default)]
    agentic: AgenticConfig,
}

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub server_address: String,
    pub encryption_key: String,
}

#[derive(Serialize, Deserialize)]
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
            gossip_address: "0.0.0.0:0".to_string(),
            max_peers: 50,
            bootstrap_peers: vec![],
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct AgenticConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub api_key: String,
}

impl Default for AgenticConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: String::new(),
            api_key: String::new(),
        }
    }
}

pub struct FullConfig {
    pub settings: Config,
    pub p2p: P2pConfig,
    pub agentic: AgenticConfig,
}

impl FullConfig {
    pub fn load() -> Self {
        let config_str = std::fs::read_to_string("config.toml")
            .expect("Failed to read config file");
        let config_file: ConfigFile = toml::from_str(&config_str)
            .expect("Failed to parse config file");
        Self {
            settings: config_file.settings,
            p2p: config_file.p2p,
            agentic: config_file.agentic,
        }
    }
}
