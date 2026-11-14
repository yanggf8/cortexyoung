//! Read-only tier placement for `cort memory list` (issue #11, phase 0).
//!
//! Nothing here touches the database: a tier is derived from `recorded_at`, the tiers epoch and
//! the clock, so the report can size the proposal before any schema exists.

use crate::memory::MemoryRecall;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

const NS_PER_SEC: i128 = 1_000_000_000;
const DAY_NS: i128 = 86_400 * NS_PER_SEC;
const SHORT_TERM_NS: i128 = 7 * DAY_NS;
const FORGETTING_NS: i128 = 60 * DAY_NS;
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

/// The spec's read-time rule: recorded before `epoch - 7d` is long-term (forgetting review due
/// 60 days after `recorded_at`); otherwise short-term, expiring at `max(recorded_at, epoch) + 7d`.
/// An expired short-term entry stays short-term -- it waits for review, it never ages into long.
pub fn place(recorded_at_ns: i128, epoch_ns: i128, now_ns: i128) -> Placement {
    if recorded_at_ns < epoch_ns - SHORT_TERM_NS {
        return Placement {
            tier: Tier::Long,
            expires_at_ns: None,
            short_term_expired: false,
            forgetting_due_at_ns: Some(recorded_at_ns + FORGETTING_NS),
        };
    }
    let expires = recorded_at_ns.max(epoch_ns) + SHORT_TERM_NS;
    Placement {
        tier: Tier::Short,
        expires_at_ns: Some(expires),
        short_term_expired: now_ns >= expires,
        forgetting_due_at_ns: None,
    }
}

/// `user` (ASCII case-insensitive, as a substring), `用戶` or `使用者` in the key or the claim.
pub fn mentions_user(key: &str, claim: &str) -> bool {
    [key, claim].iter().any(|text| {
        text.to_ascii_lowercase().contains("user")
            || text.contains("用戶")
            || text.contains("使用者")
    })
}

/// `YYYY-MM-DD` -> midnight in this machine's local zone (ns). Refuses anything else.
pub fn parse_epoch(raw: &str) -> Result<i128, String> {
    let invalid = || "memory_list_epoch_invalid: use --epoch YYYY-MM-DD".to_string();
    let b = raw.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return Err(invalid());
    }
    let num = |range: std::ops::Range<usize>| -> Result<u32, String> {
        let part = &raw[range];
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
    let ns = crate::remind::local_midnight(y as i32, m, d)?;
    // `mktime` normalises 2026-02-31 to March; a date that does not round-trip is refused.
    if local_date_of(ns) != Some(raw.to_string()) {
        return Err(invalid());
    }
    Ok(ns)
}

/// The local calendar date of `ns` as `YYYY-MM-DD`, or `None` when the zone cannot be resolved.
fn local_date_of(ns: i128) -> Option<String> {
    let stamp = crate::remind_snapshot::local_stamp(ns);
    let date = stamp.get(..10)?;
    (stamp.len() == 16).then(|| date.to_string())
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
    recorded_at_ns: i128,
    age_days: i128,
    placement: Placement,
    mentions_user: bool,
    evidence_status: String,
}

pub struct Report {
    project_id: String,
    epoch: String,
    unparsed_recorded_at: usize,
    rows: Vec<Row>,
}

/// Place every hit of an all-active recall (`memory_store::recall(root, "")`).
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
            let recorded = hit.entry.recorded_at.parse::<i128>().unwrap_or_else(|_| {
                unparsed += 1;
                0
            });
            Row {
                id: hit.entry.id.clone(),
                key: hit.entry.key.clone(),
                claim: hit.entry.claim.clone(),
                recorded_at_ns: recorded,
                age_days: (now_ns - recorded).div_euclid(DAY_NS).max(0),
                placement: place(recorded, epoch_ns, now_ns),
                mentions_user: mentions_user(&hit.entry.key, &hit.entry.claim),
                evidence_status: hit.evidence_status.clone(),
            }
        })
        .collect();
    rows.sort_by(|a, b| {
        a.recorded_at_ns
            .cmp(&b.recorded_at_ns)
            .then(a.id.cmp(&b.id))
    });
    Report {
        project_id: project_id.to_string(),
        epoch: epoch.to_string(),
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
            if let Some(due) = row.placement.forgetting_due_at_ns {
                *by_week.entry(week_monday(due)).or_default() += 1;
            }
            *by_evidence.entry(row.evidence_status.clone()).or_default() += 1;
        }
        let mut summary = json!({
            "active": self.rows.len(),
            "short": self.count(|r| r.placement.tier == Tier::Short),
            "long": self.count(|r| r.placement.tier == Tier::Long),
            "short_term_expired": self.count(|r| r.placement.short_term_expired),
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
                e.insert(
                    "recorded_at_utc".into(),
                    json!(utc_parts(row.recorded_at_ns).1),
                );
                e.insert("age_days".into(), json!(row.age_days));
                e.insert("tier".into(), json!(row.placement.tier.as_str()));
                if let Some(ns) = row.placement.expires_at_ns {
                    e.insert("expires_at_utc".into(), json!(utc_parts(ns).1));
                }
                if let Some(ns) = row.placement.forgetting_due_at_ns {
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
            "summary": summary,
            "entries": entries,
        })
    }

    /// Header line, then one TSV row per entry; no field is ever empty (`-` stands in), because
    /// tab is IFS whitespace and `read` would collapse an empty field away.
    pub fn lean(&self) -> String {
        let mut out = format!(
            "active={} short={} long={} short_term_expired={} mentions_user={} epoch={}\n",
            self.rows.len(),
            self.count(|r| r.placement.tier == Tier::Short),
            self.count(|r| r.placement.tier == Tier::Long),
            self.count(|r| r.placement.short_term_expired),
            self.count(|r| r.mentions_user),
            self.epoch
        );
        for row in &self.rows {
            let id8: String = row.id.chars().take(8).collect();
            let fields = [
                lean_field(&id8),
                utc_parts(row.recorded_at_ns).0,
                row.age_days.to_string(),
                row.placement.tier.as_str().to_string(),
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
        .map(|c| if c.is_control() { ' ' } else { c })
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
        assert!(!mentions_user("k", "用户"));
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
    }
}
