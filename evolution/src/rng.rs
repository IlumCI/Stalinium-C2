/// xoshiro256**. Every random choice in the world goes through one of these,
/// so a run is a pure function of (seed, feed bytes).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut z = seed;
        let mut s = [0u64; 4];
        for x in &mut s {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut v = z;
            v = (v ^ (v >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            v = (v ^ (v >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            *x = v ^ (v >> 31);
        }
        Rng { s }
    }

    pub fn next(&mut self) -> u64 {
        let r = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        r
    }

    pub fn below(&mut self, n: u64) -> u64 {
        ((self.next() as u128 * n as u128) >> 64) as u64
    }

    /// Stir outside entropy into the state. This is how the feed steers mutation.
    pub fn mix(&mut self, bytes: &[u8; 32]) {
        for i in 0..4 {
            self.s[i] ^= u64::from_le_bytes(bytes[i * 8..i * 8 + 8].try_into().unwrap());
        }
        if self.s == [0; 4] {
            self.s[0] = 1;
        }
        self.next();
    }

    pub fn state_bytes(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        for i in 0..4 {
            out[i * 8..i * 8 + 8].copy_from_slice(&self.s[i].to_le_bytes());
        }
        out
    }
}
