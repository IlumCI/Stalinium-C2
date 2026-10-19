use stalinium_evolution::node::{BehaviorSignal, Migrant, SoupHandle, SoupSnapshot, SoupState, Telemetry};
use crate::behavior::SwarmParams;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

const MIGRATE_EVERY: u64 = 5;
const MIGRANTS_PER_EXCHANGE: usize = 4;

pub struct EvolutionEngine {
    soup: SoupHandle,
    node_id: Vec<u8>,
    gossip_cycles: u64,
    start_time: Instant,
    pending_immigrants: Arc<RwLock<Vec<Migrant>>>,
    latest_params: SwarmParams,
}

impl EvolutionEngine {
    pub fn new(node_id: Vec<u8>) -> Self {
        let seed = {
            let mut h = blake3::Hasher::new();
            h.update(&node_id);
            h.update(b"evolution-seed");
            let hash = h.finalize();
            u64::from_le_bytes(hash.as_bytes()[..8].try_into().unwrap())
        };

        EvolutionEngine {
            soup: SoupHandle::spawn(seed, node_id.clone()),
            node_id,
            gossip_cycles: 0,
            start_time: Instant::now(),
            pending_immigrants: Arc::new(RwLock::new(Vec::new())),
            latest_params: SwarmParams::from_behavior(&BehaviorSignal::default()),
        }
    }

    pub fn tick(&self, peer_count: u32, mean_latency_ms: u32, gossip_msg_rate: u32, bytes_in: u64, bytes_out: u64) {
        self.soup.feed_telemetry(Telemetry {
            peer_count,
            mean_latency_ms,
            gossip_msg_rate,
            bytes_in,
            bytes_out,
            uptime_secs: self.start_time.elapsed().as_secs(),
        });
    }

    pub async fn receive_migrants(&self, migrants: Vec<Migrant>) {
        self.pending_immigrants.write().await.extend(migrants);
    }

    pub async fn cycle(&mut self) -> Option<Vec<Migrant>> {
        let immigrants: Vec<Migrant> = {
            let mut pending = self.pending_immigrants.write().await;
            std::mem::take(&mut *pending)
        };
        self.soup.immigrate(immigrants);

        let snap = self.soup.latest_snapshot();
        self.latest_params = SwarmParams::from_behavior(&snap.behavior);

        self.gossip_cycles += 1;

        if self.gossip_cycles % MIGRATE_EVERY == 0 {
            let emigrants = self.soup.emigrate(MIGRANTS_PER_EXCHANGE).await;
            if !emigrants.is_empty() {
                return Some(emigrants);
            }
        }
        None
    }

    pub fn snapshot(&self) -> SoupSnapshot {
        self.soup.latest_snapshot()
    }

    pub fn swarm_params(&self) -> &SwarmParams {
        &self.latest_params
    }

    pub async fn save_state(&self) -> Result<SoupState, String> {
        self.soup.save().await
    }

    pub async fn load_state(&self, state: SoupState) -> Result<(), String> {
        self.soup.load(state).await
    }

    pub fn snapshot_watch(&self) -> tokio::sync::watch::Receiver<stalinium_evolution::node::SoupSnapshot> {
        self.soup.snapshot_watch()
    }

    pub fn node_id(&self) -> &[u8] {
        &self.node_id
    }
}
