//! Synthetic readings with known truth, shaped like the votes the pipeline builds.

use std::collections::BTreeMap;

use s106::accuracy::{Key, Kind, Vote};

/// SplitMix64, the same family as the generator inside `accuracy.rs`.
pub struct Rng(u64);

impl Rng {
    /// The stream for replication `rep` of a scenario: the seed is hashed so that neighbouring
    /// replications do not share (shifted) SplitMix sequences.
    pub fn stream(seed: u64, rep: u64) -> Self {
        Rng(Rng(seed ^ rep.wrapping_mul(0xD6E8_FEB8_6659_FD93)).next())
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// One scenario's data-generating settings. `None`/0 mean "switched off".
#[derive(Debug, Clone)]
pub struct Params {
    pub schemes: usize,
    /// true accuracy of the Llm (claude), Person and External (datahub) readers
    pub acc: [f64; 3],
    /// size of the answer space for a wrong reading; `None` = wrong values never coincide
    pub k: Option<u64>,
    /// answer space for the hallucinated value on a `not_stated` cell; `None` = same as `k`
    pub k_missing: Option<u64>,
    /// share of cells with a shared corruption that Llm and Person readers follow
    pub rho_shared: f64,
    pub p_follow: f64,
    /// the External reader follows the shared corruption too (no independent source)
    pub shared_all: bool,
    /// chance that the Person reader copies the Llm reading instead of reading the cell itself
    pub anchor: f64,
    /// share of hard cells, and how much every reader's accuracy drops on them
    pub hard_share: f64,
    pub delta: f64,
    /// share of cells whose truth is `not_stated`
    pub missing: f64,
    /// chance that a wrong reading of a found cell says `not_stated`
    pub miss: f64,
    /// chance that a scheme's external readings all report another lineage stage
    pub stage: f64,
    /// V1 cells carry total/market fields and the external reader is a partial vote
    pub partial_external: bool,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            schemes: 20,
            acc: [0.9; 3],
            k: None,
            k_missing: None,
            rho_shared: 0.0,
            p_follow: 0.9,
            shared_all: false,
            anchor: 0.0,
            hard_share: 0.0,
            delta: 0.0,
            missing: 0.0,
            miss: 0.0,
            stage: 0.0,
            partial_external: false,
        }
    }
}

impl Params {
    /// The population accuracy of reader `i` on a random cell: what the estimator is trying to find.
    pub fn marginal(&self, i: usize) -> f64 {
        let a = self.acc[i];
        let mut m = (1.0 - self.hard_share) * a + self.hard_share * (a - self.delta).max(0.05);
        if i < 2 || self.shared_all {
            m *= 1.0 - self.rho_shared * self.p_follow;
        }
        if i == 2 {
            m *= 1.0 - self.stage;
        }
        if i == 1 {
            // a copied reading is right exactly when the Llm reading is
            m = self.anchor * self.marginal(0) + (1.0 - self.anchor) * m;
        }
        m
    }
}

pub const READERS: [(&str, Kind); 3] = [
    ("claude", Kind::Llm),
    ("person", Kind::Person),
    ("datahub", Kind::External),
];
const VARIABLES: [&str; 4] = ["V1", "V2", "V3", "V8"];
const OCR: u32 = 1_000_000;
const STAGE: u32 = 1_000_001;

/// What a reader claims about a cell: nothing stated, or a value (0 is the true one).
#[derive(Clone, Copy)]
enum Claim {
    NotStated,
    Value(u32),
}

/// One replication's votes and the truth of every cell.
pub struct Data {
    pub votes: Vec<Vote>,
    pub truth: BTreeMap<Key, Vote>,
}

pub fn generate(p: &Params, rng: &mut Rng) -> Data {
    let mut votes = vec![];
    let mut truth = BTreeMap::new();
    let mut next_unique = 1u32;
    for s in 0..p.schemes {
        let scheme = format!("S{s:03}");
        let staged = rng.chance(p.stage);
        for var in VARIABLES {
            let cell: Key = (scheme.clone(), var.into(), "deed".into());
            let numeric = p.partial_external && var == "V1";
            let absent = rng.chance(p.missing);
            let true_claim = if absent {
                Claim::NotStated
            } else {
                Claim::Value(0)
            };
            truth.insert(cell.clone(), vote(&cell, 0, true_claim, false, numeric));
            let hard = rng.chance(p.hard_share);
            let corrupted = rng.chance(p.rho_shared);
            let k = if absent { p.k_missing.or(p.k) } else { p.k };
            let mut claims: Vec<Claim> = Vec::with_capacity(READERS.len());
            for (r, (_, kind)) in READERS.iter().enumerate() {
                let mut acc = p.acc[r];
                if hard {
                    acc = (acc - p.delta).max(0.05);
                }
                let claim = if r == 2 && staged {
                    Claim::Value(STAGE)
                } else if r == 1 && p.anchor > 0.0 && rng.chance(p.anchor) {
                    claims[0]
                } else if (r < 2 || p.shared_all) && corrupted && rng.chance(p.p_follow) {
                    Claim::Value(OCR)
                } else if rng.chance(acc) {
                    true_claim
                } else if !absent && rng.chance(p.miss) {
                    Claim::NotStated
                } else {
                    let id = match k {
                        None => {
                            next_unique += 1;
                            next_unique
                        }
                        Some(k) => 1 + rng.below(k - 1) as u32,
                    };
                    Claim::Value(id)
                };
                let partial =
                    numeric && *kind == Kind::External && matches!(claim, Claim::Value(_));
                claims.push(claim);
                votes.push(vote(&cell, r, claim, partial, numeric));
            }
        }
    }
    Data { votes, truth }
}

/// Renders a claim as a vote. A true value is "T"; a wrong one "W<id>". In `numeric` mode (V1 cells
/// with a partial external reader) the fields are total/market, and a partial vote states the total
/// and affordable count, as `totals()` derives them; the total is 100 plus the value's id.
fn vote(cell: &Key, reader: usize, claim: Claim, partial: bool, numeric: bool) -> Vote {
    let (name, kind) = READERS[reader];
    let (status, id) = match claim {
        Claim::NotStated => ("not_stated", None),
        Claim::Value(id) => ("found", Some(id)),
    };
    let value = match id {
        None => String::new(),
        Some(0) => "T".to_string(),
        Some(id) => format!("W{id}"),
    };
    let mut fields = BTreeMap::new();
    match id {
        Some(id) if numeric => {
            fields.insert("total".to_string(), (100 + id).to_string());
            if partial {
                fields.insert("affordable".to_string(), (40 + id).to_string());
            } else {
                fields.insert("market".to_string(), "60".to_string());
            }
        }
        Some(_) => {
            fields.insert("v".to_string(), value.clone());
        }
        None => {}
    }
    Vote {
        cell: cell.clone(),
        reader: name.into(),
        kind,
        family: name.into(),
        status: status.into(),
        fields,
        partial,
        value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The realised share of right readings matches `marginal()` in every scenario of the grid.
    #[test]
    fn marginal_matches_generator() {
        for sc in crate::scenarios::grid() {
            let (mut right, mut n) = ([0usize; 3], 0usize);
            for rep in 0..400 {
                let d = generate(&sc.params, &mut Rng::stream(sc.seed, rep));
                for v in &d.votes {
                    let r = READERS
                        .iter()
                        .position(|(name, _)| *name == v.reader)
                        .unwrap();
                    right[r] += usize::from(s106::accuracy::agree(v, &d.truth[&v.cell]));
                }
                n += d.truth.len();
            }
            for (r, &k) in right.iter().enumerate() {
                let (got, want) = (k as f64 / n as f64, sc.params.marginal(r));
                assert!(
                    (got - want).abs() < 0.01,
                    "{} reader {r}: {got:.4} vs {want:.4}",
                    sc.name
                );
            }
        }
    }
}
