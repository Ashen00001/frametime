//! Small numeric helpers shared by the analysis modules.

pub fn sorted(v: &[f64]) -> Vec<f64> {
    let mut s: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    s.sort_by(|a, b| a.total_cmp(b));
    s
}

/// Percentile with linear interpolation between order statistics
/// (the "type 7" definition used by NumPy and Excel's PERCENTILE.INC).
/// `p` is in 0..=100 and `s` must be sorted ascending.
pub fn percentile_sorted(s: &[f64], p: f64) -> f64 {
    match s.len() {
        0 => f64::NAN,
        1 => s[0],
        n => {
            let rank = (p / 100.0).clamp(0.0, 1.0) * (n - 1) as f64;
            let lo = rank.floor() as usize;
            let hi = rank.ceil() as usize;
            s[lo] + (s[hi] - s[lo]) * (rank - lo as f64)
        }
    }
}

pub fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        f64::NAN
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

pub fn median(v: &[f64]) -> f64 {
    percentile_sorted(&sorted(v), 50.0)
}

/// Sample standard deviation.
pub fn std_dev(v: &[f64]) -> f64 {
    if v.len() < 2 {
        return 0.0;
    }
    let m = mean(v);
    let var = v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (v.len() - 1) as f64;
    var.sqrt()
}

/// Tiny deterministic PRNG (SplitMix64) so synthetic data and bootstrap
/// resampling are reproducible without pulling in a dependency.
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }

    /// Standard normal via Box-Muller.
    pub fn gauss(&mut self) -> f64 {
        let u1 = self.next_f64().max(1e-12);
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_matches_hand_values() {
        let v: Vec<f64> = (1..=100).map(f64::from).collect();
        assert!((percentile_sorted(&v, 50.0) - 50.5).abs() < 1e-9);
        // rank = 0.99 * 99 = 98.01 -> 99 + 0.01 * (100 - 99)
        assert!((percentile_sorted(&v, 99.0) - 99.01).abs() < 1e-9);
        assert_eq!(percentile_sorted(&v, 0.0), 1.0);
        assert_eq!(percentile_sorted(&v, 100.0), 100.0);
    }

    #[test]
    fn std_dev_known() {
        assert!((std_dev(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]) - 2.138_089_935).abs() < 1e-6);
    }

    #[test]
    fn rng_is_deterministic() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        assert_eq!(a.next_u64(), b.next_u64());
        let f = a.next_f64();
        assert!((0.0..1.0).contains(&f));
    }
}
