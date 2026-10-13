//! Extraction without the network: request shape, strict schema, wire -> v2 conversion,
//! the v1 fixture adapter, and fixture integrity checks.

use std::path::PathBuf;

use rust_decimal::Decimal;
use s106::document::{Document, Line, PageText, TextSource};
use s106::extract::{self, Info};
use s106::llm::{self, ExtractEffort};
use s106::schema::*;
use s106::wire::{self, Cite, PageMap, WireContribution, WireExtraction, WireInstalment};
use serde_json::{Value, json};

fn cite(page: u32, s: &str) -> Cite {
    Cite {
        page,
        snippet: s.into(),
    }
}

fn doc(pages: usize) -> Document {
    Document {
        path: PathBuf::from("example.pdf"),
        sha256: "0baed481b09feeb4".into(),
        ocr_version: "test".into(),
        pages: (1..=pages)
            .map(|n| PageText {
                page: n,
                source: TextSource::Ocr,
                width: 1.0,
                height: 1.0,
                lines: vec![Line {
                    text: String::new(),
                    bbox: [0.0; 4],
                }],
            })
            .collect(),
    }
}

fn info(source: &str) -> Info {
    Info {
        model: "claude-haiku-5-5".into(),
        served_by: "claude-haiku-5-5".into(),
        source: source.into(),
        usage: None,
        fixture: None,
    }
}

// ------------------------------------------------------------------------------- schema and request

#[test]
fn schema_is_strict() {
    for schema in [
        llm::schema_for::<s106::wire::WireInstrumentPart>(),
        llm::schema_for::<s106::wire::WireHousingPart>(),
        llm::schema_for::<s106::wire::WireContributionsPart>(),
    ] {
        check(&schema, "$");
    }
    fn check(v: &Value, path: &str) {
        match v {
            Value::Object(m) => {
                assert!(!m.contains_key("format"), "{path}: format keyword left in");
                assert!(
                    !m.contains_key("oneOf"),
                    "{path}: oneOf is rejected by the API"
                );
                if let Some(props) = m.get("properties").and_then(Value::as_object) {
                    assert_eq!(
                        m.get("additionalProperties"),
                        Some(&json!(false)),
                        "{path}: object not closed"
                    );
                    let req: Vec<&str> = m["required"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|r| r.as_str().unwrap())
                        .collect();
                    for k in props.keys() {
                        assert!(req.contains(&k.as_str()), "{path}.{k} not required");
                    }
                }
                for (k, c) in m {
                    check(c, &format!("{path}.{k}"));
                }
            }
            Value::Array(a) => a.iter().for_each(|c| check(c, path)),
            _ => {}
        }
    }
}

#[test]
fn request_shape() {
    let schema = llm::schema_for::<WireExtraction>();
    let req = llm::request(
        "claude-haiku-5-5",
        b"%PDF-1.4 test",
        "prompt",
        &schema,
        ExtractEffort::Medium,
    );
    let body = serde_json::to_value(&req).unwrap();
    assert_eq!(body["model"], "claude-haiku-5-5");
    let content = &body["messages"][0]["content"];
    assert_eq!(content[0]["type"], "document");
    assert_eq!(content[0]["source"]["media_type"], "application/pdf");
    assert_eq!(content[1]["text"], "prompt");
    assert_eq!(body["output_config"]["format"]["type"], "json_schema");
    assert_eq!(body["output_config"]["effort"], "medium");
    assert!(
        body.get("fallbacks").is_none(),
        "Haiku 5.5 takes no server-side fallbacks"
    );
    let opus = serde_json::to_value(llm::request(
        "claude-opus-5-5",
        b"x",
        "p",
        &schema,
        ExtractEffort::Medium,
    ))
    .unwrap();
    assert_eq!(opus["fallbacks"], "default");
}

// ------------------------------------------------------------------------------- conversion

fn contribution(phrase: &str, covenant: &str) -> WireContribution {
    WireContribution {
        name: "Bus Contribution".into(),
        amount_structure: AmountStructure::Fixed,
        amount_gbp: "683100.00".into(),
        amount_in_words: "six hundred and eighty three thousand and one hundred pounds".into(),
        index_linked: true,
        amount_evidence: cite(7, "the sum of £683,100.00"),
        conditionality: ConditionalityKind::Unconditional,
        condition_evidence: cite(0, ""),
        covenant: covenant.into(),
        covenant_evidence: cite(70, covenant),
        trigger_phrase: cite(70, phrase),
        instalments: vec![],
    }
}

#[test]
fn trigger_label_comes_only_from_the_cited_phrase() {
    let pm = PageMap::identity(109);
    let c = wire::contribution(
        &contribution(
            "within 28 days",
            "the Owner will pay the Contribution within 28 days",
        ),
        &pm,
    );
    assert_eq!(c.payment.trigger_event, Some(TriggerEvent::OnDemand));
    // a phrase outside the vocabulary leaves the label unset (the verifier then SKIPs the label check)
    let c = wire::contribution(
        &contribution(
            "as the Council may direct",
            "to pay as the Council may direct",
        ),
        &pm,
    );
    assert_eq!(c.payment.trigger_event, None);
}

#[test]
fn instalments_are_classified_and_replace_the_single_trigger() {
    let pm = PageMap::identity(45);
    let mut w = contribution(
        "",
        "To pay to the Council the Allotments Contribution in the following instalments",
    );
    w.trigger_phrase = cite(0, "");
    w.instalments = vec![
        WireInstalment {
            share_pct: "50".into(),
            trigger_phrase: cite(14, "50% prior to Occupation of the 1st (first) Dwelling"),
        },
        WireInstalment {
            share_pct: "50".into(),
            trigger_phrase: cite(14, "50% prior to Occupation of the 50th Dwelling"),
        },
    ];
    let c = wire::contribution(&w, &pm);
    assert_eq!(c.payment.instalments.len(), 2);
    assert!(
        c.payment
            .instalments
            .iter()
            .all(|i| i.trigger_event == Some(TriggerEvent::PreOccupation))
    );
    let share = c.payment.instalments[0].share_pct.as_ref().unwrap();
    assert_eq!(share.raw, "50");
    assert_eq!(share.ok(), Some(&Decimal::from(50)));
    assert_eq!(c.payment.trigger_event, None);
    assert!(c.payment.trigger_phrase.is_none());
}

#[test]
fn a_figure_overrides_structure_none_and_empty_snippets_mean_not_stated() {
    let pm = PageMap(vec![3, 4, 5]);
    let mut w = contribution(
        "on or before Implementation",
        "To pay on or before Implementation",
    );
    w.amount_structure = AmountStructure::None;
    w.amount_evidence = cite(2, "the sum of £683,100.00");
    let c = wire::contribution(&w, &pm);
    assert_eq!(c.amount.structure, AmountStructure::Fixed);
    assert_eq!(
        c.amount.evidence.unwrap().page,
        4,
        "attachment position 2 is PDF page 4"
    );
    assert!(
        c.conditionality.evidence.is_none(),
        "empty snippet = not stated"
    );
    assert_eq!(
        pm.ev(&cite(9, "beyond the attachment")).unwrap().page,
        0,
        "out-of-range pages can never re-locate"
    );
}

#[test]
fn group_terms_are_dropped_with_a_warning() {
    let output = json!({
        "instrument": {"kind": "agreement", "evidence": {"page": 1, "snippet": "Agreement pursuant to Section 106"},
                       "parent_description": "", "parent_date": "", "parent_evidence": {"page": 0, "snippet": ""},
                       "scope": "", "scope_evidence": {"page": 0, "snippet": ""}},
        "metadata": {"council": "Example District Council", "planning_references": ["EX/23/0456"], "legal_reference": "",
                     "agreement_date": "4 March 2024", "site_address": "Former Brickworks, Mill Road", "parties": []},
        "housing": {"total_units": null, "total_units_evidence": {"page": 0, "snippet": ""}, "market_units": null,
                    "market_units_evidence": {"page": 0, "snippet": ""}, "affordable_units": null,
                    "affordable_units_evidence": {"page": 0, "snippet": ""}, "affordable_habitable_rooms": null,
                    "total_habitable_rooms": null, "tenures": [],
                    "mix_table": {"rows": [], "subtotals": [], "total_units": null, "total_habitable_rooms": null,
                                  "evidence": {"page": 0, "snippet": ""}},
                    "claimed_affordable_pct": "", "claimed_affordable_basis": null,
                    "claimed_affordable_evidence": {"page": 0, "snippet": ""}},
        "contributions": [serde_json::to_value(contribution("on or before Implementation", "To pay on or before Implementation")).unwrap(),
                          {"name": "Highway Authority Contributions", "amount_structure": "none", "amount_gbp": "", "amount_in_words": "",
                           "index_linked": false, "amount_evidence": {"page": 0, "snippet": ""}, "conditionality": "unknown",
                           "condition_evidence": {"page": 0, "snippet": ""}, "covenant": "",
                           "covenant_evidence": {"page": 0, "snippet": ""},
                           "trigger_phrase": {"page": 0, "snippet": ""}, "instalments": []}],
        "variations": []
    });
    let a = extract::to_agreement(&doc(109), &output, &PageMap::identity(109), &info("replay"))
        .unwrap();
    assert_eq!(a.contributions.len(), 1);
    assert!(
        a.warnings
            .iter()
            .any(|w| w.contains("Highway Authority Contributions"))
    );
    assert_eq!(
        a.metadata.planning_references,
        vec!["EX/23/0456".to_string()]
    );
    assert_eq!(a.schema_version, "2");
}

// ------------------------------------------------------------------------------- v1 fixtures

#[test]
fn v1_fixture_replays_through_the_adapter() {
    let raw: Value = serde_json::from_str(include_str!("synthetic/v1-extraction.json")).unwrap();
    let pages: Vec<usize> = serde_json::from_value(raw["pages"].clone()).unwrap();
    let a = extract::to_agreement(&doc(5), &raw["output"], &PageMap(pages), &info("replay-v1"))
        .unwrap();
    assert_eq!(a.housing.total_units, Some(120));
    assert_eq!(a.housing.affordable_units, Some(36));
    assert_eq!(a.instrument.kind, InstrumentKind::Unknown);
    // v1 "contingent" meant "no fixed figure": structure none, conditionality unknown (not "conditional")
    let viability = a
        .contributions
        .iter()
        .find(|c| c.name == "Viability Contribution")
        .unwrap();
    assert_eq!(viability.amount.structure, AmountStructure::None);
    assert_eq!(viability.conditionality.kind, ConditionalityKind::Unknown);
    let energy = a
        .contributions
        .iter()
        .find(|c| c.name == "Energy Contribution")
        .unwrap();
    assert_eq!(energy.amount.structure, AmountStructure::Cap);
    assert_eq!(
        energy.payment.trigger_event,
        Some(TriggerEvent::OnDemand),
        "label from the cited 'within 20 Working Days' phrase"
    );
    assert!(a.warnings.iter().any(|w| w.starts_with("v1 fixture")));
}

#[test]
fn replay_refuses_a_fixture_from_a_different_pdf() {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: this test binary sets the variable before any other test reads it
    unsafe { std::env::set_var("S106_DATA", dir.path()) };
    let d = doc(3);
    let fx = extract::Fixture {
        format: extract::FIXTURE_FORMAT.into(),
        model: "claude-haiku-5-5".into(),
        served_by: "claude-haiku-5-5".into(),
        prompt_sha256: extract::prompt_sha256(),
        pdf_sha256: "a different deed".into(),
        pages: vec![1, 2, 3],
        usage: llm::Usage {
            input_tokens: 1,
            output_tokens: 1,
            attempts: 1,
            cache_write_tokens: 0,
            cache_read_tokens: 0,
        },
        recorded_at: "2026-10-13T00:00:00Z".into(),
        output: json!({}),
    };
    let path = extract::fixture_path(&d, "claude-haiku-5-5");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, serde_json::to_string(&fx).unwrap()).unwrap();
    let err = extract::extract(&d, "claude-haiku-5-5", extract::Mode::Replay, None).unwrap_err();
    assert!(err.to_string().contains("different PDF"), "{err}");

    let fx = extract::Fixture {
        pdf_sha256: d.sha256.clone(),
        prompt_sha256: "an older prompt".into(),
        ..fx
    };
    std::fs::write(&path, serde_json::to_string(&fx).unwrap()).unwrap();
    let err = extract::extract(&d, "claude-haiku-5-5", extract::Mode::Replay, None).unwrap_err();
    assert!(err.to_string().contains("different prompt"), "{err}");
}

#[test]
fn wire_outputs_deserialize_and_errors_identify_the_field() {
    for id in ["agreement", "discrepancies", "variation"] {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/synthetic")
            .join(id)
            .join("extraction.json");
        let output: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let d = doc(5);
        let pm = PageMap::identity(5);
        let original = extract::to_agreement(&d, &output, &pm, &info("replay")).unwrap();
        assert_eq!(original.schema_version, "2", "{id}");
        let mut corrupt = output;
        corrupt["housing"]["total_units"] = json!("not a count");
        let error = extract::to_agreement(&d, &corrupt, &pm, &info("replay")).unwrap_err();
        assert!(
            format!("{error:#}").contains("housing.total_units"),
            "{error:#}"
        );
    }
}

#[test]
fn usage_notes_distinguish_live_from_recorded_costs() {
    let mut live = info("live");
    assert_eq!(live.usage_note(), None);
    live.usage = Some(llm::Usage {
        input_tokens: 1000,
        output_tokens: 10,
        attempts: 1,
        cache_write_tokens: 0,
        cache_read_tokens: 0,
    });
    assert_eq!(
        live.usage_note().unwrap(),
        "live: 1000 in / 10 out tokens, ~$0.000"
    );
    let replay = Info {
        source: "replay".into(),
        ..live
    };
    assert_eq!(
        replay.usage_note().unwrap(),
        "replay: 1000 in / 10 out tokens, ~$0.000 when recorded"
    );
}
