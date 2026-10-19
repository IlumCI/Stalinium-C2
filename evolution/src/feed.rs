//! The world is open: every epoch, 32 bytes arrive from outside and become weather.
//!
//! `chain:<seed>`  deterministic hash chain, the default
//! `file:<path>`   a file's bytes, 32 at a time, then the chain once it runs dry
//! `cmd:<command>` run a command each epoch and hash what it prints (a price, a block hash, a feed)
//! `replay:<path>` the feed.log of an earlier run, byte for byte

use std::fs;
use std::process::Command;

#[derive(serde::Serialize, serde::Deserialize)]
pub enum Feed {
    Chain([u8; 32]),
    File { data: Vec<u8>, pos: usize, prev: [u8; 32] },
    Cmd { cmd: String, prev: [u8; 32] },
    Replay { lines: Vec<[u8; 32]>, i: usize },
}

fn hash2(a: &[u8], b: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(a);
    h.update(b);
    *h.finalize().as_bytes()
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<[u8; 32]> {
    let s = s.trim();
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

impl Feed {
    pub fn parse(spec: &str, seed: u64) -> Result<Feed, String> {
        let base = *blake3::hash(&seed.to_le_bytes()).as_bytes();
        let (kind, arg) = spec.split_once(':').unwrap_or((spec, ""));
        Ok(match kind {
            "chain" => Feed::Chain(if arg.is_empty() { base } else { *blake3::hash(arg.as_bytes()).as_bytes() }),
            "file" => Feed::File { data: fs::read(arg).map_err(|e| format!("{arg}: {e}"))?, pos: 0, prev: base },
            "cmd" => Feed::Cmd { cmd: arg.to_string(), prev: base },
            "replay" => {
                let text = fs::read_to_string(arg).map_err(|e| format!("{arg}: {e}"))?;
                let lines = text
                    .lines()
                    .filter_map(|l| l.split_whitespace().nth(1).and_then(unhex))
                    .collect();
                Feed::Replay { lines, i: 0 }
            }
            _ => return Err(format!("unknown feed `{spec}`")),
        })
    }

    pub fn next(&mut self) -> Result<[u8; 32], String> {
        Ok(match self {
            Feed::Chain(s) => {
                *s = *blake3::hash(s).as_bytes();
                *s
            }
            Feed::File { data, pos, prev } => {
                if *pos < data.len() {
                    let end = (*pos + 32).min(data.len());
                    *prev = hash2(prev, &data[*pos..end]);
                    *pos = end;
                } else {
                    *prev = *blake3::hash(prev).as_bytes();
                }
                *prev
            }
            Feed::Cmd { cmd, prev } => {
                let out = if cfg!(windows) {
                    Command::new("cmd").args(["/C", cmd]).output()
                } else {
                    Command::new("sh").args(["-c", cmd]).output()
                };
                let stdout = out.map(|o| o.stdout).unwrap_or_default();
                *prev = hash2(prev, &stdout);
                *prev
            }
            Feed::Replay { lines, i } => {
                let v = *lines.get(*i).ok_or("replay feed ran out")?;
                *i += 1;
                v
            }
        })
    }
}
