//! Terminal report and JSON output for one audit.

use std::fmt::Write;

use serde::Serialize;

use crate::schema::*;
use crate::verify::{CheckResult, ErrorClass, Status};

#[derive(Serialize)]
pub struct AuditJson<'a> {
    pub agreement: &'a Agreement,
    pub verification: &'a [CheckResult],
}

/// 0: all checks pass or skip; 1: Parser Errors or REVIEW flags (unproven values);
/// 2: Document Discrepancies (each still to be confirmed against the original deed).
pub fn exit_code(results: &[CheckResult]) -> i32 {
    if results.iter().any(|r| r.error_class == Some(ErrorClass::DocumentDiscrepancy)) {
        2
    } else if results.iter().any(|r| r.error_class == Some(ErrorClass::ParserError) || r.status == Status::Review) {
        1
    } else {
        0
    }
}

pub fn counts(results: &[CheckResult]) -> [usize; 6] {
    let n = |s: Status| results.iter().filter(|r| r.status == s).count();
    let e = |c: ErrorClass| results.iter().filter(|r| r.error_class == Some(c)).count();
    [n(Status::Pass), n(Status::Review), n(Status::Fail), n(Status::Skip), e(ErrorClass::ParserError), e(ErrorClass::DocumentDiscrepancy)]
}

fn gbp(d: rust_decimal::Decimal) -> String {
    let s = format!("{:.2}", d.round_dp(2));
    let (int, frac) = s.split_once('.').unwrap_or((&s, "00"));
    let mut g = String::new();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            g.push(',');
        }
        g.push(ch);
    }
    format!("£{g}.{frac}")
}

fn pg(e: &Option<Evidence>) -> String {
    e.as_ref().map_or(String::new(), |e| format!("p{}", e.page))
}

fn opt<T: std::fmt::Display>(v: Option<T>) -> String {
    v.map_or("—".into(), |v| v.to_string())
}

pub fn render(a: &Agreement, results: &[CheckResult]) -> String {
    let mut o = String::new();
    let w = 78;
    let m = &a.metadata;
    let _ = writeln!(o, "{}\n SECTION 106 AUDIT\n{}", "=".repeat(w), "=".repeat(w));
    let _ = writeln!(o, " Council        {}", opt(m.council.as_ref()));
    let _ = writeln!(o, " Planning ref   {}    Legal ref {}", if m.planning_references.is_empty() { "—".into() } else { m.planning_references.join(", ") }, opt(m.legal_reference.as_ref()));
    let _ = writeln!(o, " Dated          {}    Site {}", opt(m.agreement_date.as_ref()), opt(m.site_address.as_ref()));
    let _ = writeln!(o, " Parties        {}", m.parties.join("; "));
    let i = &a.instrument;
    let _ = writeln!(o, " Instrument     {:?} {}", i.kind, pg(&i.evidence));
    if let Some(p) = &i.parent {
        let _ = writeln!(o, "   varies       {} {} {}", p.description, opt(p.date.as_ref()), pg(&p.evidence));
    }
    if let Some(s) = &i.scope {
        let _ = writeln!(o, "   scope        {}", s.text);
    }
    let p = &a.provenance;
    let _ = writeln!(o, " Source         {} ({} pp, {}, sha256 {}…)", p.source_path, p.page_count, p.text_source, &p.sha256[..12.min(p.sha256.len())]);
    let _ = writeln!(o, " Extracted by   {}", p.extracted_by);

    let h = &a.housing;
    let _ = writeln!(o, "\n HOUSING");
    let _ = writeln!(o, "   Total residential units   {}", opt(h.total_units));
    let _ = writeln!(o, "   Market units              {}", opt(h.market_units));
    let _ = writeln!(o, "   Affordable units          {}{}", opt(h.affordable_units),
                     h.affordable_habitable_rooms.map_or(String::new(), |r| format!("  ({r} hab rooms)")));
    for t in &h.tenures {
        let _ = writeln!(o, "     - {:<24} {}{}  {}", t.tenure.label(), opt(t.units),
                         t.habitable_rooms.map_or(String::new(), |r| format!("  ({r} hab rooms)")), pg(&t.evidence));
    }
    if let (Some(pct), Some(b)) = (h.claimed_affordable_pct, h.claimed_affordable_basis) {
        let _ = writeln!(o, "   Claimed affordable        {pct}% by {b:?}  {}", pg(&h.claimed_affordable_evidence));
    }

    let _ = writeln!(o, "\n FINANCIAL CONTRIBUTIONS");
    for c in &a.contributions {
        let amt = c.amount.value_gbp.map_or("—".into(), gbp);
        let trig = if c.payment.instalments.is_empty() {
            c.payment.trigger_event.map_or("—".into(), |t| t.label().to_string())
        } else {
            c.payment.instalments.iter()
                .map(|i| format!("{}%:{}", opt(i.share_pct), i.trigger_event.map_or("?", |t| t.label())))
                .collect::<Vec<_>>().join(", ")
        };
        let _ = writeln!(o, "   {:<44}{:>14}  {:?}{}{}  {:?}  {}  {}", c.name.chars().take(43).collect::<String>(), amt,
                         c.amount.structure, if c.amount.index_linked { " idx" } else { "" },
                         "", c.conditionality.kind, trig, pg(&c.amount.evidence));
    }
    let _ = writeln!(o, "   {:<44}{:>14}", "TOTAL (stated sums and caps, unindexed)", gbp(a.total_stated_gbp()));

    if !a.variations.is_empty() {
        let _ = writeln!(o, "\n VARIATIONS");
        for v in &a.variations {
            let _ = writeln!(o, "   {:?} {}{}  {}", v.op, v.target,
                             v.new_text.as_ref().map_or(String::new(), |t| format!(" -> {}", t.chars().take(60).collect::<String>())), pg(&v.evidence));
        }
    }

    let _ = writeln!(o, "\n VERIFICATION GATE");
    for r in results {
        let tag = match r.status { Status::Pass => "PASS", Status::Review => "REVIEW", Status::Fail => "FAIL", Status::Skip => "SKIP" };
        let _ = writeln!(o, "   [{tag}] {:<9} {}", r.check_id, r.description);
        if r.status != Status::Pass || r.actual.is_some() {
            let mut bits = vec![];
            if let Some(e) = &r.expected { bits.push(format!("expected {e}")); }
            if let Some(a) = &r.actual { bits.push(format!("got {a}")); }
            if let Some(c) = r.error_class { bits.push(match c { ErrorClass::ParserError => "PARSER ERROR".into(), ErrorClass::DocumentDiscrepancy => "DOCUMENT DISCREPANCY".to_string() }); }
            if !r.detail.is_empty() { bits.push(r.detail.clone()); }
            if !r.pages.is_empty() { bits.push(format!("p{}", r.pages.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(","))); }
            let _ = writeln!(o, "             {}", bits.join(" | "));
        }
    }
    let [pass, review, fail, skip, perr, disc] = counts(results);
    let _ = writeln!(o, "{}\n {pass} pass · {review} review · {fail} fail · {skip} skip   |   {perr} parser error(s) · {disc} document discrepanc(ies)", "-".repeat(w));
    for msg in &a.warnings {
        let _ = writeln!(o, " ! {msg}");
    }
    let _ = write!(o, "{}", "=".repeat(w));
    o
}
