//! Runs the unchanged estimator on one replication and condenses replications into metrics.

use std::collections::{BTreeMap, BTreeSet};

use s106::accuracy::{CONFIRM, Estimate, Kind, Vote, agree, error, estimates};
use serde::Serialize;

use crate::generator::{Params, Rng, generate};
use crate::scenarios::Scenario;

/// What one reader's estimate looked like in one replication.
#[derive(Clone, Copy)]
pub struct Est {
    pub accuracy: f64,
    pub lower: f64,
    pub fits: bool,
    pub passes: bool,
}

/// A cell on which every reader agreed and `error()` gave a figure.
#[derive(Clone, Copy)]
pub struct CellOutcome {
    pub error: f64,
    pub wrong: bool,
    pub estimated: bool,
    pub confirmed: bool,
}

pub struct Rep {
    /// per reader (claude, person, datahub); `None` when no triplet formed
    pub est: [Option<Est>; 3],
    pub cells: Vec<CellOutcome>,
}

/// Generates the data, calls `estimates()` and `error()` as `results::ledger` does, and scores
/// every cell against the truth. The shown reading is the claude (proposal) reading; a cell is
/// looked at only when all its readers agree, as in the ledger.
pub fn replicate(p: &Params, seed: u64, rep: u64) -> Rep {
    let data = generate(p, &mut Rng::stream(seed, rep));
    let ests = estimates(&data.votes, &BTreeSet::new());
    let by_reader: BTreeMap<(String, String), &Estimate> = ests
        .iter()
        .map(|e| ((e.group.clone(), e.reader.clone()), e))
        .collect();
    let est = ["claude", "person", "datahub"].map(|r| {
        by_reader
            .get(&("G1".to_string(), r.to_string()))
            .map(|e| Est {
                accuracy: e.accuracy,
                lower: e.lower,
                fits: e.fits,
                passes: e.passes(),
            })
    });

    let mut cells: BTreeMap<_, Vec<&Vote>> = BTreeMap::new();
    for v in &data.votes {
        cells.entry(v.cell.clone()).or_default().push(v);
    }
    let passes = est[0].is_some_and(|e| e.passes);
    let mut out = vec![];
    for (key, vs) in &cells {
        let agreed = vs
            .iter()
            .enumerate()
            .all(|(i, a)| vs[i + 1..].iter().all(|b| agree(a, b)));
        let shown = vs.iter().find(|v| v.reader == "claude" && !v.partial);
        let (true, Some(shown)) = (agreed && vs.len() >= 2, shown) else {
            continue;
        };
        let Some(error) = error(shown, vs, &by_reader) else {
            continue;
        };
        let not_llm = vs.iter().any(|v| v.kind != Kind::Llm);
        out.push(CellOutcome {
            error,
            wrong: !agree(shown, &data.truth[key]),
            estimated: passes,
            confirmed: passes && error <= CONFIRM && not_llm,
        });
    }
    Rep { est, cells: out }
}

/// Error bins of the reliability table, as (label, lower edge, upper edge).
pub const BINS: [(&str, f64, f64); 5] = [
    ("0-1%", 0.0, 0.01),
    ("1-5%", 0.01, 0.05),
    ("5-20%", 0.05, 0.20),
    ("20-50%", 0.20, 0.50),
    ("50%+", 0.50, f64::INFINITY),
];

/// One row of `scenarios.csv`. Optional values are empty when they do not apply.
#[derive(Serialize)]
pub struct Row {
    pub scenario: String,
    pub reps: usize,
    pub schemes: usize,
    pub nominal_acc: String,
    pub k: String,
    pub rho_shared: f64,
    pub hard_share: f64,
    pub delta: f64,
    pub missing: f64,
    pub stage: f64,
    pub true_a1: f64,
    pub true_a2: f64,
    pub true_a3: f64,
    pub no_estimate: f64,
    pub fits_false: f64,
    pub mean_a1: f64,
    pub sd_a1: f64,
    pub bias_a1: f64,
    pub coverage_a1: f64,
    pub mean_lower_a1: f64,
    pub sd_lower_a1: f64,
    /// share of estimates whose lower bound is above the true a1 (1 - coverage)
    pub lower_above_true_a1: f64,
    /// mean of a1_hat - lower, over all estimates and over those with lower above the true a1
    pub mean_width_a1: f64,
    pub mean_width_a1_above: Option<f64>,
    pub bias_a2: f64,
    pub coverage_a2: f64,
    pub bias_a3: f64,
    pub coverage_a3: f64,
    pub gate_pass: f64,
    pub false_pass: Option<f64>,
    pub power: Option<f64>,
    pub estimated_cells: usize,
    pub estimated_wrong_share: Option<f64>,
    pub confirmed_cells: usize,
    pub confirmed_wrong: usize,
    pub confirmed_wrong_share: Option<f64>,
    pub confirmed_per_rep: f64,
    pub allwrong_cells: usize,
    pub allwrong_estimated: usize,
    pub allwrong_confirmed: usize,
    pub allwrong_error_mean: Option<f64>,
    pub allwrong_error_min: Option<f64>,
}

/// One row of `cells.csv`: estimated cells (reader gate passed) by posterior error bin.
#[derive(Serialize)]
pub struct BinRow {
    pub scenario: String,
    pub bin: &'static str,
    pub n_cells: usize,
    pub n_wrong: usize,
    pub share_wrong: Option<f64>,
    pub mean_error: Option<f64>,
}

fn mean(x: &[f64]) -> f64 {
    x.iter().sum::<f64>() / x.len().max(1) as f64
}

fn sd(x: &[f64]) -> f64 {
    let m = mean(x);
    (x.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (x.len().max(2) - 1) as f64).sqrt()
}

fn share(n: usize, d: usize) -> Option<f64> {
    (d > 0).then(|| n as f64 / d as f64)
}

pub fn summarise(sc: &Scenario, reps: &[Rep]) -> (Row, Vec<BinRow>) {
    let p = &sc.params;
    let n = reps.len();
    let truth = [p.marginal(0), p.marginal(1), p.marginal(2)];
    let with = |i: usize| -> Vec<Est> { reps.iter().filter_map(|r| r.est[i]).collect() };
    let bias = |i: usize| mean(&with(i).iter().map(|e| e.accuracy).collect::<Vec<_>>()) - truth[i];
    let coverage = |i: usize| {
        let e = with(i);
        e.iter().filter(|e| e.lower <= truth[i]).count() as f64 / e.len().max(1) as f64
    };
    let e1 = with(0);
    let acc1: Vec<f64> = e1.iter().map(|e| e.accuracy).collect();
    let gate_pass = reps
        .iter()
        .filter(|r| r.est[0].is_some_and(|e| e.passes))
        .count() as f64
        / n as f64;

    let cells: Vec<&CellOutcome> = reps.iter().flat_map(|r| &r.cells).collect();
    let estimated: Vec<_> = cells.iter().filter(|c| c.estimated).collect();
    let confirmed: Vec<_> = cells.iter().filter(|c| c.confirmed).collect();
    let allwrong: Vec<_> = cells.iter().filter(|c| c.wrong).collect();
    let wrong_errors: Vec<f64> = allwrong.iter().map(|c| c.error).collect();

    let row = Row {
        scenario: sc.name.clone(),
        reps: n,
        schemes: p.schemes,
        nominal_acc: format!("{}/{}/{}", p.acc[0], p.acc[1], p.acc[2]),
        k: match p.k_missing {
            Some(km) => format!(
                "{} ({km} on not_stated)",
                p.k.map_or("inf".into(), |k| k.to_string())
            ),
            None => p.k.map_or("inf".into(), |k| k.to_string()),
        },
        rho_shared: p.rho_shared,
        hard_share: p.hard_share,
        delta: p.delta,
        missing: p.missing,
        stage: p.stage,
        true_a1: truth[0],
        true_a2: truth[1],
        true_a3: truth[2],
        no_estimate: 1.0 - e1.len() as f64 / n as f64,
        fits_false: e1.iter().filter(|e| !e.fits).count() as f64 / e1.len().max(1) as f64,
        mean_a1: mean(&acc1),
        sd_a1: sd(&acc1),
        bias_a1: bias(0),
        coverage_a1: coverage(0),
        mean_lower_a1: mean(&e1.iter().map(|e| e.lower).collect::<Vec<_>>()),
        sd_lower_a1: sd(&e1.iter().map(|e| e.lower).collect::<Vec<_>>()),
        lower_above_true_a1: e1.iter().filter(|e| e.lower > truth[0]).count() as f64
            / e1.len().max(1) as f64,
        mean_width_a1: mean(&e1.iter().map(|e| e.accuracy - e.lower).collect::<Vec<_>>()),
        mean_width_a1_above: {
            let w: Vec<f64> = e1
                .iter()
                .filter(|e| e.lower > truth[0])
                .map(|e| e.accuracy - e.lower)
                .collect();
            (!w.is_empty()).then(|| mean(&w))
        },
        bias_a2: bias(1),
        coverage_a2: coverage(1),
        bias_a3: bias(2),
        coverage_a3: coverage(2),
        gate_pass,
        false_pass: (truth[0] < 0.90).then_some(gate_pass),
        power: (truth[0] >= 0.95).then_some(gate_pass),
        estimated_cells: estimated.len(),
        estimated_wrong_share: share(
            estimated.iter().filter(|c| c.wrong).count(),
            estimated.len(),
        ),
        confirmed_cells: confirmed.len(),
        confirmed_wrong: confirmed.iter().filter(|c| c.wrong).count(),
        confirmed_wrong_share: share(
            confirmed.iter().filter(|c| c.wrong).count(),
            confirmed.len(),
        ),
        confirmed_per_rep: confirmed.len() as f64 / n as f64,
        allwrong_cells: allwrong.len(),
        allwrong_estimated: allwrong.iter().filter(|c| c.estimated).count(),
        allwrong_confirmed: allwrong.iter().filter(|c| c.confirmed).count(),
        allwrong_error_mean: (!wrong_errors.is_empty()).then(|| mean(&wrong_errors)),
        allwrong_error_min: wrong_errors.iter().copied().min_by(f64::total_cmp),
    };

    let bins = BINS
        .iter()
        .map(|&(bin, lo, hi)| {
            // upper edge inclusive, so the 0-1% bin is exactly the cells with error <= CONFIRM
            let inside: Vec<_> = estimated
                .iter()
                .filter(|c| (c.error > lo || lo == 0.0) && c.error <= hi)
                .collect();
            let wrong = inside.iter().filter(|c| c.wrong).count();
            BinRow {
                scenario: sc.name.clone(),
                bin,
                n_cells: inside.len(),
                n_wrong: wrong,
                share_wrong: share(wrong, inside.len()),
                mean_error: (!inside.is_empty())
                    .then(|| mean(&inside.iter().map(|c| c.error).collect::<Vec<_>>())),
            }
        })
        .collect();
    (row, bins)
}
