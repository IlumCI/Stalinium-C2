//! The soup: one circular strip of memory where creatures live, copy themselves,
//! and get reaped. No fitness function anywhere. Survival is the only judge.

use crate::isa::*;
use crate::prof;
use crate::rng::Rng;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::{Rc, Weak};

const STACK: usize = 10;
const MAX_TEMPLATE: u32 = 6;
/// Cells at the start of a body that predators' keys are matched against.
const SURFACE: u32 = 8;
/// One error ages a creature by this many births in the reaper's eyes.
const ERROR_WEIGHT: i64 = 4;
/// How far `mal` scans for free space per look.
const SEARCH_WINDOW: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Regime {
    #[default]
    Commons,
    Feudal,
    Capitalist,
    /// Starts as a commons. Property, rent and revolt are instructions creatures may evolve.
    Evolved,
}

/// Fee to enclose a region, paid to the commons. Affordable only with a surplus.
const ENCLOSE_FEE: u64 = 50_000;
/// Highest rent a title holder may levy, percent.
const MAX_LEVY: u64 = 50;

/// Share of a producer's income its feudal lord takes.
const FEUDAL_RENT_PCT: u64 = 30;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Params {
    pub soup_bits: u32,
    /// One copy in this many is miscopied.
    pub copy_error: u64,
    /// Mean instructions between cosmic rays, before the weather scales it.
    pub cosmic_base: u64,
    /// The reaper keeps the soup at most this full.
    pub fill: f64,
    pub min_size: u32,
    pub max_size: u32,
    pub search_limit: u32,
    /// Slice multiplier for creatures whose body lies in sunlight.
    pub sun_bonus: u64,
    /// Pay creatures in CPU time for computing logic on feed data.
    pub tasks: bool,
    /// One birth in this many gets a cell inserted, and one in this many a cell deleted. 0 = never.
    pub indel: u64,
    pub max_merit: u32,
    /// A creature dies of old age after executing this many times its own length (Avida uses 20;
    /// at 25 here, lineages that evolved longer computations went sterile).
    pub max_age: u64,
    /// The ocean pays one extra unit of merit per this many correct guesses. 0 = no ocean.
    pub ocean_div: u32,
    pub ocean_cap: u32,
    /// Give creatures a private 256-byte memory (load/store), inherited by their children.
    pub memory: bool,
    /// Allow `infect`: take CPU from a nearby creature that shares a logic task (the Red
    /// Queen; Zaman et al. 2014). Neighbours only, within this many cells.
    pub infect: bool,
    pub infect_radius: u32,
    /// The reef: each region of the soup has a stream creatures can `emit` into and, after
    /// `tune`, `dive` from. Decoding a rival pays the reader and costs the emitter; a
    /// relative predicting you pays you. Mixed cooperation and competition (Moran & Pollack).
    pub reef: bool,
    /// Conserved energy economy instead of relative merit: a fixed total supply, each
    /// instruction costs one unit back to a commons pool, and income is the pool's payout
    /// split by score. Nothing is ever minted, so every unit is accounted for.
    pub energy: bool,
    pub energy_total: u64,
    /// Property in land (the 16 regions): none (a commons), feudal or capitalist.
    pub regime: Regime,
    /// Mutation draws only from the ops of features that are switched on, and the rest are
    /// errors. With all 51 ops in play, any one op was too rare for evolution to find.
    /// Also: a child must be copied in full, from the mother's own body.
    pub lean: bool,
    /// Allow `give`, `mate`, `learn`, `teach` and `join` (only matters when lean).
    pub social: bool,
    /// The ocean is the living population's own genomes, resampled every epoch.
    pub genome_ocean: bool,
    /// Percent of mutations drawn from an order-2 model of genomes that persisted, instead
    /// of uniformly. 0 = off.
    pub learned_mut: u32,
    /// Baseline v4: a guess earns only the bits it gets beyond the ocean's best constant
    /// guess, the score is paid in energy at divide (none lost if the mother dies before the
    /// payout), and the founder starts at a random place in the ocean.
    pub v4: bool,
    /// Order-2 memory: `dive` leaves two-byte contexts in dx and cx, and `load`/`store` index
    /// a 4 KB memory by a hash of them, so the 4-op learner predicts from the last two bytes.
    pub mem2: bool,
    /// Frequency-dependent ocean (v4 + energy): the ocean is cut into this many streams,
    /// each XORed with its own key so what is learned in one is useless in another. Each
    /// pays a fixed share of the score budget, priced by how much was scored there last
    /// epoch, so a crowded stream pays little and an empty one a lot. One child in 16
    /// moves to a random stream. 0 = off.
    pub streams: u32,
    /// v4 debt: a cycle's net ocean score below zero is carried (and inherited) instead of
    /// floored, so guessing worse than a constant guess can't pay off as a lottery on its
    /// lucky cycles. Part of `--v4` since 2026-10-16; older v4 runs have it off.
    pub debt: bool,
    /// Horizontal transfer (lean): a child cell may be copied from another living
    /// creature's body, not only the mother's. Free cells and child blocks still don't
    /// count, so junk-filled children stay impossible.
    pub hgt: bool,
    /// No copy rule at all: a creature may divide whenever it has a child block, and the
    /// child is whatever the block holds. Every copy "cheat" (junk padding, skipped cells,
    /// copying from anywhere) is allowed; heredity is up to the creatures.
    pub free_copy: bool,
    /// Energy economy: a child's body costs this much energy per cell, paid by the mother
    /// into the commons at divide, whatever the cells hold. A mother that can't pay
    /// can't divide yet. Prices corner-cutting instead of banning it. 0 = free.
    pub birth_cost: u64,
    /// v5 ecology, each switchable alone. Predation: `eat KEY` kills the creature whose body
    /// holds ax (or the next one ahead within `radius`) if KEY's complement is on its
    /// surface (first SURFACE cells), taking all its energy; one kill per cycle.
    pub predation: bool,
    /// Local world: children go near the mother, and room is made by reaping a neighbour.
    /// 1: only within `radius` (a founding colony could fill its patch and kill its own
    /// kin while empty soup lay beyond); 2: look 1, 4 and 16 radii out before reaping.
    pub local: u8,
    /// Every child crosses over with the nearest creature of about its size (within 1/8).
    pub sex: bool,
    /// The neighbour reaped for room is the poorest, not the oldest.
    pub reap_poor: bool,
    /// How far, in cells, births, reaping, mates and prey reach under v5.
    pub radius: u32,
}

/// Size of the order-2 memory, and where a two-byte context lands in it.
const MEM2: usize = 1 << 12;

fn mem2_index(ctx: u32) -> usize {
    (((ctx >> 8 & 255) * 0x9E3) ^ ((ctx & 255) * 0x1F)) as usize & (MEM2 - 1)
}

/// Bytes of genome in the genome ocean each epoch: about half a typical population.
const GENOME_OCEAN: usize = 1 << 16;

fn on_all(predation: bool, ops: &mut Vec<u8>) {
    if predation {
        ops.push(EAT);
    }
}

/// The ops mutation can produce and creatures can run: the Tierra core, plus each switched-on
/// feature's own ops. Without `lean`, all of them.
pub fn alphabet(p: &Params, has_ocean: bool) -> Vec<u8> {
    if !p.lean {
        // Every op from before predation existed, so old runs keep their alphabet.
        let mut all: Vec<u8> = (0..=JOIN).collect();
        on_all(p.predation, &mut all);
        return all;
    }
    let mut ops: Vec<u8> = (0..=SENSE).chain([NOPX]).collect();
    let on = |f: bool, xs: &[u8], ops: &mut Vec<u8>| if f { ops.extend_from_slice(xs) };
    on(p.tasks, &[NAND, IO], &mut ops);
    on(has_ocean || p.genome_ocean || p.reef, &[DIVE, GUESS], &mut ops);
    on(p.memory, &[LOAD, STORE], &mut ops);
    on(p.infect, &[INFECT], &mut ops);
    on(p.reef, &[EMIT, TUNE], &mut ops);
    on(p.regime == Regime::Evolved, &[ENCLOSE, LEVY, REVOLT], &mut ops);
    on(p.social, &[MATE, LEARN, TEACH, JOIN], &mut ops);
    on(p.social && p.energy, &[GIVE], &mut ops);
    on(p.predation && p.energy, &[EAT], &mut ops);
    ops
}

impl Default for Params {
    fn default() -> Self {
        Params {
            soup_bits: 18,
            copy_error: 1000,
            cosmic_base: 60_000,
            fill: 0.8,
            min_size: 12,
            max_size: 512,
            search_limit: 1024,
            sun_bonus: 2,
            tasks: true,
            indel: 20,
            max_merit: 1024,
            max_age: 100,
            ocean_div: 1,
            ocean_cap: 32,
            memory: true,
            infect: true,
            infect_radius: 2048,
            reef: true,
            energy: false,
            energy_total: 64_000_000,
            regime: Regime::Commons,
            lean: true,
            social: false,
            genome_ocean: false,
            learned_mut: 0,
            v4: false,
            mem2: false,
            streams: 0,
            debt: false,
            hgt: false,
            free_copy: false,
            birth_cost: 0,
            predation: false,
            local: 0,
            sex: false,
            reap_poor: false,
            radius: 1024,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Weather {
    pub sun_center: u32,
    pub sun_half: u32,
    pub cosmic_interval: u64,
    /// Bit t set: task t pays right now.
    pub rewarded: u16,
    /// Which of the two goal sets is live (modularly varying goals, Kashtan & Alon 2005).
    pub goal: u8,
    /// Seeds the numbers creatures read with `io`.
    pub data: u64,
}

/// echo, not, nand and equ always pay; the middle tasks swap between two goal sets.
const GOALS: [u16; 2] = [0b11_1001_0111, 0b11_0110_1011];

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Creature {
    pub id: u64,
    pub alive: bool,
    pub start: u32,
    pub size: u32,
    pub child: Option<(u32, u32)>,
    pub child_writes: u32,
    /// Which cells of the child have been written (lean only counts each cell once).
    pub child_seen: [u64; 8],
    pub ax: u32,
    pub bx: u32,
    pub cx: u32,
    pub dx: u32,
    pub ip: u32,
    pub sp: u8,
    pub stack: [u32; STACK],
    pub birth: u64,
    pub errors: u32,
    pub genotype: u64,
    /// Set when this creature executes code outside its own body this cycle.
    pub ran_foreign: bool,
    /// Set when this creature used `sense` this cycle.
    pub sensed: bool,
    /// Speed multiplier earned by the tasks of the parent's last cycle.
    pub merit: u32,
    /// Tasks done since the last divide, and in the last full cycle.
    pub tasks_now: u16,
    pub tasks_last: u16,
    /// Already infected someone since the last birth.
    infected: bool,
    inputs: [u32; 3],
    reads: u32,
    /// Where this lineage is in the ocean, its pending guess, and how it did.
    pub pos: u32,
    guess: u8,
    has_guess: bool,
    pub hits: u32,
    pub scored: u32,
    pub hits_last: u32,
    pub scored_last: u32,
    /// Sum over guesses of (bits matched - 4): 0 for random guessing, +4 for an exact byte.
    pub bits: i32,
    pub bits_last: i32,
    /// Instructions executed in this life.
    age: u64,
    /// Shared per-genotype table of where each jump in the genome lands, and whether this
    /// body has changed since birth (then the table no longer describes it).
    #[serde(skip)]
    cache: Option<JumpTable>,
    dirty: bool,
    /// Once dirty, the creature's own jump table, valid until its body next changes.
    #[serde(skip)]
    pcache: Vec<u32>,
    #[serde(skip)]
    pcache_ok: bool,
    /// Private memory, 256 bytes once in use. Children start with a copy of their
    /// mother's, so what one generation learns the next begins with.
    pub mem: Vec<u8>,
    /// The last byte this lineage read from the ocean, and the one before (order-2 memory).
    prev: u8,
    prev2: u8,
    /// Unpaid ocean score (zero or negative), carried across cycles and to children.
    debt: i32,
    /// Has eaten this cycle (predation allows one kill per cycle).
    ate: bool,
    /// Diving the local reef instead of the ocean, where in it, and what emitting has earned.
    reef_mode: bool,
    reef_pos: u64,
    pub emit_bits: i32,
    /// Energy balance, and the score earned this epoch that sets its share of the payout.
    pub energy: u64,
    score: u64,
    idle: u32,
    /// The mother's id, for inheritance.
    pub parent: u64,
    /// Chosen partner for the next birth (slot, id), and alliance (0 = none). Alliances are
    /// not inherited: inherited ones grew into one pool of everybody, which pays nobody
    /// for doing better and stops selection cold. Each life has to join for itself.
    mate: Option<(u32, u64)>,
    pub group: u64,
    reap_score: i64,
    /// Which cells of its own body this creature has executed, for the gene meter.
    pub ran: [u64; 8],
}

impl Creature {
    /// Whether its body has changed since birth, so it no longer matches its genotype.
    pub fn dirty_body(&self) -> bool {
        self.dirty
    }

    fn owns(&self, a: u32) -> bool {
        (a >= self.start && a < self.start + self.size)
            || matches!(self.child, Some((s, z)) if a >= s && a < s + z)
    }
}

/// Bytes each region's reef remembers.
const REEF_CAP: usize = 4096;
const REGIONS: u32 = 16;

#[derive(Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct Emitter {
    slot: u32,
    id: u64,
    genotype: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Reef {
    data: Vec<u8>,
    from: Vec<Emitter>,
    written: u64,
}

/// Offset of each jump's landing spot within the genome, with the template's length in the
/// top byte; UNKNOWN until first worked out.
type JumpTable = Rc<RefCell<Vec<u32>>>;
const UNKNOWN: u32 = u32::MAX;

#[derive(serde::Serialize, serde::Deserialize)]
pub enum Event {
    Birth {
        mother: u64,
        /// The mother's body as it is now. Cosmic rays can change it after birth.
        mother_now: u64,
        child: u64,
        child_size: u32,
        foreign: bool,
        sensed: bool,
        tasks: u16,
        /// The child's genome, for mutants only (empty when it bred true).
        code: Vec<u8>,
    },
    Death {
        genotype: u64,
        /// Cells of its own body it executed, all zero if its body had changed.
        ran: [u64; 8],
    },
}

/// The ocean's bytes: a file mapped read-only, so worlds reading the same file share one
/// copy in the page cache, or a buffer the world owns (the genome ocean, tests).
pub enum Ocean {
    Owned(Vec<u8>),
    Mapped(memmap2::Mmap),
}

impl Default for Ocean {
    fn default() -> Self {
        Ocean::Owned(Vec::new())
    }
}

impl std::ops::Deref for Ocean {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match self {
            Ocean::Owned(v) => v,
            Ocean::Mapped(m) => m,
        }
    }
}

impl From<Vec<u8>> for Ocean {
    fn from(v: Vec<u8>) -> Self {
        Ocean::Owned(v)
    }
}

impl Ocean {
    /// Map a file. Oceans are never written while a run reads them.
    pub fn map(path: &str) -> std::io::Result<Self> {
        let f = std::fs::File::open(path)?;
        // SAFETY: the mapping is read-only and the file is not modified during a run.
        Ok(Ocean::Mapped(unsafe { memmap2::Mmap::map(&f)? }))
    }

    /// The owned buffer, replacing a mapping with an empty one.
    fn owned(&mut self) -> &mut Vec<u8> {
        if let Ocean::Mapped(_) = self {
            *self = Ocean::default();
        }
        match self {
            Ocean::Owned(v) => v,
            Ocean::Mapped(_) => unreachable!(),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct World {
    pub p: Params,
    /// What mutation draws from, and the same set as a bitmask for execution.
    ops: Vec<u8>,
    enabled: u64,
    /// The same as `enabled`, one flag per byte value, for the per-step check.
    #[serde(skip)]
    op_ok: Vec<bool>,
    /// SIMD finders for every template complement, forward and backward, indexed by
    /// `finder_index`. Rebuilt after loading.
    #[serde(skip)]
    finders: Vec<(memchr::memmem::Finder<'static>, memchr::memmem::FinderRev<'static>)>,
    pub n: u32,
    mask: u32,
    pub soup: Vec<u8>,
    /// slot + 1 of whoever holds each cell, 0 if free.
    owner: Vec<u32>,
    occ: Vec<u64>,
    #[serde(skip)]
    jump_tables: HashMap<u64, Weak<RefCell<Vec<u32>>>>,
    reefs: Vec<Reef>,
    region: u32,
    pub cs: Vec<Creature>,
    free_slots: Vec<usize>,
    reaper: BTreeSet<(i64, u64, usize)>,
    pub alive: usize,
    used: u64,
    pub rng: Rng,
    pub weather: Weather,
    pub instr: u64,
    next_cosmic: u64,
    birth_seq: u64,
    next_id: u64,
    rr: usize,
    pub events: Vec<Event>,
    pub stats: Stats,
    indel_hit: bool,
    /// Read-only real-world data the creatures can dive into. Empty means no ocean.
    #[serde(skip)]
    pub ocean: Ocean,
    /// The commons: energy not held by any creature.
    pub pool: u64,
    /// Per region: who holds the title (lord or deed holder) and the lease price per cell.
    titles: Vec<Option<(usize, u64)>>,
    prices: Vec<u64>,
    /// Rent rate each region's holder has levied, percent (evolved regime).
    levies: Vec<u64>,
    /// Population mean merit x1000, refreshed each epoch. Merit buys a share of CPU, not an
    /// absolute amount (as in Avida): absolute slices let one lineage stretch a round to
    /// dozens of epochs, outlive its own age limit every turn, and stall the world.
    merit_mean_k: u64,
    /// Learned mutation: counts of op c after ops a, b in persistent genomes, at
    /// [(a * 64 + b) * 64 + c]. Empty when off.
    model: Vec<u32>,
    pub model_total: u64,
    /// v4: the ocean's best constant guess (bitwise majority byte), which guesses are
    /// scored against; energy paid per unit of score, x2^16, set at each payout; and score
    /// earned since the last payout.
    #[serde(skip)]
    base_guess: u8,
    score_rate: u64,
    score_earned: u64,
    stream_rate: Vec<u64>,
    stream_earned: Vec<u64>,
}

/// Where scratch (nopx) cells go, per epoch. Reset by the census.
#[derive(Clone, Copy, Default, Debug, serde::Serialize, serde::Deserialize)]
pub struct Stats {
    pub births: u64,
    /// nopx carried by mothers at the moment they gave birth, summed.
    pub mother_nopx: u64,
    /// nopx lost or gained between mother and child, by cause.
    pub lost_resize: u64,
    pub lost_indel: u64,
    pub lost_point: u64,
    pub gained: u64,
    pub resized: u64,
    pub infections: u64,
    pub emits: u64,
    pub reef_reads: u64,
    pub kin_reads: u64,
    /// Energy paid as rent (feudal dues or market leases), and tenants evicted.
    pub rent: u64,
    pub evictions: u64,
    pub enclosures: u64,
    pub revolts_won: u64,
    pub revolts_lost: u64,
    pub gifts: u64,
    pub gifted: u64,
    pub matings: u64,
    pub lessons: u64,
    pub joins: u64,
    /// Deaths by cause. Evictions are counted separately above.
    pub died_old: u64,
    pub died_starved: u64,
    pub died_reaped: u64,
    pub died_eaten: u64,
}

pub fn genotype_of(code: &[u8]) -> u64 {
    u64::from_le_bytes(blake3::hash(code).as_bytes()[..8].try_into().unwrap())
}

impl World {
    pub fn new(p: Params, seed: u64, ocean: impl Into<Ocean>) -> Self {
        let ocean: Ocean = ocean.into();
        let n = 1u32 << p.soup_bits;
        let mut rng = Rng::new(seed);
        // The soup starts as noise, not zeros, so there's leftover junk to stumble into.
        let ops = alphabet(&p, !ocean.is_empty());
        let enabled = ops.iter().fold(0u64, |m, &o| m | 1 << o);
        let soup = (0..n).map(|_| ops[rng.below(ops.len() as u64) as usize]).collect();
        let weather = Weather { sun_center: 0, sun_half: n / 16, cosmic_interval: p.cosmic_base, rewarded: GOALS[0], goal: 0, data: 0 };
        World {
            ops,
            enabled,
            op_ok: Vec::new(),
            finders: Vec::new(),
            n,
            mask: n - 1,
            soup,
            owner: vec![0; n as usize],
            occ: vec![0; (n as usize).div_ceil(64)],
            jump_tables: HashMap::new(),
            reefs: (0..REGIONS).map(|_| Reef { data: vec![0; REEF_CAP], from: vec![Emitter::default(); REEF_CAP], written: 0 }).collect(),
            region: n / REGIONS,
            cs: Vec::new(),
            free_slots: Vec::new(),
            reaper: BTreeSet::new(),
            alive: 0,
            used: 0,
            next_cosmic: p.cosmic_base,
            rng,
            weather,
            instr: 0,
            birth_seq: 0,
            next_id: 0,
            rr: 0,
            events: Vec::new(),
            stats: Stats::default(),
            indel_hit: false,
            pool: if p.energy { p.energy_total } else { 0 },
            titles: vec![None; REGIONS as usize],
            prices: vec![1; REGIONS as usize],
            levies: vec![0; REGIONS as usize],
            ocean,
            merit_mean_k: 1000,
            model: if p.learned_mut > 0 { vec![0; 1 << 18] } else { Vec::new() },
            model_total: 0,
            base_guess: 0,
            score_rate: 0,
            score_earned: 0,
            stream_rate: vec![0; p.streams as usize],
            stream_earned: vec![0; p.streams as usize],
            p,
        }
        .with_base_guess()
    }

    /// Which stream an ocean position lies in, when the ocean is split into streams.
    fn stream_of(&self, pos: u32) -> Option<usize> {
        let k = self.stream_rate.len() as u64;
        let n = self.ocean.len() as u64;
        (k > 0 && n > 0).then(|| ((pos as u64 % n) * k / n) as usize)
    }

    fn stream_key(&self, pos: u32) -> u8 {
        self.stream_of(pos).map(|k| (k as u8).wrapping_mul(157)).unwrap_or(0)
    }

    /// For the viewer: each living creature's span, low genotype bits, and flags (1: its
    /// genome holds both `dive` and `guess`, 2: it guessed last cycle, 4: it is broke).
    pub fn frame_spans(&self) -> Vec<(u32, u16, u16, u8)> {
        self.cs
            .iter()
            .filter(|c| c.alive)
            .map(|c| {
                let body = &self.soup[c.start as usize..(c.start + c.size) as usize];
                let learner = body.contains(&DIVE) && body.contains(&GUESS);
                let flags = learner as u8 | ((c.scored_last >= 4) as u8) << 1 | ((self.p.energy && c.energy == 0) as u8) << 2;
                (c.start, c.size.min(u16::MAX as u32) as u16, c.genotype as u16, flags)
            })
            .collect()
    }

    /// Streams with at least one creature that guessed there last cycle.
    pub fn streams_used(&self) -> u32 {
        let mut seen = vec![false; self.stream_rate.len()];
        for c in self.cs.iter().filter(|c| c.alive && c.scored_last >= 4) {
            if let Some(k) = self.stream_of(c.pos) {
                seen[k] = true;
            }
        }
        seen.iter().filter(|&&b| b).count() as u32
    }

    fn with_base_guess(mut self) -> Self {
        self.set_base_guess();
        self.build_tables();
        self
    }

    /// Lookup tables derived from the parameters, rebuilt rather than saved.
    fn build_tables(&mut self) {
        self.op_ok = (0..256u32).map(|op| op < 64 && self.enabled & 1 << op != 0).collect();
        self.finders = (1..=MAX_TEMPLATE)
            .flat_map(|len| (0..1u32 << len).map(move |bits| (len, bits)))
            .map(|(len, bits)| {
                let pat: Vec<u8> = (0..len).map(|i| (((bits >> i) & 1) ^ 1) as u8).collect();
                (memchr::memmem::Finder::new(&pat).into_owned(), memchr::memmem::FinderRev::new(&pat).into_owned())
            })
            .collect();
    }

    fn set_base_guess(&mut self) {
        if !self.p.v4 || self.ocean.is_empty() {
            return;
        }
        let mut hist = [0u64; 256];
        for &b in self.ocean.iter() {
            hist[b as usize] += 1;
        }
        let mut ones = [0u64; 8];
        for (v, &n) in hist.iter().enumerate() {
            for (k, o) in ones.iter_mut().enumerate() {
                *o += (v >> k & 1) as u64 * n;
            }
        }
        let n = self.ocean.len() as u64;
        self.base_guess = (0..8).fold(0u8, |g, k| g | (((ones[k] * 2 > n) as u8) << k));
    }

    /// Weather comes from the feed: where the sun is, how wide, how hard the cosmic rays hit.
    pub fn set_weather(&mut self, b: &[u8; 32]) {
        let n = self.n;
        self.weather.sun_center = u32::from_le_bytes(b[0..4].try_into().unwrap()) & self.mask;
        self.weather.sun_half = n / 32 + (b[4] as u32 * (n / 16)) / 255;
        let shift = (b[5] % 5) as i32 - 2;
        self.weather.cosmic_interval = if shift >= 0 {
            self.p.cosmic_base << shift
        } else {
            self.p.cosmic_base >> -shift
        };
        // Goals switch rarely, about one epoch in sixteen, so lineages have time to adapt.
        if b[6] < 16 {
            self.weather.goal ^= 1;
        }
        self.weather.rewarded = GOALS[self.weather.goal as usize];
        self.weather.data = u64::from_le_bytes(b[8..16].try_into().unwrap());
        self.rng.mix(b);
        if self.p.energy {
            self.payout();
        }
        self.jump_tables.retain(|_, t| t.strong_count() > 0);
        let alive: Vec<_> = self.cs.iter().filter(|c| c.alive).collect();
        if !alive.is_empty() {
            // The yardstick for CPU share: mean merit, or in the energy economy mean wealth.
            let total: u64 = if self.p.energy {
                alive.iter().map(|c| c.energy).sum()
            } else {
                alive.iter().map(|c| c.merit.max(1) as u64).sum()
            };
            self.merit_mean_k = (total * 1000 / alive.len() as u64).max(1);
        }
    }

    /// After loading a snapshot: reattach the ocean and rebuild the per-genotype jump
    /// tables, which are a cache and never saved.
    /// The genome ocean: what creatures dive into is a sample of the living population's
    /// own code, rebuilt every epoch from a random slot onward. Predicting it means
    /// modelling your neighbours, and it changes whenever they do.
    pub fn refresh_genome_ocean(&mut self) {
        self.ocean.owned().clear();
        let n = self.cs.len();
        if n == 0 {
            return;
        }
        let first = self.rng.below(n as u64) as usize;
        for k in 0..n {
            let c = &self.cs[(first + k) % n];
            if c.alive {
                self.ocean.owned().extend_from_slice(&self.soup[c.start as usize..(c.start + c.size) as usize]);
                if self.ocean.len() >= GENOME_OCEAN {
                    break;
                }
            }
        }
    }

    /// Replace the ocean's bytes with new data (e.g. from network telemetry).
    pub fn replace_ocean(&mut self, data: Vec<u8>) {
        *self.ocean.owned() = data;
        self.set_base_guess();
    }

    pub fn after_load(&mut self, ocean: impl Into<Ocean>) {
        self.ocean = ocean.into();
        self.set_base_guess();
        self.build_tables();
        for i in 0..self.cs.len() {
            if !self.cs[i].alive {
                continue;
            }
            let (g, size) = (self.cs[i].genotype, self.cs[i].size);
            let table = match self.jump_tables.get(&g).and_then(Weak::upgrade) {
                Some(t) if t.borrow().len() == size as usize => t,
                _ => {
                    let t = Rc::new(RefCell::new(vec![UNKNOWN; size as usize]));
                    self.jump_tables.insert(g, Rc::downgrade(&t));
                    t
                }
            };
            self.cs[i].cache = Some(table);
        }
    }

    /// Copies of `k` living genomes picked at random, to send to another island.
    pub fn emigrants(&mut self, k: usize) -> Vec<Vec<u8>> {
        let alive: Vec<usize> = (0..self.cs.len()).filter(|&i| self.cs[i].alive).collect();
        if alive.is_empty() {
            return Vec::new();
        }
        (0..k)
            .map(|_| {
                let c = &self.cs[alive[self.rng.below(alive.len() as u64) as usize]];
                self.soup[c.start as usize..(c.start + c.size) as usize].to_vec()
            })
            .collect()
    }

    /// A migrant from another island lands somewhere free, as a newborn with no merit.
    pub fn immigrate(&mut self, code: &[u8]) {
        let size = code.len() as u32;
        if size < self.p.min_size || size > self.p.max_size {
            return;
        }
        let limit = (self.p.fill * self.n as f64) as u64;
        while self.used + size as u64 > limit {
            if !self.reap_one(usize::MAX) {
                break;
            }
        }
        let from = self.rng.below(self.n as u64) as u32;
        if let Some(at) = self.find_free(from, size, self.n) {
            self.soup[at as usize..(at + size) as usize].copy_from_slice(code);
            let g = genotype_of(code);
            let slot = self.spawn(at, size, g, 1, 0);
            self.endow(slot, 10_000);
            self.events.push(Event::Birth { mother: g, mother_now: g, child: g, child_size: size, foreign: false, sensed: false, tasks: 0, code: Vec::new() });
        }
    }

    /// Half the commons is paid out each epoch. Half of that is what the land yields: split
    /// by the cells each creature occupies, enough to subsist on. The other half is split by
    /// the score each creature earned. Integer shares round down; the remainder stays put.
    fn payout(&mut self) {
        let budget = self.pool / 2;
        let scores: u128 = self.cs.iter().filter(|c| c.alive).map(|c| c.score as u128).sum();
        let land: u128 = self.cs.iter().filter(|c| c.alive).map(|c| c.size as u128).sum();
        let by_score = if scores > 0 { budget / 2 } else { 0 };
        let mut by_land = budget - by_score;
        if self.p.v4 {
            // Score is paid at divide instead. Half the budget is set aside for it, priced
            // so last epoch's score would have used it up; the pool covers any overrun. One
            // unit never buys more than one creature's share, or a lone lucky guess in a
            // world of non-guessers would take the whole half.
            by_land = budget - budget / 2;
            let alive = self.cs.iter().filter(|c| c.alive).count() as u64;
            self.score_rate = ((budget / 2) as u128 * 65536 / self.score_earned.max(alive).max(1) as u128) as u64;
            self.score_earned = 0;
            let k = self.stream_rate.len() as u64;
            for i in 0..self.stream_rate.len() {
                let floor = (alive / k).max(1);
                self.stream_rate[i] = ((budget / 2 / k) as u128 * 65536 / self.stream_earned[i].max(floor) as u128) as u64;
                self.stream_earned[i] = 0;
            }
        }
        let mut credit = vec![0u64; self.cs.len()];
        for i in 0..self.cs.len() {
            if !self.cs[i].alive {
                continue;
            }
            let c = &self.cs[i];
            let mut share = 0u64;
            if land > 0 {
                share += (by_land as u128 * c.size as u128 / land) as u64;
            }
            if scores > 0 {
                share += (by_score as u128 * c.score as u128 / scores) as u64;
            }
            self.pool -= share;
            // Under feudalism a customary share of what each producer earns goes to the
            // lord of the land it lives on, by right rather than by contract.
            let r = (c.start / self.region) as usize;
            let lord = match self.p.regime {
                Regime::Feudal => self.lord_of(r),
                Regime::Evolved => self.holder_of(r),
                _ => None,
            };
            let pct = if self.p.regime == Regime::Evolved { self.levies[r] } else { FEUDAL_RENT_PCT };
            match lord {
                Some(l) if l != i => {
                    let rent = share * pct / 100;
                    self.cs[l].energy += rent;
                    credit[i] += share - rent;
                    self.stats.rent += rent;
                }
                _ => credit[i] += share,
            }
        }
        // Alliances pool what their members earned and split it evenly; the remainder of
        // the division goes to the first member, so nothing is lost.
        let mut groups: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
        for i in 0..self.cs.len() {
            if self.cs[i].alive && self.cs[i].group != 0 {
                groups.entry(self.cs[i].group).or_default().push(i);
            }
        }
        for members in groups.values().filter(|m| m.len() > 1) {
            let total: u64 = members.iter().map(|&i| credit[i]).sum();
            let each = total / members.len() as u64;
            for &i in members {
                credit[i] = each;
            }
            credit[members[0]] += total - each * members.len() as u64;
        }
        for (i, c) in credit.into_iter().enumerate() {
            self.cs[i].energy += c;
        }
        for c in self.cs.iter_mut() {
            c.score = 0;
        }
        if self.p.regime == Regime::Capitalist {
            // Yield per cell this epoch, in thousandths of a unit.
            let yield_k = if land > 0 { (by_land as u128 * 1000 / land) as u64 } else { 0 };
            self.leases(yield_k);
        }
    }

    /// Who holds a region in the evolved regime. A dead holder's title passes to its eldest
    /// living child in the region; with no heir the land returns to the commons.
    fn holder_of(&mut self, r: usize) -> Option<usize> {
        let (slot, id) = self.titles[r]?;
        if self.cs[slot].alive && self.cs[slot].id == id {
            return Some(slot);
        }
        let heir = (0..self.cs.len())
            .filter(|&i| self.cs[i].alive && self.cs[i].parent == id && (self.cs[i].start / self.region) as usize == r)
            .min_by_key(|&i| self.cs[i].id);
        self.titles[r] = heir.map(|i| (i, self.cs[i].id));
        if heir.is_none() {
            self.levies[r] = 0;
        }
        heir
    }

    /// Property and politics as instructions (evolved regime only). Returns false on failure.
    fn politics(&mut self, c: &mut Creature, slot: usize, op: u8) -> bool {
        let r = (c.start / self.region) as usize;
        // The running creature is lifted out of `cs` while it runs, so its own title has to
        // be recognised here; asking holder_of would see an empty seat and dissolve it.
        let own = self.titles[r] == Some((slot, c.id));
        let holder = if own { Some(slot) } else { self.holder_of(r) };
        match op {
            ENCLOSE => {
                if holder.is_some() || c.energy < ENCLOSE_FEE {
                    return false;
                }
                c.energy -= ENCLOSE_FEE;
                self.pool += ENCLOSE_FEE;
                self.titles[r] = Some((slot, c.id));
                self.levies[r] = 0;
                self.stats.enclosures += 1;
            }
            LEVY => {
                if !own {
                    return false;
                }
                self.levies[r] = (c.cx as u64 % 101).min(MAX_LEVY);
            }
            REVOLT => {
                let Some(h) = holder else { return false };
                if own {
                    return false;
                }
                if c.energy > self.cs[h].energy {
                    self.titles[r] = Some((slot, c.id));
                    self.stats.revolts_won += 1;
                } else {
                    let cost = c.energy / 2;
                    c.energy -= cost;
                    self.cs[h].energy += cost;
                    self.stats.revolts_lost += 1;
                }
            }
            _ => return false,
        }
        true
    }

    /// Sex, culture and alliance. The neighbour is whoever owns the cell ax points at.
    fn social(&mut self, c: &mut Creature, slot: usize, op: u8) -> bool {
        let o = self.owner[(c.ax & self.mask) as usize];
        if o == 0 || o as usize - 1 == slot {
            return false;
        }
        let n = o as usize - 1;
        let d = c.start.wrapping_sub(self.cs[n].start) & self.mask;
        if d.min(self.n - d) > self.p.infect_radius {
            return false;
        }
        match op {
            MATE => {
                c.mate = Some((n as u32, self.cs[n].id));
                self.stats.matings += 1;
            }
            LEARN | TEACH => {
                // 16 bytes of memory, at the block cx picks. Learning costs the learner 16
                // energy, paid to the teacher; teaching is free.
                let k = (c.cx as usize & 0xF0).min(240);
                let (from, to): (Vec<u8>, bool) = if op == LEARN {
                    if self.cs[n].mem.is_empty() || (self.p.energy && c.energy < 16) {
                        return false;
                    }
                    (self.cs[n].mem[k..k + 16].to_vec(), true)
                } else {
                    if c.mem.is_empty() {
                        return false;
                    }
                    (c.mem[k..k + 16].to_vec(), false)
                };
                if to {
                    if c.mem.is_empty() {
                        c.mem = vec![0; 256];
                    }
                    c.mem[k..k + 16].copy_from_slice(&from);
                    if self.p.energy {
                        c.energy -= 16;
                        self.cs[n].energy += 16;
                    }
                } else {
                    let m = &mut self.cs[n].mem;
                    if m.is_empty() {
                        *m = vec![0; 256];
                    }
                    m[k..k + 16].copy_from_slice(&from);
                }
                self.stats.lessons += 1;
            }
            JOIN => {
                if !self.p.energy {
                    return false;
                }
                if self.cs[n].group == 0 {
                    self.cs[n].group = self.cs[n].id;
                }
                c.group = self.cs[n].group;
                self.stats.joins += 1;
            }
            _ => return false,
        }
        true
    }

    /// Sexual reproduction: from a random point on, the child's cells come from the partner's
    /// genome instead of the mother's (where the partner is long enough). A partner who has
    /// died by now contributes nothing.
    fn crossover(&mut self, s: u32, z: u32, ps: usize, pid: u64) {
        let Some(p) = self.cs.get(ps).filter(|p| p.alive && p.id == pid) else { return };
        let (pstart, plen) = (p.start, p.size);
        let overlap = z.min(plen);
        if overlap < 2 {
            return;
        }
        let cut = 1 + self.rng.below(overlap as u64 - 1) as u32;
        let genes: Vec<u8> = self.soup[(pstart + cut) as usize..(pstart + overlap) as usize].to_vec();
        self.soup[(s + cut) as usize..(s + overlap) as usize].copy_from_slice(&genes);
    }

    /// A gift: cx units of energy to whoever owns the cell ax points at. Nothing compels a
    /// return; anything like trade has to evolve on top of it.
    fn give(&mut self, c: &mut Creature, slot: usize) -> bool {
        let o = self.owner[(c.ax & self.mask) as usize];
        if o == 0 || o as usize - 1 == slot {
            return false;
        }
        let amount = (c.cx as u64).min(c.energy);
        if amount == 0 {
            return false;
        }
        c.energy -= amount;
        self.cs[o as usize - 1].energy += amount;
        self.stats.gifts += 1;
        self.stats.gifted += amount;
        true
    }

    /// Who holds a region under feudalism. The first settler founds the line; when a lord
    /// dies the title passes to its eldest living child in the region, and failing that the
    /// richest resident seizes it.
    fn lord_of(&mut self, r: usize) -> Option<usize> {
        if let Some((slot, id)) = self.titles[r] {
            if self.cs[slot].alive && self.cs[slot].id == id {
                return Some(slot);
            }
        }
        let old = self.titles[r].map(|t| t.1);
        let residents = (0..self.cs.len()).filter(|&i| self.cs[i].alive && (self.cs[i].start / self.region) as usize == r);
        let heir = residents
            .clone()
            .filter(|&i| old.is_some() && self.cs[i].parent == old.unwrap())
            .min_by_key(|&i| self.cs[i].id)
            .or_else(|| residents.max_by_key(|&i| (self.cs[i].energy, std::cmp::Reverse(self.cs[i].id))));
        self.titles[r] = heir.map(|i| (i, self.cs[i].id));
        heir
    }

    /// Capitalist land: every resident pays a market rent per cell to the region's deed
    /// holder, or is evicted. The price follows demand. A resident who can afford the deed
    /// buys it from the holder. Every movement is a transfer of energy that already exists.
    fn leases(&mut self, yield_k: u64) {
        let cells = self.region as u64;
        let occupied: u64 = self.cs.iter().filter(|c| c.alive).map(|c| c.size as u64).sum();
        let settled = (0..self.titles.len()).filter(|&r| self.cs.iter().any(|c| c.alive && (c.start / self.region) as usize == r)).count() as u64;
        let mean = (occupied / settled.max(1)).max(1);
        for r in 0..self.titles.len() {
            let residents: Vec<usize> = (0..self.cs.len()).filter(|&i| self.cs[i].alive && (self.cs[i].start / self.region) as usize == r).collect();
            let used: u64 = residents.iter().map(|&i| self.cs[i].size as u64).sum();
            // Ground rent: half of what a cell yields, scaled by how sought-after this region
            // is relative to the average. Crowded land costs more; empty land is cheap.
            // Capped at 80% of the yield: rent comes out of what the land produces, never more.
            let price = (yield_k * used.min(mean * 8 / 5) / mean / 2000).max(1);
            self.prices[r] = price;
            let holder = match self.titles[r] {
                Some((slot, id)) if self.cs[slot].alive && self.cs[slot].id == id => Some(slot),
                _ => None,
            };
            let mut evicted = Vec::new();
            for &i in &residents {
                if Some(i) == holder {
                    continue;
                }
                let owed = self.cs[i].size as u64 * price;
                let paid = owed.min(self.cs[i].energy);
                self.cs[i].energy -= paid;
                match holder {
                    Some(h) => self.cs[h].energy += paid,
                    None => self.pool += paid,
                }
                self.stats.rent += paid;
                if paid < owed {
                    evicted.push(i);
                }
            }
            for i in evicted {
                self.stats.evictions += 1;
                self.kill(i);
            }
            // The deed costs twenty epochs of the region's full rent.
            let deed = price.saturating_mul(cells).saturating_mul(20).max(1);
            let buyer = (0..self.cs.len())
                .filter(|&i| self.cs[i].alive && (self.cs[i].start / self.region) as usize == r && Some(i) != holder && self.cs[i].energy > deed)
                .max_by_key(|&i| (self.cs[i].energy, std::cmp::Reverse(self.cs[i].id)));
            if let Some(b) = buyer {
                self.cs[b].energy -= deed;
                match holder {
                    Some(h) => self.cs[h].energy += deed,
                    None => self.pool += deed,
                }
                self.titles[r] = Some((b, self.cs[b].id));
            }
        }
    }

    /// Alliances: how many groups with two or more living members, and the largest.
    pub fn alliances(&self) -> (u32, u32) {
        let mut sizes: HashMap<u64, u32> = HashMap::new();
        for c in self.cs.iter().filter(|c| c.alive && c.group != 0) {
            *sizes.entry(c.group).or_default() += 1;
        }
        let real: Vec<u32> = sizes.into_values().filter(|&n| n > 1).collect();
        (real.len() as u32, real.iter().copied().max().unwrap_or(0))
    }

    /// Regions with a living title holder, and their mean levy (percent).
    pub fn titled(&self) -> (u32, f64) {
        let held: Vec<u64> = (0..self.titles.len())
            .filter(|&r| matches!(self.titles[r], Some((s, id)) if self.cs[s].alive && self.cs[s].id == id))
            .map(|r| self.levies[r])
            .collect();
        let mean = if held.is_empty() { 0.0 } else { held.iter().sum::<u64>() as f64 / held.len() as f64 };
        (held.len() as u32, mean)
    }

    /// Inequality of energy among the living: Gini coefficient and the richest one's share.
    pub fn wealth(&self) -> (f64, f64) {
        let mut e: Vec<u64> = self.cs.iter().filter(|c| c.alive).map(|c| c.energy).collect();
        let total: u128 = e.iter().map(|&x| x as u128).sum();
        if e.is_empty() || total == 0 {
            return (0.0, 0.0);
        }
        e.sort_unstable();
        let n = e.len() as f64;
        let weighted: f64 = e.iter().enumerate().map(|(i, &x)| (i as f64 + 1.0) * x as f64).sum();
        let gini = 2.0 * weighted / (n * total as f64) - (n + 1.0) / n;
        (gini, *e.last().unwrap() as f64 / total as f64)
    }

    /// Starting energy for a founder or migrant, taken from the commons.
    fn endow(&mut self, slot: usize, amount: u64) {
        if self.p.energy {
            let gift = amount.min(self.pool);
            self.pool -= gift;
            self.cs[slot].energy += gift;
        }
    }

    /// Energy held by creatures plus the commons. In the energy economy this never changes.
    pub fn ledger(&self) -> u64 {
        self.pool + self.cs.iter().filter(|c| c.alive).map(|c| c.energy).sum::<u64>()
    }

    pub fn inject(&mut self, code: &[u8], at: u32) -> u64 {
        let size = code.len() as u32;
        for (i, &op) in code.iter().enumerate() {
            self.soup[(at + i as u32) as usize] = op;
        }
        let g = genotype_of(code);
        let slot = self.spawn(at, size, g, 1, 0);
        self.endow(slot, 100_000);
        if self.p.v4 && !self.ocean.is_empty() {
            self.cs[slot].pos = self.rng.below(self.ocean.len() as u64) as u32;
        }
        g
    }

    fn spawn(&mut self, start: u32, size: u32, genotype: u64, merit: u32, tasks: u16) -> usize {
        let slot = self.free_slots.pop().unwrap_or_else(|| {
            self.cs.push(Creature::default());
            self.cs.len() - 1
        });
        self.claim(start, size, slot);
        self.used += size as u64;
        let table = match self.jump_tables.get(&genotype).and_then(Weak::upgrade) {
            Some(t) if t.borrow().len() == size as usize => t,
            _ => {
                let t = Rc::new(RefCell::new(vec![UNKNOWN; size as usize]));
                self.jump_tables.insert(genotype, Rc::downgrade(&t));
                t
            }
        };
        let c = Creature {
            id: self.next_id,
            alive: true,
            start,
            size,
            ip: start,
            birth: self.birth_seq,
            genotype,
            merit,
            tasks_last: tasks,
            reap_score: self.birth_seq as i64,
            cache: Some(table),
            ..Default::default()
        };
        self.reaper.insert((c.reap_score, c.id, slot));
        self.next_id += 1;
        self.birth_seq += 1;
        self.alive += 1;
        self.cs[slot] = c;
        slot
    }

    /// Mark cells as owned. `occ` mirrors `owner` one bit per cell so free space can be
    /// scanned 64 cells at a time.
    /// Whether cell `a` is inside the body (not the child block) of a living creature other
    /// than the one in `slot`.
    fn in_other_body(&self, a: u32, slot: usize) -> bool {
        let o = self.owner[a as usize] as usize;
        o != 0 && o - 1 != slot && {
            let d = &self.cs[o - 1];
            d.alive && a >= d.start && a < d.start + d.size
        }
    }

    fn claim(&mut self, start: u32, size: u32, slot: usize) {
        self.owner[start as usize..(start + size) as usize].fill(slot as u32 + 1);
        self.mark_occ(start, size, true);
    }

    fn release(&mut self, start: u32, size: u32) {
        self.owner[start as usize..(start + size) as usize].fill(0);
        self.mark_occ(start, size, false);
        self.used -= size as u64;
    }

    /// Set or clear the occupancy bits of cells `start..start + size`, a word at a time.
    fn mark_occ(&mut self, start: u32, size: u32, on: bool) {
        let (mut a, end) = (start, start + size);
        while a < end {
            let bit = a % 64;
            let n = (64 - bit).min(end - a);
            let m = if n == 64 { u64::MAX } else { ((1u64 << n) - 1) << bit };
            let w = &mut self.occ[(a / 64) as usize];
            if on {
                *w |= m;
            } else {
                *w &= !m;
            }
            a += n;
        }
    }

    /// Free cells and scramble them: dead bodies decay. Without this, whole populations
    /// came to run one dead creature's leftover copy loop, and when a newcomer was
    /// allocated over it every lineage failed at once and the world went extinct.
    fn random_op(&mut self) -> u8 {
        self.ops[self.rng.below(self.ops.len() as u64) as usize]
    }

    /// The op a mutation writes at `at`. With learned mutation on, some draws come from
    /// what follows the two cells before `at` in genomes that persisted, so mutants look
    /// like code that has worked (estimation of distribution). The rest stay uniform.
    fn mutant_op(&mut self, at: u32) -> u8 {
        if self.p.learned_mut > 0 && self.rng.below(100) < self.p.learned_mut as u64 {
            let (a, b) = (self.at(at.wrapping_sub(2)) as usize, self.at(at.wrapping_sub(1)) as usize);
            let row = &self.model[(a << 6 | b) << 6..][..64];
            let total: u64 = row.iter().map(|&n| n as u64).sum();
            if total > 0 {
                let mut r = self.rng.below(total);
                for (op, &n) in row.iter().enumerate() {
                    if r < n as u64 {
                        return op as u8;
                    }
                    r -= n as u64;
                }
            }
        }
        self.random_op()
    }

    /// Teach the mutation model genomes that just proved they persist. Old counts halve
    /// as new ones come in, so the model follows what works now.
    pub fn train(&mut self, genomes: &[Vec<u8>]) {
        if self.model.is_empty() {
            return;
        }
        for g in genomes {
            for w in g.windows(3) {
                self.model[((w[0] as usize & 63) << 6 | w[1] as usize & 63) << 6 | w[2] as usize & 63] += 1;
                self.model_total += 1;
            }
        }
        if self.model_total > 1 << 22 {
            self.model.iter_mut().for_each(|n| *n /= 2);
            self.model_total = self.model.iter().map(|&n| n as u64).sum();
        }
    }

    fn decay(&mut self, start: u32, size: u32) {
        self.release(start, size);
        // The same draws as one `random_op` per cell, in order.
        let (rng, ops) = (&mut self.rng, &self.ops);
        let n = ops.len() as u64;
        for cell in &mut self.soup[start as usize..(start + size) as usize] {
            *cell = ops[rng.below(n) as usize];
        }
    }

    fn kill(&mut self, slot: usize) {
        prof::time(prof::KILL, || self.kill_inner(slot))
    }

    fn kill_inner(&mut self, slot: usize) {
        let c = std::mem::take(&mut self.cs[slot]);
        self.reaper.remove(&(c.reap_score, c.id, slot));
        self.pool += c.energy;
        self.decay(c.start, c.size);
        if let Some((s, z)) = c.child {
            self.decay(s, z + self.slack());
        }
        self.alive -= 1;
        self.free_slots.push(slot);
        self.events.push(Event::Death { genotype: c.genotype, ran: if c.dirty { [0; 8] } else { c.ran } });
    }

    /// Kill whoever is at the top of the reaper queue, sparing `spare`.
    fn reap_one(&mut self, spare: usize) -> bool {
        let victim = self.reaper.iter().find(|k| k.2 != spare).map(|k| k.2);
        match victim {
            Some(v) => {
                self.stats.died_reaped += 1;
                self.kill(v);
                true
            }
            None => false,
        }
    }

    /// A free run of `size` cells starting within `window` cells after `from`.
    fn find_free(&self, from: u32, size: u32, window: u32) -> Option<u32> {
        prof::time(prof::FIND_FREE, || self.find_free_inner(from, size, window))
    }

    fn find_free_inner(&self, from: u32, size: u32, window: u32) -> Option<u32> {
        if size == 0 || self.n % 64 != 0 {
            return self.find_free_scan(from, size, window);
        }
        if size > 64 {
            return self.find_free_big(from, size, window);
        }
        // The same answer as the scan, a word at a time: the first cell, in scan order from
        // `from`, that starts `size` free cells, without crossing the end of the soup and
        // within `limit` scanned cells. Bit b of `starts` says cells b..b+size of this word
        // and the next are all free (each round doubles the run length it checks).
        let words = self.occ.len();
        let limit = window.min(self.n) as i64 + size as i64;
        let mut w = (from / 64) as usize;
        let mut base = -((from % 64) as i64);
        while base <= limit - size as i64 {
            let next = if w + 1 < words { !self.occ[w + 1] } else { 0 };
            let x = (next as u128) << 64 | !self.occ[w] as u128;
            let (mut r, mut m) = (x, 1u32);
            while m < size {
                let s = m.min(size - m);
                r &= r >> s;
                m += s;
            }
            let mut starts = r as u64;
            if base < 0 {
                starts &= !0u64 << (-base) as u32;
            }
            let last = limit - size as i64 - base;
            if last < 63 {
                starts &= (1u64 << (last + 1)) - 1;
            }
            if starts != 0 {
                return Some(w as u32 * 64 + starts.trailing_zeros());
            }
            base += 64;
            w = if w + 1 == words { 0 } else { w + 1 };
        }
        None
    }

    /// `find_free` for children over 64 cells: the scan's answer, one occupancy word per
    /// step. A run that long can't fit between two owned cells of one word, so only a
    /// word's leading free cells (finishing a run) and trailing free cells (starting one)
    /// matter.
    fn find_free_big(&self, from: u32, size: u32, window: u32) -> Option<u32> {
        let (n, mut i, mut run, mut run_start) = (self.n, from, 0u32, from);
        let limit = window.min(n) as u64 + size as u64;
        let mut scanned = 0u64;
        while scanned < limit {
            if i == n {
                i = 0;
                run = 0;
            }
            let bit = i % 64;
            let avail = (64 - bit as u64).min(limit - scanned).min((n - i) as u64) as u32;
            let mask = if avail == 64 { !0 } else { (1u64 << avail) - 1 };
            let w = (self.occ[(i / 64) as usize] >> bit) & mask;
            if w == 0 {
                if run == 0 {
                    run_start = i;
                }
                if run + avail >= size {
                    return Some(run_start);
                }
                run += avail;
            } else {
                if run + w.trailing_zeros() >= size {
                    return Some(run_start);
                }
                let top = 63 - w.leading_zeros();
                run = avail - 1 - top;
                run_start = i + top + 1;
            }
            i += avail;
            scanned += avail as u64;
        }
        None
    }

    /// Cell-run scan for `find_free`: the reference answer, used for soups that aren't a
    /// whole number of words.
    fn find_free_scan(&self, from: u32, size: u32, window: u32) -> Option<u32> {
        let (n, mut i, mut run, mut run_start) = (self.n, from, 0u32, from);
        let limit = window.min(n) as u64 + size as u64;
        let mut scanned = 0u64;
        while scanned < limit {
            if i == n {
                // Blocks never wrap around the end of the soup.
                i = 0;
                run = 0;
            }
            // Jump over a whole stretch of owned or free cells at once, read off the
            // occupancy bits. Same answer as a cell-by-cell scan, far fewer steps.
            let bit = i % 64;
            let w = self.occ[(i / 64) as usize] >> bit;
            let avail = (64 - bit as u64).min(limit - scanned).min((n - i) as u64) as u32;
            if w & 1 == 1 {
                let k = w.trailing_ones().min(avail);
                run = 0;
                i += k;
                scanned += k as u64;
            } else {
                let k = w.trailing_zeros().min(avail);
                if run == 0 {
                    run_start = i;
                }
                if run + k >= size {
                    return Some(run_start);
                }
                run += k;
                i += k;
                scanned += k as u64;
            }
        }
        None
    }

    fn in_sun(&self, a: u32) -> bool {
        let d = a.wrapping_sub(self.weather.sun_center) & self.mask;
        d.min(self.n - d) <= self.weather.sun_half
    }

    /// Run until `budget` more instructions have executed. Round-robin, and each
    /// creature's slice is its own size, so being big buys no extra speed.
    pub fn run(&mut self, budget: u64) {
        let target = self.instr + budget;
        let mut idle_run = 0;
        while self.instr < target {
            if self.alive == 0 || idle_run > self.cs.len() {
                return;
            }
            if self.rr >= self.cs.len() {
                self.rr = 0;
            }
            let slot = self.rr;
            self.rr += 1;
            if !self.cs[slot].alive {
                continue;
            }
            let mut c = std::mem::take(&mut self.cs[slot]);
            let sun = if self.in_sun(c.start) { self.p.sun_bonus } else { 1 };
            let slice = if self.p.energy {
                // Running costs energy. Wealth buys speed: a turn is two body-lengths times the
                // creature's balance relative to the mean (1x to 64x), as far as the balance goes.
                // Without this, earning more bought nothing, and tiny fast breeders won.
                let rel = (c.energy.saturating_mul(1_000_000) / self.merit_mean_k).clamp(1000, 64_000);
                let s = c.energy.min(c.size as u64 * 2 * sun * rel / 1000);
                c.energy -= s;
                self.pool += s;
                s
            } else {
                let rel = (c.merit.max(1) as u64 * 1000 * 1000 / self.merit_mean_k).clamp(1, 64_000);
                (c.size as u64 * rel / 1000).max(1) * sun
            };
            if slice == 0 {
                // Broke: it sleeps until the next payout. Killing after a few broke turns, with
                // income arriving once an epoch, starved whole populations in a single epoch.
                c.idle += 1;
                idle_run += 1;
                self.cs[slot] = c;
                continue;
            }
            idle_run = 0;
            c.idle = 0;
            // Errors lower the reaper key; the queue entry is moved once, after the turn.
            // Nothing reads the runner's place in the queue meanwhile: `reap_one(slot)` spares
            // it, and it can't be killed during its own turn.
            let key = c.reap_score;
            // Steps that can't trigger the next cosmic ray run without checking for it.
            let mut left = slice;
            while left > 0 {
                let quiet = self.next_cosmic.saturating_sub(self.instr + 1).min(left);
                for _ in 0..quiet {
                    self.exec(&mut c, slot);
                }
                self.instr += quiet;
                left -= quiet;
                if left > 0 {
                    self.step(&mut c, slot);
                    left -= 1;
                }
            }
            if c.reap_score != key {
                self.reaper.remove(&(key, c.id, slot));
                self.reaper.insert((c.reap_score, c.id, slot));
            }
            c.age += slice;
            let old = self.p.max_age > 0 && c.age > self.p.max_age * c.size as u64;
            self.cs[slot] = c;
            // Old age: without it, a world where nobody can fit a child never turns over.
            if old {
                self.stats.died_old += 1;
                self.kill(slot);
            }
        }
    }

    /// Only ever called on the creature whose turn it is; `run` re-files its reaper key.
    fn error(&mut self, c: &mut Creature, _slot: usize) {
        c.errors += 1;
        c.reap_score -= ERROR_WEIGHT;
    }

    fn cosmic_ray(&mut self, c: &mut Creature, slot: usize) {
        let a = self.rng.below(self.n as u64) as usize;
        self.soup[a] = self.mutant_op(a as u32);
        // Whoever was hit no longer matches its genotype's jump table. The running
        // creature is out of `cs` while it runs, so it is marked directly.
        match self.owner[a] {
            0 => {}
            o if o as usize - 1 == slot => {
                c.dirty = true;
                c.pcache_ok = false;
            }
            o => {
                let v = &mut self.cs[o as usize - 1];
                v.dirty = true;
                v.pcache_ok = false;
            }
        }
        let iv = self.weather.cosmic_interval.max(1);
        self.next_cosmic = self.instr + 1 + self.rng.below(2 * iv);
    }

    #[inline]
    fn at(&self, a: u32) -> u8 {
        self.soup[(a & self.mask) as usize]
    }

    /// The nop run starting at `a`, as bits, and its length.
    fn template(&self, a: u32) -> (u32, u32) {
        let (mut bits, mut len) = (0u32, 0u32);
        while len < MAX_TEMPLATE {
            let op = self.at(a + len);
            if op > NOP1 {
                break;
            }
            bits |= (op as u32) << len;
            len += 1;
        }
        (bits, len)
    }

    fn matches(&self, q: u32, bits: u32, len: u32) -> bool {
        (0..len).all(|i| self.at(q + i) as u32 == ((bits >> i) & 1) ^ 1)
    }

    /// Find the complement of a template. Returns the address just past it.
    /// `fwd_from` is where a forward search starts, `back_from` is the instruction itself.
    /// Also returns how many steps out it looked before finding the match. Looking k steps
    /// out tries forward before backward, so the nearer match wins and forward wins ties.
    fn search(&self, bits: u32, len: u32, fwd_from: u32, back_from: u32, fwd: bool, back: bool) -> Option<(u32, u32)> {
        let limit = self.p.search_limit;
        let (finder, rfinder) = &self.finders[((1u32 << len) - 2 + bits) as usize];
        let kf = if fwd { self.search_fwd(finder, bits, len, fwd_from & self.mask, limit) } else { None };
        let kb = if back { self.search_back(rfinder, bits, len, back_from, kf.unwrap_or(limit)) } else { None };
        match (kf, kb) {
            (_, Some(k)) if kf.is_none_or(|f| k < f) => Some((back_from.wrapping_sub(k) & self.mask, k)),
            (Some(k), _) => Some(((fwd_from.wrapping_add(k) + len) & self.mask, k)),
            _ => None,
        }
    }

    /// The first k < limit with the complement at `start + k`.
    fn search_fwd(&self, f: &memchr::memmem::Finder, bits: u32, len: u32, start: u32, limit: u32) -> Option<u32> {
        let end = start as usize + (limit + len - 1) as usize;
        if limit > 0 && end <= self.soup.len() {
            return f.find(&self.soup[start as usize..end]).map(|k| k as u32);
        }
        (0..limit).find(|&k| self.matches(start.wrapping_add(k), bits, len))
    }

    /// The first k < limit with the complement ending just before `back_from - k`.
    fn search_back(&self, f: &memchr::memmem::FinderRev, bits: u32, len: u32, back_from: u32, limit: u32) -> Option<u32> {
        if limit == 0 {
            return None;
        }
        let top = back_from as i64 - len as i64;
        let low = top - (limit as i64 - 1);
        if low >= 0 && back_from as usize <= self.soup.len() {
            return f.rfind(&self.soup[low as usize..back_from as usize]).map(|r| (top - (low + r as i64)) as u32);
        }
        (0..limit).find(|&k| self.matches(back_from.wrapping_sub(len + k), bits, len))
    }

    /// A remembered jump from the instruction at `p`: where it lands and its template's
    /// length. Clean creatures share their genotype's table; dirty ones keep their own.
    fn cached_jump(&self, c: &mut Creature, p: u32) -> Option<(u32, u32)> {
        let off = p.wrapping_sub(c.start);
        if off >= c.size {
            return None;
        }
        let e = if c.dirty {
            if !c.pcache_ok {
                return None;
            }
            c.pcache[off as usize]
        } else {
            c.cache.as_ref()?.borrow()[off as usize]
        };
        (e != UNKNOWN).then(|| ((c.start + (e & 0xFF_FFFF)) & self.mask, e >> 24))
    }

    /// `search`, remembered. An answer is stored only if every cell the search looked at
    /// (the template and the cell that ended it included) lies inside the creature's own
    /// body: then it depends on the body alone, which for a clean creature is its genome,
    /// and a dirty one's own table is wiped whenever its body changes.
    fn search_cached(&self, c: &mut Creature, p: u32, bits: u32, len: u32, fwd_from: u32, fwd: bool, back: bool) -> Option<u32> {
        let found = self.search(bits, len, fwd_from, p, fwd, back);
        let off = p.wrapping_sub(c.start);
        if let Some((a, k)) = found.filter(|_| off < c.size) {
            let (lo, hi) = (c.start as i64, (c.start + c.size) as i64);
            let low = if back { p as i64 - len as i64 - k as i64 } else { fwd_from as i64 };
            let high = if fwd { fwd_from as i64 + k as i64 + len as i64 } else { p as i64 }.max(fwd_from as i64 + 1);
            if low >= lo && high <= hi {
                let e = (a as i64 - lo) as u32 | len << 24;
                if c.dirty {
                    if !c.pcache_ok {
                        c.pcache.clear();
                        c.pcache.resize(c.size as usize, UNKNOWN);
                        c.pcache_ok = true;
                    }
                    c.pcache[off as usize] = e;
                } else if let Some(t) = &c.cache {
                    t.borrow_mut()[off as usize] = e;
                }
            }
        }
        found.map(|r| r.0)
    }

    /// One instruction: fetch it, count it, fire a cosmic ray if one is due, execute. The
    /// op is fetched before the ray, so a ray landing on it changes the cell, not this step.
    fn step(&mut self, c: &mut Creature, slot: usize) {
        let p = c.ip & self.mask;
        let op = self.soup[p as usize];
        self.instr += 1;
        if self.instr >= self.next_cosmic {
            self.cosmic_ray(c, slot);
        }
        self.exec_op(c, slot, p, op);
    }

    /// `step` for a stretch known to hold no cosmic ray; `run` counts these in bulk.
    #[inline(always)]
    fn exec(&mut self, c: &mut Creature, slot: usize) {
        let p = c.ip & self.mask;
        let op = self.soup[p as usize];
        self.exec_op(c, slot, p, op);
    }

    /// Execute `op`, fetched from `p`. Reads nothing of `instr`.
    #[inline(always)]
    fn exec_op(&mut self, c: &mut Creature, slot: usize, p: u32, op: u8) {
        let off = p.wrapping_sub(c.start);
        if off < c.size {
            if off < 512 {
                c.ran[off as usize / 64] |= 1 << (off % 64);
            }
        } else if !matches!(c.child, Some((s, z)) if p.wrapping_sub(s) < z) {
            c.ran_foreign = true;
        }
        let mut next = p.wrapping_add(1);
        if !self.op_ok[op as usize] {
            self.error(c, slot);
            c.ip = next & self.mask;
            return;
        }
        match op {
            NOP0 | NOP1 => {}
            // Junk costs space, not time: a run of nopx is stepped over in one instruction.
            // Otherwise empty scratch space is a small tax that selection strips away
            // before anything useful can evolve in it.
            NOPX => {
                let mut k = 0;
                while k < 255 && self.at(next) == NOPX {
                    next = next.wrapping_add(1);
                    k += 1;
                }
            }
            OR1 => c.cx ^= 1,
            SHL => c.cx <<= 1,
            ZERO => c.cx = 0,
            IF_CZ => {
                if c.cx != 0 {
                    next = p.wrapping_add(2);
                }
            }
            SUB_AB => c.cx = c.ax.wrapping_sub(c.bx),
            SUB_AC => c.ax = c.ax.wrapping_sub(c.cx),
            INC_A => c.ax = c.ax.wrapping_add(1),
            INC_B => c.bx = c.bx.wrapping_add(1),
            DEC_C => c.cx = c.cx.wrapping_sub(1),
            INC_C => c.cx = c.cx.wrapping_add(1),
            PUSH_AX | PUSH_BX | PUSH_CX | PUSH_DX => {
                let v = [c.ax, c.bx, c.cx, c.dx][(op - PUSH_AX) as usize];
                c.stack[c.sp as usize] = v;
                c.sp = (c.sp + 1) % STACK as u8;
            }
            POP_AX | POP_BX | POP_CX | POP_DX => {
                c.sp = (c.sp + STACK as u8 - 1) % STACK as u8;
                let v = c.stack[c.sp as usize];
                match op {
                    POP_AX => c.ax = v,
                    POP_BX => c.bx = v,
                    POP_CX => c.cx = v,
                    _ => c.dx = v,
                }
            }
            JMP | JMPB | CALL | ADR | ADRB | ADRF => {
                let (target, len) = match self.cached_jump(c, p) {
                    Some((a, len)) => (Some(a), len),
                    None => {
                        let (bits, len) = self.template(p + 1);
                        let (f, b) = match op {
                            JMPB | ADRB => (false, true),
                            ADRF => (true, false),
                            _ => (true, true),
                        };
                        let from = p.wrapping_add(1 + len);
                        (if len == 0 { None } else { prof::time(prof::SEARCH, || self.search_cached(c, p, bits, len, from, f, b)) }, len)
                    }
                };
                next = p.wrapping_add(1 + len);
                if len == 0 {
                    self.error(c, slot);
                } else {
                    match target {
                        None => self.error(c, slot),
                        Some(a) => match op {
                            JMP | JMPB => next = a,
                            CALL => {
                                c.stack[c.sp as usize] = next;
                                c.sp = (c.sp + 1) % STACK as u8;
                                next = a;
                            }
                            _ => c.ax = a,
                        },
                    }
                }
            }
            RET => {
                c.sp = (c.sp + STACK as u8 - 1) % STACK as u8;
                next = c.stack[c.sp as usize];
            }
            MOV_CD => c.dx = c.cx,
            MOV_AB => c.bx = c.ax,
            MOV_IAB => {
                let dst = c.ax & self.mask;
                if c.owns(dst) {
                    let mut v = self.at(c.bx);
                    if self.rng.below(self.p.copy_error) == 0 {
                        v = self.mutant_op(dst);
                    }
                    self.soup[dst as usize] = v;
                    let mine = |a: u32| a >= c.start && a < c.start + c.size;
                    if !mine(dst) {
                        // Only copying from her own body counts toward a child. Counting
                        // any write let lineages copy their head, fill the rest with the
                        // child's leftover junk, and make every child a new genotype.
                        // And each cell counts once: writing the first half twice left the
                        // second half as junk, with the same effect.
                        if !self.p.lean {
                            c.child_writes += 1;
                        } else if mine(c.bx & self.mask) || (self.p.hgt && self.in_other_body(c.bx & self.mask, slot)) {
                            let i = c.child.map_or(usize::MAX, |(s, _)| dst.wrapping_sub(s) as usize);
                            if i >= 512 || c.child_seen[i / 64] >> (i % 64) & 1 == 0 {
                                if i < 512 {
                                    c.child_seen[i / 64] |= 1 << (i % 64);
                                }
                                c.child_writes += 1;
                            }
                        }
                    } else {
                        c.dirty = true;
                        c.pcache_ok = false;
                    }
                } else {
                    self.error(c, slot);
                }
            }
            NAND => c.cx = !(c.cx & c.dx),
            IO => self.io(c),
            LOAD | STORE if !self.p.memory => self.error(c, slot),
            LOAD => {
                let i = if self.p.mem2 { mem2_index(c.cx) } else { c.cx as usize & 255 };
                // Under v4 an empty memory guesses the ocean's commonest byte pattern, so a
                // young learner starts level with a constant guess instead of below it.
                c.cx = c.mem.get(i).copied().unwrap_or(self.base_guess) as u32;
            }
            STORE => {
                let (i, n) = if self.p.mem2 { (mem2_index(c.dx), MEM2) } else { (c.dx as usize & 255, 256) };
                if c.mem.is_empty() {
                    c.mem = vec![self.base_guess; n];
                }
                c.mem[i] = c.cx as u8;
            }
            INFECT => {
                let (bits, len) = self.template(p + 1);
                next = p.wrapping_add(1 + len);
                if !self.p.infect || !self.infect(c, slot, bits, len) {
                    self.error(c, slot);
                }
            }
            EAT => {
                let (bits, len) = self.template(p + 1);
                next = p.wrapping_add(1 + len);
                if !self.p.predation || !self.eat(c, slot, bits, len) {
                    self.error(c, slot);
                }
            }
            GUESS => {
                c.guess = c.cx as u8;
                c.has_guess = true;
            }
            DIVE => {
                let reading = if c.reef_mode {
                    self.reef_read(c)
                } else if self.ocean.is_empty() {
                    None
                } else {
                    let b = self.ocean[c.pos as usize % self.ocean.len()] ^ self.stream_key(c.pos);
                    c.pos = c.pos.wrapping_add(1);
                    Some((b, None))
                };
                match reading {
                    None => self.error(c, slot),
                    Some((b, from)) => {
                        // The byte before lands in dx, the new one in cx. Then `store` files
                        // "what came after the last byte" and `load` looks up "what came after
                        // this one": a learner is just `store load guess dive`, and `store`
                        // alone is harmless, so there is a neutral first step toward it.
                        if self.p.mem2 {
                            // Contexts end in the byte itself, so `store` still files a byte.
                            c.dx = (c.prev2 as u32) << 8 | c.prev as u32;
                            c.prev2 = c.prev;
                        } else {
                            c.dx = c.prev as u32;
                        }
                        c.prev = b;
                        if c.has_guess {
                            let s = if self.p.v4 && from.is_none() {
                                // Bits right beyond what the best constant guess got on this
                                // byte: blind or constant guessing earns nothing on average.
                                (self.base_guess ^ self.stream_key(c.pos.wrapping_sub(1)) ^ b).count_ones() as i32 - (c.guess ^ b).count_ones() as i32
                            } else {
                                4 - (c.guess ^ b).count_ones() as i32
                            };
                            c.scored += 1;
                            c.hits += (c.guess == b) as u32;
                            c.has_guess = false;
                            match from {
                                None => c.bits += s,
                                Some(e) => self.reef_score(c, e, s),
                            }
                        }
                        c.cx = if self.p.mem2 { (c.dx & 255) << 8 | b as u32 } else { b as u32 };
                    }
                }
            }
            EMIT | TUNE if !self.p.reef => self.error(c, slot),
            EMIT => {
                let r = &mut self.reefs[(c.start / self.region) as usize];
                let i = (r.written % REEF_CAP as u64) as usize;
                r.data[i] = c.cx as u8;
                r.from[i] = Emitter { slot: slot as u32, id: c.id, genotype: c.genotype };
                r.written += 1;
                self.stats.emits += 1;
            }
            TUNE => c.reef_mode = !c.reef_mode,
            ENCLOSE | LEVY | REVOLT => {
                if self.p.regime != Regime::Evolved || !self.politics(c, slot, op) {
                    self.error(c, slot);
                }
            }
            MATE | LEARN | TEACH | JOIN => {
                if !self.social(c, slot, op) {
                    self.error(c, slot);
                }
            }
            GIVE => {
                if !self.p.energy || !self.give(c, slot) {
                    self.error(c, slot);
                }
            }
            MAL => prof::time(prof::MAL, || self.mal(c, slot)),
            DIVIDE => prof::time(prof::DIVIDE, || self.divide(c, slot)),
            SENSE => {
                let d = self.weather.sun_center.wrapping_sub(p) & self.mask;
                let signed = if d >= self.n / 2 { d as i64 - self.n as i64 } else { d as i64 };
                c.dx = (signed / 16) as i32 as u32;
                c.sensed = true;
            }
            _ => self.error(c, slot),
        }
        c.ip = next & self.mask;
    }

    fn mal(&mut self, c: &mut Creature, slot: usize) {
        let size = c.cx;
        if size < self.p.min_size || size > self.p.max_size {
            self.error(c, slot);
            return;
        }
        if let Some((s, z)) = c.child.take() {
            self.decay(s, z + self.slack());
        }
        c.child_writes = 0;
        c.child_seen = [0; 8];
        let need = size + self.slack();
        if self.p.local > 0 {
            self.mal_local(c, slot, size, need);
            return;
        }
        let limit = (self.p.fill * self.n as f64) as u64;
        while self.used + need as u64 > limit {
            if !self.reap_one(slot) {
                break;
            }
        }
        let target = (c.start as i64 + (c.dx as i32 as i64) * 16).rem_euclid(self.n as i64) as u32;
        // Look near the mother first, then in a few random spots. Each look is bounded,
        // so a birth usually costs the same in a crowded soup as in an empty one. Only
        // if all of them miss does it pay for a full scan. Like Tierra, nobody is reaped
        // just to defragment: that let one mutant calling mal in a loop wipe out a world.
        let window = (need * 8).max(SEARCH_WINDOW);
        let mut spot = self.find_free(target, need, window);
        for _ in 0..8 {
            if spot.is_some() {
                break;
            }
            let from = self.rng.below(self.n as u64) as u32;
            spot = self.find_free(from, need, window);
        }
        if spot.is_none() {
            spot = self.find_free(target, need, self.n);
        }
        match spot {
            Some(a) => {
                self.claim(a, need, slot);
                self.used += need as u64;
                c.child = Some((a, size));
                c.ax = a;
            }
            None => self.error(c, slot),
        }
    }

    fn divide(&mut self, c: &mut Creature, slot: usize) {
        let Some((s, z)) = c.child else {
            self.error(c, slot);
            return;
        };
        // A child must be (nearly) all copied. At 1/2, a miscounted size gave oversized
        // children padded with junk that still lived, while undersized ones died, so
        // genome size ratcheted up into giants. Growth now comes from unbiased indels.
        // Lean: every cell. With 1/8 slack, lineages learned to skip copying their last
        // eighth (9% faster), so each child kept different leftover junk there, every birth
        // was a new genotype, and the novelty measure read zero.
        let need = if self.p.free_copy {
            0
        } else if self.p.lean {
            z
        } else {
            z - z / 8
        };
        if c.child_writes < need {
            self.error(c, slot);
            return;
        }
        if self.p.energy && self.p.birth_cost > 0 {
            let cost = self.p.birth_cost * z as u64;
            if c.energy < cost {
                self.error(c, slot);
                return;
            }
            c.energy -= cost;
            self.pool += cost;
        }
        // The block changes hands: release it from the mother, give it to the child.
        c.child = None;
        c.child_writes = 0;
        c.child_seen = [0; 8];
        self.release(s, z + self.slack());
        let resized = z != c.size;
        let (z, indels) = self.indel(s, z);
        let mate = match c.mate.take() {
            None if self.p.sex => self.mate_near(s, z, slot),
            m => m,
        };
        if let Some((ps, pid)) = mate {
            self.crossover(s, z, ps as usize, pid);
        }
        let g = genotype_of(&self.soup[s as usize..(s + z) as usize]);
        // An unchanged body still hashes to the genotype it was born with: `dirty` is set by
        // every write into a creature's own body and never cleared.
        let mother_now = if c.dirty { genotype_of(&self.soup[c.start as usize..(c.start + c.size) as usize]) } else { c.genotype };
        let count = |w: &World, a: u32, n: u32| w.soup[a as usize..(a + n) as usize].iter().filter(|&&o| o == NOPX).count() as u64;
        let (m_nopx, c_nopx) = (count(self, c.start, c.size), count(self, s, z));
        let st = &mut self.stats;
        st.births += 1;
        st.mother_nopx += m_nopx;
        st.resized += resized as u64;
        if c_nopx > m_nopx {
            st.gained += c_nopx - m_nopx;
        } else {
            let lost = m_nopx - c_nopx;
            if resized {
                st.lost_resize += lost;
            } else if indels {
                st.lost_indel += lost;
            } else {
                st.lost_point += lost;
            }
        }
        // What this cycle's work earned is paid to both, for the next cycle.
        let mut merit = 1u32;
        if self.p.tasks {
            let paid = c.tasks_now & self.weather.rewarded;
            for (t, bonus) in TASK_BONUS.iter().enumerate() {
                if paid >> t & 1 == 1 {
                    merit = merit.saturating_mul(*bonus).min(self.p.max_merit);
                }
            }
        }
        // The ocean pays for correct predictions of bytes the creature had not yet read.
        if self.p.ocean_div > 0 && !self.ocean.is_empty() {
            // Scored bit by bit, the way PAQ-style compressors predict: each guess earns the
            // bits it got right minus 4, so random guessing earns nothing, knowing the kind
            // of character earns a little, and the exact byte earns most. Every step of
            // better prediction pays. Four net bits buy one unit of merit.
            let net = (c.bits + c.emit_bits).max(0) as u32;
            let bonus = (1 + net / (4 * self.p.ocean_div)).min(self.p.ocean_cap);
            merit = merit.saturating_mul(bonus).min(self.p.max_merit);
        }
        // Score for this cycle: ocean bits plus the logic tasks that pay right now. In the
        // energy economy it only weights the next payout; it never creates energy.
        let task_pts: u64 = if self.p.tasks {
            let paid = c.tasks_now & self.weather.rewarded;
            TASK_BONUS.iter().enumerate().filter(|(t, _)| paid >> t & 1 == 1).map(|(_, b)| *b as u64).sum()
        } else {
            0
        };
        let ocean = if self.p.debt {
            let net = c.bits.saturating_add(c.debt);
            c.debt = net.min(0);
            net.max(0)
        } else {
            c.bits.max(0)
        };
        let earned = ocean as u64 + task_pts;
        if self.p.v4 && self.p.energy {
            let rate = match self.stream_of(c.pos) {
                Some(k) => {
                    self.stream_earned[k] += earned;
                    self.stream_rate[k]
                }
                None => {
                    self.score_earned += earned;
                    self.score_rate
                }
            };
            let pay = ((earned as u128 * rate as u128) >> 16).min(self.pool as u128) as u64;
            self.pool -= pay;
            c.energy += pay;
        } else {
            c.score += earned;
        }
        let child = self.spawn(s, z, g, merit, c.tasks_now);
        if self.p.energy {
            // A child starts with half of its mother's energy.
            let gift = c.energy / 2;
            c.energy -= gift;
            self.cs[child].energy = gift;
            self.cs[child].parent = c.id;
        }
        // The child swims on from where its mother stopped reading.
        let ch = &mut self.cs[child];
        ch.pos = c.pos;
        if self.p.streams > 0 && !self.ocean.is_empty() && self.rng.below(16) == 0 {
            ch.pos = self.rng.below(self.ocean.len() as u64) as u32;
        }
        ch.hits_last = c.hits;
        ch.scored_last = c.scored;
        ch.bits_last = c.bits;
        ch.mem = c.mem.clone();
        ch.prev = c.prev;
        ch.prev2 = c.prev2;
        ch.debt = c.debt;
        ch.reef_mode = c.reef_mode;
        self.events.push(Event::Birth {
            mother: c.genotype,
            mother_now,
            child: g,
            child_size: z,
            foreign: c.ran_foreign,
            sensed: c.sensed,
            tasks: c.tasks_now,
            code: if g != c.genotype { self.soup[s as usize..(s + z) as usize].to_vec() } else { Vec::new() },
        });
        c.merit = merit;
        c.tasks_last = c.tasks_now;
        c.tasks_now = 0;
        c.infected = false;
        c.ate = false;
        c.hits_last = c.hits;
        c.scored_last = c.scored;
        c.hits = 0;
        c.scored = 0;
        c.bits_last = c.bits;
        c.bits = 0;
        c.emit_bits = 0;
        c.ran_foreign = false;
        c.sensed = false;
    }

    /// With indels on, every child block comes with one spare cell so an insertion always
    /// has room. Without it, deletions would always succeed and insertions rarely, and
    /// genomes would shrink for no reason but bookkeeping.
    fn slack(&self) -> u32 {
        (self.p.indel > 0) as u32
    }

    /// Insertion and deletion mutations, so genomes can grow and shrink by more than
    /// a miscounted size. Returns the child's new size.
    fn indel(&mut self, s: u32, z: u32) -> (u32, bool) {
        if self.p.indel == 0 {
            return (z, false);
        }
        let z0 = z;
        let mut z = z;
        if self.rng.below(self.p.indel) == 0 && z > self.p.min_size {
            let at = s + self.rng.below(z as u64) as u32;
            self.soup.copy_within(at as usize + 1..(s + z) as usize, at as usize);
            z -= 1;
            self.indel_hit = true;
        }
        let end = s + z;
        if self.rng.below(self.p.indel) == 0 && z < self.p.max_size && end < self.n && self.owner[end as usize] == 0 {
            let at = s + self.rng.below(z as u64 + 1) as u32;
            self.soup.copy_within(at as usize..end as usize, at as usize + 1);
            self.soup[at as usize] = self.mutant_op(at);
            z += 1;
        }
        // A deletion followed by an insertion leaves the size alone but still counts.
        let touched = z != z0 || self.indel_hit;
        self.indel_hit = false;
        (z, touched)
    }

    /// Output cx, then read the next number from the outside into cx. If the output
    /// is a logic function of numbers this creature has read, that task counts.
    /// Next unread byte of the local reef, and who emitted it. Readers that fall behind skip
    /// to the oldest byte still kept; a reader that has caught up has nothing to read.
    fn reef_read(&mut self, c: &mut Creature) -> Option<(u8, Option<Emitter>)> {
        let r = &self.reefs[(c.start / self.region) as usize];
        c.reef_pos = c.reef_pos.max(r.written.saturating_sub(REEF_CAP as u64));
        if c.reef_pos >= r.written {
            return None;
        }
        let i = (c.reef_pos % REEF_CAP as u64) as usize;
        c.reef_pos += 1;
        self.stats.reef_reads += 1;
        Some((r.data[i], Some(r.from[i])))
    }

    /// Who earns a reef prediction. Your own bytes earn nothing. A relative predicting you
    /// pays you (signalling to kin). Predicting a rival pays you and costs them (decoding
    /// the enemy), so a signal anyone can read is a liability.
    fn reef_score(&mut self, c: &mut Creature, e: Emitter, s: i32) {
        if e.id == c.id {
            return;
        }
        let kin = e.genotype == c.genotype;
        if self.p.energy {
            // Decoding a rival takes energy from it, at most what it has; kin pay nothing.
            if !kin && s > 0 {
                if let Some(m) = self.cs.get_mut(e.slot as usize).filter(|m| m.alive && m.id == e.id) {
                    let t = (s as u64).min(m.energy);
                    m.energy -= t;
                    c.energy += t;
                }
            }
            if kin {
                self.stats.kin_reads += 1;
            }
            return;
        }
        if kin {
            self.stats.kin_reads += 1;
        } else {
            c.bits += s;
        }
        if let Some(m) = self.cs.get_mut(e.slot as usize).filter(|m| m.alive && m.id == e.id) {
            m.emit_bits += if kin { s } else { -s };
        }
    }

    /// The Red Queen. A creature can take half the CPU share of a neighbour that does a
    /// logic task it also does, once per cycle. Common ways of making a living get
    /// hunted, so rare and new ones pay. ECHO doesn't count: everyone can do it.
    /// Ring distance between two cells.
    fn ring_dist(&self, a: u32, b: u32) -> u32 {
        let d = a.wrapping_sub(b) & self.mask;
        d.min(self.n - d)
    }

    /// Living creatures other than `skip` with a body cell within `r` cells of `centre`, in
    /// scan order from `centre - r`. Free space is skipped a word at a time and each body
    /// in one step.
    fn neighbours(&self, centre: u32, r: u32, skip: usize) -> Vec<usize> {
        let mut out = Vec::new();
        self.first_neighbour(centre, r, skip, |v| {
            out.push(v);
            false
        });
        out
    }

    /// The first of `neighbours`, in the same order, for which `want` says yes: the walk
    /// stops there.
    fn first_neighbour(&self, centre: u32, r: u32, skip: usize, mut want: impl FnMut(usize) -> bool) -> Option<usize> {
        let mut out: Vec<usize> = Vec::new();
        let span = (2 * r as u64 + 1).min(self.n as u64) as u32;
        let (mut a, mut k) = (centre.wrapping_sub(r) & self.mask, 0u32);
        while k < span {
            let free = self.occ[(a / 64) as usize] >> (a % 64);
            let step = if free & 1 == 0 {
                if free == 0 { 64 - a % 64 } else { free.trailing_zeros() }
            } else {
                let o = self.owner[a as usize];
                let d = if o == 0 { None } else { Some(&self.cs[o as usize - 1]) };
                match d {
                    Some(d) if d.alive && a.wrapping_sub(d.start) < d.size => {
                        let v = o as usize - 1;
                        if v != skip && !out.contains(&v) {
                            out.push(v);
                            if want(v) {
                                return Some(v);
                            }
                        }
                        d.size - a.wrapping_sub(d.start)
                    }
                    _ => {
                        // Cells with this owner outside its body (a child being written,
                        // slack, an unowned stretch) report no one until the walk reaches
                        // the owner's body: skip the whole run at once.
                        // Stopping short (at the soup's end) is safe: the walk just resumes.
                        let mut lim = (span - k).min(self.n - a);
                        if let Some(d) = d.filter(|d| d.alive) {
                            lim = lim.min(d.start.wrapping_sub(a) & self.mask);
                        }
                        let run = &self.owner[a as usize..(a + lim) as usize];
                        let mut j = run.chunks_exact(16).take_while(|c| c.iter().all(|&x| x == o)).count() * 16;
                        j += run[j..].iter().take_while(|&&x| x == o).count();
                        (j as u32).max(1)
                    }
                }
            };
            k += step;
            a = (a + step) & self.mask;
        }
        None
    }

    /// v5 birth: a free spot within `radius` of the mother's target, reaping neighbours of
    /// the target (poorest or oldest) for room, up to eight times.
    fn mal_local(&mut self, c: &mut Creature, slot: usize, size: u32, need: u32) {
        let r = self.p.radius;
        let target = (c.start as i64 + (c.dx as i32 as i64) * 16).rem_euclid(self.n as i64) as u32;
        let widths: &[u32] = if self.p.local >= 2 { &[1, 4, 16] } else { &[1] };
        for _ in 0..8 {
            for &m in widths {
                let w = (r * m).min(self.n / 2);
                if let Some(a) = self.find_free(target.wrapping_sub(w) & self.mask, need, 2 * w) {
                    self.claim(a, need, slot);
                    self.used += need as u64;
                    c.child = Some((a, size));
                    c.ax = a;
                    return;
                }
            }
            let ns = self.neighbours(target, r, slot);
            let key = |v: usize| {
                let d = &self.cs[v];
                (if self.p.reap_poor { d.energy } else { 0 }, d.reap_score, d.id)
            };
            match ns.into_iter().min_by_key(|&v| key(v)) {
                Some(v) => {
                    self.stats.died_reaped += 1;
                    self.kill(v);
                }
                None => break,
            }
        }
        self.error(c, slot);
    }

    /// The nearest creature to the child block within `radius` whose size is within an
    /// eighth of the child's: the mate for an obligatory crossover.
    fn mate_near(&self, s: u32, z: u32, slot: usize) -> Option<(u32, u64)> {
        let mid = s + z / 2;
        self.neighbours(mid, self.p.radius, slot)
            .into_iter()
            .filter(|&v| self.cs[v].size.abs_diff(z) <= z / 8)
            .min_by_key(|&v| self.ring_dist(self.cs[v].start, mid))
            .map(|v| (v as u32, self.cs[v].id))
    }

    /// Predation. The prey is the creature whose body holds ax, or the next one ahead
    /// within `radius`; the key's complement must lie in its first SURFACE cells. The prey
    /// dies and its whole balance moves to the predator.
    fn eat(&mut self, c: &mut Creature, slot: usize, bits: u32, len: u32) -> bool {
        if !self.p.energy || c.ate || len == 0 {
            return false;
        }
        let r = self.p.radius;
        let a = c.ax & self.mask;
        let Some(v) = self.first_neighbour(a.wrapping_add(r / 2), r / 2, slot, |v| {
            let d = &self.cs[v];
            a.wrapping_sub(d.start) & self.mask < d.size || d.start.wrapping_sub(a) & self.mask <= r / 2
        }) else {
            return false;
        };
        let (vs, vz) = (self.cs[v].start, self.cs[v].size);
        if self.ring_dist(vs, c.start) > r {
            return false;
        }
        let surf = vz.min(SURFACE);
        if surf < len || !(vs..=vs + surf - len).any(|q| self.matches(q, bits, len)) {
            return false;
        }
        let e = std::mem::take(&mut self.cs[v].energy);
        c.energy += e;
        self.stats.died_eaten += 1;
        self.kill(v);
        c.ate = true;
        true
    }

    fn infect(&mut self, c: &mut Creature, slot: usize, bits: u32, len: u32) -> bool {
        if c.infected {
            return false;
        }
        let o = self.owner[(c.ax & self.mask) as usize];
        if o == 0 || o as usize - 1 == slot {
            return false;
        }
        let d = c.start.wrapping_sub(self.cs[o as usize - 1].start) & self.mask;
        if d.min(self.n - d) > self.p.infect_radius {
            return false;
        }
        let (vs, vz) = (self.cs[o as usize - 1].start, self.cs[o as usize - 1].size);
        let matched = if self.p.tasks {
            c.tasks_last & self.cs[o as usize - 1].tasks_last & 0x1FF != 0
        } else {
            // Without the task economy there are no shared tasks to target. Then infection is
            // Tierra's lock and key: the template after `infect` must find its complement
            // somewhere in the victim's body. Hosts can evolve the lock away, parasites a new key.
            len > 0 && vz >= len && (vs..=vs + vz - len).any(|q| self.matches(q, bits, len))
        };
        let v = &mut self.cs[o as usize - 1];
        if self.p.energy {
            // Theft moves energy that exists; it never makes any.
            if !matched || v.energy < 2 {
                return false;
            }
            let take = v.energy / 2;
            v.energy -= take;
            c.energy += take;
        } else {
            if !matched || v.merit < 2 {
                return false;
            }
            let take = v.merit / 2;
            v.merit -= take;
            c.merit = (c.merit + take).min(self.p.max_merit);
        }
        c.infected = true;
        self.stats.infections += 1;
        true
    }

    fn io(&mut self, c: &mut Creature) {
        let v = c.cx;
        let n = (c.reads as usize).min(3);
        for i in 0..n {
            let a = c.inputs[i];
            if v == !a {
                c.tasks_now |= 1;
            }
            if v == a {
                c.tasks_now |= 1 << 9;
            }
            for j in (0..n).filter(|&j| j != i) {
                for (t, &o) in task_outputs(a, c.inputs[j]).iter().enumerate().skip(1) {
                    if v == o {
                        c.tasks_now |= 1 << t;
                    }
                }
            }
        }
        let mut x = self.weather.data
            ^ c.id.wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (c.reads as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x = (x ^ (x >> 31)).wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^= x >> 29;
        // Avida's trick: each input's top byte is a fixed pattern, so every logic function
        // of two inputs leaves a different fingerprint and nothing matches by coincidence.
        let k = c.reads as usize % 3;
        let v = (x as u32 & 0x00FF_FFFF) | ([0x0F, 0x33, 0x55][k] << 24);
        c.inputs[k] = v;
        c.reads = c.reads.wrapping_add(1);
        c.cx = v;
    }

    /// Cells the bookkeeping says are in use, and cells actually owned. They must agree.
    pub fn audit(&self) -> (u64, u64) {
        (self.used, self.owner.iter().filter(|&&o| o != 0).count() as u64)
    }

    /// Living slots in the order the reaper would take them, next victim first.
    pub fn reap_order(&self) -> Vec<usize> {
        self.reaper.iter().map(|k| k.2).collect()
    }

    pub fn genome(&self, c: &Creature) -> &[u8] {
        &self.soup[c.start as usize..(c.start + c.size) as usize]
    }

    /// Everything that determines the future, hashed. Two honest replays agree on this.
    pub fn state_hash(&self) -> [u8; 32] {
        // Everything goes into one buffer and is hashed once: blake3 gives the same digest
        // however the input is split, and a hundred thousand tiny updates cost 4 ms an epoch.
        struct Buf(Vec<u8>);
        impl Buf {
            fn update(&mut self, b: &[u8]) {
                self.0.extend_from_slice(b);
            }
        }
        // The buffer is kept between calls: a fresh megabyte each epoch costs page faults.
        thread_local!(static SPARE: std::cell::Cell<Vec<u8>> = const { std::cell::Cell::new(Vec::new()) });
        let mut h = Buf(SPARE.take());
        h.0.clear();
        h.0.reserve(self.soup.len() + (1 << 16) + self.cs.len() * 512);
        h.update(&self.soup);
        h.update(&self.rng.state_bytes());
        h.update(&self.instr.to_le_bytes());
        h.update(&self.next_cosmic.to_le_bytes());
        h.update(&[self.weather.goal]);
        h.update(&self.pool.to_le_bytes());
        if self.p.v4 {
            h.update(&self.score_rate.to_le_bytes());
            h.update(&self.score_earned.to_le_bytes());
            for v in self.stream_rate.iter().chain(&self.stream_earned) {
                h.update(&v.to_le_bytes());
            }
        }
        for l in &self.levies {
            h.update(&l.to_le_bytes());
        }
        for (t, p) in self.titles.iter().zip(&self.prices) {
            let (s, i) = t.map(|(s, i)| (s as u64, i)).unwrap_or((u64::MAX, 0));
            h.update(&s.to_le_bytes());
            h.update(&i.to_le_bytes());
            h.update(&p.to_le_bytes());
        }
        for r in &self.reefs {
            h.update(&r.written.to_le_bytes());
            h.update(&r.data);
        }
        h.update(&(self.rr as u64).to_le_bytes());
        for c in self.cs.iter().filter(|c| c.alive) {
            for v in [c.id, c.start as u64, c.size as u64, c.ip as u64, c.birth, c.errors as u64, c.age] {
                h.update(&v.to_le_bytes());
            }
            for v in [c.ax, c.bx, c.cx, c.dx, c.sp as u32, c.merit, c.tasks_now as u32, c.reads, c.pos, c.hits, c.scored, c.bits as u32, c.prev as u32, c.infected as u32, c.reef_mode as u32, c.reef_pos as u32, c.emit_bits as u32, c.energy as u32, (c.energy >> 32) as u32, c.score as u32, c.idle, c.group as u32, c.mate.map(|m| m.1 as u32).unwrap_or(u32::MAX), c.guess as u32 | (c.has_guess as u32) << 8] {
                h.update(&v.to_le_bytes());
            }
            for v in c.stack.iter().chain(c.inputs.iter()) {
                h.update(&v.to_le_bytes());
            }
            let (s, z) = c.child.unwrap_or((u32::MAX, 0));
            h.update(&s.to_le_bytes());
            h.update(&z.to_le_bytes());
            h.update(&c.child_writes.to_le_bytes());
            h.update(&c.mem);
            if self.p.mem2 {
                h.update(&[c.prev2]);
            }
            if self.p.debt {
                h.update(&c.debt.to_le_bytes());
            }
            if self.p.predation {
                h.update(&[c.ate as u8]);
            }
        }
        let digest = *blake3::hash(&h.0).as_bytes();
        SPARE.set(h.0);
        digest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The template search as it was: one step out at a time, forward then backward.
    fn search_scan(w: &World, bits: u32, len: u32, fwd_from: u32, back_from: u32, fwd: bool, back: bool) -> Option<(u32, u32)> {
        for k in 0..w.p.search_limit {
            if fwd {
                let q = fwd_from.wrapping_add(k);
                if w.matches(q, bits, len) {
                    return Some(((q + len) & w.mask, k));
                }
            }
            if back {
                let q = back_from.wrapping_sub(len + k);
                if w.matches(q, bits, len) {
                    return Some(((q.wrapping_add(len)) & w.mask, k));
                }
            }
        }
        None
    }

    /// The SIMD template search must give exactly the scan's answer, wrap-around included.
    #[test]
    fn search_matches_scan() {
        let mut rng = Rng::new(11);
        for round in 0..300 {
            let limit = [0, 1, 7, 64, 1024][rng.below(5) as usize];
            let p = Params { soup_bits: 10, search_limit: limit, ..Default::default() };
            let mut w = World::new(p, round, Vec::new());
            // Mostly nops, so matches are common, sometimes rare.
            let nops = 20 + rng.below(80);
            for c in w.soup.iter_mut() {
                *c = if rng.below(100) < nops { rng.below(2) as u8 } else { 2 + rng.below(30) as u8 };
            }
            for _ in 0..200 {
                let len = 1 + rng.below(MAX_TEMPLATE as u64) as u32;
                let bits = rng.below(1 << len) as u32;
                let p = rng.below(w.n as u64) as u32;
                let fwd_from = p + 1 + len;
                let (fwd, back) = [(true, true), (false, true), (true, false)][rng.below(3) as usize];
                assert_eq!(
                    w.search(bits, len, fwd_from, p, fwd, back),
                    search_scan(&w, bits, len, fwd_from, p, fwd, back),
                    "round {round} p {p} len {len} bits {bits} limit {limit} fwd {fwd} back {back}"
                );
            }
        }
    }

    /// The word-at-a-time free-space search must give exactly the scan's answer.
    #[test]
    fn find_free_matches_scan() {
        let p = Params { soup_bits: 10, ..Default::default() };
        let mut w = World::new(p, 1, Vec::new());
        let mut rng = Rng::new(7);
        for round in 0..4000 {
            // Occupancy from sparse to nearly full, in runs like creatures leave.
            let fill = rng.below(100);
            let mut a = 0usize;
            w.occ.iter_mut().for_each(|x| *x = 0);
            while a < w.n as usize {
                let len = 1 + rng.below(40) as usize;
                let taken = rng.below(100) < fill;
                for c in a..(a + len).min(w.n as usize) {
                    if taken {
                        w.occ[c / 64] |= 1 << (c % 64);
                    }
                }
                a += len;
            }
            for _ in 0..20 {
                let from = rng.below(w.n as u64) as u32;
                let size = 1 + rng.below(if round % 2 == 0 { 64 } else { 600 }) as u32;
                let window = [0, 1, 63, 64, 100, 4096, w.n, w.n * 2][rng.below(8) as usize] + rng.below(3) as u32;
                assert_eq!(
                    w.find_free_inner(from, size, window),
                    w.find_free_scan(from, size, window),
                    "round {round} from {from} size {size} window {window}"
                );
            }
        }
    }
}
