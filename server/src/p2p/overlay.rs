use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use serde::{Serialize, Deserialize};
use rand::seq::SliceRandom;

#[derive(Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub addr: SocketAddr,
    pub last_seen_secs: u64,
    pub capabilities: Vec<String>,
}

pub struct PeerEntry {
    pub info: PeerInfo,
    pub last_seen: Instant,
    pub reputation: i32,
}

pub struct Overlay {
    pub peers: Arc<RwLock<HashMap<SocketAddr, PeerEntry>>>,
    pub self_addr: SocketAddr,
    pub max_peers: usize,
    pub bootstrap_addrs: Vec<SocketAddr>,
}

impl Overlay {
    pub fn new(self_addr: SocketAddr, bootstrap: Vec<SocketAddr>, max_peers: usize) -> Self {
        Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
            self_addr,
            max_peers,
            bootstrap_addrs: bootstrap,
        }
    }

    pub async fn add_peer(&self, addr: SocketAddr, capabilities: Vec<String>) {
        if addr == self.self_addr {
            return;
        }
        let mut peers = self.peers.write().await;
        if peers.len() >= self.max_peers && !peers.contains_key(&addr) {
            return;
        }
        let entry = peers.entry(addr).or_insert_with(|| PeerEntry {
            info: PeerInfo {
                addr,
                last_seen_secs: 0,
                capabilities: capabilities.clone(),
            },
            last_seen: Instant::now(),
            reputation: 0,
        });
        entry.last_seen = Instant::now();
        entry.info.last_seen_secs = 0;
        if !capabilities.is_empty() {
            entry.info.capabilities = capabilities;
        }
    }

    pub async fn remove_peer(&self, addr: &SocketAddr) {
        self.peers.write().await.remove(addr);
    }

    pub async fn get_peers(&self) -> Vec<PeerInfo> {
        let peers = self.peers.read().await;
        peers.values().map(|e| {
            let mut info = e.info.clone();
            info.last_seen_secs = e.last_seen.elapsed().as_secs();
            info
        }).collect()
    }

    pub async fn get_random_subset(&self, n: usize) -> Vec<SocketAddr> {
        let peers = self.peers.read().await;
        let addrs: Vec<SocketAddr> = peers.keys().copied().collect();
        if addrs.len() <= n {
            return addrs;
        }
        let mut rng = rand::thread_rng();
        let mut shuffled = addrs;
        shuffled.shuffle(&mut rng);
        shuffled.truncate(n);
        shuffled
    }

    pub async fn evict_stale(&self, timeout: Duration) {
        let mut peers = self.peers.write().await;
        peers.retain(|_, entry| entry.last_seen.elapsed() < timeout);
    }

    pub async fn update_reputation(&self, addr: &SocketAddr, delta: i32) {
        let mut peers = self.peers.write().await;
        if let Some(entry) = peers.get_mut(addr) {
            entry.reputation = (entry.reputation + delta).clamp(-100, 100);
            if entry.reputation <= -50 {
                peers.remove(addr);
            }
        }
    }

    pub async fn peer_count(&self) -> usize {
        self.peers.read().await.len()
    }
}
