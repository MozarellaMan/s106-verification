//! Accuracy: how often each reader is right, estimated from how often readers agree, with no
//! reader's readings taken as truth (s106-data `docs/verification-rule.md`, section 6).
//!
//! A reader is one producer of readings: an LLM reader (a `machine:proposal/<model>` labeller), a
//! person (a labeller in `labels/`), or an external source. The Datahub's permitted homes are read
//! as a reading of the deed's V1 total and affordable count, except on a scheme with a `variation`
//! source, because the Datahub may record the varied figures.
//!
//! A file listed in `proposals/superseded.csv` (columns `file`, `labeller`, `reason`) no longer
//! votes for that labeller: its readings stay on disk and in the ledger's history, as section 7 of
//! the rule says, but a later file replaced them.
//!
//! The rule asks for a latent-class model per borough. This is its simplest moment estimate (three
//! readers, one accuracy each, no borough split), which is enough while every scheme is in
//! Southwark and few readers overlap.
//!
//! The estimate assumes two readers agree on a cell only when both are right. Then the share of
//! shared cells on which readers i and j agree is a_i·a_j, and three readers give
//! a_i = sqrt(A_ij·A_ik / A_jk). Two readers from one family (one LLM model family, people, one
//! external source) can make the same mistake, which would make them agree when both are wrong and
//! inflate the estimate, so the three readers always come from three families. The lower bound is
//! the 2.5th percentile of the estimate over resamples of schemes. Schemes whose values were seen
//! while the method was built (batch 1) never count towards an estimate.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Serialize;

use crate::labels::{MACHINE, Reading};

/// (scheme, variable_id, source)
pub type Key = (String, String, String);

/// A group's estimate needs at least this accuracy at its lower bound (section 6 of the rule).
pub const GATE: f64 = 0.90;
/// A cell is `confirmed` when its estimated chance of error is at most this.
pub const CONFIRM: f64 = 0.01;
/// Each pair of readers in a triplet shares at least this many cells.
pub const MIN_SHARED: usize = 5;
const RESAMPLES: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Llm,
    Person,
    External,
}

/// The variable group a variable is estimated in. V5 is free text and is not compared.
pub fn group(variable_id: &str) -> Option<&'static str> {
    match variable_id {
        "V1" | "V2" | "V3" | "V8" => Some("G1"),
        "V4" | "V6" => Some("G2"),
        "V7" => Some("G3"),
        _ => None,
    }
}

/// One reader's reading of one cell, cut down to the fields section 4 of the rule compares.
#[derive(Debug, Clone)]
pub struct Vote {
    pub cell: Key,
    pub reader: String,
    pub kind: Kind,
    /// readers in one family can make the same mistake: the LLM's model family, `person`, or the
    /// external source's name
    pub family: String,
    pub status: String,
    pub fields: BTreeMap<String, String>,
    /// the reader states only `total` and `affordable` (the Datahub's reading of the deed's V1)
    pub partial: bool,
    /// the reading as shown when readers differ
    pub value: String,
}

/// Who a labeller is. `None` for the retrieval record, which states only whether a document is in
/// hand.
fn reader(labeller: &str) -> Option<(Kind, String)> {
    let Some(producer) = labeller.strip_prefix(MACHINE) else {
        return Some((Kind::Person, "person".into()));
    };
    let name = producer.trim_start_matches("proposal/");
    match name {
        "sources-status" => None,
        n if n.starts_with("datahub-mapping") || n.starts_with("pld-mapping") => {
            Some((Kind::External, "datahub".into()))
        }
        // `claude-opus-5-5-r1` is in the `claude` family
        n => Some((Kind::Llm, n.split('-').next().unwrap_or(n).to_string())),
    }
}

/// Statuses that say nothing was read, so they never count towards an estimate.
pub fn unread(status: &str) -> bool {
    matches!(status, "source_missing" | "no_source" | "unreadable")
}

/// The fields section 4 of the rule compares for this reading.
fn compared(r: &Reading) -> BTreeMap<String, String> {
    let mut f = r.fields();
    let keep = |f: &mut BTreeMap<String, String>, keys: &[&str]| {
        f.retain(|k, _| keys.contains(&k.as_str()))
    };
    match r.variable_id.as_str() {
        "V1" => {
            f.remove("basis");
        }
        "V2" => keep(&mut f, &["share", "basis", "split"]),
        "V3" => {
            keep(&mut f, &["target", "first_offer"]);
            // `35% habitable_rooms` and `35%` state the same target
            if let Some(t) = f.get_mut("target") {
                *t = t.split_whitespace().next().unwrap_or("").to_string();
            }
        }
        "V4" => keep(&mut f, &["route", "reviews"]),
        "V6" => keep(&mut f, &["cascade", "pil", "pil_amount"]),
        "V7" if r.status == "found" => {
            // the instruments' dates to the month
            let months: BTreeSet<&str> = r
                .value
                .split(|c: char| !(c.is_ascii_digit() || c == '-'))
                .filter(|w| w.len() >= 7 && w.as_bytes()[4] == b'-')
                .map(|w| &w[..7])
                .collect();
            f = BTreeMap::from([(
                "months".into(),
                months.into_iter().collect::<Vec<_>>().join(" "),
            )]);
        }
        "V8" => keep(&mut f, &["completed", "completed_homes"]),
        _ => f.clear(),
    }
    f
}

fn number(v: &str) -> Option<f64> {
    v.parse().ok()
}

fn shown(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{n:.0}")
    } else {
        n.to_string()
    }
}

/// A V1 reading's total and affordable count: total minus market, or the sum of the tenures.
fn totals(f: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let total = f.get("total").and_then(|v| number(v));
    if let Some(t) = total {
        out.insert("total".into(), shown(t));
    }
    let tenures: Vec<&String> = f
        .iter()
        .filter(|(k, _)| !["total", "market", "basis"].contains(&k.as_str()))
        .map(|(_, v)| v)
        .collect();
    let affordable = match (total, f.get("market").and_then(|v| number(v))) {
        (Some(t), Some(m)) => Some(t - m),
        // a total with no tenure at all states no affordable homes
        (Some(_), None) if tenures.is_empty() => Some(0.0),
        _ => {
            let parts: Vec<f64> = tenures.iter().filter_map(|v| number(v)).collect();
            (!parts.is_empty()).then(|| parts.iter().sum())
        }
    };
    if let Some(a) = affordable {
        out.insert("affordable".into(), shown(a));
    }
    out
}

/// The (file, labeller) pairs `proposals/superseded.csv` lists; none when the file is absent.
pub fn superseded(root: &Path) -> anyhow::Result<BTreeSet<(String, String)>> {
    let path = root.join("proposals/superseded.csv");
    if !path.exists() {
        return Ok(BTreeSet::new());
    }
    let mut r = csv::Reader::from_path(&path)?;
    let mut out = BTreeSet::new();
    for row in r.deserialize::<BTreeMap<String, String>>() {
        let row = row?;
        let get = |c: &str| row.get(c).map(|v| v.trim().to_string()).unwrap_or_default();
        out.insert((get("file"), get("labeller")));
    }
    Ok(out)
}

/// Whether a reading comes from a (file, labeller) pair that no longer votes. A reading's origin is
/// `<file>:<line>`.
pub fn is_superseded(r: &Reading, superseded: &BTreeSet<(String, String)>) -> bool {
    let file = r
        .origin
        .rsplit_once(':')
        .map_or(r.origin.as_str(), |(f, _)| f);
    superseded.contains(&(file.to_string(), r.labeller.clone()))
}

/// Every reader's vote, from the latest reading per labeller and cell: proposals, labels and the
/// Datahub's reading of the deed's V1.
pub fn votes(readings: &[Reading]) -> Vec<Vote> {
    let varied: BTreeSet<&str> = readings
        .iter()
        .filter(|r| r.source == "variation")
        .map(|r| r.scheme.as_str())
        .collect();
    let mut out = vec![];
    for r in readings {
        let Some((kind, family)) = reader(&r.labeller) else {
            continue;
        };
        let cell = (r.scheme.clone(), r.variable_id.clone(), r.source.clone());
        let vote = |cell: Key, fields, partial| Vote {
            cell,
            reader: r.labeller.clone(),
            kind,
            family: family.clone(),
            status: r.status.clone(),
            fields,
            partial,
            value: r.value.clone(),
        };
        out.push(vote(cell, compared(r), false));
        if kind == Kind::External
            && r.source == "datahub"
            && r.variable_id == "V1"
            && r.status == "found"
            && !varied.contains(r.scheme.as_str())
        {
            let deed = (r.scheme.clone(), "V1".into(), "deed".into());
            out.push(vote(deed, totals(&r.fields()), true));
        }
    }
    out
}

/// Whether two votes on one cell say the same thing. A partial vote is compared on its own fields.
pub fn agree(a: &Vote, b: &Vote) -> bool {
    match (a.partial, b.partial) {
        (false, false) => a.status == b.status && a.fields == b.fields,
        (true, true) => a.fields == b.fields,
        (true, false) | (false, true) => {
            let (p, o) = if a.partial { (a, b) } else { (b, a) };
            let of = totals(&o.fields);
            o.status == "found" && p.fields.iter().all(|(k, v)| of.get(k) == Some(v))
        }
    }
}

// --------------------------------------------------------------------------------- estimates

/// One reader's estimated accuracy in one variable group.
#[derive(Debug, Clone, Serialize)]
pub struct Estimate {
    pub group: String,
    pub reader: String,
    pub family: String,
    /// the two readers it was estimated against
    pub against: [String; 2],
    /// the fewest cells any two of the three readers share
    pub shared: usize,
    /// the point estimate; above 1 when the agreement pattern does not fit the model
    pub accuracy: f64,
    /// the 2.5th percentile over resamples of schemes
    pub lower: f64,
    /// the model fits: the lower bound is at most 1 (a point estimate can pass 1 by chance)
    pub fits: bool,
}

impl Estimate {
    /// The reader's readings in this group can make a cell `estimated`.
    pub fn passes(&self) -> bool {
        self.fits && self.lower >= GATE
    }
}

/// Agreements and shared cells per pair of readers, each scheme weighted by how often a resample
/// drew it.
type Pairs = BTreeMap<(String, String), (f64, f64)>;

fn pairs(cells: &BTreeMap<&Key, Vec<&Vote>>, weight: &BTreeMap<&str, f64>) -> Pairs {
    let mut out: Pairs = BTreeMap::new();
    for (k, vs) in cells {
        let w = weight.get(k.0.as_str()).copied().unwrap_or(0.0);
        if w == 0.0 {
            continue;
        }
        for (i, a) in vs.iter().enumerate() {
            for b in vs[i + 1..].iter().filter(|b| b.reader != a.reader) {
                let (x, y) = if a.reader < b.reader { (a, b) } else { (b, a) };
                let e = out.entry((x.reader.clone(), y.reader.clone())).or_default();
                e.0 += w * f64::from(u8::from(agree(a, b)));
                e.1 += w;
            }
        }
    }
    out
}

fn pair<'a>(p: &'a Pairs, a: &str, b: &str) -> Option<&'a (f64, f64)> {
    let k = if a < b {
        (a.into(), b.into())
    } else {
        (b.into(), a.into())
    };
    p.get(&k)
}

/// a_i = sqrt(A_ij·A_ik / A_jk); `None` when j and k never agree.
fn triplet(p: &Pairs, i: &str, j: &str, k: &str) -> Option<f64> {
    let rate = |a, b| pair(p, a, b).filter(|x| x.1 > 0.0).map(|x| x.0 / x.1);
    let (ij, ik, jk) = (rate(i, j)?, rate(i, k)?, rate(j, k)?);
    (jk > 0.0).then(|| (ij * ik / jk).sqrt())
}

/// A fixed-seed generator, so the same inputs always give the same bounds.
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

/// Every reader's estimate per group that the votes allow, leaving out the `seen` schemes.
pub fn estimates(votes: &[Vote], seen: &BTreeSet<String>) -> Vec<Estimate> {
    let mut out = vec![];
    let groups: BTreeSet<&str> = votes.iter().filter_map(|v| group(&v.cell.1)).collect();
    for g in groups {
        let mut cells: BTreeMap<&Key, Vec<&Vote>> = BTreeMap::new();
        for v in votes.iter().filter(|v| {
            group(&v.cell.1) == Some(g) && !seen.contains(&v.cell.0) && !unread(&v.status)
        }) {
            cells.entry(&v.cell).or_default().push(v);
        }
        cells.retain(|_, vs| vs.len() >= 2);
        let schemes: Vec<&str> = cells
            .keys()
            .map(|k| k.0.as_str())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let all: BTreeMap<&str, f64> = schemes.iter().map(|s| (*s, 1.0)).collect();
        let p = pairs(&cells, &all);
        let family: BTreeMap<&str, &str> = cells
            .values()
            .flatten()
            .map(|v| (v.reader.as_str(), v.family.as_str()))
            .collect();
        let shared = |a: &str, b: &str| pair(&p, a, b).map_or(0, |x| x.1 as usize);
        for (&i, &fi) in &family {
            // the triplet whose least-shared pair shares the most cells
            let best = family
                .iter()
                .flat_map(|(&j, &fj)| family.iter().map(move |(&k, &fk)| (j, fj, k, fk)))
                .filter(|&(j, fj, k, fk)| j < k && fi != fj && fi != fk && fj != fk)
                .map(|(j, _, k, _)| (shared(i, j).min(shared(i, k)).min(shared(j, k)), j, k))
                .filter(|&(n, _, _)| n >= MIN_SHARED)
                .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(a.1)).then(b.2.cmp(a.2)));
            let Some((n, j, k)) = best else {
                continue;
            };
            let Some(accuracy) = triplet(&p, i, j, k) else {
                continue;
            };
            let mut rng = SplitMix(0x5106);
            let mut draws: Vec<f64> = (0..RESAMPLES)
                .map(|_| {
                    let mut weight: BTreeMap<&str, f64> = BTreeMap::new();
                    for _ in 0..schemes.len() {
                        *weight.entry(schemes[rng.below(schemes.len())]).or_default() += 1.0;
                    }
                    triplet(&pairs(&cells, &weight), i, j, k).unwrap_or(0.0)
                })
                .collect();
            draws.sort_by(f64::total_cmp);
            let lower = draws[RESAMPLES * 25 / 1000];
            out.push(Estimate {
                group: g.into(),
                reader: i.into(),
                family: fi.into(),
                against: [j.into(), k.into()],
                shared: n,
                accuracy,
                lower,
                fits: lower <= 1.0,
            });
        }
    }
    out
}

/// The chance that `shown` is wrong, given every vote on its cell and the readers' lower-bound
/// accuracies; `None` when no reader that voted has a fitting estimate. Two wrong readers are
/// taken to give the same wrong value whenever they agree, which can only raise the figure.
pub fn error(
    shown: &Vote,
    votes: &[&Vote],
    estimates: &BTreeMap<(String, String), &Estimate>,
) -> Option<f64> {
    let g = group(&shown.cell.1)?;
    let known: Vec<(&Vote, f64)> = votes
        .iter()
        .filter_map(|v| {
            let e = estimates.get(&(g.to_string(), v.reader.clone()))?;
            e.fits.then_some((*v, e.lower.clamp(0.0, 1.0)))
        })
        .collect();
    if known.is_empty() {
        return None;
    }
    let likelihood = |c: &Vote| {
        known
            .iter()
            .map(|(v, a)| if agree(v, c) { *a } else { 1.0 - a })
            .product::<f64>()
    };
    // each distinct full reading is one candidate value; one more for "none of them"
    let mut candidates: Vec<&Vote> = vec![];
    for v in votes.iter().filter(|v| !v.partial) {
        if !candidates.iter().any(|c| agree(c, v)) {
            candidates.push(v);
        }
    }
    let none: f64 = known.iter().map(|(_, a)| 1.0 - a).product();
    let total: f64 = candidates.iter().map(|c| likelihood(c)).sum::<f64>() + none;
    (total > 0.0).then(|| 1.0 - likelihood(shown) / total)
}
