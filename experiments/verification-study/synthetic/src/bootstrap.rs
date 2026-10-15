//! A second interval method for the coverage diagnostic, from the same bootstrap draws.
//!
//! `estimates()` keeps its draws private and reports only the percentile lower bound (the 2.5th
//! percentile). To report the basic (reverse percentile) bound `2 * estimate - q97.5` beside it, this
//! file repeats the library's resampling step by step: the same cells (two or more votes, no unread
//! status), the same sorted scheme list, the same SplitMix seed 0x5106, 1000 resamples of schemes,
//! and the same triplet formula on weighted agreement counts. `replicate()` checks that the
//! 2.5th percentile of these draws equals the library's `lower` exactly, so the mirror cannot drift
//! silently; if `accuracy.rs` changes its resampling, that check fails and this file must follow.

use std::collections::{BTreeMap, BTreeSet};

use s106::accuracy::{Key, Vote, agree, group, unread};

const RESAMPLES: usize = 1000;

struct SplitMix(u64);

impl SplitMix {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) % n as u64) as usize
    }
}

/// Weighted (agreements, shared cells) between two readers.
fn rate(
    cells: &BTreeMap<&Key, Vec<&Vote>>,
    w: &BTreeMap<&str, f64>,
    a: &str,
    b: &str,
) -> Option<f64> {
    let (mut agreed, mut shared) = (0.0, 0.0);
    for (k, vs) in cells {
        let wk = w.get(k.0.as_str()).copied().unwrap_or(0.0);
        if wk == 0.0 {
            continue;
        }
        for (i, x) in vs.iter().enumerate() {
            for y in &vs[i + 1..] {
                if (x.reader == a && y.reader == b) || (x.reader == b && y.reader == a) {
                    agreed += wk * f64::from(u8::from(agree(x, y)));
                    shared += wk;
                }
            }
        }
    }
    (shared > 0.0).then(|| agreed / shared)
}

/// a_i = sqrt(A_ij A_ik / A_jk), as `triplet()` in `accuracy.rs`.
fn triplet(
    cells: &BTreeMap<&Key, Vec<&Vote>>,
    w: &BTreeMap<&str, f64>,
    i: &str,
    j: &str,
    k: &str,
) -> Option<f64> {
    let (ij, ik, jk) = (
        rate(cells, w, i, j)?,
        rate(cells, w, i, k)?,
        rate(cells, w, j, k)?,
    );
    (jk > 0.0).then(|| (ij * ik / jk).sqrt())
}

/// The sorted bootstrap draws of reader `i`'s accuracy in group G1 against `j` and `k`, as
/// `estimates()` computes them (with no `seen` schemes).
pub fn draws(votes: &[Vote], i: &str, j: &str, k: &str) -> Vec<f64> {
    let mut cells: BTreeMap<&Key, Vec<&Vote>> = BTreeMap::new();
    for v in votes
        .iter()
        .filter(|v| group(&v.cell.1) == Some("G1") && !unread(&v.status))
    {
        cells.entry(&v.cell).or_default().push(v);
    }
    cells.retain(|_, vs| vs.len() >= 2);
    let schemes: Vec<&str> = cells
        .keys()
        .map(|k| k.0.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut rng = SplitMix(0x5106);
    let mut out: Vec<f64> = (0..RESAMPLES)
        .map(|_| {
            let mut w: BTreeMap<&str, f64> = BTreeMap::new();
            for _ in 0..schemes.len() {
                *w.entry(schemes[rng.below(schemes.len())]).or_default() += 1.0;
            }
            triplet(&cells, &w, i, j, k).unwrap_or(0.0)
        })
        .collect();
    out.sort_by(f64::total_cmp);
    out
}

/// The percentile lower bound (what `estimates()` reports) and the basic bootstrap lower bound
/// `2 * estimate - q97.5`.
pub fn lower_bounds(draws: &[f64], estimate: f64) -> (f64, f64) {
    let q025 = draws[RESAMPLES * 25 / 1000];
    let q975 = draws[RESAMPLES * 975 / 1000];
    (q025, 2.0 * estimate - q975)
}
