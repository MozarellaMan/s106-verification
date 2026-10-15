//! The scenario grid of the design, one entry per variant.

use crate::generator::Params;

pub struct Scenario {
    /// e.g. `S3-K2`
    pub name: String,
    pub what: &'static str,
    pub params: Params,
    pub seed: u64,
}

pub fn grid() -> Vec<Scenario> {
    let mut out: Vec<Scenario> = vec![];
    let mut add = |name: String, what: &'static str, params: Params| {
        let seed = 0x5106_0000 + out.len() as u64;
        out.push(Scenario {
            name,
            what,
            params,
            seed,
        });
    };
    let base = Params::default;

    add(
        "S1".into(),
        "ideal: unique wrong values, no correlation",
        base(),
    );

    for s in [5, 8] {
        add(
            format!("S2-n{s}"),
            "small number of schemes (A4)",
            Params {
                schemes: s,
                ..base()
            },
        );
    }

    for k in [2, 3, 5] {
        add(
            format!("S3-K{k}"),
            "chance agreement on a small answer space (A1)",
            Params {
                acc: [0.9, 0.8, 0.8],
                k: Some(k),
                ..base()
            },
        );
    }

    for rho in [0.05, 0.1, 0.2] {
        add(
            format!("S4-rho{}", (rho * 100.0) as u32),
            "shared OCR corruption followed by Llm and Person (A2)",
            Params {
                rho_shared: rho,
                ..base()
            },
        );
    }

    add(
        "S5a".into(),
        "item difficulty: 30% hard cells, accuracy drops by 0.4, unique wrong values (A2, A3)",
        Params {
            acc: [0.95; 3],
            hard_share: 0.3,
            delta: 0.4,
            ..base()
        },
    );

    for m in [0.2, 0.4] {
        add(
            format!("S6-m{}", (m * 100.0) as u32),
            "cells whose truth is not_stated, hallucinated value from a 2-value space (other wrong values unique)",
            Params {
                missing: m,
                miss: 0.5,
                k_missing: Some(2),
                ..base()
            },
        );
    }

    for st in [0.1, 0.3] {
        add(
            format!("S7-st{}", (st * 100.0) as u32),
            "external reader reports another lineage stage on whole schemes",
            Params {
                stage: st,
                ..base()
            },
        );
    }

    for (label, k) in [("Kinf", None), ("K3", Some(3))] {
        add(
            format!("S8-{label}"),
            "one good reader with two poor partners",
            Params {
                acc: [0.95, 0.6, 0.6],
                k,
                ..base()
            },
        );
    }

    for s in [20, 50] {
        for a1 in [0.88, 0.92] {
            add(
                format!("S9-n{s}-a{}", (a1 * 100.0) as u32),
                "reader 1 near the gate",
                Params {
                    schemes: s,
                    acc: [a1, 0.9, 0.9],
                    ..base()
                },
            );
        }
    }

    add(
        "S10".into(),
        "chance agreement and shared corruption together",
        Params {
            acc: [0.85; 3],
            k: Some(3),
            rho_shared: 0.1,
            ..base()
        },
    );

    for rho in [0.05, 0.1] {
        add(
            format!("S11-rho{}", (rho * 100.0) as u32),
            "shared corruption followed by every reader, External included (no independent source)",
            Params {
                rho_shared: rho,
                shared_all: true,
                ..base()
            },
        );
    }

    for q in [0.3, 0.6] {
        add(
            format!("S12-q{}", (q * 100.0) as u32),
            "Person copies the Llm reading with prob q, else reads independently",
            Params {
                anchor: q,
                ..base()
            },
        );
    }
    // S5b was added after the others; it is appended so that every earlier scenario keeps its
    // seed, then moved next to S5a for the tables.
    add(
        "S5b".into(),
        "item difficulty as S5a, wrong values from a 3-value answer space as S3-K3 (A1, A2, A3)",
        Params {
            acc: [0.95; 3],
            hard_share: 0.3,
            delta: 0.4,
            k: Some(3),
            ..base()
        },
    );
    let s5b = out.pop().unwrap();
    let at = out.iter().position(|s| s.name == "S5a").unwrap() + 1;
    out.insert(at, s5b);
    out
}
