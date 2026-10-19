//! Channon's component-normalised activity (ALife VIII, 2002): the shadow-resetting method.
//!
//! After every census the shadow is reset to an exact copy of the real population. Over the
//! next interval it mirrors every real birth and death, but picks parents and victims at
//! random. A component (genotype) gains normalised activity for each census it persists in
//! the real world, and loses it for each census it persists in the shadow: so it only
//! accumulates activity by outlasting what chance would have kept from the same start.
//! Components seen in one census but not the one before are screened out as isolated.
//!
//! Unbounded dynamics (Bedau, Snyder & Packard's class 3) means normalised cumulative (or
//! median) activity keeps growing while new adaptively significant components keep coming.

use crate::rng::Rng;
use std::collections::{HashMap, HashSet};

/// Normalised activity a component must reach to count as adaptively significant.
const SIGNIFICANT: i64 = 5;
/// Shadow-only genotypes created by mutation get ids with this bit set, never real ones.
const SHADOW_ID: u64 = 1 << 63;

#[derive(Clone, Copy, Default)]
pub struct Normalised {
    /// Sum of normalised activity over components alive in the real world.
    pub cum: i64,
    pub median: i64,
    /// Components that became adaptively significant this census, per real component alive.
    pub new_per_component: f64,
    pub new: u32,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ResetShadow {
    pop: Vec<u64>,
    prev_real: HashSet<u64>,
    activity: HashMap<u64, i64>,
    significant: HashSet<u64>,
    rng: Rng,
    next_id: u64,
}

impl ResetShadow {
    pub fn new(seed: u64, founder: u64) -> Self {
        ResetShadow {
            pop: vec![founder],
            prev_real: HashSet::new(),
            activity: HashMap::new(),
            significant: HashSet::new(),
            rng: Rng::new(seed ^ 0xC4A2_202E),
            next_id: SHADOW_ID,
        }
    }

    /// A real birth, mirrored: a random shadow parent has a child, mutant if the real one was.
    pub fn birth(&mut self, mutant: bool) {
        if self.pop.is_empty() {
            return;
        }
        let g = if mutant {
            self.next_id += 1;
            self.next_id
        } else {
            self.pop[self.rng.below(self.pop.len() as u64) as usize]
        };
        self.pop.push(g);
    }

    /// A real death, mirrored: someone in the shadow dies at random.
    pub fn death(&mut self) {
        if !self.pop.is_empty() {
            let i = self.rng.below(self.pop.len() as u64) as usize;
            self.pop.swap_remove(i);
        }
    }

    /// Score this interval, then reset the shadow to the real population.
    /// `real`: each living genotype and its population.
    pub fn census(&mut self, real: &[(u64, u32)]) -> Normalised {
        let in_shadow: HashSet<u64> = self.pop.iter().copied().collect();
        let in_real: HashSet<u64> = real.iter().map(|r| r.0).collect();
        // Both runs started this interval from the same population, prev_real; anything
        // not in it is an isolated newcomer and is screened out of both.
        for &g in &self.prev_real {
            let delta = in_real.contains(&g) as i64 - in_shadow.contains(&g) as i64;
            if delta != 0 {
                *self.activity.entry(g).or_default() += delta;
            }
        }
        let mut out = Normalised::default();
        let mut alive: Vec<i64> = Vec::with_capacity(real.len());
        for &(g, _) in real {
            let a = self.activity.get(&g).copied().unwrap_or(0);
            alive.push(a);
            if a >= SIGNIFICANT && self.significant.insert(g) {
                out.new += 1;
            }
        }
        out.cum = alive.iter().sum();
        alive.sort_unstable();
        out.median = alive.get(alive.len() / 2).copied().unwrap_or(0);
        out.new_per_component = if real.is_empty() { 0.0 } else { out.new as f64 / real.len() as f64 };
        // Forget components gone from both runs; their activity can never count again.
        self.activity.retain(|g, _| in_real.contains(g));
        self.significant.retain(|g| in_real.contains(g));
        self.prev_real = in_real;
        self.pop = real.iter().flat_map(|&(g, n)| std::iter::repeat_n(g, n as usize)).collect();
        out
    }
}
