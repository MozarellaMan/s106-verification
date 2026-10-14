//! The explorer's pure logic: colours, bins, sizes and number formats. No browser types, so the
//! tests run on the host with `cargo test`.

use crate::site::{Check, Finding, Lineage, Reading, Retrieval, Site, Status};

impl Site {
    /// Affordable share of homes with a recorded tenure; None when no tenure was recorded.
    pub fn affordable_share(&self) -> Option<f64> {
        (self.known > 0).then(|| self.affordable as f64 / self.known as f64)
    }

    pub fn name(&self) -> &str {
        self.site.as_deref().unwrap_or("(no site name)")
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Colour {
    Affordable,
    Status,
}

/// Affordable share: one hue, light to dark, in even 10-point bins. No bin edge sits on a policy
/// threshold, so no colour reads as a pass or a fail.
pub const SHARE: [(&str, &str); 5] = [
    ("#d3e4ec", "under 10%"),
    ("#a3c6d6", "10 to 20%"),
    ("#6fa3bd", "20 to 30%"),
    ("#3f7f9f", "30 to 40%"),
    ("#1d5674", "40% or more"),
];
pub const UNKNOWN: &str = "#9a9aa3";

/// Which `SHARE` bin a share (0 to 1) falls in.
pub fn share_bin(share: f64) -> usize {
    ((share * 10.0) as usize).min(SHARE.len() - 1)
}

/// Datahub status: neutral hues, none red or green.
pub fn status_colour(s: Status) -> &'static str {
    match s {
        Status::Completed => "#4e79a7",
        Status::Commenced => "#e0b445",
        Status::Approved => "#b07aa1",
        Status::Lapsed => "#9c755f",
        Status::Superseded => "#7fb7c9",
        Status::Other => UNKNOWN,
    }
}

const STATUSES: [Status; 6] = [
    Status::Completed,
    Status::Commenced,
    Status::Approved,
    Status::Lapsed,
    Status::Superseded,
    Status::Other,
];

pub fn colour(site: &Site, by: Colour) -> &'static str {
    match by {
        Colour::Affordable => match site.affordable_share() {
            None => UNKNOWN,
            Some(a) => SHARE[share_bin(a)].0,
        },
        Colour::Status => status_colour(site.status),
    }
}

pub fn legend(by: Colour) -> Vec<(&'static str, &'static str)> {
    match by {
        Colour::Affordable => SHARE
            .into_iter()
            .chain([(UNKNOWN, "tenure not recorded")])
            .collect(),
        Colour::Status => STATUSES
            .into_iter()
            .map(|s| (status_colour(s), s.label()))
            .collect(),
    }
}

/// Dot radius in pixels: area scales with homes; every site under about 30 homes shares the
/// smallest size so it stays visible.
pub fn radius(homes: u32) -> f64 {
    (homes as f64).sqrt().max(5.5) / 2.2
}

/// 12345 -> "12,345"
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn percent(part: u64, whole: u64) -> String {
    if whole == 0 {
        "n/a".into()
    } else {
        format!("{:.1}%", 100.0 * part as f64 / whole as f64)
    }
}

/// A check level in words.
pub fn check_label(check: Check) -> &'static str {
    match check {
        Check::Confirmed => {
            "readers agree, one of them not an AI model; estimated chance of error 1% or less"
        }
        Check::Estimated => {
            "readers agree, and readings like it are estimated at least 90% accurate; fit for \
             totals, not for this scheme alone"
        }
        Check::Proposed => "not verified: one reading, readers who differ, or no error rate yet",
        Check::Unread => "not read",
    }
}

/// The style class of a cell at this check level, so the whole cell shows how far it is verified.
pub fn check_class(check: Check) -> &'static str {
    match check {
        Check::Confirmed => "independent",
        Check::Estimated => "checked",
        Check::Proposed | Check::Unread => "unchecked",
    }
}

/// A short tag shown before a value: how far it is verified.
pub fn check_tag(check: Check) -> &'static str {
    match check {
        Check::Unread => "not read",
        Check::Proposed => "unverified reading",
        Check::Estimated => "estimated",
        Check::Confirmed => "confirmed",
    }
}

/// Counts of a scheme's cells by how far they are verified, leaving out empty levels, e.g.
/// "41 cells: 3 confirmed, 30 unverified, 8 not read".
pub fn check_counts(readings: &[Reading]) -> String {
    let count = |c: Check| readings.iter().filter(|r| r.check == c).count();
    let parts: Vec<String> = [
        (Check::Confirmed, "confirmed"),
        (Check::Estimated, "estimated"),
        (Check::Proposed, "unverified"),
        (Check::Unread, "not read"),
    ]
    .into_iter()
    .filter(|&(c, _)| count(c) > 0)
    .map(|(c, words)| format!("{} {words}", count(c)))
    .collect();
    format!("{} cells: {}", readings.len(), parts.join(", "))
}

/// A ledger source in words.
pub fn source_label(source: &str) -> String {
    match source {
        "committee_report" => "Committee report".into(),
        "deed" => "Deed".into(),
        "datahub" => "Datahub".into(),
        "epc" => "EPC".into(),
        "gla" => "GLA".into(),
        "applicant_offer" => "Applicant's offer".into(),
        "decision_notice" => "Decision notice".into(),
        "viability" => "Viability".into(),
        "s106_monitoring" => "S106 monitoring".into(),
        "variation" => "Variation".into(),
        other => other.replace('_', " "),
    }
}

/// A ledger value in words: codes spelled out, dates written out and codebook words unsnaked,
/// e.g. "total=84; SR=16; SO=7" -> "total homes 84; social rent 16; shared ownership 7". An
/// ambiguous value's alternatives ("… OR …") read "either …, or …".
pub fn humanize(value: &str) -> String {
    let alternatives: Vec<String> = value.split(" OR ").map(humanize_one).collect();
    match alternatives.as_slice() {
        [one] => one.clone(),
        many => format!("either {}", many.join(", or ")),
    }
}

fn humanize_one(value: &str) -> String {
    value
        .split("; ")
        .map(|part| {
            let pair = part
                .split_once('=')
                .filter(|(k, _)| k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
            if let Some((key, v)) = pair {
                let v = code_word(v);
                return match key {
                    "total" => format!("total homes {v}"),
                    "market" => format!("market homes {v}"),
                    "share" => format!("affordable share {v}"),
                    "split" => format!("tenure split {v}"),
                    "route" => format!("viability route: {v}"),
                    "reviews" => format!("viability reviews: {v}"),
                    "cascade" => format!("tenure cascade: {v}"),
                    "pil" => format!("payment in lieu: {v}"),
                    "pil_amount" => format!("payment in lieu amount: {v}"),
                    "target" => format!("policy target {v}"),
                    "first_offer" => format!("first offer: {v}"),
                    "offer_doc" => format!("offer document: {v}"),
                    other => format!("{} {v}", tenure(other).unwrap_or(other)),
                };
            }
            if let Some((code, rest)) = part.split_once(": ")
                && let Some(t) = tenure(code)
            {
                return format!("{t}: {rest}");
            }
            if let Some(d) = part.strip_prefix("orig ") {
                return format!("original agreement dated {}", long_date(d));
            }
            if let Some(d) = part.strip_prefix("DoV ") {
                return format!("deed of variation dated {}", long_date(d));
            }
            if let Some(r) = part.strip_prefix("s73 ") {
                return format!("section 73 permission {r}");
            }
            part.split(' ')
                .map(|w| tenure(w).unwrap_or(w))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("; ")
        .replace('_', " ")
        .replace("<=", "≤")
}

/// A tenure code in words.
fn tenure(code: &str) -> Option<&'static str> {
    Some(match code {
        "SR" => "social rent",
        "SO" => "shared ownership",
        "INT" => "intermediate",
        "AR" => "affordable rent",
        "LAR" => "London Affordable Rent",
        "LLR" => "London Living Rent",
        "DMS" => "discount market sale",
        _ => return None,
    })
}

/// A codebook value in words: its first word if that is a code, and tenure pairs such as
/// "SR:SO" as "social rent to shared ownership".
fn code_word(v: &str) -> String {
    let (head, tail) = v.split_once(' ').map_or((v, ""), |(h, t)| (h, t));
    let head = match head {
        "viability_tested" => "viability tested".to_string(),
        "fast_track" => "Fast Track".to_string(),
        "not_applicable" => "not applicable".to_string(),
        "not_stated" => "not stated".to_string(),
        "early" => "early stage review".to_string(),
        "late" => "late stage review".to_string(),
        h => h.to_string(),
    };
    let tail: Vec<String> = tail
        .split(' ')
        .map(|w| {
            let codes: Option<Vec<&str>> = w.split(':').map(tenure).collect();
            match codes {
                Some(names) if names.len() > 1 => names.join(" to "),
                _ => w.to_string(),
            }
        })
        .collect();
    [head, tail.join(" ")]
        .into_iter()
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// "2014-03-28" -> "28 March 2014"; anything else (a partly legible date) as it is.
fn long_date(d: &str) -> String {
    const MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let mut it = d.splitn(3, '-');
    let (Some(y), Some(m), Some(rest)) = (it.next(), it.next(), it.next()) else {
        return d.to_string();
    };
    let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    match (
        y.parse::<u32>(),
        m.parse::<usize>(),
        rest[..digits].parse::<u32>(),
    ) {
        (Ok(y), Ok(m @ 1..=12), Ok(day)) => {
            format!("{day} {} {y}{}", MONTHS[m - 1], &rest[digits..])
        }
        _ => d.to_string(),
    }
}

/// A basis worth printing next to a value: not "not stated", and not already in the value.
pub fn basis_note(value: &str, basis: Option<&str>) -> Option<String> {
    let b = basis?.replace('_', " ").replace("other:", "other: ");
    (b != "not stated" && !value.contains(&b)).then(|| format!(" ({b})"))
}

/// V3 holds the policy target and the applicant's first offer in one cell; the target belongs
/// to the Target stage and the rest to Offer. Returns (target part, the rest), each if present.
/// An ambiguous cell's alternatives ("… OR …") are split one by one and kept in order.
pub fn split_target(value: &str) -> (Option<String>, Option<String>) {
    let (mut targets, mut rests) = (Vec::new(), Vec::new());
    for alternative in value.split(" OR ") {
        let (target, rest): (Vec<&str>, Vec<&str>) = alternative
            .split("; ")
            .partition(|p| p.starts_with("target="));
        if !target.is_empty() {
            targets.push(target.join("; "));
        }
        if !rest.is_empty() {
            rests.push(rest.join("; "));
        }
    }
    let join = |v: Vec<String>| (!v.is_empty()).then(|| v.join(" OR "));
    (join(targets), join(rests))
}

/// The lineage stage whose documents a retrieval source is; None for the Datahub record.
pub fn retrieval_stage(source: &str) -> Option<Lineage> {
    match source {
        "applicant_offer" | "viability" => Some(Lineage::Offer),
        "committee_report" | "decision_notice" | "gla" => Some(Lineage::Decision),
        "deed" => Some(Lineage::Obligation),
        "variation" | "s73" => Some(Lineage::Variation),
        "epc" | "s106_monitoring" => Some(Lineage::Completion),
        _ => None,
    }
}

/// Documents not in hand, in words: "Viability: 13 listed, not fetched".
pub fn not_in_hand(r: &Retrieval) -> Option<String> {
    let raw = r.not_in_hand.as_deref()?;
    let words = raw
        .split(", ")
        .map(|p| {
            let (kind, n) = p.rsplit_once(" x").unwrap_or((p, "1"));
            let kind = match kind {
                "listed_not_fetched" => "listed, not fetched",
                "not_found_planapps" => "not found on the planning register",
                "not_retrieved" => "not retrieved",
                other => other,
            };
            format!("{n} {kind}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!("{}: {words}", source_label(&r.source)))
}

/// Whether a variation document's reading changes a figure the deed set: a variation cell for
/// homes (V1) or affordable share (V2) with a `key=value` part (total, SR, share, …) that the
/// deed's cell also has, with a different value. Keys only one side states, and free text, are
/// not read as a change. Some(true) when every
/// cell deciding that is `confirmed`, Some(false) when it rests on a reading that isn't, None when no figure changed (a variation that only lists dates, or repeats the deed).
pub fn variation_changes(readings: &[Reading]) -> Option<bool> {
    let parts = |r: &Reading| -> Vec<(String, String)> {
        r.value
            .as_deref()
            .unwrap_or_default()
            .split("; ")
            .filter_map(|p| p.split_once('='))
            .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
            .collect()
    };
    let mut changed = None;
    for v in readings
        .iter()
        .filter(|r| r.source == "variation" && has_value(r))
        .filter(|r| r.variable == "V1" || r.variable == "V2")
    {
        let Some(deed) = readings
            .iter()
            .find(|d| d.source == "deed" && d.variable == v.variable && has_value(d))
        else {
            continue;
        };
        let was = parts(deed);
        let differs = parts(v)
            .iter()
            .any(|(k, val)| was.iter().any(|(k2, val2)| k == k2 && val != val2));
        if !differs {
            continue;
        }
        let checked = v.check >= Check::Confirmed && deed.check >= Check::Confirmed;
        changed = Some(changed.unwrap_or(true) && checked);
    }
    changed
}

/// A reading with something to show (a value, or a doubt about one), as against a status alone.
pub fn has_value(r: &Reading) -> bool {
    match r.status {
        Finding::Found | Finding::Ambiguous | Finding::Inconsistent => true,
        Finding::NotStated
        | Finding::SourceMissing
        | Finding::NoSource
        | Finding::Unreadable
        | Finding::NotApplicable
        | Finding::Unread => false,
    }
}

/// A doubt worth printing next to a value; None for a plain finding.
pub fn doubt(f: Finding) -> Option<&'static str> {
    match f {
        Finding::Ambiguous => Some("ambiguous"),
        Finding::Inconsistent => Some("inconsistent"),
        _ => None,
    }
}

/// The other readings as one line per status, e.g. "Not stated: deed V5, gla V1".
pub fn status_lines(readings: &[Reading]) -> Vec<String> {
    let mut lines: Vec<(&str, Vec<String>)> = Vec::new();
    for r in readings.iter().filter(|r| !has_value(r)) {
        let status = match r.status {
            Finding::NotStated => "Not stated",
            Finding::SourceMissing => "No document",
            Finding::NoSource => "No such source",
            Finding::Unreadable => "Unreadable",
            Finding::NotApplicable => "Not applicable",
            Finding::Unread => "Not read",
            Finding::Found | Finding::Ambiguous | Finding::Inconsistent => continue,
        };
        let item = format!("{} ({})", r.name, source_label(&r.source).to_lowercase());
        match lines.iter_mut().find(|(s, _)| *s == status) {
            Some((_, items)) => items.push(item),
            None => lines.push((status, vec![item])),
        }
    }
    lines
        .into_iter()
        .map(|(s, items)| format!("{s}: {}", items.join(", ")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(homes: u32, known: u32, affordable: u32) -> Site {
        Site {
            borough: "Alpha".into(),
            reference: "1".into(),
            site: None,
            status: Status::Approved,
            year: 2020,
            homes,
            affordable,
            known,
            completed: 0,
            completion_date: None,
            updated: "2026-01-02".into(),
            unlocated: false,
            lat: 51.5,
            lon: -0.1,
            readings: Vec::new(),
            retrieval: Vec::new(),
        }
    }

    #[test]
    fn share_bins_have_even_edges() {
        assert_eq!(share_bin(0.0), 0);
        assert_eq!(share_bin(0.0999), 0);
        assert_eq!(share_bin(0.1), 1);
        assert_eq!(share_bin(0.35), 3, "no edge at the 35% threshold");
        assert_eq!(share_bin(0.4), 4);
        assert_eq!(share_bin(1.0), 4);
    }

    #[test]
    fn share_is_of_recorded_tenure() {
        // 10 of the 20 homes with a recorded tenure, though the site has 100 homes.
        assert_eq!(site(100, 20, 10).affordable_share(), Some(0.5));
        assert_eq!(site(100, 0, 0).affordable_share(), None);
        assert_eq!(colour(&site(100, 0, 0), Colour::Affordable), UNKNOWN);
        assert_eq!(colour(&site(100, 100, 40), Colour::Affordable), SHARE[4].0);
    }

    #[test]
    fn legends_name_every_colour() {
        assert_eq!(legend(Colour::Affordable).len(), SHARE.len() + 1);
        let status = legend(Colour::Status);
        assert_eq!(status.len(), STATUSES.len());
        assert!(status.iter().any(|&(_, t)| t == "Superseded"));
    }

    #[test]
    fn numbers_read_as_labelled() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(553_662), "553,662");
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(percent(1, 3), "33.3%");
        assert_eq!(percent(5, 0), "n/a");
    }

    fn reading(source: &str, variable: &str, status: Finding) -> Reading {
        Reading {
            stage: Lineage::Decision,
            variable: variable.into(),
            name: format!("{variable} name"),
            source: source.into(),
            status,
            value: None,
            basis: None,
            pages: None,
            check: Check::Proposed,
        }
    }

    #[test]
    fn statuses_without_a_value_share_a_line() {
        let r = [
            reading("deed", "V1", Finding::Found),
            reading("gla", "V1", Finding::SourceMissing),
            reading("deed", "V5", Finding::NotStated),
            reading("committee_report", "V5", Finding::NotStated),
            reading("datahub", "V2", Finding::Unread),
        ];
        assert!(has_value(&r[0]));
        assert_eq!(
            status_lines(&r),
            [
                "No document: V1 name (gla)",
                "Not stated: V5 name (deed), V5 name (committee report)",
                "Not read: V2 name (datahub)"
            ]
        );
        assert_eq!(check_label(Check::Unread), "not read");
        // Every status in the codebook (labels.rs STATUSES) reads.
        for s in ["no_source", "unreadable", "source_missing", ""] {
            assert!(Finding::from_ledger(s).is_some(), "{s}");
        }
    }

    #[test]
    fn a_variation_changes_figures_only_when_they_differ() {
        let cell = |source: &str, variable: &str, value: &str, check: Check| Reading {
            value: Some(value.into()),
            check,
            ..reading(source, variable, Finding::Found)
        };
        // 09/AP/1870's shape: the variation repeats the deed's total and lists dates.
        let same = [
            cell("deed", "V1", "total=668; SR=123; SO=47", Check::Confirmed),
            cell("variation", "V1", "total=668", Check::Proposed),
            cell(
                "variation",
                "V7",
                "orig 2010-02-17; DoV 2010-12-17",
                Check::Proposed,
            ),
            cell("variation", "V2", "INT -> market: all", Check::Proposed),
        ];
        assert_eq!(variation_changes(&same), None);
        // 19/AP/1710's shape: intermediate homes go to 0.
        let mut changed = vec![
            cell("deed", "V1", "total=262; SR=64; INT=28", Check::Confirmed),
            cell(
                "variation",
                "V1",
                "total=292; SR=64; INT=0",
                Check::Proposed,
            ),
        ];
        assert_eq!(
            variation_changes(&changed),
            Some(false),
            "rests on an unverified reading"
        );
        changed[1].check = Check::Confirmed;
        assert_eq!(variation_changes(&changed), Some(true));
    }

    #[test]
    fn checked_cells_look_checked() {
        assert_eq!(check_class(Check::Unread), "unchecked");
        assert_eq!(check_class(Check::Proposed), "unchecked");
        assert_eq!(check_class(Check::Estimated), "checked");
        assert_eq!(check_class(Check::Confirmed), "independent");
        assert_eq!(check_tag(Check::Proposed), "unverified reading");
        assert_eq!(check_tag(Check::Confirmed), "confirmed");
        assert!(Check::Proposed < Check::Estimated && Check::Estimated < Check::Confirmed);
        let mut r = vec![reading("deed", "V1", Finding::Found); 3];
        r[0].check = Check::Confirmed;
        r[1].check = Check::Unread;
        assert_eq!(
            check_counts(&r),
            "3 cells: 1 confirmed, 1 unverified, 1 not read"
        );
    }

    #[test]
    fn values_read_as_words() {
        assert_eq!(
            humanize("total=84; market=61; SR=16; SO=7"),
            "total homes 84; market homes 61; social rent 16; shared ownership 7"
        );
        assert_eq!(
            humanize("orig 2010-02-17; s73 10/AP/2734; DoV 2014-02-04"),
            "original agreement dated 17 February 2010; section 73 permission 10/AP/2734; \
             deed of variation dated 4 February 2014"
        );
        assert_eq!(
            humanize("orig 2021-11-?? (day illegible)"),
            "original agreement dated 2021-11-?? (day illegible)"
        );
        assert_eq!(
            humanize("cascade=no; pil=yes; pil_amount=not_stated"),
            "tenure cascade: no; payment in lieu: yes; payment in lieu amount: not stated"
        );
        assert_eq!(
            humanize("route=viability_tested; reviews=early OR route=fast_track; reviews=early"),
            "either viability route: viability tested; viability reviews: early stage review, \
             or viability route: Fast Track; viability reviews: early stage review"
        );
        assert_eq!(
            humanize("share=35.0%; split=64:25 SR:SO (habitable rooms)"),
            "affordable share 35.0%; tenure split 64:25 social rent to shared ownership (habitable rooms)"
        );
        assert_eq!(
            humanize("SR: target rents; SO: income <=£90,000"),
            "social rent: target rents; shared ownership: income ≤£90,000"
        );
        assert_eq!(
            humanize("INT -> market: all"),
            "intermediate -> market: all"
        );
        assert_eq!(
            basis_note("share 35.0% (habitable rooms)", Some("habitable_rooms")),
            None
        );
        assert_eq!(
            basis_note("total 84", Some("units")).as_deref(),
            Some(" (units)")
        );
        assert_eq!(basis_note("total 84", Some("not_stated")), None);
        assert_eq!(
            basis_note("pil 1", Some("other:cash")).as_deref(),
            Some(" (other: cash)")
        );
    }

    #[test]
    fn target_and_offer_split() {
        assert_eq!(
            split_target("target=35% habitable_rooms; first_offer=30%; offer_doc=x"),
            (
                Some("target=35% habitable_rooms".into()),
                Some("first_offer=30%; offer_doc=x".into())
            )
        );
        assert_eq!(
            split_target("first_offer=30%"),
            (None, Some("first_offer=30%".into()))
        );
        assert_eq!(
            split_target("target=35%; first_offer=a OR target=30%; first_offer=b"),
            (
                Some("target=35% OR target=30%".into()),
                Some("first_offer=a OR first_offer=b".into())
            )
        );
    }

    #[test]
    fn missing_documents_in_words() {
        let r = Retrieval {
            source: "viability".into(),
            documents: 13,
            in_hand: 0,
            not_in_hand: Some("listed_not_fetched x13".into()),
        };
        assert_eq!(
            not_in_hand(&r).as_deref(),
            Some("Viability: 13 listed, not fetched")
        );
        assert_eq!(retrieval_stage("viability"), Some(Lineage::Offer));
        assert_eq!(retrieval_stage("datahub"), None);
    }

    #[test]
    fn small_sites_share_the_smallest_dot() {
        assert_eq!(radius(10), radius(30));
        assert!(radius(1000) > radius(250));
    }
}
