//! A thin, read-only client for the Planning London Datahub (PLD) and its raw snapshots.
//!
//! PLD is an Elasticsearch API (`planningdata.london.gov.uk/api-guest/`, guest header from the
//! GLA's API document; data under OGL v3). It is not quality-checked on receipt, so analyses never
//! read it live: every response is saved verbatim with its request and retrieval time, and the
//! reconciliation cites the snapshot file and its SHA-256. Field meanings are not interpreted here.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const BASE_URL: &str = "https://planningdata.london.gov.uk/api-guest";
/// The guest access header the GLA publishes for anyone to use, in its "Planning London Datahub
/// API" connection document (v1, linked from the PLD pages on the London Datastore). It is a public
/// guest credential, not a leaked secret: leave it as it is.
const GUEST_HEADER: (&str, &str) = ("X-API-AllowRequest", "be2rmRnt&");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub method: String,
    pub url: String,
    pub body: Option<Value>,
}

/// One PLD response exactly as received.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub request: Request,
    /// RFC 3339, UTC
    pub retrieved_at: String,
    pub response: Value,
}

/// Where a snapshot lives and what it hashed to, for citing it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotRef {
    /// repo-relative path
    pub path: String,
    pub sha256: String,
    pub retrieved_at: String,
}

pub fn snapshots_root(repo_root: &Path) -> PathBuf {
    repo_root.join("snapshots/pld")
}

/// Native TLS with the system's roots, so a corporate or sandbox proxy CA installed there works.
fn agent() -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(60)))
        .tls_config(
            TlsConfig::builder()
                .provider(TlsProvider::NativeTls)
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into()
}

fn send(req: &Request) -> Result<Value> {
    let agent = agent();
    let resp = match &req.body {
        None => agent
            .get(&req.url)
            .header(GUEST_HEADER.0, GUEST_HEADER.1)
            .call(),
        Some(body) => agent
            .post(&req.url)
            .header(GUEST_HEADER.0, GUEST_HEADER.1)
            .send_json(body),
    };
    let mut resp = resp.with_context(|| format!("{} {}", req.method, req.url))?;
    resp.body_mut()
        .read_json()
        .with_context(|| format!("{} returned non-JSON", req.url))
}

/// `GET {index}/_source/{id}`: one document, e.g. `applications`, `Southwark-19_AP_1773`.
pub fn get_source(index: &str, id: &str) -> Result<Snapshot> {
    let request = Request {
        method: "GET".into(),
        url: format!("{BASE_URL}/{index}/_source/{id}"),
        body: None,
    };
    fetch(request)
}

/// `POST {index}/_search` with an Elasticsearch query body.
pub fn search(index: &str, body: Value) -> Result<Snapshot> {
    let request = Request {
        method: "POST".into(),
        url: format!("{BASE_URL}/{index}/_search"),
        body: Some(body),
    };
    fetch(request)
}

fn fetch(request: Request) -> Result<Snapshot> {
    let response = send(&request)?;
    Ok(Snapshot {
        request,
        retrieved_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        response,
    })
}

/// File-system-safe name for a snapshot's subject (an id or a query label).
pub fn safe(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Write `snapshots/pld/<index>/<subject>/<retrieved_at>.json`. Never overwrites.
pub fn save(repo_root: &Path, index: &str, subject: &str, snap: &Snapshot) -> Result<SnapshotRef> {
    let dir = snapshots_root(repo_root)
        .join(safe(index))
        .join(safe(subject));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.json", safe(&snap.retrieved_at)));
    if path.exists() {
        bail!(
            "{} already exists; snapshots are never overwritten",
            path.display()
        );
    }
    let bytes = serde_json::to_vec_pretty(snap)?;
    std::fs::write(&path, &bytes)?;
    Ok(SnapshotRef {
        path: path
            .strip_prefix(repo_root)
            .unwrap_or(&path)
            .display()
            .to_string(),
        sha256: hex(&bytes),
        retrieved_at: snap.retrieved_at.clone(),
    })
}

/// Read a saved snapshot back, with the reference that cites it.
pub fn load(repo_root: &Path, path: &Path) -> Result<(Snapshot, SnapshotRef)> {
    let full = if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(path)
    };
    let bytes = std::fs::read(&full).with_context(|| format!("reading {}", full.display()))?;
    let snap: Snapshot = serde_json::from_slice(&bytes)
        .with_context(|| format!("{} is not a PLD snapshot", full.display()))?;
    let r = SnapshotRef {
        path: full
            .strip_prefix(repo_root)
            .unwrap_or(&full)
            .display()
            .to_string(),
        sha256: hex(&bytes),
        retrieved_at: snap.retrieved_at.clone(),
    };
    Ok((snap, r))
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
