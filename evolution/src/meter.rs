//! Is this world still inventing things?
//!
//! Bedau & Packard's evolutionary activity (1992), measured against Rechtsteiner &
//! Bedau's neutral shadow (ECAL 1999): a ghost population that copies every real
//! birth and death but picks who breeds and who dies at random. Genotypes that
//! outlast what chance alone would allow are counted as adaptive novelty.

use crate::isa::TASKS;
use crate::rng::Rng;
use crate::world::{Event, Stats, World};
use std::collections::HashMap;

#[derive(serde::Serialize, serde::Deserialize)]
struct Geno {
    size: u32,
    pop: u32,
    activity: u64,
    crossed: bool,
    name: String,
    /// Bred true at least once while running foreign code / while reading the weather.
    foreign: bool,
    sensing: bool,
    announced: bool,
    /// Tasks this lineage performed in a cycle that bred true.
    tasks: u16,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Ghost {
    pop: u32,
    activity: u64,
    crossed: bool,
}

pub struct Census {
    pub pop: usize,
    pub diversity: usize,
    pub size_classes: usize,
    pub mean_size: f64,
    pub min_size: u32,
    pub max_size: u32,
    pub mean_activity: f64,
    pub new_activity: u64,
    pub shadow_diversity: usize,
    pub shadow_new_activity: u64,
    pub excess: i64,
    pub births: u64,
    pub deaths: u64,
    pub dominant: String,
    /// Creatures that did each task in their last cycle.
    pub tasks: [u32; 10],
    pub computing: u32,
    pub mean_merit: f64,
    /// Mean scratch (nopx) cells per living creature.
    pub scratch: f64,
    pub stats: Stats,
    /// Creatures that scored guesses last cycle, their mean accuracy, and the best one.
    pub divers: u32,
    pub ocean_acc: f64,
    pub ocean_best: f64,
    /// Mean (bits matched - 4) per guess among divers: 0 is chance, 4 is perfect.
    pub ocean_bits: f64,
    pub modes: crate::modes::Modes,
    pub norm: crate::channon::Normalised,
    pub genes: crate::genes::GeneStats,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Meter {
    genos: HashMap<u64, Geno>,
    names: HashMap<u32, u32>,
    ghosts: HashMap<u64, Ghost>,
    ghost_pop: Vec<u64>,
    next_ghost: u64,
    ghost_rng: Rng,
    dead_ghost_activity: Vec<u64>,
    pub theta: Option<u64>,
    warmup: u64,
    pub excess: i64,
    ancestor: u64,
    ancestor_size: u32,
    firsts: Vec<String>,
    lineages: [u32; 2],
    tasks_seen: u16,
    ocean_level: u32,
    modes: crate::modes::Tracker,
    channon: crate::channon::ResetShadow,
    genes: crate::genes::GeneMeter,
    sizes_established: HashMap<u32, ()>,
    dominant: u64,
}

fn letters(mut k: u32) -> String {
    let mut s = Vec::new();
    for _ in 0..3 {
        s.push(b'a' + (k % 26) as u8);
        k /= 26;
    }
    s.reverse();
    String::from_utf8(s).unwrap()
}

impl Meter {
    pub fn new(ancestor: u64, ancestor_size: u32, seed: u64, warmup: u64) -> Self {
        let mut m = Meter {
            genos: HashMap::new(),
            names: HashMap::new(),
            ghosts: HashMap::new(),
            ghost_pop: vec![0],
            next_ghost: 1,
            ghost_rng: Rng::new(seed ^ 0x5EED_0F_6405),
            dead_ghost_activity: Vec::new(),
            theta: None,
            warmup,
            excess: 0,
            ancestor,
            ancestor_size,
            firsts: Vec::new(),
            lineages: [0; 2],
            tasks_seen: 0,
            ocean_level: 0,
            modes: crate::modes::Tracker::new(),
            channon: crate::channon::ResetShadow::new(seed, ancestor),
            genes: crate::genes::GeneMeter::new(seed),
            sizes_established: HashMap::new(),
            dominant: ancestor,
        };
        m.born(ancestor, ancestor_size);
        m.ghosts.insert(0, Ghost { pop: 1, ..Default::default() });
        m.sizes_established.insert(ancestor_size, ());
        m
    }

    pub fn name_of(&self, g: u64) -> String {
        self.genos.get(&g).map(|x| x.name.clone()).unwrap_or_else(|| format!("{g:016x}"))
    }

    fn born(&mut self, g: u64, size: u32) {
        let names = &mut self.names;
        self.genos
            .entry(g)
            .or_insert_with(|| {
                let k = names.entry(size).or_insert(0);
                let name = format!("{size:04}{}", letters(*k));
                *k += 1;
                Geno { size, pop: 0, activity: 0, crossed: false, name, foreign: false, sensing: false, announced: false, tasks: 0 }
            })
            .pop += 1;
    }

    fn first(&mut self, key: &'static str, out: &mut Vec<String>, text: String) {
        if !self.firsts.iter().any(|k| k == key) {
            self.firsts.push(key.to_string());
            out.push(text);
        }
    }

    /// Fold this epoch's births and deaths in, take a census, return notable firsts.
    pub fn census(&mut self, w: &mut World) -> (Census, Vec<String>) {
        let mut notes = Vec::new();
        let (mut births, mut deaths) = (0u64, 0u64);
        for ev in std::mem::take(&mut w.events) {
            crate::prof::time(crate::prof::GENE_EVENTS, || self.genes.event(&ev));
            match ev {
                Event::Birth { mother, mother_now, child, child_size, foreign, sensed, tasks, .. } => {
                    births += 1;
                    let mutant = mother != child;
                    self.channon.birth(mutant);
                    self.born(child, child_size);
                    // Only a lineage that breeds true counts as having the trait.
                    if !mutant && mother_now == child {
                        let g = self.genos.get_mut(&child).unwrap();
                        g.foreign |= foreign;
                        g.sensing |= sensed;
                        g.tasks |= tasks;
                    }
                    // The shadow mirrors this birth with a parent picked by chance.
                    let i = self.ghost_rng.below(self.ghost_pop.len() as u64) as usize;
                    let mut gid = self.ghost_pop[i];
                    if mutant {
                        gid = self.next_ghost;
                        self.next_ghost += 1;
                    }
                    self.ghost_pop.push(gid);
                    self.ghosts.entry(gid).or_default().pop += 1;
                }
                Event::Death { genotype, .. } => {
                    self.channon.death();
                    deaths += 1;
                    if let Some(g) = self.genos.get_mut(&genotype) {
                        g.pop -= 1;
                    }
                    if !self.ghost_pop.is_empty() {
                        let i = self.ghost_rng.below(self.ghost_pop.len() as u64) as usize;
                        let gid = self.ghost_pop.swap_remove(i);
                        if let Some(gh) = self.ghosts.get_mut(&gid) {
                            gh.pop -= 1;
                        }
                    }
                }
            }
        }

        // Activity: each genotype's population, summed over every census it has lived through.
        let theta = self.theta;
        let mut new_activity = 0;
        let mut crossed_now: Vec<u64> = Vec::new();
        let mut size_pop: HashMap<u32, u32> = HashMap::new();
        let (mut total_size, mut total_act, mut pop) = (0u64, 0u64, 0usize);
        let (mut min_size, mut max_size) = (u32::MAX, 0);
        let mut best = (0u32, 0u64);
        for (&id, g) in self.genos.iter_mut().filter(|(_, g)| g.pop > 0) {
            g.activity += g.pop as u64;
            if let Some(t) = theta {
                if !g.crossed && g.activity >= t {
                    g.crossed = true;
                    new_activity += 1;
                    crossed_now.push(id);
                }
            }
            *size_pop.entry(g.size).or_default() += g.pop;
            total_size += g.size as u64 * g.pop as u64;
            total_act += g.activity;
            pop += g.pop as usize;
            min_size = min_size.min(g.size);
            max_size = max_size.max(g.size);
            if g.pop > best.0 || (g.pop == best.0 && id < best.1) {
                best = (g.pop, id);
            }
        }
        // A trait is news once its lineage holds at least 20 living members.
        let mut found = Vec::new();
        for g in self.genos.values_mut() {
            if g.pop >= 20 && !g.announced && (g.foreign || g.sensing) {
                g.announced = true;
                found.push((g.foreign, g.name.clone(), g.size, g.pop));
            }
        }
        found.sort();
        let mut learned = Vec::new();
        for g in self.genos.values().filter(|g| g.pop >= 20) {
            let new = g.tasks & !self.tasks_seen;
            for t in (0..10).filter(|t| new >> t & 1 == 1) {
                learned.push((t, g.name.clone(), g.pop));
            }
        }
        learned.sort();
        for (t, name, pop) in learned {
            if self.tasks_seen >> t & 1 == 0 {
                self.tasks_seen |= 1 << t;
                notes.push(format!("{name} computes {} on numbers from the outside world and passes it on: {pop} alive", TASKS[t].to_uppercase()));
            }
        }
        for (foreign, name, size, pop) in found {
            let k = if foreign { 0 } else { 1 };
            self.lineages[k] += 1;
            if self.lineages[k] > 5 {
                continue;
            }
            notes.push(if foreign {
                if size * 4 < self.ancestor_size * 3 {
                    format!("{name} is a parasite: {size} cells, too small to copy itself, it breeds on a neighbour's code ({pop} alive)")
                } else {
                    format!("{name} breeds true by borrowing a neighbour's code: {size} cells, {pop} alive")
                }
            } else {
                format!("{name} breeds true while reading the weather: {size} cells, {pop} alive")
            });
        }
        self.genos.retain(|_, g| g.pop > 0);
        let diversity = self.genos.len();
        let persistent: Vec<crate::modes::Persistent> = self
            .genos
            .iter()
            .map(|(_, g)| crate::modes::Persistent { pop: g.pop, tasks: g.tasks, size: g.size })
            .collect();
        crossed_now.sort_unstable();
        let mut fresh: Vec<Vec<u8>> = Vec::new();
        if !crossed_now.is_empty() {
            let mut want: std::collections::HashSet<u64> = crossed_now.iter().copied().collect();
            for c in w.cs.iter().filter(|c| c.alive) {
                if want.remove(&c.genotype) {
                    fresh.push(w.genome(c).to_vec());
                }
            }
        }
        let modes = self.modes.update(&persistent, &fresh);
        w.train(&fresh);
        let real: Vec<(u64, u32)> = { let mut v: Vec<(u64, u32)> = self.genos.iter().map(|(&g, x)| (g, x.pop)).collect(); v.sort_unstable(); v };
        let norm = self.channon.census(&real);
        let genes = crate::prof::time(crate::prof::GENE_CENSUS, || self.genes.census(w));

        let mut shadow_new = 0;
        for gh in self.ghosts.values_mut().filter(|g| g.pop > 0) {
            gh.activity += gh.pop as u64;
            if let Some(t) = theta {
                if !gh.crossed && gh.activity >= t {
                    gh.crossed = true;
                    shadow_new += 1;
                }
            }
        }
        if self.theta.is_none() {
            for gh in self.ghosts.values().filter(|g| g.pop == 0) {
                self.dead_ghost_activity.push(gh.activity);
            }
        }
        self.ghosts.retain(|_, g| g.pop > 0);
        let shadow_diversity = self.ghosts.len();

        // After warmup, fix the bar: what the luckiest 1% of neutral genotypes reached.
        self.warmup = self.warmup.saturating_sub(1);
        if self.theta.is_none() && self.warmup == 0 {
            let mut all: Vec<u64> = self.dead_ghost_activity.drain(..).collect();
            all.extend(self.ghosts.values().map(|g| g.activity));
            all.sort_unstable();
            let t = all.get(all.len() * 99 / 100).copied().unwrap_or(1).max(2);
            self.theta = Some(t);
            notes.push(format!("novelty bar set from the neutral shadow: a genotype must reach {t} activity to count"));
        }
        self.excess += new_activity as i64 - shadow_new as i64;

        let mut sizes: Vec<_> = size_pop.iter().map(|(&s, &p)| (s, p)).collect();
        sizes.sort_unstable();
        for (size, p) in sizes {
            if p >= 20 && self.sizes_established.insert(size, ()).is_none() {
                notes.push(format!("a new body size takes hold: {size} cells ({p} alive)"));
            }
        }
        if best.1 != self.dominant && best.0 > 0 {
            self.dominant = best.1;
            notes.push(format!("{} is now the most common genotype ({} alive)", self.name_of(best.1), best.0));
        }
        if !self.genos.contains_key(&self.ancestor) {
            self.first("ancestor_gone", &mut notes, "the ancestor's exact genome is extinct".into());
        }

        let mut task_counts = [0u32; 10];
        let mut scratch_sum = 0u64;
        let (mut divers, mut acc_sum, mut acc_best, mut bits_sum) = (0u32, 0f64, 0f64, 0f64);
        let mut at_level = [0u32; 10];
        let (mut computing, mut merit_sum) = (0u32, 0u64);
        for c in w.cs.iter().filter(|c| c.alive) {
            for (t, k) in task_counts.iter_mut().enumerate() {
                *k += (c.tasks_last >> t & 1) as u32;
            }
            computing += (c.tasks_last != 0) as u32;
            merit_sum += c.merit as u64;
            if c.scored_last >= 4 {
                let acc = c.hits_last as f64 / c.scored_last as f64;
                divers += 1;
                acc_sum += acc;
                bits_sum += c.bits_last as f64 / c.scored_last as f64;
                acc_best = acc_best.max(acc);
                for (k, n) in at_level.iter_mut().enumerate() {
                    *n += (acc >= k as f64 / 10.0) as u32;
                }
            }
            scratch_sum += w.genome(c).iter().filter(|&&o| o == crate::isa::NOPX).count() as u64;
        }
        // Ocean milestones: 20 creatures guessing at least k-in-10 of the bytes they dive for.
        while (self.ocean_level as usize) < at_level.len() && at_level[self.ocean_level as usize] >= 20 {
            let k = self.ocean_level;
            notes.push(if k == 0 {
                format!("{} creatures now dive into the ocean and guess what they'll find", at_level[0])
            } else {
                format!("{} creatures now predict the ocean right at least {}% of the time", at_level[k as usize], k * 10)
            });
            self.ocean_level += 1;
        }
        let n = size_pop.len();
        let c = Census {
            pop,
            diversity,
            size_classes: n,
            mean_size: if pop > 0 { total_size as f64 / pop as f64 } else { 0.0 },
            min_size: if pop > 0 { min_size } else { 0 },
            max_size,
            mean_activity: if diversity > 0 { total_act as f64 / diversity as f64 } else { 0.0 },
            new_activity,
            shadow_diversity,
            shadow_new_activity: shadow_new,
            excess: self.excess,
            births,
            deaths,
            dominant: self.name_of(self.dominant),
            tasks: task_counts,
            computing,
            mean_merit: if pop > 0 { merit_sum as f64 / pop as f64 } else { 0.0 },
            scratch: if pop > 0 { scratch_sum as f64 / pop as f64 } else { 0.0 },
            stats: std::mem::take(&mut w.stats),
            divers,
            ocean_acc: if divers > 0 { acc_sum / divers as f64 } else { 0.0 },
            ocean_best: acc_best,
            ocean_bits: if divers > 0 { bits_sum / divers as f64 } else { 0.0 },
            modes,
            norm,
            genes,
        };
        (c, notes)
    }

    /// The most common living genotypes, for the genebank.
    pub fn top(&self, k: usize) -> Vec<(u64, String, u32, u64)> {
        let mut v: Vec<_> = self.genos.iter().map(|(&id, g)| (id, g.name.clone(), g.pop, g.activity)).collect();
        v.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
        v.truncate(k);
        v
    }
}
