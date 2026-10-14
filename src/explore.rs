//! Data for the results explorer (`explorer/`): one JSON file of London sites, baked from a
//! Datahub baseline run in the data repository.
//!
//! Only Datahub-derived figures go in, as boroughs recorded them: the site is meant to be public,
//! and nothing scheme-level from deeds or the ledger is public before human review. The counting
//! rules (what counts as permitted, superseded, a tenure group) are the baseline run's own, read
//! from its `applications.csv`; this only adds each site's centroid from the same snapshot and
//! drops sites whose every proposed home was superseded by a later permission.

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    proposed: u32,
    affordable: u32,
    gains_completed: u32,
    units_superseded: u32,
    tenure_unknown: u32,
    completion_date: String,
    last_updated: String,
    site: String,
}

/// A site as the explorer reads it. Field names are short because there are ~20,000 of them.
#[derive(Serialize)]
pub struct Site {
    pub id: String,
    pub borough: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub site: String,
    pub status: String,
    pub year: i32,
    /// homes proposed (gains)
    pub homes: u32,
    /// of which affordable (low-cost rent plus intermediate)
    pub affordable: u32,
    /// homes with a recorded tenure; affordable shares are of these
    pub known: u32,
    /// homes with an actual completion date
    pub completed: u32,
    pub completion_date: String,
    pub updated: String,
    pub lat: f64,
    pub lon: f64,
}

/// Where the figures came from, shown on the site's footer.
#[derive(Serialize)]
pub struct Sources {
    pub applications: String,
    pub snapshot: String,
    pub boundaries: String,
    pub sites: usize,
    pub without_centroid: usize,
}

#[derive(Serialize)]
pub struct Bundle {
    pub sources: Sources,
    pub sites: Vec<Site>,
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
    let (mut sites, mut without_centroid) = (Vec::new(), 0);
    for row in reader.deserialize() {
        let a: Application = row.with_context(|| format!("parsing {}", path.display()))?;
        let Some(year) = a.permission_year else {
            continue;
        };
        if a.permitted != "True" || a.units_superseded >= a.proposed {
            continue;
        }
        let Some(&(lat, lon)) = centroids.get(&a.id) else {
            without_centroid += 1;
            continue;
        };
        sites.push(Site {
            id: a.id,
            borough: a.borough,
            reference: a.reference,
            site: a.site,
            status: a.status.trim().to_string(),
            year,
            homes: a.proposed,
            affordable: a.affordable,
            known: a.proposed.saturating_sub(a.tenure_unknown),
            completed: a.gains_completed,
            completion_date: a.completion_date,
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
            sites: sites.len(),
            without_centroid,
        },
        sites,
    })
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
        let hits = body["response"]["hits"]["hits"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for hit in hits {
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
            "id,lpa,borough,ref,status,decision,appeal,permitted,permission_year,bo_system,proposed,affordable,affordable_pct,gains_completed,units_superseded,tenure_unknown,s106_flag,fast_track,inlieu,completion_date,last_updated,site\n\
             A-1,A,Alpha,1,Completed,Approved,,True,2016,x,40,10,25.0,40,0,4,,,,01/02/2020,2026-01-02T00:00:00Z,Gasworks\n\
             A-2,A,Alpha,2,Refused,Refused,,False,,x,30,0,0.0,0,0,30,,,,,2026-01-02T00:00:00Z,Refused site\n\
             A-3,A,Alpha,3,Superseded,Approved,,True,2015,x,20,5,25.0,0,20,0,,,,,2026-01-02T00:00:00Z,Replaced\n\
             A-4,A,Alpha,4,Approved,Approved,,True,2020,x,12,0,0.0,0,0,12,,,,,2026-01-02T00:00:00Z,No centroid\n",
        )
        .unwrap();
        std::fs::create_dir(root.join("snap")).unwrap();
        let hits = serde_json::json!({"response": {"hits": {"hits": [
            {"_source": {"id": "A-1", "centroid": {"lat": "51.5000012", "lon": -0.1}}},
            {"_source": {"id": "A-3", "centroid": {"lat": 51.6, "lon": -0.2}}},
        ]}}});
        let mut gz = flate2::write::GzEncoder::new(
            File::create(root.join("snap/Alpha.json.gz")).unwrap(),
            flate2::Compression::default(),
        );
        gz.write_all(hits.to_string().as_bytes()).unwrap();
        gz.finish().unwrap();

        let inputs = Inputs {
            applications: "applications.csv".into(),
            snapshot: "snap".into(),
            boundaries: "boroughs.geojson".into(),
        };
        let bundle = bake(root, &inputs).unwrap();
        assert_eq!(
            bundle.sites.len(),
            1,
            "refused, wholly superseded and centroid-less sites are left out"
        );
        assert_eq!(bundle.sources.without_centroid, 1);
        let s = &bundle.sites[0];
        assert_eq!(
            (s.homes, s.affordable, s.known, s.completed),
            (40, 10, 36, 40)
        );
        assert_eq!((s.lat, s.lon, s.year), (51.5, -0.1, 2016));
        assert_eq!(s.updated, "2026-01-02");
    }
}
