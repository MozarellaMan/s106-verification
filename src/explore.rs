//! Data for the results explorer (`explorer/`): one JSON file of London sites, baked from a
//! Datahub baseline run in the data repository.
//!
//! Only Datahub-derived figures go in, as boroughs recorded them: the site is meant to be public,
//! and nothing scheme-level from deeds or the ledger is public before human review. The counting
//! rules (what counts as permitted, superseded, a tenure group) are the baseline run's own, read
//! from its `applications.csv`: every count is of final homes (`final_*`, homes no later permission
//! replaced), like the baseline's headline. This only adds each site's centroid from the same
//! snapshot, and counts every record it leaves out so the page can say so.
//!
//! A private build (`--results`) also adds each scheme's cells from a results ledger, with their
//! check levels. Those are unreviewed readings: that build is for local viewing, never for Pages.

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use serde::Deserialize;
use serde_json::Value;

// The file format, shared with the explorer so the two can't drift apart.
#[path = "../explorer/src/site.rs"]
mod site;
pub use site::{Bundle, Check, Finding, Lineage, Reading, Retrieval, Site, Sources, Status};

/// One row of the baseline run's `tables/applications.csv` (only the columns used here).
#[derive(Deserialize)]
struct Application {
    id: String,
    borough: String,
    #[serde(rename = "ref")]
    reference: String,
    status: String,
    permitted: String,
    permission_year: Option<i32>,
    final_homes: u32,
    final_affordable: u32,
    final_tenure_unknown: u32,
    final_completed: u32,
    completion_date: String,
    last_updated: String,
    site: String,
}

/// One row of a results folder's `retrieval.csv`.
#[derive(Deserialize)]
struct RetrievalRow {
    scheme: String,
    source: String,
    documents: u32,
    in_hand: u32,
    not_in_hand: String,
}

/// Paths are relative to the data root, so the footer names them as the data repository does.
#[derive(Clone)]
pub struct Inputs {
    pub applications: PathBuf,
    pub snapshot: PathBuf,
    pub boundaries: PathBuf,
    /// a results folder (`results/<batch>`) whose `ledger.csv` to add: private builds only
    pub results: Option<PathBuf>,
    /// a results folder whose checked deed cells (and nothing else) to add: fit to publish
    pub deed_cells: Option<PathBuf>,
}

/// One row of a results folder's `ledger.csv` (only the columns used here).
#[derive(Deserialize)]
struct LedgerRow {
    scheme: String,
    stage: Lineage,
    variable_id: String,
    variable: String,
    source: String,
    status: String,
    value: String,
    basis: String,
    pages: String,
    check: Check,
}

pub fn bake(root: &Path, inputs: &Inputs) -> Result<Bundle> {
    let centroids = centroids(&root.join(&inputs.snapshot))?;
    let path = root.join(&inputs.applications);
    let mut reader =
        csv::Reader::from_path(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut sites = Vec::new();
    let (mut records, mut not_permitted, mut no_decision_year) = (0, 0, 0);
    let (mut wholly_superseded, mut without_centroid) = (0, 0);
    for row in reader.deserialize() {
        let a: Application = row.with_context(|| format!("parsing {}", path.display()))?;
        records += 1;
        if a.permitted != "True" {
            not_permitted += 1;
            continue;
        }
        let Some(year) = a.permission_year else {
            no_decision_year += 1;
            continue;
        };
        if a.final_homes == 0 {
            wholly_superseded += 1;
            continue;
        }
        let Some(&(lat, lon)) = centroids.get(&a.id) else {
            without_centroid += 1;
            continue;
        };
        sites.push(Site {
            borough: a.borough,
            reference: a.reference,
            site: non_empty(a.site),
            status: Status::from_datahub(&a.status),
            year,
            homes: a.final_homes,
            affordable: a.final_affordable,
            known: a.final_homes.saturating_sub(a.final_tenure_unknown),
            completed: a.final_completed,
            completion_date: non_empty(a.completion_date),
            updated: a.last_updated.chars().take(10).collect(),
            lat,
            lon,
            readings: Vec::new(),
            retrieval: Vec::new(),
        });
    }
    if sites.is_empty() {
        bail!("no permitted sites with a centroid in {}", path.display());
    }
    let unmatched = match (&inputs.results, &inputs.deed_cells) {
        (Some(_), Some(_)) => bail!("a build takes either --results or --deed-cells, not both"),
        (Some(dir), None) => add_results(&root.join(dir), &mut sites, Keep::All)?,
        (None, Some(dir)) => add_results(&root.join(dir), &mut sites, Keep::CheckedDeedCells)?,
        (None, None) => Vec::new(),
    };
    let from = inputs.results.as_ref().or(inputs.deed_cells.as_ref());
    let name = |p: &Path| p.display().to_string();
    Ok(Bundle {
        sources: Sources {
            applications: name(&inputs.applications),
            snapshot: name(&inputs.snapshot),
            boundaries: name(&inputs.boundaries),
            snapshot_date: date_in_words(&inputs.snapshot)?,
            records,
            not_permitted,
            no_decision_year,
            wholly_superseded,
            without_centroid,
            results: inputs.results.as_deref().map(name),
            deed_cells: inputs.deed_cells.as_deref().map(name),
            unmatched,
            data_commit: from.and_then(|_| commit(root)),
        },
        sites,
    })
}

/// Which of a results folder's rows a build takes.
#[derive(Clone, Copy, PartialEq)]
enum Keep {
    /// every ledger cell and retrieval row: a private build
    All,
    /// only cells read from the deed that a person has checked, and no retrieval rows: the
    /// public build's whitelist
    CheckedDeedCells,
}

impl Keep {
    fn reading(self, r: &Reading) -> bool {
        match self {
            Keep::All => true,
            Keep::CheckedDeedCells => r.source == "deed" && r.check >= Check::Anchored,
        }
    }
}

/// Attach each scheme's ledger cells, and its retrieval rows if the folder has them, to the one
/// site with that reference; returns the schemes that matched no site, or more than one. Schemes
/// left with nothing to show are skipped.
fn add_results(dir: &Path, sites: &mut [Site], keep: Keep) -> Result<Vec<String>> {
    let mut by_scheme: HashMap<String, (Vec<Reading>, Vec<Retrieval>)> = HashMap::new();
    let ledger = dir.join("ledger.csv");
    let mut reader =
        csv::Reader::from_path(&ledger).with_context(|| format!("reading {}", ledger.display()))?;
    for row in reader.deserialize() {
        let r: LedgerRow = row.with_context(|| format!("parsing {}", ledger.display()))?;
        let status = Finding::from_ledger(&r.status).with_context(|| {
            format!(
                "{}: unknown status {:?} for {}",
                ledger.display(),
                r.status,
                r.scheme
            )
        })?;
        let reading = Reading {
            stage: r.stage,
            variable: r.variable_id,
            name: r.variable,
            source: r.source,
            status,
            value: non_empty(r.value),
            basis: non_empty(r.basis),
            pages: non_empty(r.pages),
            check: r.check,
        };
        if keep.reading(&reading) {
            by_scheme.entry(r.scheme).or_default().0.push(reading);
        }
    }
    let retrieval = dir.join("retrieval.csv");
    if keep == Keep::All && retrieval.exists() {
        let mut reader = csv::Reader::from_path(&retrieval)
            .with_context(|| format!("reading {}", retrieval.display()))?;
        for row in reader.deserialize() {
            let r: RetrievalRow =
                row.with_context(|| format!("parsing {}", retrieval.display()))?;
            by_scheme.entry(r.scheme).or_default().1.push(Retrieval {
                source: r.source,
                documents: r.documents,
                in_hand: r.in_hand,
                not_in_hand: non_empty(r.not_in_hand),
            });
        }
    }
    let mut unmatched = Vec::new();
    for (scheme, (readings, retrieval)) in by_scheme {
        let mut found = sites.iter_mut().filter(|s| s.reference == scheme);
        match (found.next(), found.next()) {
            (Some(site), None) => (site.readings, site.retrieval) = (readings, retrieval),
            _ => unmatched.push(scheme),
        }
    }
    unmatched.sort();
    Ok(unmatched)
}

/// The data repository's short commit, so a private build's screenshots can be traced.
fn commit(root: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn non_empty(s: String) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// A snapshot folder is named for its retrieval time (`2026-10-14T00_14_09Z`).
fn date_in_words(snapshot: &Path) -> Result<String> {
    let name = snapshot.file_name().unwrap_or_default().to_string_lossy();
    let date = chrono::NaiveDate::parse_from_str(name.get(..10).unwrap_or(""), "%Y-%m-%d")
        .with_context(|| format!("{} is not named for its date", snapshot.display()))?;
    Ok(date.format("%-d %B %Y").to_string())
}

/// Datahub record id -> (lat, lon), from every `*.json.gz` search response in a snapshot folder.
fn centroids(dir: &Path) -> Result<HashMap<String, (f64, f64)>> {
    let mut out = HashMap::new();
    let entries = std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
    for entry in entries {
        let path = entry?.path();
        if !path.to_string_lossy().ends_with(".json.gz") {
            continue;
        }
        let file = File::open(&path).with_context(|| format!("opening {}", path.display()))?;
        let body: Value = serde_json::from_reader(GzDecoder::new(file))
            .with_context(|| format!("parsing {}", path.display()))?;
        for hit in body["response"]["hits"]["hits"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let source = &hit["_source"];
            let (Some(id), Some(lat), Some(lon)) = (
                source["id"].as_str(),
                number(&source["centroid"]["lat"]),
                number(&source["centroid"]["lon"]),
            ) else {
                continue;
            };
            out.insert(id.to_string(), (round(lat), round(lon)));
        }
    }
    Ok(out)
}

/// The Datahub writes coordinates as strings on some records and numbers on others.
fn number(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str()?.parse().ok())
}

/// Five decimal places is about a metre, finer than a centroid means.
fn round(x: f64) -> f64 {
    (x * 1e5).round() / 1e5
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn bakes_permitted_sites_with_centroids() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("applications.csv"),
            "id,borough,ref,status,permitted,permission_year,final_homes,final_affordable,final_tenure_unknown,final_completed,completion_date,last_updated,site\n\
             A-1,Alpha,1,Completed,True,2016,40,10,4,40,01/02/2020,2026-01-02T00:00:00Z,Gasworks\n\
             A-2,Alpha,2,Refused,False,,0,0,30,0,,2026-01-02T00:00:00Z,Refused site\n\
             A-3,Alpha,3,Superseded,True,2015,0,0,0,0,,2026-01-02T00:00:00Z,Replaced\n\
             A-4,Alpha,4,Approved,True,2020,12,0,12,0,,2026-01-02T00:00:00Z,No centroid\n\
             A-5,Alpha,5,Approved,True,,15,0,0,0,,2026-01-02T00:00:00Z,No decision year\n\
             A-6,Alpha,6,lapsed,True,2019,30,9,0,0,,2026-01-02T00:00:00Z,\n",
        )
        .unwrap();
        let snap = "2026-10-14T00_14_09Z";
        std::fs::create_dir(root.join(snap)).unwrap();
        let hits = serde_json::json!({"response": {"hits": {"hits": [
            {"_source": {"id": "A-1", "centroid": {"lat": "51.5000012", "lon": -0.1}}},
            {"_source": {"id": "A-3", "centroid": {"lat": 51.6, "lon": -0.2}}},
            {"_source": {"id": "A-6", "centroid": {"lat": 51.4, "lon": -0.3}}},
        ]}}});
        let mut gz = flate2::write::GzEncoder::new(
            File::create(root.join(snap).join("Alpha.json.gz")).unwrap(),
            flate2::Compression::default(),
        );
        gz.write_all(hits.to_string().as_bytes()).unwrap();
        gz.finish().unwrap();

        let inputs = Inputs {
            applications: "applications.csv".into(),
            snapshot: snap.into(),
            boundaries: "boroughs.geojson".into(),
            results: None,
            deed_cells: None,
        };
        let bundle = bake(root, &inputs).unwrap();
        assert!(bundle.sources.results.is_none());
        assert!(bundle.sites.iter().all(|s| s.readings.is_empty()));
        let src = &bundle.sources;
        assert_eq!(src.snapshot_date, "14 October 2026");
        assert_eq!(
            (src.records, src.not_permitted, src.no_decision_year),
            (6, 1, 1)
        );
        assert_eq!((src.wholly_superseded, src.without_centroid), (1, 1));
        assert_eq!(bundle.sites.len(), 2);

        let s = &bundle.sites[0];
        assert_eq!(
            (s.homes, s.affordable, s.known, s.completed),
            (40, 10, 36, 40)
        );
        assert_eq!((s.lat, s.lon, s.year), (51.5, -0.1, 2016));
        assert_eq!(s.updated, "2026-01-02");
        assert!(s.status == Status::Completed);

        let s = &bundle.sites[1];
        assert!(
            s.status == Status::Lapsed,
            "status is folded case-insensitively"
        );
        assert_eq!(
            (s.site.as_deref(), s.completion_date.as_deref()),
            (None, None)
        );

        // A private build adds the ledger's cells to the matching site, and names the rest.
        std::fs::create_dir(root.join("results")).unwrap();
        std::fs::write(
            root.join("results/ledger.csv"),
            "scheme,stage,variable_id,variable,source,status,value,basis,doc_ids,pages,check,reader,origin\n\
             1,obligation,V1,Total homes,deed,found,total=40; SR=10,units,D-1,D-1:3,proposed,machine:x,p.csv:2\n\
             1,decision,V1,Total homes,gla,source_missing,,,,,proposed,machine:x,p.csv:3\n\
             9,decision,V1,Total homes,deed,,,,,,unread,,\n",
        )
        .unwrap();
        std::fs::write(
            root.join("results/retrieval.csv"),
            "scheme,source,documents,in_hand,not_in_hand\n\
             1,viability,3,0,listed_not_fetched x3\n\
             1,deed,1,1,\n",
        )
        .unwrap();
        let private = Inputs {
            results: Some("results".into()),
            ..inputs.clone()
        };
        let bundle = bake(root, &private).unwrap();
        assert_eq!(bundle.sources.results.as_deref(), Some("results"));
        assert_eq!(bundle.sources.unmatched, ["9"]);
        let r = &bundle.sites[0].readings;
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].stage, Lineage::Obligation);
        assert_eq!(r[1].status, Finding::SourceMissing);
        assert_eq!(
            (r[0].value.as_deref(), r[0].check),
            (Some("total=40; SR=10"), Check::Proposed)
        );
        assert_eq!(r[1].value, None);
        let got = &bundle.sites[0].retrieval;
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].not_in_hand.as_deref(), Some("listed_not_fetched x3"));
        assert_eq!(got[1].not_in_hand, None);
        assert!(bundle.sites[1].readings.is_empty());

        // A public build keeps only deed cells a person has checked, and no retrieval rows.
        std::fs::write(
            root.join("results/ledger.csv"),
            "scheme,stage,variable_id,variable,source,status,value,basis,doc_ids,pages,check,reader,origin\n\
             1,obligation,V1,Total homes,deed,found,total=40; SR=10,units,D-1,D-1:3,anchored,human:AB,l.csv:2\n\
             1,obligation,V2,Share,deed,found,share=25%,units,D-1,D-1:4,proposed,machine:x,p.csv:3\n\
             1,decision,V1,Total homes,committee_report,found,total=40,units,C-1,C-1:2,anchored,human:AB,l.csv:4\n\
             9,obligation,V1,Total homes,deed,found,total=9,units,D-9,D-9:1,proposed,machine:x,p.csv:5\n",
        )
        .unwrap();
        let public = Inputs {
            deed_cells: Some("results".into()),
            ..inputs.clone()
        };
        let bundle = bake(root, &public).unwrap();
        assert_eq!(bundle.sources.results, None);
        assert_eq!(bundle.sources.deed_cells.as_deref(), Some("results"));
        assert!(bundle.sources.unmatched.is_empty(), "9 has nothing to show");
        let r = &bundle.sites[0].readings;
        assert_eq!(r.len(), 1);
        assert_eq!(
            (r[0].source.as_str(), r[0].check),
            ("deed", Check::Anchored)
        );
        assert!(bundle.sites[0].retrieval.is_empty());

        let both = Inputs {
            results: Some("results".into()),
            ..public
        };
        assert!(bake(root, &both).is_err());
    }
}
