//! Channon's shadow-resetting test with genes as the components, the way Geb passed it
//! (ECAL 2001, ALife VIII 2002): there the components were production rules, counted only
//! when expressed, not whole genotypes. Whole-genome hashes in an asexual soup mark every
//! point mutant as a brand-new component, so a gene that keeps working across many
//! genotypes never gets credit for it.
//!
//! A gene is a stretch of code from one label to the next. A label is a nop run that is
//! not the operand of a jump, call, address or infect. A genotype expresses a gene once
//! some member with an unchanged body has executed one of its cells; a gene is present in
//! the world when a living genotype expresses it.
//!
//! The shadow is reset to the real population at every census. It mirrors each real birth
//! with a random parent. A mutant child loses as many of its parent's genes, picked
//! uniformly, as the real mutant lost of its mother's, each replaced by a shadow-only
//! gene. Shadow sets keep unexpressed genes too, so a mutation lands on silent code as
//! often as in the real world. It mirrors each death by removing its oldest
//! member, as the reaper and old age do in the real world, not a random one.
//!
//! A second shadow, run the same way with its own dice, is scored against the first by the
//! same rule. Neither has selection in it, so whatever it reads as significant is what
//! chance alone produces here: the false-positive rate, measured on this run's own
//! trajectory. Real novelty means `new` well above `null_new`.

use crate::isa::*;
use crate::rng::Rng;
use crate::world::{Event, World};
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{BuildHasherDefault, Hasher};

/// Keys here are hashes or counters, so one multiply mixes them enough; SipHash made the
/// census cost more than the epoch's own bookkeeping.
#[derive(Default, Clone, Copy)]
struct Mix(u64);

impl Hasher for Mix {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(8) ^ b as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }
    fn write_u64(&mut self, x: u64) {
        self.0 = (self.0 ^ x).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
    fn write_u32(&mut self, x: u32) {
        self.write_u64(x as u64);
    }
}

type Map<K, V> = HashMap<K, V, BuildHasherDefault<Mix>>;
type Set<K> = HashSet<K, BuildHasherDefault<Mix>>;

/// Normalised activity a gene must reach to count as adaptively significant.
const SIGNIFICANT: i64 = 5;
/// Shadow-only genes get ids with this bit set; real gene hashes have it cleared.
const SHADOW: u64 = 1 << 63;

#[derive(Clone, Copy, Default)]
pub struct GeneStats {
    /// Genes present in the real world now.
    pub alive: u32,
    /// Sum and median of normalised activity over genes present now.
    pub cum: i64,
    pub median: i64,
    /// Genes that became adaptively significant for the first time ever this census.
    pub new: u32,
    /// The same count for the null shadow scored against the shadow.
    pub null_new: u32,
}

/// One genotype's genes: (hash, start, length) in genome order, and which cells any clean
/// member has executed.
#[derive(serde::Serialize, serde::Deserialize)]
struct Genome {
    genes: Vec<(u64, u16, u16)>,
    ran: [u64; 8],
}

impl Genome {
    fn expressed(&self) -> impl Iterator<Item = u64> + '_ {
        self.genes.iter().filter(|g| self.covers(g.1, g.2)).map(|g| g.0)
    }

    fn flagged(&self) -> Vec<(u64, bool)> {
        self.genes.iter().map(|g| (g.0, self.covers(g.1, g.2))).collect()
    }

    fn covers(&self, start: u16, len: u16) -> bool {
        (start..start + len).any(|i| self.ran[i as usize / 64] >> (i % 64) & 1 == 1)
    }
}

/// Split a genome into genes at its labels.
pub fn split(code: &[u8]) -> Vec<(u64, u16, u16)> {
    let takes_template = |op: u8| matches!(op, JMP | JMPB | CALL | ADR | ADRB | ADRF | INFECT);
    let mut starts = vec![0usize];
    for i in 1..code.len() {
        let nop = code[i] <= NOP1;
        let run_start = nop && code[i - 1] > NOP1;
        if run_start && !takes_template(code[i - 1]) {
            starts.push(i);
        }
    }
    starts.dedup();
    starts.push(code.len());
    starts
        .windows(2)
        .filter(|w| w[1] > w[0])
        .map(|w| {
            // A gene is a few cells long; blake3's setup cost dwarfed hashing them.
            let mut h = 0xCBF2_9CE4_8422_2325u64 ^ (w[1] - w[0]) as u64;
            for &b in &code[w[0]..w[1]] {
                h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01B3);
            }
            h ^= h >> 29;
            h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9) ^ (h >> 32);
            let h = h & !SHADOW;
            (h, w[0] as u16, (w[1] - w[0]) as u16)
        })
        .collect()
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct GeneMeter {
    genomes: Map<u64, Genome>,
    /// Shadow individuals in birth order, oldest first, each an index into `sets`: every
    /// gene of the genome, flagged if expressed.
    shadow: VecDeque<u32>,
    sets: Vec<Vec<(u64, bool)>>,
    prev_real: Set<u64>,
    activity: Map<u64, i64>,
    /// Every gene that has ever been significant: one that dies out and comes back is
    /// not new a second time.
    significant: Set<u64>,
    rng: Rng,
    next_shadow: u64,
    null: VecDeque<u32>,
    null_rng: Rng,
    null_activity: Map<u64, i64>,
    null_significant: Set<u64>,
}

impl GeneMeter {
    pub fn new(seed: u64) -> Self {
        GeneMeter {
            genomes: Map::default(),
            shadow: VecDeque::new(),
            sets: Vec::new(),
            prev_real: Set::default(),
            activity: Map::default(),
            significant: Set::default(),
            rng: Rng::new(seed ^ 0x6E4E_5EED),
            next_shadow: SHADOW,
            null: VecDeque::new(),
            null_rng: Rng::new(seed ^ 0x0A11_5EED),
            null_activity: Map::default(),
            null_significant: Set::default(),
        }
    }

    /// Mirror one interval's births and deaths. Call with every event, in order.
    pub fn event(&mut self, ev: &Event) {
        match ev {
            Event::Birth { mother, child, code, .. } => {
                // Genes of the mother the child no longer has.
                let mut lost = 0;
                if !code.is_empty() {
                    let genes = split(code);
                    lost = match self.genomes.get(mother) {
                        Some(m) => m.genes.iter().filter(|h| !genes.iter().any(|g| g.0 == h.0)).count(),
                        None => 1,
                    };
                    self.genomes.entry(*child).or_insert(Genome { genes, ran: [0; 8] });
                }
                for null in [false, true] {
                    let (pop, rng) = if null { (&self.null, &mut self.null_rng) } else { (&self.shadow, &mut self.rng) };
                    if pop.is_empty() {
                        continue;
                    }
                    let parent = pop[rng.below(pop.len() as u64) as usize];
                    let id = if lost > 0 { self.mutate(parent, lost, null) } else { parent };
                    if null { self.null.push_back(id) } else { self.shadow.push_back(id) }
                }
            }
            Event::Death { genotype, ran } => {
                if let Some(g) = self.genomes.get_mut(genotype) {
                    for (a, b) in g.ran.iter_mut().zip(ran) {
                        *a |= b;
                    }
                }
                self.shadow.pop_front();
                self.null.pop_front();
            }
        }
    }

    /// A copy of shadow set `parent` with `k` of its genes swapped for fresh shadow genes.
    fn mutate(&mut self, parent: u32, k: usize, null: bool) -> u32 {
        let mut set = self.sets[parent as usize].clone();
        for _ in 0..k {
            self.next_shadow += 1;
            if set.is_empty() {
                set.push((self.next_shadow, true));
            } else {
                // The new gene sits where the old one did, so it is expressed if that was.
                let rng = if null { &mut self.null_rng } else { &mut self.rng };
                let i = rng.below(set.len() as u64) as usize;
                set[i].0 = self.next_shadow;
            }
        }
        self.sets.push(set);
        (self.sets.len() - 1) as u32
    }

    /// Score the interval against the shadow, then reset the shadow to the real world.
    pub fn census(&mut self, w: &World) -> GeneStats {
        // Fold in what the living have executed, and learn any genome not yet seen.
        for c in w.cs.iter().filter(|c| c.alive) {
            let g = self.genomes.entry(c.genotype).or_insert_with(|| Genome { genes: split(w.genome(c)), ran: [0; 8] });
            if !c.dirty_body() {
                for (a, b) in g.ran.iter_mut().zip(c.ran) {
                    *a |= b;
                }
            }
        }
        let living: Set<u64> = w.cs.iter().filter(|c| c.alive).map(|c| c.genotype).collect();
        let in_real: Set<u64> = living.iter().flat_map(|g| self.genomes[g].expressed()).collect();
        let in_shadow = self.present(&self.shadow);
        let in_null = self.present(&self.null);
        for &g in &self.prev_real {
            let delta = in_real.contains(&g) as i64 - in_shadow.contains(&g) as i64;
            if delta != 0 {
                *self.activity.entry(g).or_default() += delta;
            }
        }
        let mut out = GeneStats { alive: in_real.len() as u32, ..Default::default() };
        let mut acts: Vec<i64> = Vec::with_capacity(in_real.len());
        for &g in &in_real {
            let a = self.activity.get(&g).copied().unwrap_or(0);
            acts.push(a);
            if a >= SIGNIFICANT && self.significant.insert(g) {
                out.new += 1;
            }
        }
        out.cum = acts.iter().sum();
        acts.sort_unstable();
        out.median = acts.get(acts.len() / 2).copied().unwrap_or(0);
        self.activity.retain(|g, _| in_real.contains(g));
        // The null shadow plays the real world's part, against the shadow, by the same rule.
        for &g in &self.prev_real {
            let delta = in_null.contains(&g) as i64 - in_shadow.contains(&g) as i64;
            if delta != 0 {
                *self.null_activity.entry(g).or_default() += delta;
            }
        }
        for &g in &in_null {
            if self.null_activity.get(&g).copied().unwrap_or(0) >= SIGNIFICANT && self.null_significant.insert(g) {
                out.null_new += 1;
            }
        }
        self.null_activity.retain(|g, _| in_null.contains(g));
        self.genomes.retain(|g, _| living.contains(g));
        self.prev_real = in_real;

        // Reset: the shadow becomes the living, oldest first, each with its expressed genes.
        self.sets.clear();
        self.shadow.clear();
        self.null.clear();
        let mut index: Map<u64, u32> = Map::default();
        for slot in w.reap_order() {
            let gt = w.cs[slot].genotype;
            let id = *index.entry(gt).or_insert_with(|| {
                self.sets.push(self.genomes[&gt].flagged());
                (self.sets.len() - 1) as u32
            });
            self.shadow.push_back(id);
        }
        self.null = self.shadow.clone();
        out
    }

    /// Expressed real genes carried by anyone in a shadow population. Many individuals
    /// share a set, so each distinct set is read once.
    fn present(&self, pop: &VecDeque<u32>) -> Set<u64> {
        let mut used = vec![false; self.sets.len()];
        for &s in pop {
            used[s as usize] = true;
        }
        self.sets.iter().zip(&used).filter(|(_, u)| **u).flat_map(|(set, _)| set.iter().filter(|g| g.1).map(|g| g.0)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ancestor_splits_at_labels_not_operands() {
        let code = assemble(ANCESTOR).unwrap();
        let genes = split(&code);
        // Labels: start, reproduction loop, copy procedure, copy loop, exit, end.
        assert_eq!(genes.len(), 6, "{genes:?}");
        assert_eq!(genes.iter().map(|g| g.2 as usize).sum::<usize>(), code.len());
        assert_eq!(genes[0].1, 0);
    }

    #[test]
    fn one_point_mutation_changes_one_gene() {
        let code = assemble(ANCESTOR).unwrap();
        let mut m = code.clone();
        m[4] = INC_A; // `zero`, inside the first gene
        let (a, b) = (split(&code), split(&m));
        let had: HashSet<u64> = a.iter().map(|g| g.0).collect();
        assert_eq!(b.iter().filter(|g| !had.contains(&g.0)).count(), 1);
    }
}
