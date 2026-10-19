use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Adaptive scan timing policy inspired by adversarial RL scan evasion
/// (arXiv:2610.08864) and PLAA packet-level timing attacks (arXiv:2606.28439).
///
/// Modulates inter-probe delays and effective scan rate to evade statistical
/// timing analysis by IDS/NIDS. Instead of constant-rate scanning (trivially
/// detectable via entropy analysis of inter-arrival times), applies:
///
///   1. Burst-then-pause: short bursts of probes followed by variable silence.
///   2. Jittered delays: each inter-probe gap drawn from a truncated
///      distribution so the timing signature does not cluster.
///   3. Rate dampening: when consecutive timeouts suggest rate limiting or
///      active defence, the effective rate drops exponentially.
pub struct AdaptiveScanPolicy {
    enabled: bool,
    consecutive_timeouts: AtomicU64,
    probes_in_burst: AtomicU64,
    burst_size: u64,
    base_delay_us: u64,
}

impl AdaptiveScanPolicy {
    pub fn new(enabled: bool) -> Self {
        AdaptiveScanPolicy {
            enabled,
            consecutive_timeouts: AtomicU64::new(0),
            probes_in_burst: AtomicU64::new(0),
            burst_size: 8,
            base_delay_us: 5_000,
        }
    }

    pub fn effective_rate(&self, base_rate: f64) -> f64 {
        if !self.enabled {
            return base_rate;
        }
        let timeouts = self.consecutive_timeouts.load(Ordering::Relaxed);
        let dampening = 0.5_f64.powi(timeouts.min(6) as i32);
        base_rate * dampening
    }

    pub fn record_timeout(&self) {
        self.consecutive_timeouts.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_success(&self) {
        self.consecutive_timeouts.store(0, Ordering::Relaxed);
    }

    pub async fn delay(&self) {
        if !self.enabled {
            return;
        }

        let burst_pos = self.probes_in_burst.fetch_add(1, Ordering::Relaxed);

        if burst_pos >= self.burst_size {
            self.probes_in_burst.store(0, Ordering::Relaxed);
            let pause_ms = self.burst_pause_ms();
            tokio::time::sleep(Duration::from_millis(pause_ms)).await;
            return;
        }

        let jitter = self.jittered_delay_us();
        if jitter > 0 {
            tokio::time::sleep(Duration::from_micros(jitter)).await;
        }
    }

    fn burst_pause_ms(&self) -> u64 {
        let timeouts = self.consecutive_timeouts.load(Ordering::Relaxed);
        let base = 50 + timeouts * 200;
        let entropy = simple_prng() % 100;
        base + entropy
    }

    fn jittered_delay_us(&self) -> u64 {
        let entropy = simple_prng() % self.base_delay_us;
        self.base_delay_us / 2 + entropy
    }
}

fn simple_prng() -> u64 {
    use std::time::SystemTime;
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos() as u64;
    seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) % 1000
}
