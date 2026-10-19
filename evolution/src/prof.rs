//! Wall-clock timers for the hot spots. Built only with `--features prof`; otherwise
//! `time` is a plain call and costs nothing.

pub const RUN: usize = 0;
pub const SEARCH: usize = 1;
pub const MAL: usize = 2;
pub const DIVIDE: usize = 3;
pub const CENSUS: usize = 4;
pub const STATE_HASH: usize = 5;
pub const KILL: usize = 6;
pub const FIND_FREE: usize = 7;
pub const GENE_EVENTS: usize = 8;
pub const GENE_CENSUS: usize = 9;
const NAMES: [&str; 10] = ["run (all)", "template search", "mal", "divide", "census", "state hash", "kill", "find_free", "gene events", "gene census"];

#[cfg(feature = "prof")]
mod imp {
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    pub static NS: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];
    pub static CALLS: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];

    #[inline]
    pub fn time<R>(i: usize, f: impl FnOnce() -> R) -> R {
        let t = std::time::Instant::now();
        let r = f();
        NS[i].fetch_add(t.elapsed().as_nanos() as u64, Relaxed);
        CALLS[i].fetch_add(1, Relaxed);
        r
    }
}

#[cfg(feature = "prof")]
pub use imp::time;

#[cfg(not(feature = "prof"))]
#[inline(always)]
pub fn time<R>(_: usize, f: impl FnOnce() -> R) -> R {
    f()
}

#[cfg(feature = "prof")]
pub fn report(total_secs: f64) {
    use std::sync::atomic::Ordering::Relaxed;
    eprintln!("{:<16} {:>9} {:>7} {:>12} {:>9}", "region", "seconds", "share", "calls", "ns/call");
    for (i, name) in NAMES.iter().enumerate() {
        let ns = imp::NS[i].load(Relaxed);
        let calls = imp::CALLS[i].load(Relaxed);
        eprintln!(
            "{name:<16} {:>9.2} {:>6.1}% {calls:>12} {:>9}",
            ns as f64 / 1e9,
            100.0 * ns as f64 / 1e9 / total_secs,
            if calls > 0 { ns / calls } else { 0 }
        );
    }
}

#[cfg(not(feature = "prof"))]
pub fn report(_: f64) {
    let _ = NAMES;
}
