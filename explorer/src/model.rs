//! The explorer's pure logic: colours, bins, sizes and number formats. No browser types, so the
//! tests run on the host with `cargo test`.

use crate::site::{Site, Status};

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
            lat: 51.5,
            lon: -0.1,
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

    #[test]
    fn small_sites_share_the_smallest_dot() {
        assert_eq!(radius(10), radius(30));
        assert!(radius(1000) > radius(250));
    }
}
