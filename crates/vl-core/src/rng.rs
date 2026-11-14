//! Deterministic hashing and random numbers. Integer-only, identical on every platform.

/// splitmix64 finaliser.
#[inline]
pub fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[inline]
pub fn hash2(a: u64, b: u64) -> u64 {
    mix64(mix64(a.wrapping_add(0x9e37_79b9_7f4a_7c15)) ^ b.wrapping_mul(0xd6e8_feb8_6659_fd93))
}

#[inline]
pub fn hash3(a: u64, b: u64, c: u64) -> u64 {
    hash2(hash2(a, b), c)
}

#[inline]
pub fn hash4(a: u64, b: u64, c: u64, d: u64) -> u64 {
    hash2(hash3(a, b, c), d)
}

/// 24 random bits as a float in [0, 1).
#[inline]
pub fn unit_f32(h: u64) -> f32 {
    ((h >> 40) as u32) as f32 * (1.0 / 16_777_216.0)
}

/// PCG32 (O'Neill 2014): small, fast, statistically solid.
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
    inc: u64,
}

impl Rng {
    pub fn new(seed: u64, stream: u64) -> Rng {
        let mut r = Rng { state: 0, inc: (stream << 1) | 1 };
        r.next_u32();
        r.state = r.state.wrapping_add(seed);
        r.next_u32();
        r
    }
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        let old = self.state;
        self.state = old.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(self.inc);
        let xorshifted = (((old >> 18) ^ old) >> 27) as u32;
        xorshifted.rotate_right((old >> 59) as u32)
    }
    /// Uniform in [0, 1).
    #[inline]
    pub fn f32(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 * (1.0 / 16_777_216.0)
    }
    #[inline]
    pub fn f64(&mut self) -> f64 {
        let hi = (self.next_u32() >> 5) as u64;
        let lo = (self.next_u32() >> 6) as u64;
        ((hi << 26) | lo) as f64 * (1.0 / 9_007_199_254_740_992.0)
    }
    #[inline]
    pub fn below(&mut self, n: u32) -> u32 {
        ((self.next_u32() as u64 * n as u64) >> 32) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcg_reference_stream() {
        // Reference values of pcg32 with seed 42, stream 54 (from the PCG reference demo).
        let mut r = Rng::new(42, 54);
        let got: Vec<u32> = (0..6).map(|_| r.next_u32()).collect();
        assert_eq!(got, vec![0xa15c02b7, 0x7b47f409, 0xba1d3330, 0x83d2f293, 0xbfa4784b, 0xcbed606e]);
    }

    #[test]
    fn uniformity() {
        let mut r = Rng::new(7, 1);
        let n = 200_000;
        let mut sum = 0.0f64;
        let mut bins = [0u32; 16];
        for _ in 0..n {
            let v = r.f32();
            assert!((0.0..1.0).contains(&v));
            sum += v as f64;
            bins[(v * 16.0) as usize] += 1;
        }
        assert!((sum / n as f64 - 0.5).abs() < 0.005);
        for b in bins {
            assert!((b as f64 - n as f64 / 16.0).abs() < 0.03 * n as f64 / 16.0 * 4.0);
        }
    }
}
