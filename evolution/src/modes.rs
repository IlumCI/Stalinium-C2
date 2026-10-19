//! Open-endedness measured four ways (MODES: Dolson, Vostinar, Wiser & Ofria 2019), plus an
//! observer (Hughes et al. 2024). Everything looks only at persistent phenotypes (task
//! profile + size band) present for PERSIST censuses in a row; genotypes churn too fast here.
//!
//! change:     persistent phenotypes now that weren't, one window ago
//! novelty:    persistent phenotypes (task profile + size band) never seen before, cumulative
//! complexity: the most complex task profile among persistent phenotypes, in NAND-steps
//! ecology:    effective number of persistent phenotypes (exp of Shannon entropy)
//! observer:   bits of surprise per instruction of each new persistent genome, under a model
//!             that has read every earlier one. Open-ended means it stays surprising.

use std::collections::{HashMap, HashSet};

/// Rough NAND-count of each task, matching TASK_BONUS = 2^cost.
const TASK_COST: [u32; 10] = [1, 1, 2, 2, 3, 3, 4, 4, 5, 1];
const WINDOW: u64 = 50;
const PERSIST: u32 = 20;
const SYMS: usize = 64;

pub struct Persistent {
    pub pop: u32,
    pub tasks: u16,
    pub size: u32,
}

#[derive(Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct Modes {
    pub change: u32,
    pub novelty: u32,
    pub complexity: u32,
    pub ecology: f64,
    pub observer_bits: f64,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Tracker {
    window_prev: HashSet<u32>,
    streak: HashMap<u32, u32>,
    censuses: u64,
    seen: HashSet<u32>,
    counts: Vec<u32>,
    totals: Vec<u32>,
    last: Modes,
}

fn phenotype(p: &Persistent) -> u32 {
    p.tasks as u32 | (p.size / 32) << 16
}

impl Tracker {
    pub fn new() -> Self {
        Tracker {
            window_prev: HashSet::new(),
            streak: HashMap::new(),
            censuses: 0,
            seen: HashSet::new(),
            counts: vec![0; SYMS * SYMS * SYMS],
            totals: vec![0; SYMS * SYMS],
            last: Modes::default(),
        }
    }

    /// `living`: every genotype alive now. `fresh`: genomes of lineages that beat the
    /// shadow's bar this census.
    pub fn update(&mut self, living: &[Persistent], fresh: &[Vec<u8>]) -> Modes {
        self.censuses += 1;
        let present: HashSet<u32> = living.iter().map(phenotype).collect();
        self.streak.retain(|k, _| present.contains(k));
        for &k in &present {
            *self.streak.entry(k).or_default() += 1;
        }
        let alive: Vec<&Persistent> = living.iter().filter(|p| self.streak[&phenotype(p)] >= PERSIST).collect();
        let now: HashSet<u32> = alive.iter().map(|p| phenotype(p)).collect();
        if self.censuses % WINDOW == 0 {
            self.last.change = now.difference(&self.window_prev).count() as u32;
            self.window_prev = now;
        }

        for p in &alive {
            if self.seen.insert(phenotype(p)) {
                self.last.novelty += 1;
            }
        }
        self.last.complexity = alive
            .iter()
            .map(|p| (0..10).filter(|t| p.tasks >> t & 1 == 1).map(|t| TASK_COST[t]).sum())
            .max()
            .unwrap_or(0);

        // Ordered, so the float sum below comes out the same in every process.
        let mut by_pheno: std::collections::BTreeMap<u32, u64> = std::collections::BTreeMap::new();
        for p in &alive {
            *by_pheno.entry(phenotype(p)).or_default() += p.pop as u64;
        }
        let total: u64 = by_pheno.values().sum();
        let entropy: f64 = by_pheno
            .values()
            .map(|&n| {
                let q = n as f64 / total as f64;
                -q * q.ln()
            })
            .sum();
        self.last.ecology = if total > 0 { entropy.exp() } else { 0.0 };

        if !fresh.is_empty() {
            let bits: f64 = fresh.iter().map(|g| self.observe(g)).sum();
            self.last.observer_bits = bits / fresh.len() as f64;
        }
        self.last
    }

    /// Order-2 model over instructions: surprise of a genome before learning from it.
    fn observe(&mut self, genome: &[u8]) -> f64 {
        if genome.len() < 3 {
            return 0.0;
        }
        let mut bits = 0.0;
        for w in genome.windows(3) {
            let ctx = (w[0] as usize % SYMS) * SYMS + w[1] as usize % SYMS;
            let i = ctx * SYMS + w[2] as usize % SYMS;
            let p = (self.counts[i] as f64 + 0.5) / (self.totals[ctx] as f64 + 0.5 * SYMS as f64);
            bits -= p.log2();
            self.counts[i] += 1;
            self.totals[ctx] += 1;
        }
        bits / (genome.len() - 2) as f64
    }
}
