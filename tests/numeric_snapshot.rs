//! The enforced "possible weakening" gate: what the parser does on the probe corpus is
//! snapshotted, so any change in what it accepts shows up as a reviewable diff.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use proptest::prelude::*;
use s106::audit::{escape_probe, probe_rendering};
use s106::quantity::{self, scan};

const PROBES_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/numeric-probes.txt");
const KEYS: [&str; 5] = [
    "snippet",
    "amount_in_words",
    "amount_gbp",
    "share_pct",
    "claimed_affordable_pct",
];

fn collect(v: &serde_json::Value, key: Option<&str>, out: &mut BTreeSet<String>) {
    match v {
        serde_json::Value::Object(m) => m.iter().for_each(|(k, v)| collect(v, Some(k), out)),
        serde_json::Value::Array(a) => a.iter().for_each(|v| collect(v, key, out)),
        serde_json::Value::String(s) if !s.is_empty() && key.is_some_and(|k| KEYS.contains(&k)) => {
            out.insert(escape_probe(s));
        }
        _ => {}
    }
}

fn fixture_probes() -> BTreeSet<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut out = BTreeSet::new();
    for dir in ["llm", "llm-v1"] {
        let mut stack: Vec<PathBuf> = vec![root.join(dir)];
        while let Some(p) = stack.pop() {
            for entry in std::fs::read_dir(&p).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "json") {
                    let v: serde_json::Value =
                        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
                    collect(&v["output"], None, &mut out);
                }
            }
        }
    }
    out
}

#[test]
fn probes_cover_fixtures() {
    let wanted = fixture_probes();
    let file = std::fs::read_to_string(PROBES_PATH).unwrap();
    if std::env::var_os("S106_REGENERATE_PROBES").is_some() {
        // Keep the constructed controls already in the file; add whatever the fixtures need.
        let mut all: BTreeSet<String> = file
            .lines()
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();
        all.extend(wanted);
        let text: String = all.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(PROBES_PATH, text).unwrap();
        return;
    }
    let have: BTreeSet<&str> = file.lines().collect();
    let missing: Vec<&String> = wanted
        .iter()
        .filter(|w| !have.contains(w.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "fixtures/numeric-probes.txt is missing {} fixture string(s), e.g. {:?}; \
         add them with `S106_REGENERATE_PROBES=1 cargo test --test numeric_snapshot probes_cover_fixtures`, \
         then rerun `cargo test --test numeric_snapshot` and review the snapshot diff",
        missing.len(),
        missing.first()
    );
}

#[test]
fn parser_behaviour_on_probes() {
    insta::assert_snapshot!(probe_rendering());
}

#[test]
fn fingerprint_is_the_hash_of_the_rendering() {
    assert_eq!(
        s106::audit::parser_fingerprint(),
        s106::audit::sha256_hex(&probe_rendering())
    );
}

fn interesting() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<String>(),
        "[0-9£%,. \\-a-zA-Zé—–µ]{0,40}",
        "(£?[0-9]{1,3}(,[0-9]{3})*(\\.[0-9]{1,3})? ?(per cent|%|pounds)? ?){1,4}",
    ]
}

proptest! {
    #[test]
    fn scan_never_panics_and_spans_are_valid(text in interesting()) {
        let s = scan(&text);
        let ok = |r: &std::ops::Range<usize>| {
            r.start <= r.end && r.end <= text.len()
                && text.is_char_boundary(r.start) && text.is_char_boundary(r.end)
        };
        for q in &s.quantities {
            prop_assert!(ok(&q.span), "quantity span {:?} in {:?}", q.span, text);
        }
        for e in &s.errors {
            prop_assert!(ok(&e.span), "error span {:?} in {:?}", e.span, text);
            prop_assert!(ok(&e.candidate), "candidate {:?} in {:?}", e.candidate, text);
        }
        let mut spans: Vec<_> = s.quantities.iter().map(|q| q.span.clone()).collect();
        spans.sort_by_key(|r| r.start);
        for w in spans.windows(2) {
            prop_assert!(w[0].end <= w[1].start, "overlap {:?} in {:?}", w, text);
        }
    }

    #[test]
    fn whole_input_readers_never_panic(text in interesting()) {
        let _ = quantity::decimal(&text);
        let _ = quantity::integer(&text);
        let _ = quantity::amount(&text);
    }
}
