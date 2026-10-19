use std::collections::VecDeque;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::RwLock;
use serde::{Serialize, Deserialize};

use super::peer::PeerManager;

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
    CapabilityAnnounce { node_id: String, origin: SocketAddr, capability_json: Vec<u8> },
    ShardDistribute { job_id: String, origin: SocketAddr, ttl: u8, payload: Vec<u8> },
    ActivationForward { job_id: String, request_id: String, stage: u32, data: Vec<u8> },
    GradientSubmit { job_id: String, round: u32, node_id: String, payload: Vec<u8> },
    AggregatedUpdate { job_id: String, round: u32, payload: Vec<u8> },
}

pub struct GossipEngine {
    peer_mgr: Arc<PeerManager>,
    socket: Arc<UdpSocket>,
    seen: Arc<RwLock<(VecDeque<String>, HashSet<String>)>>,
    pub command_rx: Arc<RwLock<Vec<(String, Vec<u8>)>>>,
    pub result_rx: Arc<RwLock<Vec<(String, Vec<u8>)>>>,
}

impl GossipEngine {
    pub async fn new(
        bind_addr: SocketAddr,
        peer_mgr: Arc<PeerManager>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let socket = UdpSocket::bind(bind_addr).await?;
        Ok(Self {
            peer_mgr,
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
        let targets = self.peer_mgr.get_random_subset(fanout).await;
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
            origin: self.peer_mgr.self_addr,
            ttl: 5,
            payload,
        };
        self.broadcast(&msg, FANOUT).await;
    }

    pub async fn relay_result(&self, id: String, payload: Vec<u8>) {
        let msg = GossipMessage::ResultRelay {
            id,
            origin: self.peer_mgr.self_addr,
            ttl: 5,
            payload,
        };
        self.broadcast(&msg, FANOUT).await;
    }

    async fn handle_message(&self, msg: GossipMessage, from: SocketAddr) {
        match msg {
            GossipMessage::Ping { sender } => {
                self.peer_mgr.add_peer(sender, vec![]).await;
                let peers = self.peer_mgr.get_random_subset(10).await;
                let pong = GossipMessage::Pong {
                    sender: self.peer_mgr.self_addr,
                    peers,
                };
                if let Ok(data) = serde_json::to_vec(&pong) {
                    let _ = self.socket.send_to(&data, from).await;
                }
            }
            GossipMessage::Pong { sender, peers } => {
                self.peer_mgr.add_peer(sender, vec![]).await;
                for peer_addr in peers {
                    self.peer_mgr.add_peer(peer_addr, vec![]).await;
                }
            }
            GossipMessage::PeerAnnounce { addr, capabilities } => {
                self.peer_mgr.add_peer(addr, capabilities).await;
            }
            GossipMessage::PeerLeave { addr } => {
                self.peer_mgr.remove_peer(&addr).await;
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
            GossipMessage::CapabilityAnnounce { node_id, origin, capability_json } => {
                self.peer_mgr.add_peer(origin, vec!["compute".to_string()]).await;
                self.command_rx.write().await.push((
                    format!("cap:{}", node_id),
                    capability_json,
                ));
            }
            GossipMessage::ShardDistribute { job_id, origin, ttl, payload } => {
                let id = format!("shard:{}", job_id);
                if self.mark_seen(&id).await {
                    self.command_rx.write().await.push((id.clone(), payload.clone()));
                    if ttl > 0 {
                        let relay = GossipMessage::ShardDistribute {
                            job_id, origin, ttl: ttl - 1, payload,
                        };
                        self.broadcast(&relay, FANOUT).await;
                    }
                }
            }
            GossipMessage::ActivationForward { job_id, request_id, stage: _, data } => {
                self.command_rx.write().await.push((
                    format!("act:{}:{}", job_id, request_id),
                    data,
                ));
            }
            GossipMessage::GradientSubmit { job_id, round, node_id, payload } => {
                self.command_rx.write().await.push((
                    format!("grad:{}:{}:{}", job_id, round, node_id),
                    payload,
                ));
            }
            GossipMessage::AggregatedUpdate { job_id, round, payload } => {
                let id = format!("agg:{}:{}", job_id, round);
                if self.mark_seen(&id).await {
                    self.command_rx.write().await.push((id, payload.clone()));
                    let relay = GossipMessage::AggregatedUpdate {
                        job_id, round, payload,
                    };
                    self.broadcast(&relay, FANOUT).await;
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

            let targets = self.peer_mgr.get_random_subset(FANOUT).await;
            if targets.is_empty() {
                for addr in &self.peer_mgr.bootstrap_addrs {
                    let ping = GossipMessage::Ping {
                        sender: self.peer_mgr.self_addr,
                    };
                    if let Ok(data) = serde_json::to_vec(&ping) {
                        let _ = self.socket.send_to(&data, addr).await;
                    }
                }
            } else {
                for addr in targets {
                    let ping = GossipMessage::Ping {
                        sender: self.peer_mgr.self_addr,
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
            self.peer_mgr.evict_stale(Duration::from_secs(300)).await;
        }
    }
}
