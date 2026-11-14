//! Read-only tier placement for `cort memory list` (issue #11, phase 0).
//!
//! Nothing here touches the database: a tier is derived from `recorded_at`, the tiers epoch and
//! the clock, so the report can size the proposal before any schema exists.

use crate::memory::MemoryRecall;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

const NS_PER_SEC: i128 = 1_000_000_000;
pub(crate) const DAY_NS: i128 = 86_400 * NS_PER_SEC;
pub(crate) const SHORT_TERM_NS: i128 = 7 * DAY_NS;
pub(crate) const FORGETTING_NS: i128 = 60 * DAY_NS;
const CLAIM_CHARS: usize = 160;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Short,
    Long,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Short => "short",
            Self::Long => "long",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub tier: Tier,
    pub expires_at_ns: Option<i128>,
    pub short_term_expired: bool,
    pub forgetting_due_at_ns: Option<i128>,
}

/// `recorded_at` as ns, or `None` when it is not an integer or lies outside the UTC-day range in
/// which a date can be printed. The one guard shared by `build` and the lifecycle fold.
pub(crate) fn parse_recorded_ns(raw: &str) -> Option<i128> {
    raw.parse::<i128>()
        .ok()
        .filter(|ns| (-719_528..=2_932_896).contains(&ns.div_euclid(DAY_NS)))
}

/// The spec's read-time rule: recorded before `epoch - 7d` is long-term (forgetting review due
/// 60 days after `recorded_at`); otherwise short-term, expiring at `max(recorded_at, epoch) + 7d`.
/// An expired short-term entry stays short-term -- it waits for review, it never ages into long.
pub fn place(recorded_at_ns: i128, epoch_ns: i128, now_ns: i128) -> Placement {
    if recorded_at_ns < epoch_ns.saturating_sub(SHORT_TERM_NS) {
        return Placement {
            tier: Tier::Long,
            expires_at_ns: None,
            short_term_expired: false,
            forgetting_due_at_ns: Some(recorded_at_ns.saturating_add(FORGETTING_NS)),
        };
    }
    let expires = recorded_at_ns.max(epoch_ns).saturating_add(SHORT_TERM_NS);
    Placement {
        tier: Tier::Short,
        expires_at_ns: Some(expires),
        short_term_expired: now_ns >= expires,
        forgetting_due_at_ns: None,
    }
}

/// `user` (ASCII case-insensitive, as a substring), `用戶`, the simplified `用户`, or `使用者`
/// in the key or the claim.
pub fn mentions_user(key: &str, claim: &str) -> bool {
    [key, claim].iter().any(|text| {
        text.to_ascii_lowercase().contains("user")
            || text.contains("用戶")
            || text.contains("用户")
            || text.contains("使用者")
    })
}

/// `YYYY-MM-DDTHH:MM:SSZ` for a nanosecond instant: the form `recorded_at_utc` uses.
pub fn utc_iso(ns: i128) -> String {
    utc_parts(ns).1
}

/// `YYYY-MM-DD` -> midnight in this machine's local zone (ns). Refuses anything else.
pub fn parse_epoch(raw: &str) -> Result<i128, String> {
    let invalid = || "memory_list_epoch_invalid: use --epoch YYYY-MM-DD".to_string();
    let b = raw.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return Err(invalid());
    }
    let num = |range: std::ops::Range<usize>| -> Result<u32, String> {
        let part = raw.get(range).ok_or_else(invalid)?;
        if part.bytes().all(|c| c.is_ascii_digit()) {
            part.parse().map_err(|_| invalid())
        } else {
            Err(invalid())
        }
    };
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return Err(invalid());
    }
    let unresolved = || {
        "memory_list_epoch_unresolved: local time zone unavailable; pass --epoch YYYY-MM-DD"
            .to_string()
    };
    let ns = crate::remind::local_midnight(y as i32, m, d).map_err(|_| unresolved())?;
    // `mktime` normalises 2026-02-31 to March, and a day whose local midnight does not exist
    // (a DST gap) lands on 01:00; a date that does not round-trip to 00:00 is refused.
    let (date, time) = crate::remind_snapshot::local_parts(ns).ok_or_else(unresolved)?;
    if date != raw || time != "00:00" {
        return Err(invalid());
    }
    Ok(ns)
}

/// The local calendar date of `ns` as `YYYY-MM-DD`, or `None` when the zone cannot be resolved.
fn local_date_of(ns: i128) -> Option<String> {
    crate::remind_snapshot::local_parts(ns).map(|(date, _)| date)
}

/// Today's local midnight as `(YYYY-MM-DD, ns)`; refused when the zone cannot be resolved.
pub fn default_epoch(now_ns: i128) -> Result<(String, i128), String> {
    let date = local_date_of(now_ns).ok_or_else(|| {
        "memory_list_epoch_unresolved: local time zone unavailable; pass --epoch YYYY-MM-DD"
            .to_string()
    })?;
    let ns = parse_epoch(&date)?;
    Ok((date, ns))
}

fn utc_parts(ns: i128) -> (String, String) {
    let seconds = ns.div_euclid(NS_PER_SEC);
    let days = i64::try_from(seconds.div_euclid(86_400)).unwrap_or(0);
    let time = seconds.rem_euclid(86_400);
    let (y, m, d) = crate::measure::civil_from_days(days);
    (
        format!("{y:04}-{m:02}-{d:02}"),
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
            time / 3600,
            time % 3600 / 60,
            time % 60
        ),
    )
}

/// The Monday (UTC) of the week containing `ns`.
fn week_monday(ns: i128) -> String {
    let days = i64::try_from(ns.div_euclid(DAY_NS)).unwrap_or(0);
    // 1970-01-01 was a Thursday.
    let monday = days - (days + 3).rem_euclid(7);
    let (y, m, d) = crate::measure::civil_from_days(monday);
    format!("{y:04}-{m:02}-{d:02}")
}

struct Row {
    id: String,
    key: String,
    claim: String,
    /// `None` when `recorded_at` is unparsable or outside the representable UTC range.
    recorded_at_ns: Option<i128>,
    age_days: Option<i128>,
    placement: Option<Placement>,
    mentions_user: bool,
    evidence_status: String,
}

impl Row {
    fn tier_is(&self, tier: Tier) -> bool {
        self.placement.is_some_and(|p| p.tier == tier)
    }
}

pub struct Report {
    project_id: String,
    epoch: String,
    epoch_ns: i128,
    unparsed_recorded_at: usize,
    rows: Vec<Row>,
}

/// Place every hit of an all-active recall (`memory_store::recall_all(root, "", true)`, hidden entries included).
pub fn build(
    project_id: &str,
    epoch: &str,
    epoch_ns: i128,
    now_ns: i128,
    recall: &MemoryRecall,
) -> Report {
    let mut unparsed = 0;
    let mut rows: Vec<Row> = recall
        .hits
        .iter()
        .map(|hit| {
            // Same UTC-day guard as `memory.rs`: outside it the date cannot be printed.
            let recorded = parse_recorded_ns(&hit.entry.recorded_at);
            if recorded.is_none() {
                unparsed += 1;
            }
            Row {
                id: hit.entry.id.clone(),
                key: hit.entry.key.clone(),
                claim: hit.entry.claim.clone(),
                recorded_at_ns: recorded,
                age_days: recorded.map(|r| (now_ns - r).div_euclid(DAY_NS).max(0)),
                placement: recorded.map(|r| place(r, epoch_ns, now_ns)),
                mentions_user: mentions_user(&hit.entry.key, &hit.entry.claim),
                evidence_status: hit.evidence_status.clone(),
            }
        })
        .collect();
    // Oldest first; rows with an unknown time go last (`None` would sort before every `Some`).
    rows.sort_by(|a, b| {
        a.recorded_at_ns
            .is_none()
            .cmp(&b.recorded_at_ns.is_none())
            .then(a.recorded_at_ns.cmp(&b.recorded_at_ns))
            .then(a.id.cmp(&b.id))
    });
    Report {
        project_id: project_id.to_string(),
        epoch: epoch.to_string(),
        epoch_ns,
        unparsed_recorded_at: unparsed,
        rows,
    }
}

impl Report {
    fn count(&self, pred: impl Fn(&Row) -> bool) -> usize {
        self.rows.iter().filter(|r| pred(r)).count()
    }

    pub fn to_json(&self) -> Value {
        let mut by_week = BTreeMap::<String, usize>::new();
        let mut by_evidence = BTreeMap::<String, usize>::new();
        for row in &self.rows {
            if let Some(due) = row.placement.and_then(|p| p.forgetting_due_at_ns) {
                *by_week.entry(week_monday(due)).or_default() += 1;
            }
            *by_evidence.entry(row.evidence_status.clone()).or_default() += 1;
        }
        let mut summary = json!({
            "active": self.rows.len(),
            "short": self.count(|r| r.tier_is(Tier::Short)),
            "long": self.count(|r| r.tier_is(Tier::Long)),
            "short_term_expired": self.count(|r| r.placement.is_some_and(|p| p.short_term_expired)),
            "mentions_user": self.count(|r| r.mentions_user),
            "forgetting_due_by_week": by_week,
            "by_evidence": by_evidence,
        });
        if self.unparsed_recorded_at > 0 {
            summary["unparsed_recorded_at"] = json!(self.unparsed_recorded_at);
        }
        let entries: Vec<Value> = self
            .rows
            .iter()
            .map(|row| {
                let mut e = Map::new();
                e.insert("id".into(), json!(row.id));
                e.insert("key".into(), json!(row.key));
                if let Some(ns) = row.recorded_at_ns {
                    e.insert("recorded_at_utc".into(), json!(utc_parts(ns).1));
                }
                if let Some(age) = row.age_days {
                    e.insert("age_days".into(), json!(age));
                }
                e.insert("tier".into(), json!(tier_name(row.placement)));
                if let Some(ns) = row.placement.and_then(|p| p.expires_at_ns) {
                    e.insert("expires_at_utc".into(), json!(utc_parts(ns).1));
                }
                if let Some(ns) = row.placement.and_then(|p| p.forgetting_due_at_ns) {
                    e.insert("forgetting_due_at_utc".into(), json!(utc_parts(ns).1));
                }
                e.insert("mentions_user".into(), json!(row.mentions_user));
                e.insert("evidence_status".into(), json!(row.evidence_status));
                e.insert("claim".into(), json!(row.claim));
                Value::Object(e)
            })
            .collect();
        json!({
            "project_id": self.project_id,
            "epoch": self.epoch,
            "epoch_utc": utc_parts(self.epoch_ns).1,
            "summary": summary,
            "entries": entries,
        })
    }

    /// Header line, then one TSV row per entry; no field is ever empty (`-` stands in), because
    /// tab is IFS whitespace and `read` would collapse an empty field away.
    pub fn lean(&self) -> String {
        let mut out = format!(
            "active={} short={} long={} short_term_expired={} mentions_user={} epoch={} epoch_utc={}",
            self.rows.len(),
            self.count(|r| r.tier_is(Tier::Short)),
            self.count(|r| r.tier_is(Tier::Long)),
            self.count(|r| r.placement.is_some_and(|p| p.short_term_expired)),
            self.count(|r| r.mentions_user),
            self.epoch,
            utc_parts(self.epoch_ns).1
        );
        if self.unparsed_recorded_at > 0 {
            out.push_str(&format!(
                " unparsed_recorded_at={}",
                self.unparsed_recorded_at
            ));
        }
        out.push('\n');
        for row in &self.rows {
            let id8: String = row.id.chars().take(8).collect();
            let fields = [
                lean_field(&id8),
                row.recorded_at_ns
                    .map_or_else(|| "-".to_string(), |ns| utc_parts(ns).0),
                row.age_days
                    .map_or_else(|| "-".to_string(), |a| a.to_string()),
                tier_name(row.placement).to_string(),
                lean_field(&row.evidence_status),
                if row.mentions_user { "y" } else { "n" }.to_string(),
                lean_field(&row.key),
                lean_field(&cut_claim(&row.claim)),
            ];
            out.push_str(&fields.join("\t"));
            out.push('\n');
        }
        out
    }
}

fn tier_name(placement: Option<Placement>) -> &'static str {
    placement.map_or("unknown", |p| p.tier.as_str())
}

fn cut_claim(claim: &str) -> String {
    if claim.chars().count() > CLAIM_CHARS {
        let head: String = claim.chars().take(CLAIM_CHARS).collect();
        format!("{head}…")
    } else {
        claim.to_string()
    }
}

fn lean_field(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_control() || c == '\u{2028}' || c == '\u{2029}' {
                ' '
            } else {
                c
            }
        })
        .collect();
    if cleaned.trim().is_empty() {
        "-".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPOCH: i128 = 1_000_000 * NS_PER_SEC;

    #[test]
    fn place_boundaries() {
        let now = EPOCH + 100 * DAY_NS;
        // Exactly epoch - 7d is NOT "before" epoch - 7d: short, expiring at epoch + 7d.
        let at = place(EPOCH - SHORT_TERM_NS, EPOCH, now);
        assert_eq!(at.tier, Tier::Short);
        assert_eq!(at.expires_at_ns, Some(EPOCH + SHORT_TERM_NS));
        assert!(at.short_term_expired);
        assert_eq!(at.forgetting_due_at_ns, None);
        // One second earlier is long, forgetting due recorded + 60d.
        let rec = EPOCH - SHORT_TERM_NS - NS_PER_SEC;
        let before = place(rec, EPOCH, now);
        assert_eq!(before.tier, Tier::Long);
        assert_eq!(before.expires_at_ns, None);
        assert!(!before.short_term_expired);
        assert_eq!(before.forgetting_due_at_ns, Some(rec + 60 * DAY_NS));
        // One second later is short.
        assert_eq!(
            place(EPOCH - SHORT_TERM_NS + NS_PER_SEC, EPOCH, now).tier,
            Tier::Short
        );
        // Recorded after the epoch: expires at recorded + 7d; expired exactly at that instant.
        let rec = EPOCH + DAY_NS;
        let exp = rec + SHORT_TERM_NS;
        let fresh = place(rec, EPOCH, exp - NS_PER_SEC);
        assert_eq!(fresh.expires_at_ns, Some(exp));
        assert!(!fresh.short_term_expired);
        assert!(place(rec, EPOCH, exp).short_term_expired);
    }

    #[test]
    fn mentions_user_cases() {
        assert!(mentions_user("k", "the User chose A"));
        assert!(mentions_user("USERS", "x"));
        assert!(mentions_user("k", "用戶 2026-10-28"));
        assert!(mentions_user("k", "使用者說"));
        assert!(mentions_user("使用者", "x"));
        assert!(!mentions_user("deploy", "Option B is current"));
        assert!(mentions_user("k", "用户"));
    }

    #[test]
    fn week_monday_is_a_monday() {
        // 2026-11-14 is a Saturday; its Monday is 2026-11-09.
        assert_eq!(week_monday(1_794_618_000 * NS_PER_SEC), "2026-11-09");
        // A Monday maps to itself, and the Sunday before it to the previous Monday.
        assert_eq!(week_monday(1_793_577_600 * NS_PER_SEC), "2026-11-02");
        assert_eq!(week_monday(1_793_577_599 * NS_PER_SEC), "2026-10-26");
    }

    #[test]
    fn lean_field_never_empty() {
        assert_eq!(lean_field(""), "-");
        assert_eq!(lean_field(" \t\n"), "-");
        assert_eq!(lean_field("a\tb\nc\rd"), "a b c d");
        assert_eq!(lean_field("a\u{2028}b\u{2029}c"), "a b c");
        assert_eq!(lean_field("\u{2028}\u{2029}"), "-");
    }

    #[test]
    fn parse_epoch_rejects_malformed_dates() {
        for bad in [
            "2026-02-31",
            "2026-1-01",
            "2026-13-01",
            "abcd-ef-gh",
            "２０２６-11-14",
            "é2026-11-1",
        ] {
            let err = parse_epoch(bad).unwrap_err();
            assert!(err.starts_with("memory_list_epoch_invalid"), "{bad}: {err}");
        }
    }

    #[test]
    fn parse_epoch_round_trips_local_date() {
        let ns = parse_epoch("2026-11-14").unwrap();
        assert_eq!(local_date_of(ns), Some("2026-11-14".to_string()));
    }

    #[test]
    fn default_epoch_is_todays_local_midnight() {
        let now = 1_794_650_000 * NS_PER_SEC;
        let (_, ns) = default_epoch(now).unwrap();
        assert!(ns <= now);
        assert!(now - ns < 25 * 3_600 * NS_PER_SEC);
    }

    #[test]
    fn place_never_overflows_at_either_extreme() {
        let p = place(i128::MIN + 1, i128::MIN + 2, 0);
        assert_eq!(p.tier, Tier::Short);
        let long = place(i128::MIN, i128::MAX, 0);
        assert_eq!(long.tier, Tier::Long);
        let near_max = place(i128::MAX - 1, i128::MAX, 0);
        assert_eq!(near_max.tier, Tier::Short);
        // epoch - 7d saturates at i128::MIN rather than wrapping.
        assert_eq!(place(0, i128::MIN, 0).tier, Tier::Short);
        let far = place(i128::MAX - 8 * DAY_NS, i128::MAX, 0);
        assert_eq!(far.tier, Tier::Long);
        assert_eq!(far.forgetting_due_at_ns, Some(i128::MAX));
    }

    #[test]
    fn place_saturates_at_the_extreme() {
        let p = place(i128::MAX, EPOCH, EPOCH);
        assert_eq!(p.expires_at_ns, Some(i128::MAX));
    }
}
