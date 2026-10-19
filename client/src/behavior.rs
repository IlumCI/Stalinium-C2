use stalinium_evolution::node::BehaviorSignal;

/// Network communication parameters derived from the evolutionary soup's
/// phenotype. The soup's ecological dynamics modulate timing, fanout, and
/// routing -- producing traffic patterns that evolve continuously rather
/// than following a static profile.
///
/// arXiv:2603.15655 - agents evolve steganographic communication spontaneously
/// arXiv:2609.17320 - emergent opaque communication without explicit incentive
#[derive(Clone, Debug)]
pub struct SwarmParams {
    pub beacon_interval_ms: u64,
    pub gossip_fanout: usize,
    pub jitter_range: (u32, u32),
    pub stealth_routing: bool,
    pub intensity: f64,
    pub evolved_tasks: Vec<usize>,
}

const BASE_BEACON_MS: u64 = 2000;
const MIN_BEACON_MS: u64 = 500;
const MAX_BEACON_MS: u64 = 30000;
const BASE_FANOUT: usize = 3;

impl SwarmParams {
    pub fn from_behavior(signal: &BehaviorSignal) -> Self {
        let jitter_factor = signal.timing_jitter_ms as f64 / 1000.0;
        let stealth_stretch = 1.0 + signal.stealth * 5.0;
        let aggression_compress = 1.0 - signal.aggression * 0.5;
        let beacon_interval_ms = ((BASE_BEACON_MS as f64) * stealth_stretch * aggression_compress
            + jitter_factor * 500.0) as u64;
        let beacon_interval_ms = beacon_interval_ms.clamp(MIN_BEACON_MS, MAX_BEACON_MS);

        let fanout_raw = (BASE_FANOUT as f64 * signal.fanout_modifier).round() as usize;
        let gossip_fanout = fanout_raw.clamp(1, 10);

        let jitter_base = signal.timing_jitter_ms / 4;
        let jitter_max = signal.timing_jitter_ms;
        let jitter_range = (jitter_base.max(5), jitter_max.max(50));

        let stealth_routing = signal.stealth > 0.5 && signal.aggression < 0.3;

        let intensity = (signal.aggression * 2.0).clamp(0.0, 2.0);

        let evolved_tasks: Vec<usize> = signal
            .task_vector
            .iter()
            .enumerate()
            .filter(|(_, &count)| count > 0)
            .map(|(i, _)| i)
            .collect();

        SwarmParams {
            beacon_interval_ms,
            gossip_fanout,
            jitter_range,
            stealth_routing,
            intensity,
            evolved_tasks,
        }
    }

    pub fn jittered_delay(&self, base_ms: u64, entropy_seed: u64) -> u64 {
        let range = self.jitter_range.1.saturating_sub(self.jitter_range.0) as u64;
        if range == 0 {
            return base_ms;
        }
        let jitter = (entropy_seed % range) + self.jitter_range.0 as u64;
        base_ms.saturating_add(jitter)
    }
}
