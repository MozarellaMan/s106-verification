//! Data for the results explorer (`explorer/`): one JSON file of London sites, baked from a
//! Datahub baseline run in the data repository.
//!
//! Only Datahub-derived figures go in, as boroughs recorded them: the site is meant to be public,
//! and nothing scheme-level from deeds or the ledger is public before human review. The counting
//! rules (what counts as permitted, superseded, a tenure group) are the baseline run's own, read
//! from its `applications.csv`: every count is of final homes (`final_*`, homes no later permission
//! replaced), like the baseline's headline. This only adds each site's centroid from the same
//! snapshot, and counts every record it leaves out so the page can say so.

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
pub use site::{Bundle, Site, Sources, Status};

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

/// Paths are relative to the data root, so the footer names them as the data repository does.
pub struct Inputs {
    pub applications: PathBuf,
    pub snapshot: PathBuf,
    pub boundaries: PathBuf,
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
        });
    }
    if sites.is_empty() {
        bail!("no permitted sites with a centroid in {}", path.display());
    }
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
        },
        sites,
    })
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
        };
        let bundle = bake(root, &inputs).unwrap();
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
    }
}
