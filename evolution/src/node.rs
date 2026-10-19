//! A living soup running on a dedicated OS thread, fed by network telemetry.
//!
//! World is !Send (Rc<RefCell<...>> inside jump tables), so it lives on its own
//! thread. The async side communicates via channels: telemetry in, migrants and
//! census snapshots out. Architecture follows arXiv:2401.17224 (gossip-mediated
//! asynchronous migration with self-adaptive rate) and arXiv:2409.01469
//! (heterogeneous swarm substrates for open-ended evolution).

use crate::feed::Feed;
use crate::isa;
use crate::meter::Meter;
use crate::world::{Ocean, Params, World};
use std::sync::mpsc;
use tokio::sync::{oneshot, watch};

/// Network telemetry that becomes the soup's weather and ocean.
#[derive(Clone, Debug, Default)]
pub struct Telemetry {
    pub peer_count: u32,
    pub mean_latency_ms: u32,
    pub gossip_msg_rate: u32,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub uptime_secs: u64,
}

/// A snapshot of the soup's state, cheap to send across threads.
#[derive(Clone, Debug, Default)]
pub struct SoupSnapshot {
    pub epoch: u64,
    pub population: usize,
    pub diversity: usize,
    pub alive_genes: u32,
    pub gene_novelty: u32,
    pub excess: i64,
    pub dominant: String,
    pub computing: u32,
}

/// Serialised creature genome for migration between nodes.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Migrant {
    pub genome: Vec<u8>,
    pub source_node: Vec<u8>,
    pub epoch: u64,
}

enum Command {
    Telemetry(Telemetry),
    Immigrate(Vec<Migrant>),
    Emigrate { count: usize, reply: oneshot::Sender<Vec<Migrant>> },
    Snapshot(oneshot::Sender<SoupSnapshot>),
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

/// Convert network telemetry into 32 bytes of weather for the soup.
/// Peer count and latency shape the sun position and width; gossip rate
/// and bandwidth become the cosmic ray interval; raw bytes are hashed
/// to produce the data field.
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

/// Build an ocean from recent telemetry history. The ocean is a byte stream
/// that creatures try to predict; using network telemetry means creatures
/// that evolve to model network patterns get rewarded.
fn build_telemetry_ocean(history: &[Telemetry]) -> Vec<u8> {
    let mut ocean = Vec::with_capacity(history.len() * 32);
    for t in history {
        ocean.extend_from_slice(&telemetry_to_weather_bytes(t));
    }
    ocean
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
    let mut telemetry_history: Vec<Telemetry> = Vec::new();
    let mut latest_telemetry = Telemetry::default();

    loop {
        // Drain all pending commands without blocking
        loop {
            match rx.try_recv() {
                Ok(Command::Telemetry(t)) => {
                    latest_telemetry = t.clone();
                    telemetry_history.push(t);
                    if telemetry_history.len() > TELEMETRY_OCEAN_SIZE {
                        telemetry_history.remove(0);
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
                    let _ = reply.send(make_snapshot(epoch, &w, &m));
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

        // Mix telemetry into weather bytes
        let mut weather_bytes = fb;
        if !telemetry_history.is_empty() {
            let tb = telemetry_to_weather_bytes(&latest_telemetry);
            for (a, b) in weather_bytes.iter_mut().zip(tb.iter()) {
                *a ^= *b;
            }
        }

        w.set_weather(&weather_bytes);

        // Periodically refresh the ocean from telemetry history
        if epoch % 10 == 0 && !telemetry_history.is_empty() {
            let ocean_bytes = build_telemetry_ocean(&telemetry_history);
            w.replace_ocean(ocean_bytes);
        }

        w.run(EPOCH_INSTR);
        let (c, _notes) = m.census(&mut w);

        let snap = SoupSnapshot {
            epoch,
            population: c.pop,
            diversity: c.diversity,
            alive_genes: c.genes.alive,
            gene_novelty: c.genes.new,
            excess: c.norm.cum,
            dominant: c.dominant.clone(),
            computing: c.computing,
        };
        let _ = snap_tx.send(snap);

        epoch += 1;

        if w.alive == 0 {
            // Reinject ancestor on extinction
            let g = w.inject(&ancestor, w.n / 2);
            m = Meter::new(g, ancestor.len() as u32, seed.wrapping_add(epoch), WARMUP);
        }

        // Yield to OS briefly so we don't starve the async runtime
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

fn make_snapshot(epoch: u64, w: &World, _m: &Meter) -> SoupSnapshot {
    SoupSnapshot {
        epoch,
        population: w.alive,
        diversity: 0,
        alive_genes: 0,
        gene_novelty: 0,
        excess: 0,
        dominant: String::new(),
        computing: 0,
    }
}
