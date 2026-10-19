use std::collections::VecDeque;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::RwLock;
use serde::{Serialize, Deserialize};

use super::overlay::Overlay;

const MAX_SEEN: usize = 10_000;
const GOSSIP_INTERVAL: Duration = Duration::from_secs(30);
const FANOUT: usize = 3;
const MAX_UDP_PAYLOAD: usize = 65535;

#[derive(Serialize, Deserialize, Clone)]
pub enum GossipMessage {
    Ping { sender: SocketAddr },
    Pong { sender: SocketAddr, peers: Vec<SocketAddr> },
    PeerAnnounce { addr: SocketAddr, capabilities: Vec<String> },
    PeerLeave { addr: SocketAddr },
    CommandRelay { id: String, origin: SocketAddr, ttl: u8, payload: Vec<u8> },
    ResultRelay { id: String, origin: SocketAddr, ttl: u8, payload: Vec<u8> },
}

pub struct GossipEngine {
    overlay: Arc<Overlay>,
    socket: Arc<UdpSocket>,
    seen: Arc<RwLock<(VecDeque<String>, HashSet<String>)>>,
    pub command_rx: Arc<RwLock<Vec<(String, Vec<u8>)>>>,
    pub result_rx: Arc<RwLock<Vec<(String, Vec<u8>)>>>,
}

impl GossipEngine {
    pub async fn new(
        bind_addr: SocketAddr,
        overlay: Arc<Overlay>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let socket = UdpSocket::bind(bind_addr).await?;
        Ok(Self {
            overlay,
            socket: Arc::new(socket),
            seen: Arc::new(RwLock::new((VecDeque::new(), HashSet::new()))),
            command_rx: Arc::new(RwLock::new(Vec::new())),
            result_rx: Arc::new(RwLock::new(Vec::new())),
        })
    }

    pub fn start(self: &Arc<Self>) {
        let engine = Arc::clone(self);
        tokio::spawn(async move { engine.listen().await });

        let engine = Arc::clone(self);
        tokio::spawn(async move { engine.periodic_gossip().await });

        let engine = Arc::clone(self);
        tokio::spawn(async move { engine.stale_eviction().await });
    }

    async fn mark_seen(&self, id: &str) -> bool {
        let mut guard = self.seen.write().await;
        let (queue, set) = &mut *guard;
        if set.contains(id) {
            return false;
        }
        if queue.len() >= MAX_SEEN {
            if let Some(old) = queue.pop_front() {
                set.remove(&old);
            }
        }
        set.insert(id.to_string());
        queue.push_back(id.to_string());
        true
    }

    pub async fn broadcast(&self, msg: &GossipMessage, fanout: usize) {
        let targets = self.overlay.get_random_subset(fanout).await;
        let data = match serde_json::to_vec(msg) {
            Ok(d) => d,
            Err(_) => return,
        };
        for addr in targets {
            let _ = self.socket.send_to(&data, addr).await;
        }
    }

    pub async fn relay_command(&self, id: String, payload: Vec<u8>) {
        let msg = GossipMessage::CommandRelay {
            id,
            origin: self.overlay.self_addr,
            ttl: 5,
            payload,
        };
        self.broadcast(&msg, FANOUT).await;
    }

    pub async fn relay_result(&self, id: String, payload: Vec<u8>) {
        let msg = GossipMessage::ResultRelay {
            id,
            origin: self.overlay.self_addr,
            ttl: 5,
            payload,
        };
        self.broadcast(&msg, FANOUT).await;
    }

    async fn handle_message(&self, msg: GossipMessage, from: SocketAddr) {
        match msg {
            GossipMessage::Ping { sender } => {
                self.overlay.add_peer(sender, vec![]).await;
                let peers = self.overlay.get_random_subset(10).await;
                let pong = GossipMessage::Pong {
                    sender: self.overlay.self_addr,
                    peers,
                };
                if let Ok(data) = serde_json::to_vec(&pong) {
                    let _ = self.socket.send_to(&data, from).await;
                }
            }
            GossipMessage::Pong { sender, peers } => {
                self.overlay.add_peer(sender, vec![]).await;
                for peer_addr in peers {
                    self.overlay.add_peer(peer_addr, vec![]).await;
                }
            }
            GossipMessage::PeerAnnounce { addr, capabilities } => {
                self.overlay.add_peer(addr, capabilities).await;
            }
            GossipMessage::PeerLeave { addr } => {
                self.overlay.remove_peer(&addr).await;
            }
            GossipMessage::CommandRelay { id, origin, ttl, payload } => {
                if self.mark_seen(&id).await {
                    self.command_rx.write().await.push((id.clone(), payload.clone()));
                    if ttl > 0 {
                        let relay = GossipMessage::CommandRelay {
                            id,
                            origin,
                            ttl: ttl - 1,
                            payload,
                        };
                        self.broadcast(&relay, FANOUT).await;
                    }
                }
            }
            GossipMessage::ResultRelay { id, origin, ttl, payload } => {
                if self.mark_seen(&id).await {
                    self.result_rx.write().await.push((id.clone(), payload.clone()));
                    if ttl > 0 {
                        let relay = GossipMessage::ResultRelay {
                            id,
                            origin,
                            ttl: ttl - 1,
                            payload,
                        };
                        self.broadcast(&relay, FANOUT).await;
                    }
                }
            }
        }
    }

    async fn listen(&self) {
        let mut buf = vec![0u8; MAX_UDP_PAYLOAD];
        loop {
            match self.socket.recv_from(&mut buf).await {
                Ok((n, from)) => {
                    if let Ok(msg) = serde_json::from_slice::<GossipMessage>(&buf[..n]) {
                        self.handle_message(msg, from).await;
                    }
                }
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    async fn periodic_gossip(&self) {
        loop {
            tokio::time::sleep(GOSSIP_INTERVAL).await;

            let targets = self.overlay.get_random_subset(FANOUT).await;
            if targets.is_empty() {
                for addr in &self.overlay.bootstrap_addrs {
                    let ping = GossipMessage::Ping {
                        sender: self.overlay.self_addr,
                    };
                    if let Ok(data) = serde_json::to_vec(&ping) {
                        let _ = self.socket.send_to(&data, addr).await;
                    }
                }
            } else {
                for addr in targets {
                    let ping = GossipMessage::Ping {
                        sender: self.overlay.self_addr,
                    };
                    if let Ok(data) = serde_json::to_vec(&ping) {
                        let _ = self.socket.send_to(&data, addr).await;
                    }
                }
            }
        }
    }

    async fn stale_eviction(&self) {
        loop {
            tokio::time::sleep(Duration::from_secs(120)).await;
            self.overlay.evict_stale(Duration::from_secs(300)).await;
        }
    }
}
